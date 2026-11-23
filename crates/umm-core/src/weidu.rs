//! WeiDU mods, for Infinity Engine games (Baldur's Gate: Enhanced Edition and its kin). Such a
//! mod is an installer, `setup-<mod>.exe`, with the mod's folder: run in the game folder, it
//! asks which components to install and patches the game's files (`override/`, the dialog
//! tables in `lang/`), noting what it did in `WeiDU.log`. Each mod builds on what those before
//! it changed, so a profile's WeiDU mods install together, in the profile's order.
//!
//! UMM runs them on a copy of the game: a folder inside the game folder (`.umm-weidu`), where
//! the large files WeiDU only reads (the `data/` archives, movies, music) are linked rather than
//! copied. The first time, each installer runs in a console of its own, and the user picks the
//! components; UMM reads them back from `WeiDU.log` and passes them the next times
//! (`--force-install-list`). What the installers changed or added is kept, and while mods are
//! deployed it is in the game folder, through the journal, so the game comes back as it was.
//! The run is redone by itself when the game's files, the profile's WeiDU mods or their order
//! change.

use crate::patcher::PatchedFile;
use crate::{CoreError, Result, Session};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use umm_deploy::{GameLock, Plan, PlannedFile};
use umm_gamedef::{CompiledDef, RelPath, IN_GAME_BACKUP_DIR};
use umm_store::fsutil::{hash_file, unix_now, write_atomic, IoCtx};
use umm_store::{GameDir, ModMeta, Profile};

/// In the game folder: the copy the installers run on, removed afterwards.
pub const WEIDU_SCRATCH: &str = ".umm-weidu";
/// In the game folder: where an installer lists its components, removed afterwards.
const WEIDU_LIST: &str = ".umm-weidu-list";
/// Game files not linked into the listing folder, when linking is not possible, are copied
/// up to this size (archives are left out: listing does not read them).
const LIST_COPY_LIMIT: u64 = 16 * 1024 * 1024;
/// Under the game's generated folder: one folder per profile.
const WEIDU: &str = "weidu";
const RECORD: &str = "weidu.json";
const OUT: &str = "files";
/// An installer may wait for the user's answers.
const TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);
/// Large folders WeiDU reads but never writes: linked into the copy, not copied.
const LINKED_DIRS: &[&str] = &["data", "movies", "music", "manuals"];

/// Whether a file (as placed in the game folder) is a WeiDU installer: `setup-<mod>.exe` at
/// the top.
pub fn is_weidu_setup(path: &str) -> bool {
    let p = path.to_lowercase();
    !p.contains('/') && p.starts_with("setup-") && p.ends_with(".exe")
}

/// Where a WeiDU mod's files sit inside it, from its `.tp2` (`<mod>/<mod>.tp2`,
/// `<mod>/setup-<mod>.tp2` or `setup-<mod>.tp2`): the folder holding its installer and its
/// mod folder, which goes to the game folder as it is (a placement chosen for it does not
/// apply). Some mods come without their installer, `setup-<mod>.exe`, which is WeiDU itself
/// renamed: UMM then supplies WeiDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeiduLayout {
    /// The folder inside the mod (`/`-separated, as spelled there; empty for its top).
    pub root: String,
    /// `setup-<mod>.exe`, lower-cased.
    pub setup: String,
    /// The mod ships its installer.
    pub has_setup: bool,
    /// Its `.tp2`, relative to `root` (as spelled in the mod).
    pub tp2: String,
}

pub fn weidu_layout<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<WeiduLayout> {
    let files: Vec<&str> = files.into_iter().collect();
    let mut best: Option<(usize, String, String, String)> = None;
    for f in &files {
        let lower = f.to_lowercase();
        if !lower.ends_with(".tp2") {
            continue;
        }
        let comps: Vec<&str> = lower.split('/').collect();
        let n = comps.len();
        let stem = comps[n - 1].trim_end_matches(".tp2");
        let (depth, name) =
            if n >= 2 && (stem == comps[n - 2] || stem == format!("setup-{}", comps[n - 2])) {
                (n - 2, comps[n - 2].to_string())
            } else if let Some(name) = stem.strip_prefix("setup-") {
                (n - 1, name.to_string())
            } else {
                continue;
            };
        if best.as_ref().is_none_or(|(d, _, _, _)| depth < *d) {
            let root = f.split('/').take(depth).collect::<Vec<_>>().join("/");
            let tp2 = f.split('/').skip(depth).collect::<Vec<_>>().join("/");
            best = Some((depth, root, name, tp2));
        }
    }
    let (_, root, name, tp2) = best?;
    let setup = format!("setup-{name}.exe");
    let full = if root.is_empty() {
        setup.clone()
    } else {
        format!("{}/{setup}", root.to_lowercase())
    };
    let has_setup = files.iter().any(|f| f.to_lowercase() == full);
    Some(WeiduLayout {
        root,
        setup,
        has_setup,
        tp2,
    })
}

/// A mod's WeiDU installer, for showing: `None` when the mod is not a WeiDU mod.
pub fn weidu_setup_of(meta: &ModMeta) -> Option<String> {
    weidu_layout(meta.files.keys().map(String::as_str)).map(|l| l.setup)
}

/// The mods with components chosen in the record, in the profile's order: those an install
/// covers.
fn chosen(mods: &[(ModMeta, String)], r: &Record) -> Vec<(ModMeta, String)> {
    mods.iter()
        .filter(|(m, _)| r.choices.iter().any(|c| c.mod_id == m.id))
        .cloned()
        .collect()
}

/// Where UMM keeps WeiDU for mods that come without their installer.
fn weidu_tool(data_root: &Path) -> PathBuf {
    data_root.join("tools").join("weidu.exe")
}

/// The mod an installer is for: `setup-<mod>.exe` → `<mod>`, lower-cased.
fn mod_key(setup: &str) -> String {
    let s = setup.to_lowercase();
    s.trim_start_matches("setup-")
        .trim_end_matches(".exe")
        .to_string()
}

/// A WeiDU mod's components, as chosen.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeiduChoice {
    pub mod_id: String,
    /// The installer, `setup-<mod>.exe`.
    pub setup: String,
    pub language: Option<u32>,
    /// The components installed, with their names from `WeiDU.log`.
    pub components: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    key: String,
    /// Every WeiDU mod's components, kept when it leaves the profile (it may come back).
    #[serde(default)]
    choices: Vec<WeiduChoice>,
    #[serde(default)]
    files: Vec<PatchedFile>,
    #[serde(default)]
    output: String,
    #[serde(default)]
    ran_unix: u64,
    #[serde(default)]
    error: Option<String>,
    /// The mods the last successful install included.
    #[serde(default)]
    installed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeiduState {
    /// Every mod has its components chosen; not installed yet.
    NotRun,
    Current,
    /// The game, the mods or their order changed: installed again at the next launch.
    Outdated,
    /// A mod has no components chosen yet: run from the Mods tab.
    NeedsChoices,
    Failed,
}

#[derive(Debug, Clone)]
pub struct WeiduStatus {
    pub state: WeiduState,
    /// A mod comes without its installer, and UMM has no WeiDU to stand in for it.
    pub needs_weidu: bool,
    /// The profile's WeiDU mods, in order, with their components.
    pub mods: Vec<WeiduChoice>,
    pub files: usize,
    pub output: String,
    pub error: Option<String>,
    pub ran_unix: u64,
    /// The mods the last successful install included: the others are not in the game yet.
    pub installed: Vec<String>,
}

fn record_dir(game: &GameDir, profile: &str) -> PathBuf {
    game.generated_dir().join(WEIDU).join(profile)
}

/// Forgets a deleted mod in every profile's WeiDU install: its components, and the install
/// itself, which held what it changed (installed again without it at the next launch; gone
/// when it was the profile's only WeiDU mod).
pub(crate) fn forget_mod(game: &GameDir, mod_id: &str) {
    let Ok(rd) = fs::read_dir(game.generated_dir().join(WEIDU)) else {
        return;
    };
    for e in rd.flatten() {
        let profile = e.file_name().to_string_lossy().into_owned();
        let mut r = load_record(game, &profile);
        if !r.choices.iter().any(|c| c.mod_id == mod_id) {
            continue;
        }
        r.choices.retain(|c| c.mod_id != mod_id);
        r.installed.retain(|m| m != mod_id);
        if r.choices.is_empty() {
            let _ = fs::remove_dir_all(e.path());
        } else {
            r.key.clear();
            let _ = save_record(game, &profile, &r);
        }
    }
}

fn load_record(game: &GameDir, profile: &str) -> Record {
    fs::read(record_dir(game, profile).join(RECORD))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_record(game: &GameDir, profile: &str, r: &Record) -> Result<()> {
    let p = record_dir(game, profile).join(RECORD);
    let text = serde_json::to_vec_pretty(r).map_err(|e| CoreError::Invalid(e.to_string()))?;
    write_atomic(&p, &text).ctx(|| format!("writing {}", p.display()))?;
    Ok(())
}

fn mtime(m: &fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64)
}

/// Every file in a folder (`/`-separated, relative to it) with its size and time, skipping
/// UMM's own folders.
fn snapshot(root: &Path) -> HashMap<String, (u64, u64)> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.depth() == 1
                && (n.eq_ignore_ascii_case(WEIDU_SCRATCH)
                    || n.eq_ignore_ascii_case(WEIDU_LIST)
                    || n.eq_ignore_ascii_case(IN_GAME_BACKUP_DIR)))
        })
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = RelPath::from_path(e.path().strip_prefix(root).ok()?).ok()?;
            let m = e.metadata().ok()?;
            Some((rel.as_str().to_string(), (m.len(), mtime(&m))))
        })
        .collect()
}

/// The components `WeiDU.log` lists for an installer's mod (`~MOD/MOD.TP2~ #0 #3 // Name`),
/// with the language.
fn logged(log: &str, setup: &str) -> (Option<u32>, Vec<(u32, String)>) {
    let want = mod_key(setup);
    let mut language = None;
    let mut out = Vec::new();
    for line in log.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('~') else {
            continue;
        };
        let Some((tp2, rest)) = rest.split_once('~') else {
            continue;
        };
        let file = tp2.rsplit(['/', '\\']).next().unwrap_or(tp2).to_lowercase();
        let name = file.trim_end_matches(".tp2").trim_start_matches("setup-");
        if name != want {
            continue;
        }
        let (nums, comment) = rest.split_once("//").unwrap_or((rest, ""));
        let mut it = nums
            .split_whitespace()
            .filter_map(|t| t.strip_prefix('#')?.parse::<u32>().ok());
        let (Some(lang), Some(comp)) = (it.next(), it.next()) else {
            continue;
        };
        language = Some(lang);
        out.retain(|(c, _)| *c != comp);
        out.push((comp, comment.trim().to_string()));
    }
    (language, out)
}

impl Session {
    /// The profile's WeiDU mods, in its order: each with its installer.
    fn weidu_mods(&self, def: &CompiledDef, profile: &Profile) -> Vec<(ModMeta, String)> {
        let game = self.data.game(&def.def.id);
        profile
            .enabled_ids()
            .filter_map(|id| umm_store::load_mod(&game, id).ok())
            .filter_map(|m| {
                let setup = weidu_layout(m.files.keys().map(String::as_str))?.setup;
                Some((m, setup))
            })
            .collect()
    }

    /// A WeiDU mod's files where they go in the game folder (its layout's root removed),
    /// with their stored copies and hashes.
    fn weidu_files(&self, def: &CompiledDef, meta: &ModMeta) -> Vec<(RelPath, PathBuf, String)> {
        let Some(layout) = weidu_layout(meta.files.keys().map(String::as_str)) else {
            return Vec::new();
        };
        let files_root = self.data.game(&def.def.id).mod_files_dir(&meta.id);
        let n = layout.root.split('/').filter(|s| !s.is_empty()).count();
        meta.files
            .iter()
            .filter_map(|(file, rec)| {
                let rel = RelPath::parse(file).ok()?;
                let inside = rel
                    .components()
                    .zip(layout.root.split('/').filter(|s| !s.is_empty()))
                    .all(|(a, b)| a.eq_ignore_ascii_case(b));
                (inside && rel.depth() > n).then(|| {
                    (
                        rel.strip_components(n),
                        rel.to_path(&files_root),
                        rec.hash.clone(),
                    )
                })
            })
            .collect()
    }

    /// WeiDU itself, for mods that come without their installer: the copy the user gave UMM,
    /// else another WeiDU mod's installer (each is WeiDU renamed).
    fn weidu_exe(&self, def: &CompiledDef) -> Option<PathBuf> {
        let tool = weidu_tool(self.data.root());
        if tool.is_file() {
            return Some(tool);
        }
        let game = self.data.game(&def.def.id);
        umm_store::list_mods(&game).ok()?.into_iter().find_map(|m| {
            let layout = weidu_layout(m.files.keys().map(String::as_str))?;
            if !layout.has_setup {
                return None;
            }
            self.weidu_files(def, &m)
                .into_iter()
                .find(|(t, _, _)| t.as_str().eq_ignore_ascii_case(&layout.setup))
                .map(|(_, src, _)| src)
        })
    }

    /// Keeps a copy of WeiDU (`weidu.exe`, from weidu.org), for mods that come without their
    /// installer.
    pub fn set_weidu_exe(&self, path: &Path) -> Result<()> {
        let ok = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            && path.is_file();
        if !ok {
            return Err(CoreError::Invalid(format!(
                "{} is not a program",
                path.display()
            )));
        }
        let tool = weidu_tool(self.data.root());
        if let Some(p) = tool.parent() {
            fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
        }
        fs::copy(path, &tool).ctx(|| format!("copying {}", path.display()))?;
        Ok(())
    }

    /// What the installers' run depends on: the game files they read (the key file, the
    /// dialog tables), the mods in order with their files, and the components chosen.
    fn weidu_key(
        &self,
        def: &CompiledDef,
        root: &Path,
        mods: &[(ModMeta, String)],
        choices: &[WeiduChoice],
    ) -> String {
        let mut key = String::new();
        let mut game: Vec<(String, (u64, u64))> = snapshot(root)
            .into_iter()
            .filter(|(rel, _)| {
                let r = rel.to_lowercase();
                r == "chitin.key" || (r.starts_with("lang/") && r.ends_with(".tlk"))
            })
            .collect();
        game.sort();
        for (rel, (len, t)) in game {
            key.push_str(&format!("game {rel} {len} {t}\n"));
        }
        for (m, setup) in mods {
            let mut files: Vec<(String, String)> = self
                .weidu_files(def, m)
                .into_iter()
                .map(|(t, _, h)| (t.as_str().to_string(), h))
                .collect();
            files.sort();
            key.push_str(&format!("mod {} {setup}\n", m.id));
            for (t, h) in files {
                key.push_str(&format!("  {t} {h}\n"));
            }
            if let Some(c) = choices.iter().find(|c| c.mod_id == m.id) {
                key.push_str(&format!(
                    "  choice {:?} {:?}\n",
                    c.language,
                    c.components.iter().map(|(n, _)| n).collect::<Vec<_>>()
                ));
            }
        }
        key
    }

    /// How the profile's WeiDU mods stand, or `None` when it has none.
    pub fn weidu_status(&self, game_id: &str, profile: &str) -> Result<Option<WeiduStatus>> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        let p = umm_store::load_profile(&game, profile)?;
        let mods = self.weidu_mods(&def, &p);
        if mods.is_empty() {
            return Ok(None);
        }
        let r = load_record(&game, profile);
        let choices: Vec<WeiduChoice> = mods
            .iter()
            .map(|(m, setup)| {
                r.choices
                    .iter()
                    .find(|c| c.mod_id == m.id)
                    .cloned()
                    .unwrap_or(WeiduChoice {
                        mod_id: m.id.clone(),
                        setup: setup.clone(),
                        language: None,
                        components: Vec::new(),
                    })
            })
            .collect();
        let deployed = game.journal_path().exists();
        // The install covers the mods with components chosen; the others wait, left out.
        let chosen_mods = chosen(&mods, &r);
        let state = if chosen_mods.is_empty() {
            WeiduState::NeedsChoices
        } else if r.ran_unix == 0 {
            WeiduState::NotRun
        } else if r.error.is_some() {
            WeiduState::Failed
        } else if deployed {
            WeiduState::Current
        } else {
            match self.install_path(&def.def)? {
                Some((root, _))
                    if self.weidu_key(&def, &root, &chosen_mods, &r.choices) != r.key =>
                {
                    WeiduState::Outdated
                }
                _ => WeiduState::Current,
            }
        };
        let needs_weidu = mods.iter().any(|(m, _)| {
            weidu_layout(m.files.keys().map(String::as_str)).is_some_and(|l| !l.has_setup)
        }) && self.weidu_exe(&def).is_none();
        Ok(Some(WeiduStatus {
            needs_weidu,
            state,
            mods: choices,
            files: r.files.len(),
            output: r.output,
            error: r.error,
            ran_unix: r.ran_unix,
            installed: r.installed,
        }))
    }

    /// Installs the profile's WeiDU mods with components chosen on a copy of the game (the
    /// others are left out until theirs are).
    pub fn run_weidu(&self, game_id: &str, profile: &str) -> Result<WeiduStatus> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(game_id.into()));
        }
        let root = self.require_install(&def.def)?;
        let p = umm_store::load_profile(&game, profile)?;
        let mut r = load_record(&game, profile);
        self.weidu_install(&def, &p, &root, &mut r)?;
        self.weidu_status(game_id, profile)?
            .ok_or_else(|| CoreError::Invalid("the profile has no WeiDU mods".into()))
    }

    /// Runs the installers with the components chosen, recording the outcome in `r`. Every
    /// mod needs its components chosen first (in the Mods tab).
    fn weidu_install(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        root: &Path,
        r: &mut Record,
    ) -> Result<()> {
        let game = self.data.game(&def.def.id);
        let mods = chosen(&self.weidu_mods(def, profile), r);
        if mods.is_empty() {
            return Err(CoreError::Invalid(
                "choose a WeiDU mod's components first (Components… in the Mods tab)".into(),
            ));
        }
        let scratch = root.join(WEIDU_SCRATCH);
        let out_dir = record_dir(&game, &profile.name).join(OUT);
        let outcome = self.weidu_in(def, root, &scratch, &out_dir, &mods, &mut r.choices);
        let _ = fs::remove_dir_all(&scratch);
        r.ran_unix = unix_now();
        match outcome {
            Ok((files, output)) => {
                r.files = files;
                r.output = output;
                r.error = None;
                r.installed = mods.iter().map(|(m, _)| m.id.clone()).collect();
            }
            Err((e, output)) => {
                r.installed.clear();
                r.files.clear();
                r.output = output;
                r.error = Some(e.to_string());
            }
        }
        r.key = self.weidu_key(def, root, &mods, &r.choices);
        save_record(&game, &profile.name, r)?;
        match &r.error {
            Some(e) => Err(CoreError::Invalid(format!(
                "the WeiDU mods were not installed: {e}"
            ))),
            None => Ok(()),
        }
    }

    #[allow(clippy::type_complexity)]
    fn weidu_in(
        &self,
        def: &CompiledDef,
        root: &Path,
        scratch: &Path,
        out_dir: &Path,
        mods: &[(ModMeta, String)],
        choices: &mut Vec<WeiduChoice>,
    ) -> std::result::Result<(Vec<PatchedFile>, String), (CoreError, String)> {
        let quiet = |e: CoreError| (e, String::new());
        if scratch.exists() {
            fs::remove_dir_all(scratch)
                .ctx(|| format!("removing {}", scratch.display()))
                .map_err(|e| quiet(e.into()))?;
        }
        // The game, copied; its large read-only folders linked.
        let before = snapshot(root);
        for rel in before.keys() {
            let src = root.join(rel);
            let dst = scratch.join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)
                    .ctx(|| format!("creating {}", p.display()))
                    .map_err(|e| quiet(e.into()))?;
            }
            let top = rel.split('/').next().unwrap_or_default().to_lowercase();
            let linked = rel.contains('/')
                && LINKED_DIRS.contains(&top.as_str())
                && fs::hard_link(&src, &dst).is_ok();
            if !linked {
                fs::copy(&src, &dst)
                    .ctx(|| format!("copying {rel} for the WeiDU installers"))
                    .map_err(|e| quiet(e.into()))?;
            }
        }
        // The mods' own files: the installers and the mods' folders. Never deployed.
        let mut own: Vec<String> = Vec::new();
        for (m, _) in mods {
            for (target, source, _) in self.weidu_files(def, m) {
                let dst = target.to_path(scratch);
                if let Some(p) = dst.parent() {
                    fs::create_dir_all(p)
                        .ctx(|| format!("creating {}", p.display()))
                        .map_err(|e| quiet(e.into()))?;
                }
                fs::copy(&source, &dst)
                    .ctx(|| format!("copying {}", source.display()))
                    .map_err(|e| quiet(e.into()))?;
                own.push(target.as_str().to_lowercase());
            }
        }
        // Mods without their installer get WeiDU, under the installer's name.
        for (m, setup) in mods {
            if scratch.join(setup).is_file() {
                continue;
            }
            let Some(exe) = self.weidu_exe(def) else {
                return Err(quiet(CoreError::Invalid(format!(
                    "{} comes without WeiDU's installer ({setup}): give UMM weidu.exe, from weidu.org",
                    m.name
                ))));
            };
            fs::copy(&exe, scratch.join(setup))
                .ctx(|| format!("copying {}", exe.display()))
                .map_err(|e| quiet(e.into()))?;
            own.push(setup.to_lowercase());
        }
        // The mods' top folders (`<mod>/`, where WeiDU also keeps its backups).
        let own_dirs: Vec<String> = own
            .iter()
            .filter_map(|p| p.split_once('/').map(|(d, _)| format!("{d}/")))
            .collect();
        let start = snapshot(scratch);

        let mut output = String::new();
        for (m, setup) in mods {
            // Checked before: every mod has its components chosen. Its installer runs with
            // them, without its menu; its window shows for any question a component asks.
            let Some(chosen) = choices.iter().find(|c| c.mod_id == m.id).cloned() else {
                continue;
            };
            if chosen.components.is_empty() {
                output.push_str(&format!(
                    "[UMM] {}: no components chosen, skipped.\n",
                    m.name
                ));
                continue;
            }
            let mut args: Vec<String> = vec![
                "--noautoupdate".into(),
                "--language".into(),
                chosen.language.unwrap_or(0).to_string(),
                "--force-install-list".into(),
            ];
            args.extend(chosen.components.iter().map(|(n, _)| n.to_string()));
            args.push("--skip-at-view".into());
            args.push("--no-exit-pause".into());
            let chosen = Some(chosen);
            let log_before = fs::read_to_string(scratch.join("WeiDU.log")).unwrap_or_default();
            let code = run_console(&scratch.join(setup), scratch, &args).map_err(|e| {
                (
                    CoreError::Invalid(format!("running {setup}: {e}")),
                    output.clone(),
                )
            })?;
            // WeiDU's own log of the run, `setup-<mod>.debug`, beside the installer.
            let debug = fs::read_to_string(
                scratch.join(format!("{}.debug", setup.trim_end_matches(".exe"))),
            )
            .unwrap_or_default();
            let tail: Vec<&str> = debug.lines().rev().take(40).collect();
            output.push_str(&format!("=== {} ({setup}) ===\n", m.name));
            output.push_str(&tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
            output.push('\n');
            let log = fs::read_to_string(scratch.join("WeiDU.log")).unwrap_or_default();
            let (language, components) = logged(&log, setup);
            if code != Some(0) && log == log_before {
                return Err((
                    CoreError::Invalid(format!(
                        "{setup} failed (exit code {code:?}); see its log below"
                    )),
                    output,
                ));
            }
            choices.retain(|c| c.mod_id != m.id);
            choices.push(WeiduChoice {
                mod_id: m.id.clone(),
                setup: setup.clone(),
                language: language.or(chosen.and_then(|c| c.language)),
                components,
            });
        }

        // What the installers changed or added: kept for deploys.
        let end = snapshot(scratch);
        let mut files = Vec::new();
        if out_dir.exists() {
            fs::remove_dir_all(out_dir)
                .ctx(|| format!("removing {}", out_dir.display()))
                .map_err(|e| (e.into(), output.clone()))?;
        }
        let mut changed: Vec<&String> = end
            .iter()
            .filter(|(rel, m)| start.get(*rel) != Some(*m))
            .map(|(rel, _)| rel)
            .collect();
        changed.sort();
        for rel in changed {
            let lower = rel.to_lowercase();
            if own.contains(&lower)
                || own_dirs.iter().any(|d| lower.starts_with(d))
                || lower.ends_with(".debug")
                || lower == "weidu.conf"
            {
                continue;
            }
            let src = scratch.join(rel);
            let dst = RelPath::parse(rel)
                .map_err(|e| (e.into(), output.clone()))?
                .to_path(out_dir);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)
                    .ctx(|| format!("creating {}", p.display()))
                    .map_err(|e| (e.into(), output.clone()))?;
            }
            fs::copy(&src, &dst)
                .ctx(|| format!("keeping {rel}"))
                .map_err(|e| (e.into(), output.clone()))?;
            let m = fs::metadata(&dst).ok();
            files.push(PatchedFile {
                target: rel.clone(),
                hash: hash_file(&dst).unwrap_or_default(),
                size: m.map_or(0, |m| m.len()),
            });
        }
        // The game folder itself must be as it was: an installer finds the game it runs in.
        let after = snapshot(root);
        let touched: Vec<&String> = after
            .iter()
            .filter(|(rel, m)| before.get(*rel) != Some(*m))
            .map(|(rel, _)| rel)
            .chain(before.keys().filter(|rel| !after.contains_key(*rel)))
            .collect();
        if !touched.is_empty() {
            output.push_str(&format!(
                "\n[UMM] The game folder itself changed during the install: {}. Verify the game's files in Steam to restore them.",
                touched.iter().take(20).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        Ok((files, output))
    }

    /// Puts the profile's installed WeiDU mods in a deploy plan: the mods' own files are left
    /// out, and what their installers changed or added goes in. A run out of date is redone
    /// first when `run` (a deploy) and every mod has its components chosen; a preview leaves
    /// the files out, saying why.
    pub(crate) fn add_weidu(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        root: &Path,
        plan: &mut Plan,
        run: bool,
    ) -> Result<()> {
        let all = self.weidu_mods(def, profile);
        if all.is_empty() {
            return Ok(());
        }
        let game = self.data.game(&def.def.id);
        plan.files
            .retain(|f| !all.iter().any(|(m, _)| m.id == f.owner));
        let mut r = load_record(&game, &profile.name);
        let mods = chosen(&all, &r);
        let skip = |plan: &mut Plan, list: &[(ModMeta, String)], why: &str| {
            for (m, setup) in list {
                plan.skipped.push(umm_deploy::Skipped {
                    owner: m.id.clone(),
                    file: setup.clone(),
                    reason: why.into(),
                });
            }
        };
        // Mods without components chosen are left out (and shown off) until they are.
        let waiting: Vec<(ModMeta, String)> = all
            .iter()
            .filter(|(m, _)| !mods.iter().any(|(x, _)| x.id == m.id))
            .cloned()
            .collect();
        skip(
            plan,
            &waiting,
            "its components are not chosen yet (Components… in the Mods tab)",
        );
        if mods.is_empty() {
            return Ok(());
        }
        let outdated = r.ran_unix == 0
            || r.error.is_some()
            || self.weidu_key(def, root, &mods, &r.choices) != r.key;
        if outdated {
            if !run {
                skip(
                    plan,
                    &mods,
                    "the profile's WeiDU mods are installed again first (the game or the mods changed)",
                );
                return Ok(());
            }
            self.weidu_install(def, profile, root, &mut r)?;
        }
        let out_dir = record_dir(&game, &profile.name).join(OUT);
        let owner = mods.last().map(|(m, _)| m.id.clone()).unwrap_or_default();
        for f in &r.files {
            let target = RelPath::parse(&f.target)?;
            plan.files.retain(|p| p.target.key() != target.key());
            plan.files.push(PlannedFile {
                source: target.to_path(&out_dir),
                owner: owner.clone(),
                hash: f.hash.clone(),
                size: f.size,
                writable: false,
                target,
            });
        }
        plan.files.sort_by_key(|f| f.target.key());
        Ok(())
    }
}

/// A WeiDU mod's languages and components, as WeiDU lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeiduComponents {
    pub languages: Vec<(u32, String)>,
    /// The language the components are listed in.
    pub language: u32,
    /// By number, with their names (`Group -> Choice` for one of a group's alternatives).
    pub components: Vec<(u32, String)>,
    /// The components chosen for the profile, if any (in the language chosen then).
    pub chosen: Option<(Option<u32>, Vec<u32>)>,
}

/// The languages `--list-languages` prints, one per line: `0:English`.
fn parse_languages(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| {
            let (n, name) = l.trim().split_once(':')?;
            Some((n.trim().parse().ok()?, name.trim().to_string()))
        })
        .collect()
}

impl Session {
    /// Lists a WeiDU mod's languages and its components in `language` (else the one chosen,
    /// else the first), by asking WeiDU (`--list-languages`, `--list-components`) in a folder
    /// of the mod's files, the game folder given for it to read.
    pub fn weidu_components(
        &self,
        game_id: &str,
        profile: &str,
        mod_id: &str,
        language: Option<u32>,
    ) -> Result<WeiduComponents> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        let root = self.require_install(&def.def)?;
        let meta = umm_store::load_mod(&game, mod_id)?;
        let layout = weidu_layout(meta.files.keys().map(String::as_str))
            .ok_or_else(|| CoreError::Invalid(format!("{} is not a WeiDU mod", meta.name)))?;
        // WeiDU only runs in a game folder (`--game` does not do): a folder inside the game's,
        // its files linked (WeiDU writes none of them when listing; those it may write are
        // copied), the mod's files beside them.
        let dir = root.join(WEIDU_LIST);
        let _ = fs::remove_dir_all(&dir);
        for rel in snapshot(&root).keys() {
            let lower = rel.to_lowercase();
            let src = root.join(rel);
            let dst = dir.join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
            }
            let written =
                lower == "weidu.conf" || lower == "weidu.log" || lower.ends_with(".debug");
            if !written && fs::hard_link(&src, &dst).is_ok() {
                continue;
            }
            let small = fs::metadata(&src).is_ok_and(|m| m.len() <= LIST_COPY_LIMIT);
            if written || small {
                fs::copy(&src, &dst).ctx(|| format!("copying {rel}"))?;
            }
        }
        for (target, source, _) in self.weidu_files(&def, &meta) {
            let dst = target.to_path(&dir);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
            }
            fs::copy(&source, &dst).ctx(|| format!("copying {}", source.display()))?;
        }
        let setup = dir.join(&layout.setup);
        if !setup.is_file() {
            let exe = self.weidu_exe(&def).ok_or_else(|| {
                CoreError::Invalid(format!(
                    "{} comes without WeiDU's installer ({}): give UMM weidu.exe, from weidu.org",
                    meta.name, layout.setup
                ))
            })?;
            fs::copy(&exe, &setup).ctx(|| format!("copying {}", exe.display()))?;
        }
        let ask = |args: &[&str]| -> Result<String> {
            let mut all: Vec<String> = vec!["--noautoupdate".into()];
            all.extend(args.iter().map(|a| a.to_string()));
            run_quiet(&setup, &dir, &all).map_err(|e| {
                CoreError::Invalid(format!("asking {} for its components: {e}", layout.setup))
            })
        };
        let outcome = (|| {
            let languages = parse_languages(&ask(&["--list-languages", &layout.tp2])?);
            let r = load_record(&game, profile);
            let chosen = r.choices.iter().find(|c| c.mod_id == mod_id).map(|c| {
                (
                    c.language,
                    c.components.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                )
            });
            let language = language
                .or(chosen.as_ref().and_then(|(l, _)| *l))
                .or(languages.first().map(|(n, _)| *n))
                .unwrap_or(0);
            let listed = ask(&["--list-components", &layout.tp2, &language.to_string()])?;
            let (_, components) = logged(&listed, &layout.setup);
            if components.is_empty() {
                return Err(CoreError::Invalid(format!(
                    "{} listed no components: {}",
                    layout.setup,
                    listed.lines().rev().take(5).collect::<Vec<_>>().join(" / ")
                )));
            }
            Ok(WeiduComponents {
                languages,
                language,
                components,
                chosen,
            })
        })();
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    /// Sets a WeiDU mod's components for the profile: the install runs with them, without
    /// asking, at the next install or launch.
    pub fn set_weidu_choice(
        &self,
        game_id: &str,
        profile: &str,
        mod_id: &str,
        language: u32,
        components: Vec<(u32, String)>,
    ) -> Result<WeiduStatus> {
        let game = self.data.game(game_id);
        let meta = umm_store::load_mod(&game, mod_id)?;
        let layout = weidu_layout(meta.files.keys().map(String::as_str))
            .ok_or_else(|| CoreError::Invalid(format!("{} is not a WeiDU mod", meta.name)))?;
        let mut r = load_record(&game, profile);
        r.choices.retain(|c| c.mod_id != mod_id);
        r.choices.push(WeiduChoice {
            mod_id: mod_id.to_string(),
            setup: layout.setup,
            language: Some(language),
            components,
        });
        save_record(&game, profile, &r)?;
        self.weidu_status(game_id, profile)?
            .ok_or_else(|| CoreError::Invalid("the profile has no WeiDU mods".into()))
    }
}

/// Runs WeiDU without a window, for an answer it prints, and returns what it printed. It
/// should not ask anything: one that waits for input is stopped after a minute.
fn run_quiet(exe: &Path, dir: &Path, args: &[String]) -> std::io::Result<String> {
    use std::io::Read;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "it did not answer within a minute",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(reader.join().unwrap_or_default())
}

/// Runs a WeiDU installer in a console window of its own, where the user answers its
/// questions, and waits for it. Returns its exit code.
fn run_console(exe: &Path, dir: &Path, args: &[String]) -> std::io::Result<Option<i32>> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    #[cfg(not(windows))]
    {
        cmd.stdin(std::process::Stdio::null());
    }
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if let Some(s) = child.try_wait()? {
            return Ok(s.code());
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "it did not finish within 4 hours",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_weidu_mods_layout() {
        let l = weidu_layout([
            "setup-bg1npc.exe",
            "bg1npc/bg1npc.tp2",
            "bg1npc/lib/iconv/iconv.exe",
        ])
        .unwrap();
        assert_eq!(
            (
                l.root.as_str(),
                l.setup.as_str(),
                l.has_setup,
                l.tp2.as_str()
            ),
            ("", "setup-bg1npc.exe", true, "bg1npc/bg1npc.tp2")
        );
        // Wrapped in a folder, and without its installer.
        let l = weidu_layout([
            "BG1NPC-v32/bg1npc/setup-bg1npc.tp2",
            "BG1NPC-v32/bg1npc/x.d",
        ])
        .unwrap();
        assert_eq!(
            (l.root.as_str(), l.setup.as_str(), l.has_setup),
            ("BG1NPC-v32", "setup-bg1npc.exe", false)
        );
        let l = weidu_layout(["setup-tweaks.tp2", "tweaks/a.baf"]).unwrap();
        assert_eq!(l.root, "");
        assert!(weidu_layout(["data/x.tp2", "readme.txt"]).is_none());
    }

    #[test]
    fn reads_listed_languages_and_components() {
        assert_eq!(
            parse_languages("Possible languages:\n0:English\n1:Francais\n"),
            [(0, "English".into()), (1, "Francais".into())]
        );
        let (_, c) = logged(
            "~BG1NPC/BG1NPC.TP2~ #0 #0 // Required Modifications: v32\n~BG1NPC/BG1NPC.TP2~ #0 #12 // Portraits -> Original\n",
            "setup-bg1npc.exe",
        );
        assert_eq!(
            c,
            [
                (0, "Required Modifications: v32".into()),
                (12, "Portraits -> Original".into())
            ]
        );
    }
}
