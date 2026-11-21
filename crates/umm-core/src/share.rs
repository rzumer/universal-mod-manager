//! Sharing profiles with r2modman: `.r2z` files and Thunderstore profile codes.

use crate::ts::{package_mod_id, TsInstallReport};
use crate::{CoreError, Result, Session};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use umm_gamedef::RelPath;
use umm_store::fsutil::IoCtx;
use umm_thunderstore::index::compare_versions;
use umm_thunderstore::profile::{self, Export, ExportMod};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ProfileExport {
    /// The `.r2z` zip.
    pub r2z: Vec<u8>,
    pub mods: usize,
    pub files: Vec<String>,
    /// Mods in the profile that are not Thunderstore packages, so are not in the export.
    pub left_out: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProfileImport {
    /// The profile created for the import.
    pub profile: String,
    pub installed: TsInstallReport,
    /// Packages the export names that Thunderstore no longer has, or versions it no longer
    /// has (the latest was installed instead).
    pub notes: Vec<String>,
    pub files: usize,
    /// Files in the export that were not copied (unsafe types or paths, or outside BepInEx).
    pub skipped: Vec<String>,
}

impl Session {
    /// Builds an r2modman export of the profile: its Thunderstore packages and the config
    /// files in its runtime folder (configs appear there once the game has run).
    pub fn profile_export(&self, id: &str, profile_name: &str) -> Result<ProfileExport> {
        let game = self.data.game(id);
        let p = umm_store::load_profile(&game, profile_name)?;
        let mut mods = Vec::new();
        let mut left_out = Vec::new();
        for e in &p.mods {
            match umm_store::load_mod(&game, &e.id) {
                Ok(m) => match m.thunderstore {
                    Some(t) => mods.push(ExportMod {
                        name: t.full_name,
                        version: t.version,
                        enabled: e.enabled,
                    }),
                    None => left_out.push(m.name),
                },
                Err(_) => left_out.push(e.id.clone()),
            }
        }
        if mods.is_empty() {
            return Err(CoreError::Invalid(
                "the profile has no Thunderstore packages to export".into(),
            ));
        }

        let rt = game.runtime_dir(profile_name);
        let mut files = BTreeMap::new();
        if rt.is_dir() {
            for entry in WalkDir::new(&rt).follow_links(false).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(&rt) else { continue };
                let Ok(rel) = RelPath::from_path(rel) else { continue };
                let path = rel.as_str();
                let lower = path.to_lowercase();
                let in_config = ["bepinex/config/", "shimloader/cfg/"]
                    .iter()
                    .any(|d| lower.starts_with(d));
                // As r2modman: all of the loader's config folder, other config-type files,
                // but not package manifests, the .NET runtime or UMM's own bookkeeping.
                let plugin_manifest = (lower.starts_with("bepinex/plugins/")
                    || lower.starts_with("shimloader/"))
                    && lower.ends_with("/manifest.json");
                let excluded = plugin_manifest
                    || ["dotnet/", ".umm"].iter().any(|d| lower.starts_with(d));
                let wanted = in_config || (profile::is_config_file(path) && !excluded);
                if wanted {
                    let body = fs::read(entry.path()).ctx(|| format!("reading {}", entry.path().display()))?;
                    files.insert(path.to_string(), body);
                }
            }
        }
        let export = Export {
            profile_name: profile_name.to_string(),
            mods,
            files,
            skipped: Vec::new(),
        };
        Ok(ProfileExport {
            r2z: profile::build_r2z(&export)?,
            mods: export.mods.len(),
            files: export.files.keys().cloned().collect(),
            left_out,
        })
    }

    /// Uploads the profile's export to Thunderstore and returns its code. This publishes the
    /// mod list and configs; the interface asks first.
    pub fn profile_export_code(&self, id: &str, profile_name: &str) -> Result<String> {
        let e = self.profile_export(id, profile_name)?;
        Ok(profile::upload(self.http(), &e.r2z)?)
    }

    /// Imports a profile code into a new profile.
    pub fn profile_import_code(
        &self,
        id: &str,
        code: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<ProfileImport> {
        progress("Downloading the profile");
        let r2z = profile::download(self.http(), code)?;
        self.profile_import(id, &r2z, progress)
    }

    /// Imports an `.r2z` file into a new profile.
    pub fn profile_import_file(
        &self,
        id: &str,
        path: &Path,
        progress: &mut dyn FnMut(&str),
    ) -> Result<ProfileImport> {
        let r2z = fs::read(path).ctx(|| format!("reading {}", path.display()))?;
        self.profile_import(id, &r2z, progress)
    }

    /// Creates a profile named after the export, installs its packages at their versions,
    /// sets which are enabled, and copies its config files into the profile's runtime folder.
    pub fn profile_import(
        &self,
        id: &str,
        r2z: &[u8],
        progress: &mut dyn FnMut(&str),
    ) -> Result<ProfileImport> {
        let export = profile::read_r2z(r2z)?;
        let (packages, _) = self.ts_packages(id, false)?;

        // A game keeps one version of each package, shared by its profiles, so an import never
        // downgrades one: a newer installed version is kept, and the summary says so.
        let installed = self.installed_versions(id)?;
        let mut wanted = Vec::new();
        let mut notes = Vec::new();
        for m in &export.mods {
            let Some(p) = packages
                .iter()
                .find(|p| p.full_name.eq_ignore_ascii_case(&m.name))
            else {
                notes.push(format!("{} is not on Thunderstore for this game", m.name));
                continue;
            };
            let have = installed
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&p.full_name))
                .map(|(_, v)| v.clone())
                .filter(|h| p.version(h).is_some());
            let listed = p.version(&m.version).is_some();
            match have {
                Some(h) if !listed || compare_versions(&h, &m.version) == Ordering::Greater => {
                    notes.push(format!(
                        "Kept {} {h} (the profile had {})",
                        p.name.replace('_', " "),
                        m.version
                    ));
                    wanted.push((p.full_name.clone(), Some(h)));
                }
                _ if listed => wanted.push((p.full_name.clone(), Some(m.version.clone()))),
                _ => {
                    notes.push(format!(
                        "{} {} is no longer on Thunderstore; installed the latest instead",
                        m.name, m.version
                    ));
                    wanted.push((p.full_name.clone(), None));
                }
            }
        }

        let name = self.free_profile_name(id, &export.profile_name);
        self.create_profile(id, &name, None)?;
        let installed = match self.ts_install(id, &wanted, &name, progress) {
            Ok(r) => r,
            Err(e) => {
                let _ = self.delete_profile(id, &name);
                return Err(e);
            }
        };

        // ts_install enables everything it touches; apply the export's choices.
        let game = self.data.game(id);
        let mut p = umm_store::load_profile(&game, &name)?;
        for m in &export.mods {
            let mod_id = package_mod_id(&m.name);
            if let Some(e) = p.mods.iter_mut().find(|e| e.id == mod_id) {
                e.enabled = m.enabled;
            }
        }
        umm_store::save_profile(&game, &p)?;

        // Configs go in the runtime folder, where the loader reads them; the game folder only
        // ever gets what a deployment puts there.
        progress("Copying configs");
        let rt = game.runtime_dir(&name);
        let dirs = self
            .def(id)?
            .def
            .runtime_loader()
            .map_or(&[][..], |rl| rl.runtime_dirs());
        let mut skipped = export.skipped.clone();
        let mut copied = 0;
        for (path, body) in &export.files {
            let first = path.split('/').next().unwrap_or_default();
            if !dirs.iter().any(|d| first.eq_ignore_ascii_case(d)) {
                skipped.push(path.clone());
                continue;
            }
            let dest = RelPath::parse(path)?.to_path(&rt);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&dest, body).ctx(|| format!("writing {}", dest.display()))?;
            copied += 1;
        }

        Ok(ProfileImport {
            profile: name,
            installed,
            notes,
            files: copied,
            skipped,
        })
    }

    /// `name`, or `name (2)`, `name (3)`... if taken. Falls back to "Imported" for a name that
    /// cannot be a file name.
    fn free_profile_name(&self, id: &str, name: &str) -> String {
        let base = match RelPath::parse(name.trim()) {
            Ok(p) if !p.is_root() && !name.contains('/') => name.trim().to_string(),
            _ => "Imported".to_string(),
        };
        let taken = self.profiles(id);
        let is_taken = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
        if !is_taken(&base) {
            return base;
        }
        (2..)
            .map(|i| format!("{base} ({i})"))
            .find(|n| !is_taken(n))
            .expect("some name is free")
    }
}
