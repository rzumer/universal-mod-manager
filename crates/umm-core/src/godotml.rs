//! The Godot Mod Loader in a game that does not have it built in.
//!
//! The loader's own setup script, run with `--setup-create-override-cfg`, writes an
//! `override.cfg` that adds its autoloads, and (Godot 4) a `godot/` copy of the pack's
//! `.godot` data folder, next to the exe. UMM runs it once in a scratch copy of the game
//! folder, never in the game folder itself, keeps what it wrote, and deploys that like any
//! other file, with the loader's `addons/`: the revert takes it all away again. The setup is
//! run again when the game's exe or the loader changes.
//!
//! A mod that hooks the game's scripts makes the loader write `mod-hooks.zip` next to the exe
//! (and ask for a restart). It belongs to the profile's mods, not the game: after a session
//! UMM moves it to the profile's runtime folder, and puts it back for the next one.

use crate::{CoreError, Result, Session};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use umm_deploy::PlannedFile;
use umm_gamedef::{GameDef, RelPath, GODOTML_LOADER_DIR};
use umm_store::fsutil::{hash_file, IoCtx};

/// Runs the loader's setup: the exe in the scratch folder, with its arguments.
pub type SetupRunner = Arc<dyn Fn(&Path, &[String]) -> io::Result<()> + Send + Sync>;

/// The loader's setup script, and the arguments that make it write `override.cfg` and quit.
pub const SETUP_ARGS: &[&str] = &[
    "--headless",
    "--script",
    "res://addons/mod_loader/mod_loader_setup.gd",
    "--setup-create-override-cfg",
    "--only-setup",
];
const SETUP_TIMEOUT: Duration = Duration::from_secs(300);
/// Who the setup's files belong to, as shown in a deploy's file list.
pub const SETUP_OWNER: &str = "godot-mod-loader-setup";

const CACHE: &str = "godotml-setup";
const STAGING: &str = "godotml-staging";
const KEY_FILE: &str = "key.txt";
const LIST_FILE: &str = "files.json";
const OUT: &str = "out";

/// Starts the exe and waits for it to quit, without a window.
pub(crate) fn run_setup(exe: &Path, args: &[String]) -> io::Result<()> {
    crate::windows_program(exe)?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if start.elapsed() > SETUP_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "it did not quit within 5 minutes",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Gives each class in a Godot class cache the `is_abstract` and `is_tool` keys Godot 4.5
/// requires: entries without them are skipped, and the loader (7.0.1) writes its own without
/// them, so none of its classes would exist.
pub(crate) fn complete_class_cache(text: &str) -> String {
    let mut pieces = text.split("{\n");
    let mut out = pieces.next().unwrap_or_default().to_string();
    for piece in pieces {
        out.push_str("{\n");
        let entry = piece.split('}').next().unwrap_or_default();
        for key in ["is_abstract", "is_tool"] {
            if !entry.contains(&format!("\"{key}\":")) {
                out.push_str(&format!("\"{key}\": false,\n"));
            }
        }
        out.push_str(piece);
    }
    out
}

/// The loader's pack of hooked game scripts, next to the exe.
pub const HOOK_PACK: &str = "mod-hooks.zip";
/// Marks that UMM looks after the game folder's hook pack for this session (it was not the
/// user's own); holds the profile's name.
const HOOKS_MARKER: &str = "godotml-hooks.owned";

impl Session {
    /// After a deploy: the profile's hook pack goes next to the exe, and UMM takes charge of
    /// that path until the revert. A hook pack the user already had there is left alone.
    pub(crate) fn place_godotml_hooks(
        &self,
        game: &umm_store::GameDir,
        root: &Path,
        profile: &str,
    ) {
        let dst = root.join(HOOK_PACK);
        let marker = game.generated_dir().join(HOOKS_MARKER);
        if dst.exists() && !marker.exists() {
            return;
        }
        let _ = fs::create_dir_all(game.generated_dir());
        if fs::write(&marker, profile).is_err() {
            return;
        }
        let src = game.runtime_dir(profile).join(HOOK_PACK);
        if src.is_file() {
            // Without it the loader makes it again, and asks for a restart.
            let _ = fs::copy(&src, &dst);
        }
    }

    /// Before a revert: the hook pack, as the game left it, moves to the profile's runtime
    /// folder. Nothing happens unless UMM took charge of it at deploy.
    pub(crate) fn take_godotml_hooks(&self, game: &umm_store::GameDir, root: &Path) -> Result<()> {
        let marker = game.generated_dir().join(HOOKS_MARKER);
        let Ok(profile) = fs::read_to_string(&marker) else {
            return Ok(());
        };
        let src = root.join(HOOK_PACK);
        if src.is_file() {
            let dir = game.runtime_dir(profile.trim());
            fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
            let dst = dir.join(HOOK_PACK);
            if fs::rename(&src, &dst).is_err() {
                fs::copy(&src, &dst).ctx(|| format!("keeping {}", src.display()))?;
                fs::remove_file(&src).ctx(|| format!("removing {}", src.display()))?;
            }
        }
        fs::remove_file(&marker).ctx(|| format!("removing {}", marker.display()))?;
        Ok(())
    }
}

/// Whether a planned file is one of the loader's own (`addons/...`).
pub(crate) fn is_loader_file(f: &PlannedFile) -> bool {
    f.target
        .first()
        .is_some_and(|c| c.eq_ignore_ascii_case(GODOTML_LOADER_DIR))
}

/// Hardlinks `src` to `dst`, or copies it when that is not possible (another drive).
fn link_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    if fs::hard_link(src, dst).is_err() {
        fs::copy(src, dst)?;
    }
    Ok(())
}

impl Session {
    /// The files the loader's setup makes for this game (`override.cfg`, `godot/...`), for
    /// the game folder: from the cache, or made now by running the setup in a scratch copy of
    /// the game. `loader` is the loader's own files, as the profile deploys them.
    pub(crate) fn godotml_setup_files(
        &self,
        def: &GameDef,
        root: &Path,
        loader: &[PlannedFile],
    ) -> Result<Vec<PlannedFile>> {
        let exe_rel = def
            .launch
            .as_ref()
            .map(|l| RelPath::parse(&l.exe))
            .transpose()?
            .ok_or_else(|| CoreError::NoLaunch(def.id.clone()))?;
        let exe = exe_rel.to_path(root);
        let meta = fs::metadata(&exe).ctx(|| format!("reading {}", exe.display()))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let mut key = format!("exe {} {} {modified}\n", exe_rel.as_str(), meta.len());
        let mut sorted: Vec<&PlannedFile> = loader.iter().collect();
        sorted.sort_by_key(|f| f.target.key());
        for f in sorted {
            key.push_str(&format!("{} {}\n", f.target.as_str(), f.hash));
        }

        let cache = self.data.game(&def.id).generated_dir().join(CACHE);
        let list_path = cache.join(LIST_FILE);
        let cached = fs::read_to_string(cache.join(KEY_FILE)).ok().as_deref() == Some(&key);
        let list: Vec<(String, u64, String)> = match cached
            .then(|| fs::read(&list_path).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            Some(l) => l,
            None => self.make_godotml_setup(def, root, &exe_rel, loader, &cache, &key)?,
        };
        let out = cache.join(OUT);
        list.into_iter()
            .map(|(rel, size, hash)| {
                let target = RelPath::parse(&rel)?;
                Ok(PlannedFile {
                    source: target.to_path(&out),
                    writable: rel.to_ascii_lowercase().ends_with(".cfg"),
                    target,
                    owner: SETUP_OWNER.into(),
                    hash,
                    size,
                })
            })
            .collect()
    }

    /// Runs the setup in a scratch copy of the game and keeps what it wrote in `cache`.
    fn make_godotml_setup(
        &self,
        def: &GameDef,
        root: &Path,
        exe_rel: &RelPath,
        loader: &[PlannedFile],
        cache: &Path,
        key: &str,
    ) -> Result<Vec<(String, u64, String)>> {
        let staging = self.data.game(&def.id).generated_dir().join(STAGING);
        if staging.exists() {
            fs::remove_dir_all(&staging).ctx(|| format!("removing {}", staging.display()))?;
        }
        // The game's files: the exe and packs linked (read only, and large), the rest
        // copied, so nothing the setup writes can reach the game folder.
        for entry in walkdir::WalkDir::new(root).min_depth(1) {
            let entry = entry.map_err(|e| CoreError::Invalid(e.to_string()))?;
            let rel = entry.path().strip_prefix(root).expect("under root");
            let dst = staging.join(rel);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&dst).ctx(|| format!("creating {}", dst.display()))?;
                continue;
            }
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
            }
            let lower = rel.to_string_lossy().to_ascii_lowercase();
            let read_only = lower.ends_with(".exe") || lower.ends_with(".pck");
            let r = if read_only {
                link_or_copy(entry.path(), &dst)
            } else {
                fs::copy(entry.path(), &dst).map(|_| ())
            };
            r.ctx(|| format!("copying {} for the loader's setup", entry.path().display()))?;
        }
        for f in loader {
            let dst = f.target.to_path(&staging);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).ctx(|| format!("creating {}", p.display()))?;
            }
            fs::copy(&f.source, &dst).ctx(|| format!("copying {}", f.source.display()))?;
        }
        // Started outside Steam, a Steam game would otherwise restart through it.
        if let Some(app) = def.detect.steam_app_id {
            let p = staging.join("steam_appid.txt");
            if !p.exists() {
                fs::write(&p, app.to_string()).ctx(|| format!("writing {}", p.display()))?;
            }
        }

        let exe = exe_rel.to_path(&staging);
        let args: Vec<String> = SETUP_ARGS.iter().map(|s| s.to_string()).collect();
        let runner = self.godotml_runner();
        runner(&exe, &args).map_err(|e| {
            CoreError::Invalid(format!(
                "the Godot Mod Loader's setup for {} failed to run: {e}",
                def.name
            ))
        })?;
        let override_cfg = staging.join("override.cfg");
        if !override_cfg.is_file() {
            return Err(CoreError::Invalid(format!(
                "the Godot Mod Loader's setup for {} did not finish (it wrote no override.cfg); its log is in the game's user folder, under logs/modloader.log",
                def.name
            )));
        }

        let cache_cfg = staging.join("godot/global_script_class_cache.cfg");
        if let Ok(text) = fs::read_to_string(&cache_cfg) {
            fs::write(&cache_cfg, complete_class_cache(&text))
                .ctx(|| format!("writing {}", cache_cfg.display()))?;
        }

        // Keep what it wrote: override.cfg, and godot/ when it made one.
        let out = cache.join(OUT);
        if out.exists() {
            fs::remove_dir_all(&out).ctx(|| format!("removing {}", out.display()))?;
        }
        fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
        let _ = fs::remove_file(cache.join(KEY_FILE));
        fs::rename(&override_cfg, out.join("override.cfg"))
            .ctx(|| format!("keeping {}", override_cfg.display()))?;
        let godot = staging.join("godot");
        if godot.is_dir() {
            fs::rename(&godot, out.join("godot")).ctx(|| format!("keeping {}", godot.display()))?;
        }
        let _ = fs::remove_dir_all(&staging);

        let mut list = Vec::new();
        for entry in walkdir::WalkDir::new(&out).min_depth(1) {
            let entry = entry.map_err(|e| CoreError::Invalid(e.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(&out).expect("under out");
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let hash =
                hash_file(entry.path()).ctx(|| format!("hashing {}", entry.path().display()))?;
            list.push((rel, size, hash));
        }
        let json = serde_json::to_vec(&list).expect("serializable");
        fs::write(cache.join(LIST_FILE), json)
            .ctx(|| "writing the setup's file list".to_string())?;
        fs::write(cache.join(KEY_FILE), key).ctx(|| "writing the setup's key".to_string())?;
        Ok(list)
    }

    fn godotml_runner(&self) -> SetupRunner {
        self.godotml_runner
            .lock()
            .expect("not poisoned")
            .clone()
            .unwrap_or_else(|| Arc::new(run_setup))
    }

    /// Replaces how the loader's setup is run (tests pass a fake).
    #[doc(hidden)]
    pub fn set_godotml_runner(&self, runner: SetupRunner) {
        *self.godotml_runner.lock().expect("not poisoned") = Some(runner);
    }
}

/// The folder the setup's kept files are in, for a game (for tests and the interface).
pub fn setup_cache(game: &umm_store::GameDir) -> PathBuf {
    game.generated_dir().join(CACHE).join(OUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_cache_entries_get_the_keys_godot_requires() {
        let text = "list=[{\n\"base\": &\"Node\",\n\"class\": &\"Game\",\n\"icon\": \"\",\n\"is_abstract\": false,\n\"is_tool\": true,\n\"language\": &\"GDScript\",\n\"path\": \"res://game.gd\"\n}, {\n\"base\": &\"Object\",\n\"class\": &\"ModLoaderLog\",\n\"icon\": \"\",\n\"language\": &\"GDScript\",\n\"path\": \"res://addons/mod_loader/api/log.gd\"\n}]\n";
        let fixed = complete_class_cache(text);
        assert_eq!(
            fixed,
            text.replace(
                "}, {\n\"base\": &\"Object\"",
                "}, {\n\"is_abstract\": false,\n\"is_tool\": false,\n\"base\": &\"Object\""
            ),
            "only the entry without them changes"
        );
        assert_eq!(complete_class_cache(&fixed), fixed);
    }
}
