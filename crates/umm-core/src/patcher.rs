//! Patchers: mods whose job is a program that patches the game's own files (the Fallout
//! Anniversary Patcher turns Fallout 3 1.7.0.4 back into 1.7.0.3, which loads FOSE by
//! itself). Copied into the game folder, such a mod does nothing until someone runs it, and
//! run there it changes the game for good.
//!
//! UMM runs the patcher once, with the user's approval, on a copy of the files at the top of
//! the game folder, and keeps the files it changed. While mods are deployed, those replace the
//! game's own through the journal, so the originals come back afterwards (and Steam's file
//! checks and updates see the game as it shipped). The patcher runs again by itself once the
//! game's files or the patcher change (a game update). Its own files are never deployed.

use crate::{CoreError, Result, Session};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use umm_deploy::{GameLock, Plan, PlannedFile, Skipped};
use umm_gamedef::{CompiledDef, RelPath};
use umm_store::fsutil::{hash_file, unix_now, IoCtx};
use umm_store::{GameDir, ModMeta, Profile};

/// Under the game's generated folder: one folder per patcher mod.
const PATCHES: &str = "patches";
const RECORD: &str = "patch.json";
const OUT: &str = "files";
const SCRATCH: &str = "patch-scratch";
const TIMEOUT: Duration = Duration::from_secs(600);

/// Whether a file (a path inside the mod, as placed) is a patcher program: an `.exe` at the
/// top named like one (`Patcher.exe`, `FNVpatch.exe`).
pub fn is_patcher_exe(path: &str) -> bool {
    let p = path.to_lowercase();
    !p.contains('/') && p.ends_with(".exe") && (p.contains("patcher") || p.contains("patch.exe"))
}

/// A mod's patcher program as placed (its placement applied, not the install rules), for
/// showing: `None` when the mod is not a patcher. A mod with delta patches (`.xdelta`) is
/// not: the patcher program beside them (Delta Patcher) is only there for applying them by
/// hand, and UMM applies them itself.
pub fn patcher_of(meta: &ModMeta) -> Option<String> {
    if meta.files.keys().any(|f| crate::is_delta_patch(f)) {
        return None;
    }
    meta.files.keys().find_map(|f| {
        let rel = RelPath::parse(f).ok()?;
        let placed = match &meta.placement {
            Some(p) => p.place(&rel).ok()?,
            None => rel,
        };
        is_patcher_exe(placed.as_str()).then(|| placed.as_str().to_string())
    })
}

/// What UMM keeps of a patcher's run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Record {
    /// The user said UMM may run it.
    approved: bool,
    /// What it ran on (the game's top files, the patcher's files): it runs again once this
    /// changes.
    #[serde(default)]
    key: String,
    #[serde(default)]
    files: Vec<PatchedFile>,
    /// What it printed.
    #[serde(default)]
    output: String,
    #[serde(default)]
    ran_unix: u64,
    #[serde(default)]
    error: Option<String>,
}

/// A game file as the patcher left it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchedFile {
    /// Its path in the game folder.
    pub target: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    /// Never run, or not approved.
    NotRun,
    /// Run on the game as it is now.
    Current,
    /// The game (or the patcher) changed since: it runs again at the next deploy.
    Outdated,
    /// Its last run failed.
    Failed,
}

#[derive(Debug, Clone)]
pub struct PatcherStatus {
    /// The patcher program, in the game folder's terms.
    pub exe: String,
    pub approved: bool,
    pub state: PatchState,
    /// The game files it patches.
    pub files: Vec<PatchedFile>,
    pub output: String,
    pub error: Option<String>,
    pub ran_unix: u64,
}

pub(crate) fn patch_dir(game: &GameDir, mod_id: &str) -> PathBuf {
    game.generated_dir().join(PATCHES).join(mod_id)
}

/// Forgets a deleted mod's patcher run: approval and results.
pub(crate) fn forget_mod(game: &GameDir, mod_id: &str) {
    let _ = fs::remove_dir_all(patch_dir(game, mod_id));
}

fn load_record(game: &GameDir, mod_id: &str) -> Record {
    fs::read(patch_dir(game, mod_id).join(RECORD))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_record(game: &GameDir, mod_id: &str, r: &Record) -> Result<()> {
    let dir = patch_dir(game, mod_id);
    fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
    let p = dir.join(RECORD);
    let text = serde_json::to_vec_pretty(r).map_err(|e| CoreError::Invalid(e.to_string()))?;
    umm_store::fsutil::write_atomic(&p, &text).ctx(|| format!("writing {}", p.display()))?;
    Ok(())
}

/// The game files a patcher may change, relative to the game folder: those at its top (the
/// exe and its neighbours), and a Bethesda game's plugins (the ESM Patcher patches
/// `Fallout3.esm`), not its large archives.
fn candidates(def: &CompiledDef, root: &Path) -> Result<Vec<String>> {
    let files_in = |dir: &Path| -> Vec<String> {
        fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    if !root.is_dir() {
        return Err(CoreError::NotInstalled(def.def.id.clone()));
    }
    let mut v = files_in(root);
    if let Some(data) = umm_gamedef::setup::bethesda_data_folder(&def.def) {
        v.extend(
            files_in(&root.join(data))
                .into_iter()
                .filter(|n| {
                    let n = n.to_lowercase();
                    umm_gamedef::PLUGIN_EXTENSIONS
                        .iter()
                        .any(|e| n.ends_with(&format!(".{e}")))
                })
                .map(|n| format!("{data}/{n}")),
        );
    }
    v.sort_by_key(|n| n.to_lowercase());
    Ok(v)
}

/// Every file in a folder, with its size and time, relative to it (`/`-separated).
fn snapshot(root: &Path) -> std::collections::HashMap<String, (u64, u64)> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(root).ok()?;
            let rel = RelPath::from_path(rel).ok()?;
            let m = e.metadata().ok()?;
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            Some((rel.as_str().to_string(), (m.len(), modified)))
        })
        .collect()
}

/// Where a patcher runs: a copy of the game's files it may change.
pub(crate) fn scratch_dir(game: &GameDir) -> PathBuf {
    game.generated_dir().join(SCRATCH)
}

impl Session {
    /// A mod's files where they would go in the game folder (its placement and the install
    /// rules applied), with their stored copies.
    pub(crate) fn placed_files(
        &self,
        def: &CompiledDef,
        meta: &ModMeta,
    ) -> Vec<(RelPath, PathBuf, String)> {
        let game = self.data.game(&def.def.id);
        let files_root = game.mod_files_dir(&meta.id);
        meta.files
            .iter()
            .filter_map(|(file, rec)| {
                let rel = RelPath::parse(file).ok()?;
                let placed = match &meta.placement {
                    Some(p) => p.place(&rel).ok()?,
                    None => rel.clone(),
                };
                let target = def.map_mod_file(&meta.id, &placed).ok()??;
                Some((target, rel.to_path(&files_root), rec.hash.clone()))
            })
            .collect()
    }

    /// The mod's patcher program, if it is a patcher.
    fn patcher_in(&self, def: &CompiledDef, meta: &ModMeta) -> Option<String> {
        let placed: Vec<String> = self
            .placed_files(def, meta)
            .into_iter()
            .map(|(t, _, _)| t.as_str().to_string())
            .collect();
        if placed.iter().any(|t| crate::is_delta_patch(t)) {
            return None;
        }
        placed.into_iter().find(|t| is_patcher_exe(t))
    }

    /// What the patcher's run depends on: the game files it patched (size and time, as a
    /// game update changes them) and the patcher's own files. Not the game's other files:
    /// logs the game and its script extender write at the top of the folder change with
    /// every session.
    fn patch_key(
        &self,
        def: &CompiledDef,
        meta: &ModMeta,
        root: &Path,
        patched: &[PatchedFile],
    ) -> Result<String> {
        let mut key = String::new();
        for f in patched {
            let p = RelPath::parse(&f.target)?.to_path(root);
            // A patched file the game no longer has: changed, as far as the run goes.
            let (len, modified) = match fs::metadata(&p) {
                Ok(m) => (
                    m.len(),
                    m.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs()),
                ),
                Err(_) => (0, 0),
            };
            key.push_str(&format!("game {} {len} {modified}\n", f.target));
        }
        let mut mine: Vec<(String, String)> = self
            .placed_files(def, meta)
            .into_iter()
            .map(|(t, _, h)| (t.as_str().to_string(), h))
            .collect();
        mine.sort();
        for (t, h) in mine {
            key.push_str(&format!("mod {t} {h}\n"));
        }
        Ok(key)
    }

    /// The mod's patcher and how its run stands, or `None` when the mod is not a patcher.
    pub fn patcher_status(&self, game_id: &str, mod_id: &str) -> Result<Option<PatcherStatus>> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        let meta = umm_store::load_mod(&game, mod_id)?;
        let Some(exe) = self.patcher_in(&def, &meta) else {
            return Ok(None);
        };
        let r = load_record(&game, mod_id);
        // While deployed, the game folder holds the patched files: its key says nothing.
        let deployed = game.journal_path().exists();
        let state = if !r.approved || r.ran_unix == 0 && r.error.is_none() {
            PatchState::NotRun
        } else if r.error.is_some() {
            PatchState::Failed
        } else if deployed {
            PatchState::Current
        } else {
            match self.install_path(&def.def)? {
                Some((root, _)) if self.patch_key(&def, &meta, &root, &r.files)? != r.key => {
                    PatchState::Outdated
                }
                _ => PatchState::Current,
            }
        };
        Ok(Some(PatcherStatus {
            exe,
            approved: r.approved,
            state,
            files: r.files,
            output: r.output,
            error: r.error,
            ran_unix: r.ran_unix,
        }))
    }

    /// Runs the mod's patcher on a copy of the game's top files, and keeps what it changed.
    /// Running it is the user's approval to run it again by itself after a game update.
    pub fn run_patcher(&self, game_id: &str, mod_id: &str) -> Result<PatcherStatus> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(game_id.into()));
        }
        let root = self.require_install(&def.def)?;
        let meta = umm_store::load_mod(&game, mod_id)?;
        let mut r = load_record(&game, mod_id);
        r.approved = true;
        self.patch(&def, &meta, &root, &mut r)?;
        self.patcher_status(game_id, mod_id)?
            .ok_or_else(|| CoreError::Invalid(format!("{} has no patcher", meta.name)))
    }

    /// Runs the patcher (approved), recording the outcome in `r`.
    fn patch(&self, def: &CompiledDef, meta: &ModMeta, root: &Path, r: &mut Record) -> Result<()> {
        let game = self.data.game(&def.def.id);
        let exe = self
            .patcher_in(def, meta)
            .ok_or_else(|| CoreError::Invalid(format!("{} has no patcher", meta.name)))?;
        let scratch = scratch_dir(&game);
        let outcome = self.patch_in(def, meta, root, &exe, &scratch);
        let _ = fs::remove_dir_all(&scratch);
        r.ran_unix = unix_now();
        match outcome {
            Ok((files, output)) => {
                r.files = files;
                r.output = output;
                r.error = None;
            }
            Err((e, output)) => {
                r.files.clear();
                r.output = output;
                r.error = Some(e.to_string());
            }
        }
        r.key = self.patch_key(def, meta, root, &r.files)?;
        save_record(&game, &meta.id, r)?;
        match &r.error {
            Some(e) => Err(CoreError::Invalid(format!(
                "{exe} did not patch the game: {e}"
            ))),
            None => Ok(()),
        }
    }

    /// The run itself, in `scratch`: the game's files it may change copied (never linked:
    /// the patcher writes to them), then the patcher's files, then the patcher run there.
    /// Returns the game files it changed, kept in the mod's patch folder, and what it printed.
    ///
    /// A patcher that finds the real game folder by itself (from the registry) patches that
    /// instead: its changes are kept all the same, and the game's originals put back from
    /// the untouched copy.
    #[allow(clippy::type_complexity)]
    fn patch_in(
        &self,
        def: &CompiledDef,
        meta: &ModMeta,
        root: &Path,
        exe: &str,
        scratch: &Path,
    ) -> std::result::Result<(Vec<PatchedFile>, String), (CoreError, String)> {
        let quiet = |e: CoreError| (e, String::new());
        if scratch.exists() {
            fs::remove_dir_all(scratch)
                .ctx(|| format!("removing {}", scratch.display()))
                .map_err(|e| quiet(e.into()))?;
        }
        fs::create_dir_all(scratch)
            .ctx(|| format!("creating {}", scratch.display()))
            .map_err(|e| quiet(e.into()))?;
        let mut before: Vec<(String, String)> = Vec::new();
        for name in candidates(def, root).map_err(quiet)? {
            let dst = scratch.join(&name);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)
                    .ctx(|| format!("creating {}", p.display()))
                    .map_err(|e| quiet(e.into()))?;
            }
            fs::copy(root.join(&name), &dst)
                .ctx(|| format!("copying {name} for the patcher"))
                .map_err(|e| quiet(e.into()))?;
            let h = hash_file(&dst)
                .ctx(|| format!("hashing {name}"))
                .map_err(|e| quiet(e.into()))?;
            before.push((name, h));
        }
        for (target, source, _) in self.placed_files(def, meta) {
            let dst = target.to_path(scratch);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)
                    .ctx(|| format!("creating {}", p.display()))
                    .map_err(|e| quiet(e.into()))?;
            }
            fs::copy(&source, &dst)
                .ctx(|| format!("copying {}", source.display()))
                .map_err(|e| quiet(e.into()))?;
        }
        // Everything in the real game folder, to tell afterwards what else a patcher that
        // found it by itself changed there.
        let real_before = snapshot(root);
        let output = run(&scratch.join(exe), scratch).map_err(|e| {
            (
                CoreError::Invalid(format!("running {exe}: {e}")),
                String::new(),
            )
        })?;
        let out_dir = patch_dir(&self.data.game(&def.def.id), &meta.id).join(OUT);
        if out_dir.exists() {
            fs::remove_dir_all(&out_dir)
                .ctx(|| format!("removing {}", out_dir.display()))
                .map_err(|e| (e.into(), output.clone()))?;
        }
        let mut files = Vec::new();
        let mut output = output;
        let copied: Vec<String> = before.iter().map(|(n, _)| n.to_lowercase()).collect();
        for (name, old) in before {
            let copy = scratch.join(&name);
            let real = root.join(&name);
            // A file the patcher removed is left alone: UMM only swaps files in.
            let in_copy = hash_file(&copy).ok().filter(|h| *h != old);
            let in_game = hash_file(&real).ok().filter(|h| *h != old);
            let (now, patched) = match (in_copy, in_game) {
                (Some(h), _) => (h, copy.clone()),
                (None, Some(h)) => {
                    // Patched in the game folder itself: keep that, then the original back.
                    let tmp = out_dir.with_extension("tmp");
                    fs::create_dir_all(&tmp)
                        .ctx(|| format!("creating {}", tmp.display()))
                        .map_err(|e| (e.into(), output.clone()))?;
                    let kept = tmp.join(name.replace('/', "_"));
                    fs::copy(&real, &kept)
                        .ctx(|| format!("keeping the patched {name}"))
                        .map_err(|e| (e.into(), output.clone()))?;
                    fs::copy(&copy, &real)
                        .ctx(|| format!("putting the original {name} back"))
                        .map_err(|e| (e.into(), output.clone()))?;
                    output.push_str(&format!(
                        "\n[UMM] It patched {name} in the game folder itself; UMM kept the patched copy and put the original back."
                    ));
                    (h, kept)
                }
                (None, None) => continue,
            };
            let kept = RelPath::parse(&name)
                .map_err(|e| (e.into(), output.clone()))?
                .to_path(&out_dir);
            if let Some(p) = kept.parent() {
                fs::create_dir_all(p)
                    .ctx(|| format!("creating {}", p.display()))
                    .map_err(|e| (e.into(), output.clone()))?;
            }
            fs::copy(&patched, &kept)
                .ctx(|| format!("keeping the patched {name}"))
                .map_err(|e| (e.into(), output.clone()))?;
            let size = fs::metadata(&kept).map(|m| m.len()).unwrap_or(0);
            files.push(PatchedFile {
                target: name,
                hash: now,
                size,
            });
        }
        let _ = fs::remove_dir_all(out_dir.with_extension("tmp"));

        // Anything else it changed in the real game folder. A file it added is moved out into
        // its results (deployed like the rest, so it is gone again afterwards); a file UMM had
        // no copy of cannot be put back, and is reported.
        let real_after = snapshot(root);
        let mut not_restored = Vec::new();
        for (rel, m) in &real_after {
            if copied.contains(&rel.to_lowercase()) {
                continue;
            }
            match real_before.get(rel) {
                Some(old) if old == m => {}
                Some(_) => not_restored.push(format!("changed {rel}")),
                None => {
                    let real = root.join(rel);
                    let kept = RelPath::parse(rel)
                        .map_err(|e| (e.into(), output.clone()))?
                        .to_path(&out_dir);
                    if let Some(p) = kept.parent() {
                        fs::create_dir_all(p)
                            .ctx(|| format!("creating {}", p.display()))
                            .map_err(|e| (e.into(), output.clone()))?;
                    }
                    fs::copy(&real, &kept)
                        .ctx(|| format!("keeping {rel}"))
                        .map_err(|e| (e.into(), output.clone()))?;
                    fs::remove_file(&real)
                        .ctx(|| format!("removing {rel} from the game folder"))
                        .map_err(|e| (e.into(), output.clone()))?;
                    output.push_str(&format!(
                        "\n[UMM] It added {rel} to the game folder itself; UMM moved it into the patcher's results."
                    ));
                    files.push(PatchedFile {
                        target: rel.clone(),
                        hash: hash_file(&kept).unwrap_or_default(),
                        size: m.0,
                    });
                }
            }
        }
        for rel in real_before.keys() {
            if !real_after.contains_key(rel) && !copied.contains(&rel.to_lowercase()) {
                not_restored.push(format!("removed {rel}"));
            }
        }
        if !not_restored.is_empty() {
            not_restored.sort();
            output.push_str(&format!(
                "\n[UMM] It also changed the game folder itself, and UMM had no copy to put back: {}. Verify the game's files in Steam to restore them.",
                not_restored.join(", ")
            ));
        }
        if files.is_empty() {
            return Err((
                CoreError::Invalid(
                    "it changed none of the game's files (an unsupported game version?)".into(),
                ),
                output,
            ));
        }
        Ok((files, output))
    }

    /// Puts patchers' results (and delta patches', see [`crate::delta`]) in a deploy plan: each patcher mod's own files are left out,
    /// and the game files it patched go in, replacing the game's (protected or not: the user
    /// approved the patcher). A run that is out of date is redone first when `run` (a
    /// deploy); otherwise (a preview) the files are left out with the reason. Returns the
    /// game files patched.
    pub(crate) fn add_patches(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        root: &Path,
        plan: &mut Plan,
        run: bool,
    ) -> Result<Vec<RelPath>> {
        let game = self.data.game(&def.def.id);
        let mut patched = Vec::new();
        crate::delta::skip_unsupported(root, plan);
        for id in profile.enabled_ids() {
            let Ok(meta) = umm_store::load_mod(&game, id) else {
                continue;
            };
            // Delta patches (`.xdelta`), applied in the profile's order.
            if let Some(files) = self.add_deltas(def, &meta, root, plan, run)? {
                patched.extend(files);
                continue;
            }
            let Some(exe) = self.patcher_in(def, &meta) else {
                continue;
            };
            plan.files.retain(|f| f.owner != meta.id);
            let mut r = load_record(&game, id);
            let skip = |plan: &mut Plan, reason: String| {
                plan.skipped.push(Skipped {
                    owner: meta.id.clone(),
                    file: exe.clone(),
                    reason,
                })
            };
            if !r.approved {
                skip(
                    plan,
                    format!("{exe} is a patcher: run it from the Mods tab to patch the game"),
                );
                continue;
            }
            let outdated = r.key != self.patch_key(def, &meta, root, &r.files)?;
            if outdated || r.error.is_some() {
                if !run {
                    let why = if outdated {
                        format!("{exe} runs again first: the game changed since it last ran")
                    } else {
                        format!("{exe} failed last time; it runs again first")
                    };
                    skip(plan, why);
                    continue;
                }
                self.patch(def, &meta, root, &mut r)?;
            }
            let out_dir = patch_dir(&game, id).join(OUT);
            for f in &r.files {
                let target = RelPath::parse(&f.target)?;
                plan.files.retain(|p| p.target.key() != target.key());
                plan.files.push(PlannedFile {
                    source: target.to_path(&out_dir),
                    owner: meta.id.clone(),
                    hash: f.hash.clone(),
                    size: f.size,
                    writable: false,
                    target: target.clone(),
                });
                patched.push(target);
            }
        }
        plan.files.sort_by_key(|f| f.target.key());
        Ok(patched)
    }

    /// The game files patched in a deploy (whose journal this is): files that replaced the
    /// game's own with a patcher's.
    pub(crate) fn patched_in(&self, game_id: &str, journal: &umm_deploy::Journal) -> Vec<String> {
        let dir = self.data.game(game_id).generated_dir().join(PATCHES);
        journal
            .ops
            .iter()
            .filter_map(|o| match o {
                umm_deploy::Op::Replace { path, src, .. } if src.starts_with(&dir) => {
                    Some(path.as_str().to_string())
                }
                _ => None,
            })
            .collect()
    }
}

/// Runs a patcher in `dir` and waits for it, returning what it printed. Its window shows:
/// some patchers ask for a click before they quit.
fn run(exe: &Path, dir: &Path) -> std::io::Result<String> {
    let spawned = std::process::Command::new(exe)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    // A patcher that must run as administrator (its manifest says so): through Windows' own
    // prompt, which the user answers. What it prints cannot be read that way.
    #[cfg(windows)]
    if let Err(e) = &spawned {
        const ERROR_ELEVATION_REQUIRED: i32 = 740;
        if e.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) {
            run_elevated(exe, dir)?;
            return Ok(
                "[UMM] It needs administrator rights: Windows asked you to allow it. What it printed cannot be shown."
                    .into(),
            );
        }
    }
    let mut child = spawned?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        let mut e = String::new();
        if let Some(o) = err.as_mut() {
            let _ = o.read_to_string(&mut e);
        }
        if !e.is_empty() {
            s.push_str(&e);
        }
        s
    });
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "it did not quit within 10 minutes",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(reader.join().unwrap_or_default().trim().to_string())
}

/// Runs a program as administrator in `dir` ("Run as administrator": Windows asks the user)
/// and waits for it.
#[cfg(windows)]
fn run_elevated(exe: &Path, dir: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain([0]).collect() };
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let directory = wide(dir.as_os_str());
    // SAFETY: a plain C struct, all zeroes being valid; the strings outlive the call.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    // SAFETY: `info` is filled in as the call expects.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = std::io::Error::last_os_error();
        const ERROR_CANCELLED: i32 = 1223;
        if e.raw_os_error() == Some(ERROR_CANCELLED) {
            return Err(std::io::Error::other(
                "it needs administrator rights, and Windows' request for them was declined",
            ));
        }
        return Err(e);
    }
    let process = info.hProcess;
    if process.is_null() {
        return Err(std::io::Error::other("Windows did not start it"));
    }
    let start = Instant::now();
    let outcome = loop {
        // SAFETY: a process handle Windows gave, closed below.
        if unsafe { WaitForSingleObject(process, 100) } == WAIT_OBJECT_0 {
            break Ok(());
        }
        if start.elapsed() > TIMEOUT {
            // SAFETY: as above.
            unsafe { TerminateProcess(process, 1) };
            break Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "it did not quit within 10 minutes",
            ));
        }
    };
    // SAFETY: as above.
    unsafe { CloseHandle(process) };
    outcome
}
