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
            Err(e) => report.errors.push(format!("{}: {e}", op.label())),
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
        Op::IniSet {
            path,
            section,
            key,
            value,
            orig,
            created,
            section_added,
        } => {
            clean_tmp(path, faults)?;
            if !exists(path) {
                return Ok(()); // the file is gone: nothing to put back
            }
            let text = crate::apply::read_text(path)?;
            let now = crate::ini::get(&text, section, key);
            if now.as_deref() == Some(value.as_str()) {
                let mut back = crate::ini::set(&text, section, key, orig.as_deref());
                if *section_added {
                    back = crate::ini::remove_empty_section(&back, section);
                }
                if *created && !crate::ini::has_keys(&back) {
                    // UMM made the file and nothing else was put in it.
                    faults.tick()?;
                    remove_file_force(path).ctx(|| format!("removing {}", path.display()))?;
                } else {
                    crate::apply::write_text(path, &back, faults)?;
                }
                report.restored += 1;
            } else if now != *orig {
                // Changed while deployed (by the user or the game): keep their value.
                report
                    .kept_game_versions
                    .push(crate::apply::op_label(path, section, key));
            }
            Ok(())
        }
        Op::ListAdd {
            path,
            key,
            format,
            added,
            created,
            block_added,
        } => {
            clean_tmp(path, faults)?;
            if !exists(path) {
                return Ok(()); // the file is gone: nothing to take out
            }
            let text =
                format.decode(&fs::read(path).ctx(|| format!("reading {}", path.display()))?);
            let Some(entries) = format.get(&text, key) else {
                return Ok(()); // the game dropped the list
            };
            // By what makes them the same: the game may have written UMM's own differently.
            let ours: Vec<String> = added.iter().map(|a| format.id(a)).collect();
            let left: Vec<String> = entries
                .iter()
                .filter(|e| !ours.contains(&format.id(e)))
                .cloned()
                .collect();
            if left.len() == entries.len() {
                return Ok(());
            }
            let back = if left.is_empty() && *block_added {
                format.set(&text, key, None)
            } else {
                format.set(&text, key, Some(&left))
            };
            let Some(back) = back else {
                return Ok(());
            };
            faults.tick()?;
            if *created && back.trim().is_empty() {
                // UMM made the file and nothing else was put in it.
                remove_file_force(path).ctx(|| format!("removing {}", path.display()))?;
            } else {
                crate::apply::write_bytes(path, &format.encode(&back), faults)?;
            }
            report.restored += 1;
            Ok(())
        }
        Op::External {
            path,
            hash,
            orig_hash,
            backup,
            tolerated,
            ..
        } => revert_external(
            path,
            hash,
            orig_hash.as_deref(),
            backup,
            tolerated,
            quarantine,
            report,
            faults,
        ),
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

/// Copies `from` over `to` through a temporary sibling, so `to` never holds a partial file.
fn copy_into(from: &Path, to: &Path, faults: &mut Faults) -> Result<(), DeployError> {
    let tmp = tmp_path(to);
    faults.tick()?;
    fs::copy(from, &tmp).ctx(|| format!("copying {} to {}", from.display(), tmp.display()))?;
    umm_store::fsutil::clear_readonly(&tmp).ctx(|| format!("updating {}", tmp.display()))?;
    faults.tick()?;
    fs::rename(&tmp, to).ctx(|| format!("restoring {}", to.display()))?;
    Ok(())
}

/// Undoes a file written outside the game folder: the original comes back (or the file goes,
/// if there was none). A version neither UMM's nor the original (the game or the user changed
/// it meanwhile) is set aside in quarantine first.
/// Whether a list file holds only entries the game may write itself (and comments): the game
/// rewriting its plugin list on start (Fallout 3 adds its header and its own masters).
fn only_tolerated(path: &Path, tolerated: &[String]) -> bool {
    if tolerated.is_empty() {
        return false;
    }
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.trim_start_matches('*').to_lowercase())
        .all(|l| tolerated.contains(&l))
}

#[allow(clippy::too_many_arguments)]
fn revert_external(
    path: &Path,
    hash: &str,
    orig_hash: Option<&str>,
    backup: &Path,
    tolerated: &[String],
    quarantine: &Path,
    report: &mut RevertReport,
    faults: &mut Faults,
) -> Result<(), DeployError> {
    clean_tmp(path, faults)?;
    clean_tmp(backup, faults)?;
    let now = if exists(path) {
        Some(hash_file(path).ctx(|| format!("hashing {}", path.display()))?)
    } else {
        None
    };
    let name = path.file_name().map(|n| n.to_owned()).unwrap_or_default();
    let set_aside = |report: &mut RevertReport, faults: &mut Faults| -> Result<(), DeployError> {
        if only_tolerated(path, tolerated) {
            return Ok(());
        }
        let q = quarantine.join(crate::apply::EXTERNAL_BACKUPS).join(&name);
        faults.tick()?;
        if let Some(parent) = q.parent() {
            fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        }
        fs::copy(path, &q).ctx(|| format!("quarantining {}", path.display()))?;
        report.quarantined.push(path.display().to_string());
        Ok(())
    };
    match orig_hash {
        Some(orig) if exists(backup) => {
            if now.as_deref() == Some(orig) {
                // Never replaced, or already restored.
            } else {
                if now.as_deref().is_some_and(|n| n != hash) {
                    set_aside(report, faults)?;
                }
                copy_into(backup, path, faults)?;
                report.restored += 1;
            }
            faults.tick()?;
            remove_file_force(backup).ctx(|| format!("removing {}", backup.display()))?;
            Ok(())
        }
        Some(orig) => match now.as_deref() {
            Some(n) if n == orig => Ok(()),
            None => Err(DeployError::Changed(format!(
                "{}: both the original file and its backup are missing",
                path.display()
            ))),
            Some(n) if n == hash => Err(DeployError::Changed(format!(
                "{}: the backup of the original file is missing",
                path.display()
            ))),
            Some(_) => {
                report.kept_game_versions.push(path.display().to_string());
                Ok(())
            }
        },
        None => {
            if let Some(n) = &now {
                if n != hash {
                    set_aside(report, faults)?;
                }
                faults.tick()?;
                remove_file_force(path).ctx(|| format!("removing {}", path.display()))?;
                report.removed += 1;
            }
            Ok(())
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
