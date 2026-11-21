use crate::apply::tmp_path;
use crate::journal::{Journal, Op, Phase};
use crate::{DeployError, Faults};
use std::fs;
use std::io;
use std::path::Path;
use umm_store::fsutil::{
    dir_is_empty, hash_file, move_file, remove_empty_dirs, remove_file_force, IoCtx,
};
use umm_store::GameDir;

#[derive(Debug, Clone, Default)]
pub struct RevertReport {
    /// Original game files put back.
    pub restored: usize,
    /// Mod files removed from the game folder.
    pub removed: usize,
    /// Files moved aside instead of deleted because their contents were not what UMM placed
    /// (the game or the user changed them).
    pub quarantined: Vec<String>,
    /// Game files that changed while mods were deployed (e.g. a game update); the new version
    /// was kept and the outdated backup quarantined.
    pub kept_game_versions: Vec<String>,
    /// Folders UMM created that were not empty, so were left in place.
    pub dirs_left: Vec<String>,
    pub errors: Vec<String>,
}

/// Undoes a journal. Each entry is checked against the disk rather than trusting the recorded
/// phase, so this is correct after a crash at any point and harmless to run twice.
/// The journal is deleted only when everything was undone.
pub fn revert(
    game: &GameDir,
    journal: &mut Journal,
    faults: &mut Faults,
) -> Result<RevertReport, DeployError> {
    let jpath = game.journal_path();
    faults.tick()?;
    journal.phase = Phase::Reverting;
    journal.save(&jpath)?;

    let quarantine = game.quarantine_dir().join(journal.started_unix.to_string());
    let mut report = RevertReport::default();
    for op in journal.ops.iter().rev() {
        match revert_op(journal, op, &quarantine, &mut report, faults) {
            Ok(()) => {}
            Err(e @ DeployError::InjectedCrash(_)) => return Err(e),
            Err(e) => report.errors.push(format!("{}: {e}", op.path())),
        }
    }

    faults.tick()?;
    remove_empty_dirs(&journal.backup_root);
    if report.errors.is_empty() && journal.backup_root.exists() {
        report.errors.push(format!(
            "backup folder {} still holds files",
            journal.backup_root.display()
        ));
    }
    if !report.errors.is_empty() {
        return Err(DeployError::RevertIncomplete(report));
    }
    faults.tick()?;
    fs::remove_file(&jpath).ctx(|| format!("removing {}", jpath.display()))?;
    Ok(report)
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// True if `dst` still holds exactly what UMM put there.
fn is_placed(dst: &Path, src: &Path, hash: &str, size: u64) -> io::Result<bool> {
    if same_file::is_same_file(dst, src).unwrap_or(false) {
        return Ok(true);
    }
    if fs::metadata(dst)?.len() != size {
        return Ok(false);
    }
    Ok(hash_file(dst)? == hash)
}

fn quarantine_file(
    path: &Path,
    rel: &umm_gamedef::RelPath,
    base: &Path,
) -> Result<(), DeployError> {
    let dst = rel.to_path(base);
    move_file(path, &dst)
        .ctx(|| format!("quarantining {} to {}", path.display(), dst.display()))?;
    Ok(())
}

fn revert_op(
    j: &Journal,
    op: &Op,
    quarantine: &Path,
    report: &mut RevertReport,
    faults: &mut Faults,
) -> Result<(), DeployError> {
    match op {
        Op::Mkdir { path } => {
            let p = j.target(path);
            if !p.is_dir() {
                return Ok(());
            }
            if dir_is_empty(&p).ctx(|| format!("reading {}", p.display()))? {
                faults.tick()?;
                fs::remove_dir(&p).ctx(|| format!("removing {}", p.display()))?;
            } else {
                report.dirs_left.push(path.to_string());
            }
            Ok(())
        }
        Op::Add {
            path,
            src,
            hash,
            size,
            ..
        } => {
            let dst = j.target(path);
            clean_tmp(&dst, faults)?;
            if !exists(&dst) {
                return Ok(());
            }
            faults.tick()?;
            if is_placed(&dst, src, hash, *size).ctx(|| format!("checking {}", dst.display()))? {
                remove_file_force(&dst).ctx(|| format!("removing {}", dst.display()))?;
                report.removed += 1;
            } else {
                quarantine_file(&dst, path, quarantine)?;
                report.quarantined.push(path.to_string());
            }
            Ok(())
        }
        Op::Replace {
            path,
            src,
            hash,
            size,
            orig_hash,
            ..
        } => {
            let dst = j.target(path);
            let backup = j.backup(path);
            clean_tmp(&dst, faults)?;
            let placed =
                || is_placed(&dst, src, hash, *size).ctx(|| format!("checking {}", dst.display()));
            let is_original = || {
                Ok::<_, DeployError>(
                    hash_file(&dst).ctx(|| format!("hashing {}", dst.display()))? == *orig_hash,
                )
            };

            if exists(&backup) {
                if !exists(&dst) {
                    faults.tick()?;
                } else if placed()? {
                    faults.tick()?;
                    remove_file_force(&dst).ctx(|| format!("removing {}", dst.display()))?;
                    report.removed += 1;
                    faults.tick()?;
                } else if is_original()? {
                    faults.tick()?;
                    remove_file_force(&backup).ctx(|| format!("removing {}", backup.display()))?;
                    return Ok(());
                } else {
                    // The game replaced the file while mods were deployed: keep its version.
                    faults.tick()?;
                    quarantine_file(&backup, path, quarantine)?;
                    report.kept_game_versions.push(path.to_string());
                    return Ok(());
                }
                move_file(&backup, &dst).ctx(|| format!("restoring {}", dst.display()))?;
                report.restored += 1;
                Ok(())
            } else if !exists(&dst) {
                Err(DeployError::Changed(format!(
                    "{path}: both the original file and its backup are missing"
                )))
            } else if is_original()? {
                Ok(()) // never replaced, or already restored
            } else if placed()? {
                Err(DeployError::Changed(format!(
                    "{path}: the backup of the original file is missing"
                )))
            } else {
                report.kept_game_versions.push(path.to_string());
                Ok(())
            }
        }
    }
}

fn clean_tmp(dst: &Path, faults: &mut Faults) -> Result<(), DeployError> {
    let tmp = tmp_path(dst);
    if exists(&tmp) {
        faults.tick()?;
        remove_file_force(&tmp).ctx(|| format!("removing {}", tmp.display()))?;
    }
    Ok(())
}
