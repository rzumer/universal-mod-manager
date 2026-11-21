use crate::journal::{Journal, Method, Op, Phase, JOURNAL_VERSION};
use crate::revert::{revert, RevertReport};
use crate::{DeployError, Faults, Plan};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use umm_gamedef::{RelPath, IN_GAME_BACKUP_DIR};
use umm_store::fsutil::{clear_readonly, hash_file, unix_now, IoCtx};
use umm_store::GameDir;

/// Inspects the game folder and turns a plan into journal operations. Reads only.
pub fn prepare(
    game: &GameDir,
    plan: &Plan,
    game_root: &Path,
    prefer_copy: bool,
) -> Result<Journal, DeployError> {
    if !game_root.is_dir() {
        return Err(DeployError::GameRootMissing(game_root.to_path_buf()));
    }
    let game_root = fs::canonicalize(game_root)
        .ctx(|| format!("resolving {}", game_root.display()))
        .map(strip_verbatim)?;
    if !game_root
        .components()
        .any(|c| matches!(c, Component::Normal(_)))
    {
        return Err(DeployError::UnsafeGameRoot(game_root));
    }

    let hardlink_ok = !prefer_copy && same_volume(&game.mods_dir(), &game_root);
    let (backup_root, backup_in_game) = if same_volume(game.root(), &game_root) {
        (game.backups_dir(), false)
    } else {
        (game_root.join(IN_GAME_BACKUP_DIR), true)
    };

    let mut new_dirs: BTreeMap<String, RelPath> = BTreeMap::new();
    let mut file_ops = Vec::new();
    for f in &plan.files {
        for dir in f.target.ancestors() {
            if new_dirs.contains_key(&dir.key()) {
                continue;
            }
            let p = dir.to_path(&game_root);
            match fs::symlink_metadata(&p) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(DeployError::LinkInPath(dir.to_string()))
                }
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(DeployError::TargetIsFile(dir.to_string())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    new_dirs.insert(dir.key(), dir);
                }
                Err(e) => {
                    return Err(
                        umm_store::fsutil::io_err(e, format!("inspecting {}", p.display())).into(),
                    )
                }
            }
        }

        let method = if f.writable || !hardlink_ok {
            Method::Copy
        } else {
            Method::Hardlink
        };
        let p = f.target.to_path(&game_root);
        let op = match fs::symlink_metadata(&p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Op::Add {
                path: f.target.clone(),
                method,
                src: f.source.clone(),
                hash: f.hash.clone(),
                size: f.size,
            },
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("inspecting {}", p.display())).into(),
                )
            }
            Ok(m) if m.file_type().is_symlink() => {
                return Err(DeployError::LinkInPath(f.target.to_string()))
            }
            Ok(m) if m.is_dir() => return Err(DeployError::TargetIsDir(f.target.to_string())),
            Ok(_) => Op::Replace {
                path: f.target.clone(),
                method,
                src: f.source.clone(),
                hash: f.hash.clone(),
                size: f.size,
                orig_hash: hash_file(&p).ctx(|| format!("hashing {}", p.display()))?,
            },
        };
        file_ops.push(op);
    }

    let mut dirs: Vec<RelPath> = new_dirs.into_values().collect();
    dirs.sort_by(|a, b| a.depth().cmp(&b.depth()).then_with(|| a.cmp(b)));
    let mut ops: Vec<Op> = dirs.into_iter().map(|path| Op::Mkdir { path }).collect();
    ops.extend(file_ops);

    Ok(Journal {
        version: JOURNAL_VERSION,
        game: plan.game.clone(),
        profile: plan.profile.clone(),
        game_root,
        backup_root,
        backup_in_game,
        started_unix: unix_now(),
        phase: Phase::Applying,
        ops,
    })
}

/// Saves the journal, then performs it. On an ordinary error everything is rolled back
/// before returning. A simulated crash returns at once, leaving the journal for recovery.
pub fn apply(
    game: &GameDir,
    journal: &mut Journal,
    faults: &mut Faults,
) -> Result<(), DeployError> {
    let jpath = game.journal_path();
    if jpath.exists() {
        return Err(DeployError::AlreadyDeployed(jpath));
    }
    faults.tick()?;
    journal.phase = Phase::Applying;
    journal.save(&jpath)?;

    let result = (|| {
        for op in &journal.ops {
            apply_op(journal, op, faults)?;
        }
        faults.tick()?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            journal.phase = Phase::Deployed;
            journal.save(&jpath)?;
            Ok(())
        }
        Err(e @ DeployError::InjectedCrash(_)) => Err(e),
        Err(cause) => {
            let cause = Box::new(cause);
            match revert(game, journal, &mut Faults::none()) {
                Ok(report) => Err(DeployError::RolledBack { cause, report }),
                Err(DeployError::RevertIncomplete(report)) => {
                    Err(DeployError::RollbackFailed { cause, report })
                }
                Err(other) => Err(DeployError::RollbackFailed {
                    cause,
                    report: RevertReport {
                        errors: vec![other.to_string()],
                        ..Default::default()
                    },
                }),
            }
        }
    }
}

fn apply_op(j: &Journal, op: &Op, faults: &mut Faults) -> Result<(), DeployError> {
    match op {
        Op::Mkdir { path } => {
            faults.tick()?;
            let p = j.target(path);
            match fs::create_dir(&p) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && p.is_dir() => Ok(()),
                Err(e) => {
                    Err(umm_store::fsutil::io_err(e, format!("creating {}", p.display())).into())
                }
            }
        }
        Op::Add {
            path, method, src, ..
        } => {
            let dst = j.target(path);
            if fs::symlink_metadata(&dst).is_ok() {
                return Err(DeployError::Changed(path.to_string()));
            }
            place(src, &dst, *method, faults)
        }
        Op::Replace {
            path, method, src, ..
        } => {
            let dst = j.target(path);
            let backup = j.backup(path);
            if !dst.is_file() {
                return Err(DeployError::Changed(path.to_string()));
            }
            faults.tick()?;
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
            }
            faults.tick()?;
            fs::rename(&dst, &backup).ctx(|| format!("backing up {}", dst.display()))?;
            place(src, &dst, *method, faults)
        }
    }
}

pub(crate) fn tmp_path(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dst.with_file_name(format!("{name}.umm-tmp"))
}

/// Puts a stored file at `dst`. A hardlink is created atomically; a copy goes to a temporary
/// sibling first and is renamed into place, so `dst` never holds a partial file.
fn place(src: &Path, dst: &Path, method: Method, faults: &mut Faults) -> Result<(), DeployError> {
    faults.tick()?;
    if method == Method::Hardlink && fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    let tmp = tmp_path(dst);
    fs::copy(src, &tmp).ctx(|| format!("copying {} to {}", src.display(), tmp.display()))?;
    clear_readonly(&tmp).ctx(|| format!("updating {}", tmp.display()))?;
    faults.tick()?;
    fs::rename(&tmp, dst).ctx(|| format!("placing {}", dst.display()))?;
    Ok(())
}

/// `\\?\C:\x` → `C:\x`, so journal paths stay readable and comparable.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

#[cfg(unix)]
fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fn existing(p: &Path) -> Option<std::fs::Metadata> {
        p.ancestors().find_map(|a| fs::metadata(a).ok())
    }
    match (existing(a), existing(b)) {
        (Some(x), Some(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

#[cfg(windows)]
fn same_volume(a: &Path, b: &Path) -> bool {
    fn prefix(p: &Path) -> Option<String> {
        let existing = p.ancestors().find(|a| a.exists())?;
        let c = fs::canonicalize(existing).ok()?;
        match c.components().next()? {
            Component::Prefix(px) => Some(px.as_os_str().to_string_lossy().to_lowercase()),
            _ => None,
        }
    }
    matches!((prefix(a), prefix(b)), (Some(x), Some(y)) if x == y)
}
