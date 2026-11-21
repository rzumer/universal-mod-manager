//! Thunderstore packages for a game set up as "BepInEx from Thunderstore": search, install
//! with dependencies, update. Packages are stored as ordinary mods, laid out for the game.

use crate::{CoreError, Result, Session};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use umm_deploy::GameLock;
use umm_gamedef::GameDef;
use umm_store::{fsutil, ImportOptions, Layout, ProfileEntry, ThunderstoreMeta};
use umm_thunderstore::ecosystem::{package_index_url, SteamMatch};
use umm_thunderstore::index::{compare_versions, split_dependency, Package};
use umm_thunderstore::{layout, resolve, Ecosystem};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TsSort {
    /// Pinned first, then rating (Thunderstore's own order).
    #[default]
    Rating,
    Downloads,
    Updated,
    Name,
}

#[derive(Debug, Clone, Default)]
pub struct TsQuery {
    pub text: String,
    pub category: Option<String>,
    pub sort: TsSort,
    pub include_deprecated: bool,
    pub include_nsfw: bool,
    pub installed_only: bool,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct TsItem {
    pub package: Package,
    /// The installed version, if any.
    pub installed: Option<String>,
    pub update: bool,
}

#[derive(Debug, Clone)]
pub struct TsPage {
    pub total: usize,
    pub items: Vec<TsItem>,
    /// Every category in the community, by name.
    pub categories: Vec<String>,
    /// The package list is an old copy because Thunderstore could not be reached.
    pub stale: bool,
    /// The community's BepInEx pack, `Owner-Name`.
    pub loader_package: Option<String>,
    /// A BepInEx pack is installed for the game.
    pub loader_installed: bool,
    /// The installed pack cannot load in this game (built for the other architecture).
    pub loader_warning: Option<String>,
    /// A known pack for the game's build to offer instead (`Owner-Name`).
    pub loader_suggestion: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TsInstallReport {
    /// `Owner-Name`, version, and the version it replaced.
    pub installed: Vec<(String, String, Option<String>)>,
    /// Dependencies Thunderstore no longer lists, as `Owner-Name-x.y.z`.
    pub missing: Vec<String>,
    /// Problems to show, such as a BepInEx pack built for the other architecture.
    pub warnings: Vec<String>,
    /// Other changes made along the way, such as an old BepInEx pack turned off.
    pub notes: Vec<String>,
}

/// A package looked up by name.
#[derive(Debug, Clone)]
pub struct TsLookup {
    pub package: Package,
    /// The game's community lists it.
    pub listed: bool,
    /// When not listed: how the community's moderators reviewed it (`rejected`,
    /// `unreviewed`), or `None` if it was never listed there.
    pub review_status: Option<String>,
}

/// A package installed for a game: its mod id and Thunderstore details.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub mod_id: String,
    pub meta: ThunderstoreMeta,
}

pub(crate) struct TsCache {
    pub(crate) packages: HashMap<String, (Arc<Vec<Package>>, bool)>,
}

impl TsCache {
    pub(crate) fn new() -> Self {
        TsCache {
            packages: HashMap::new(),
        }
    }
}

/// Mod managers published on Thunderstore. They are not mods for the game, so search leaves
/// them out unless the Tools category is picked.
const MOD_MANAGERS: &[&str] = &["ebkr-r2modman", "Kesomannen-GaleModManager"];

/// r2modman or Gale. Only these two, by name: categories and descriptions cannot tell mod
/// managers from mods reliably (real mods and libraries are tagged "Tools" too).
pub fn is_mod_manager(p: &Package) -> bool {
    MOD_MANAGERS.iter().any(|m| m.eq_ignore_ascii_case(&p.full_name))
}

/// Mod id for a package: its `Owner-Name`, as a slug.
pub fn package_mod_id(full_name: &str) -> String {
    umm_gamedef::slug(full_name).unwrap_or_else(|| "package".into())
}

impl Session {
    /// The game's community and Thunderstore data; refused unless the game is set up for
    /// BepInEx from Thunderstore.
    fn ts_game(&self, id: &str) -> Result<(GameDef, String, Arc<Ecosystem>)> {
        let def = self.def(id)?.def;
        let community = match (&def.loader, def.community()) {
            (Some(l), Some(c)) if l.from_thunderstore() => c.to_string(),
            _ => {
                return Err(CoreError::Invalid(format!(
                    "{} is not set up for BepInEx from Thunderstore; change its mod loader first",
                    def.name
                )))
            }
        };
        let eco = self.ecosystem(false)?;
        if eco.community(&community).is_none() {
            return Err(CoreError::Invalid(format!(
                "Thunderstore has no community `{community}`"
            )));
        }
        Ok((def, community, eco))
    }

    fn ts_match<'a>(eco: &'a Ecosystem, community: &str) -> SteamMatch<'a> {
        eco.community(community).expect("checked by ts_game")
    }

    /// The community's packages, cached for an hour (`refresh` downloads them again).
    pub fn ts_packages(&self, id: &str, refresh: bool) -> Result<(Arc<Vec<Package>>, bool)> {
        let (_, community, eco) = self.ts_game(id)?;
        let mut cache = self.ts.lock().unwrap_or_else(|e| e.into_inner());
        if !refresh {
            if let Some(hit) = cache.packages.get(&community) {
                let file = self.ts_index_file(&community);
                let young = umm_thunderstore::cache::age(&file)
                    .is_some_and(|a| a <= umm_thunderstore::index::MAX_AGE);
                if young || hit.1 {
                    return Ok(hit.clone());
                }
            }
        }
        let url = package_index_url(&Self::ts_match(&eco, &community));
        let (list, fresh) = umm_thunderstore::index::load(
            self.http(),
            &url,
            &self.ts_index_file(&community),
            refresh,
        )?;
        let stale = fresh == umm_thunderstore::cache::Freshness::Stale;
        let entry = (Arc::new(list), stale);
        cache.packages.insert(community, entry.clone());
        Ok(entry)
    }

    fn ts_index_file(&self, community: &str) -> std::path::PathBuf {
        self.thunderstore_dir()
            .join("communities")
            .join(format!("{community}.json.gz"))
    }

    /// Thunderstore packages installed for the game, by `Owner-Name`.
    pub fn ts_installed(&self, id: &str) -> Result<Vec<InstalledPackage>> {
        Ok(self
            .mods(id)?
            .into_iter()
            .filter_map(|m| {
                m.thunderstore.map(|meta| InstalledPackage {
                    mod_id: m.id,
                    meta,
                })
            })
            .collect())
    }

    pub(crate) fn installed_versions(&self, id: &str) -> Result<HashMap<String, String>> {
        Ok(self
            .ts_installed(id)?
            .into_iter()
            .map(|p| (p.meta.full_name, p.meta.version))
            .collect())
    }

    pub fn ts_search(&self, id: &str, q: &TsQuery) -> Result<TsPage> {
        let (_, _, eco) = self.ts_game(id)?;
        let (packages, stale) = self.ts_packages(id, false)?;
        let installed_pkgs = self.ts_installed(id)?;
        let loader_installed = installed_pkgs.iter().any(|p| p.meta.loader);
        let installed: HashMap<String, String> = installed_pkgs
            .into_iter()
            .map(|p| (p.meta.full_name, p.meta.version))
            .collect();
        // The community's BepInEx pack for this build of the game. An IL2CPP game never gets
        // the plain Mono pack, a Mono game never an IL2CPP one (game-specific packs like
        // BepInExPack_AmongUs do not say which they are). Among the rest, BepInEx's own pack
        // if listed, else the most downloaded.
        let il2cpp = self
            .install_path(&self.def(id)?.def)?
            .is_some_and(|(p, _)| p.join("GameAssembly.dll").is_file());
        let loader_package = packages
            .iter()
            .filter(|p| eco.modloader(&p.full_name).is_some_and(|m| m.loader == "bepinex"))
            .filter(|p| !p.deprecated)
            .filter(|p| {
                if il2cpp {
                    !p.full_name.eq_ignore_ascii_case("BepInEx-BepInExPack")
                } else {
                    !p.full_name.to_lowercase().contains("il2cpp")
                }
            })
            .max_by_key(|p| {
                let own = p.owner == "BepInEx" && p.name.starts_with("BepInExPack");
                (own, p.downloads)
            })
            .map(|p| p.full_name.clone());
        let have = |p: &Package| {
            installed
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&p.full_name))
                .map(|(_, v)| v.clone())
        };
        let text = q.text.trim().to_lowercase();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut hits: Vec<&Package> = packages
            .iter()
            .filter(|p| q.include_deprecated || !p.deprecated || have(p).is_some())
            .filter(|p| q.include_nsfw || !p.nsfw)
            .filter(|p| q.category.as_deref() == Some("Tools") || !is_mod_manager(p))
            .filter(|p| !q.installed_only || have(p).is_some())
            .filter(|p| {
                q.category
                    .as_ref()
                    .is_none_or(|c| p.categories.iter().any(|x| x == c))
            })
            .filter(|p| {
                words.is_empty() || {
                    let hay = format!("{} {} {}", p.name, p.owner, p.description).to_lowercase();
                    words.iter().all(|w| hay.contains(w))
                }
            })
            .collect();
        match q.sort {
            TsSort::Rating => hits.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.rating.cmp(&a.rating))),
            TsSort::Downloads => hits.sort_by(|a, b| b.downloads.cmp(&a.downloads)),
            TsSort::Updated => hits.sort_by(|a, b| b.updated.cmp(&a.updated)),
            TsSort::Name => hits.sort_by_key(|p| p.name.to_lowercase()),
        }
        let mut categories: Vec<String> = packages
            .iter()
            .flat_map(|p| p.categories.iter().cloned())
            .collect();
        categories.sort();
        categories.dedup();
        let limit = if q.limit == 0 { 50 } else { q.limit };
        let items = hits
            .iter()
            .skip(q.offset)
            .take(limit)
            .map(|p| {
                let installed = have(p);
                let update = match (&installed, p.latest()) {
                    (Some(i), Some(l)) => compare_versions(&l.version, i) == Ordering::Greater,
                    _ => false,
                };
                TsItem {
                    package: (*p).clone(),
                    installed,
                    update,
                }
            })
            .collect();
        let mismatch = self.loader_mismatch(id);
        Ok(TsPage {
            total: hits.len(),
            items,
            categories,
            stale,
            loader_package,
            loader_installed,
            loader_warning: mismatch.as_ref().map(|m| m.message.clone()),
            loader_suggestion: mismatch.and_then(|m| m.suggestion),
        })
    }

    /// Installed packages with a newer version: `(Owner-Name, installed, latest)`.
    pub fn ts_updates(&self, id: &str) -> Result<Vec<(String, String, String)>> {
        let (packages, _) = self.ts_packages(id, false)?;
        Ok(resolve::updates(&packages, &self.installed_versions(id)?))
    }

    /// Installs packages (version `None` = latest) with their dependencies, and makes sure all
    /// of them are enabled in `profile`. Refused while mods are deployed.
    pub fn ts_install(
        &self,
        id: &str,
        wanted: &[(String, Option<String>)],
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        self.install_with(id, wanted, profile, progress, None)
    }

    /// Looks up a package by `Owner-Name`, including one the game's community does not list
    /// (not submitted there, unreviewed, or rejected by its moderators).
    pub fn ts_lookup(&self, id: &str, full_name: &str) -> Result<TsLookup> {
        let (_, community, _) = self.ts_game(id)?;
        let (packages, _) = self.ts_packages(id, false)?;
        if let Some(p) = packages
            .iter()
            .find(|p| p.full_name.eq_ignore_ascii_case(full_name.trim()))
        {
            return Ok(TsLookup {
                package: p.clone(),
                listed: true,
                review_status: None,
            });
        }
        let f = umm_thunderstore::index::fetch_package(self.http(), full_name.trim())?;
        let review_status = f
            .listings
            .iter()
            .find(|(c, _)| c == &community)
            .map(|(_, s)| s.clone());
        Ok(TsLookup {
            package: f.package,
            listed: false,
            review_status,
        })
    }

    /// Installs a package the game's community does not list, by `Owner-Name` (version
    /// `None` = latest), with its dependencies. The caller has shown the user that it is not
    /// listed and why ([`Session::ts_lookup`]).
    pub fn ts_install_unlisted(
        &self,
        id: &str,
        full_name: &str,
        version: Option<String>,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let mut found = self.ts_lookup(id, full_name)?;
        if found.listed {
            return self.ts_install(id, &[(found.package.full_name, version)], profile, progress);
        }
        if let Some(v) = &version {
            if found.package.version(v).is_none() {
                found.package.versions.push(umm_thunderstore::index::PackageVersion {
                    version: v.clone(),
                    dependencies: Vec::new(),
                    size: 0,
                });
            }
        }
        let wanted = [(found.package.full_name.clone(), version)];
        self.install_with(id, &wanted, profile, progress, Some(found.package))
    }

    fn install_with(
        &self,
        id: &str,
        wanted: &[(String, Option<String>)],
        profile: &str,
        progress: &mut dyn FnMut(&str),
        extra: Option<Package>,
    ) -> Result<TsInstallReport> {
        let (_, community, eco) = self.ts_game(id)?;
        let m = Self::ts_match(&eco, &community);
        let (listed, _) = self.ts_packages(id, false)?;
        // A package from outside the community's list joins it for this install only.
        let combined: Vec<Package>;
        let packages: &[Package] = match extra {
            Some(p) => {
                combined = listed.iter().cloned().chain(std::iter::once(p)).collect();
                &combined
            }
            None => &listed,
        };
        let game = self.data.game(id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        let installed = self.installed_versions(id)?;
        let installed_pkgs = self.ts_installed(id)?;
        let recorded: HashMap<String, (String, Vec<String>)> = installed_pkgs
            .iter()
            .map(|p| {
                let m = &p.meta;
                (m.full_name.to_lowercase(), (m.version.clone(), m.dependencies.clone()))
            })
            .collect();

        // One BepInEx pack per game. A pack asked for by name, else one already installed, is
        // the game's pack; a dependency on any other pack counts as met by it (mods often name
        // a game's original pack, which may not suit the game any more).
        let is_loader = |name: &str| eco.modloader(name).is_some();
        let in_profile: Vec<String> = umm_store::load_profile(&game, profile)?
            .mods
            .into_iter()
            .filter(|e| e.enabled)
            .map(|e| e.id)
            .collect();
        let chosen_loader: Option<String> = wanted
            .iter()
            .map(|(n, _)| n.clone())
            .find(|n| is_loader(n))
            .or_else(|| {
                // The pack this profile uses, else any installed one.
                let loaders = || installed_pkgs.iter().filter(|p| p.meta.loader);
                loaders()
                    .find(|p| in_profile.contains(&p.mod_id))
                    .or_else(|| loaders().next())
                    .map(|p| p.meta.full_name.clone())
            });
        let other_loader = |dep: &str| {
            split_dependency(dep).is_some_and(|(n, _)| {
                is_loader(n)
                    && chosen_loader
                        .as_deref()
                        .is_some_and(|c| !c.eq_ignore_ascii_case(n))
            })
        };

        // Dependencies come from each package's manifest: an installed package's record, or
        // the zip, downloaded once and kept for the install below.
        let mut zips: HashMap<(String, String), Vec<u8>> = HashMap::new();
        let mut net_error: Option<CoreError> = None;
        let plan = {
            let mut deps_of = |pkg: &Package, version: &str| -> std::result::Result<Vec<String>, String> {
                if let Some((have, deps)) = recorded.get(&pkg.full_name.to_lowercase()) {
                    if compare_versions(have, version) != Ordering::Less {
                        return Ok(deps.iter().filter(|d| !other_loader(d)).cloned().collect());
                    }
                }
                let key = (pkg.full_name.clone(), version.to_string());
                if !zips.contains_key(&key) {
                    progress(&format!("Downloading {} {}", pkg.name, version));
                    let bytes = self.http().get(&pkg.download_url(version)).map_err(|e| {
                        let msg = e.to_string();
                        net_error = Some(e.into());
                        msg
                    })?;
                    zips.insert(key.clone(), bytes);
                }
                let manifest = layout::manifest(&zips[&key]).map_err(|e| e.to_string())?;
                let deps = manifest.map(|m| m.dependencies).unwrap_or_default();
                Ok(deps.into_iter().filter(|d| !other_loader(d)).collect())
            };
            resolve::resolve(packages, &installed, wanted, &mut deps_of)
        };
        let plan = match (plan, net_error) {
            (Ok(p), _) => p,
            (Err(_), Some(e)) => return Err(e),
            (Err(msg), None) => return Err(CoreError::Invalid(msg)),
        };
        let mut report = TsInstallReport {
            missing: plan.missing.clone(),
            ..Default::default()
        };
        for step in &plan.steps {
            let pkg = packages
                .iter()
                .find(|p| p.full_name == step.full_name)
                .expect("resolved from this list");
            let key = (step.full_name.clone(), step.version.clone());
            let zip = match zips.remove(&key) {
                Some(z) => z,
                None => {
                    progress(&format!("Downloading {} {}", pkg.name, step.version));
                    self.http().get(&pkg.download_url(&step.version))?
                }
            };
            progress(&format!("Installing {} {}", pkg.name, step.version));
            self.store_package(id, &community, m, &eco, pkg, &step.version, &zip)?;
            report
                .installed
                .push((step.full_name.clone(), step.version.clone(), step.replaces.clone()));
        }

        let mut p = umm_store::load_profile(&game, profile)?;
        let mut changed = false;
        // A pack installed by name replaces any other pack in this profile: turned off, kept.
        if let Some(chosen) = wanted.iter().map(|(n, _)| n).find(|n| is_loader(n)) {
            for other in installed_pkgs
                .iter()
                .filter(|o| o.meta.loader && !o.meta.full_name.eq_ignore_ascii_case(chosen))
            {
                if let Some(e) = p.mods.iter_mut().find(|e| e.id == other.mod_id && e.enabled) {
                    e.enabled = false;
                    changed = true;
                    report.notes.push(format!(
                        "Turned off {} in this profile; {} replaces it",
                        other.meta.full_name, chosen
                    ));
                }
            }
        }
        for name in &plan.all {
            let mod_id = package_mod_id(name);
            match p.mods.iter_mut().find(|e| e.id == mod_id) {
                Some(e) if !e.enabled => {
                    e.enabled = true;
                    changed = true;
                }
                Some(_) => {}
                None => {
                    p.mods.push(ProfileEntry {
                        id: mod_id,
                        enabled: true,
                    });
                    changed = true;
                }
            }
        }
        if changed {
            umm_store::save_profile(&game, &p)?;
        }
        report.warnings.extend(self.loader_architecture_warning(id));
        Ok(report)
    }

    /// Updates installed packages (all of them when `only` is empty) to their latest versions,
    /// with any new dependencies. Profiles keep their order.
    pub fn ts_update(
        &self,
        id: &str,
        only: &[String],
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<TsInstallReport> {
        let wanted: Vec<(String, Option<String>)> = self
            .ts_updates(id)?
            .into_iter()
            .filter(|(n, _, _)| only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(n)))
            .map(|(n, _, latest)| (n, Some(latest)))
            .collect();
        if wanted.is_empty() {
            return Ok(TsInstallReport::default());
        }
        self.ts_install(id, &wanted, profile, progress)
    }

    /// Names of installed packages that depend on the mod `mod_id`.
    pub fn ts_dependents(&self, id: &str, mod_id: &str) -> Result<Vec<String>> {
        let installed = self.ts_installed(id)?;
        let Some(target) = installed.iter().find(|p| p.mod_id == mod_id) else {
            return Ok(Vec::new());
        };
        let prefix = format!("{}-", target.meta.full_name.to_lowercase());
        Ok(installed
            .iter()
            .filter(|p| p.mod_id != mod_id)
            .filter(|p| {
                p.meta
                    .dependencies
                    .iter()
                    .any(|d| d.to_lowercase().starts_with(&prefix))
            })
            .map(|p| p.meta.full_name.clone())
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    fn store_package(
        &self,
        id: &str,
        community: &str,
        m: SteamMatch,
        eco: &Ecosystem,
        pkg: &Package,
        version: &str,
        zip: &[u8],
    ) -> Result<()> {
        let game = self.data.game(id);
        let mod_id = package_mod_id(&pkg.full_name);
        if let Ok(existing) = umm_store::load_mod(&game, &mod_id) {
            let same = existing
                .thunderstore
                .as_ref()
                .is_some_and(|t| t.full_name.eq_ignore_ascii_case(&pkg.full_name));
            if !same {
                return Err(CoreError::Invalid(format!(
                    "a mod with the id `{mod_id}` is already stored and is not {}; remove or rename it first",
                    pkg.full_name
                )));
            }
        }
        let manifest = layout::manifest(zip)?.unwrap_or_default();
        let loader = eco.modloader(&pkg.full_name);
        let files = layout::plan(zip, &pkg.full_name, &m.r2.install_rules, loader)?;
        let staging = game.staging_dir().join(fsutil::unique_suffix());
        let result = (|| -> Result<()> {
            layout::write(&files, &staging)?;
            umm_store::import_mod(
                &game,
                &staging,
                &ImportOptions {
                    id: Some(mod_id.clone()),
                    name: Some(pkg.name.replace('_', " ")),
                    version: Some(version.to_string()),
                    source: Some(format!(
                        "https://thunderstore.io/c/{community}/p/{}/{}/",
                        pkg.owner, pkg.name
                    )),
                    unwrap: false,
                    keep_top_dirs: Vec::new(),
                    layout: Layout::Game,
                    thunderstore: Some(ThunderstoreMeta {
                        community: community.to_string(),
                        full_name: pkg.full_name.clone(),
                        version: version.to_string(),
                        dependencies: manifest.dependencies.clone(),
                        description: Some(pkg.description.clone()).filter(|d| !d.is_empty()),
                        website: Some(pkg.website.clone()).filter(|w| !w.is_empty()),
                        loader: loader.is_some(),
                    }),
                    replace: true,
                },
            )?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    }
}
