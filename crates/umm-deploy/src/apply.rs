use crate::journal::{is_folder_root, resolve, Journal, Method, Op, Phase, JOURNAL_VERSION};
use crate::revert::{revert, RevertReport};
use crate::{DeployError, Faults, Plan};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use umm_gamedef::{RelPath, IN_GAME_BACKUP_DIR};
use umm_store::fsutil::{clear_readonly, hash_file, unix_now, IoCtx};
use umm_store::GameDir;

/// Where the originals of files outside the game folder are kept, in the backups.
pub(crate) const EXTERNAL_BACKUPS: &str = ".external";

/// The spelling of target paths on disk. Mods are made for Windows, which ignores case, so a
/// mod's `Data/Textures/x.dds` goes in the game's `data/textures/` where the file system is
/// case-sensitive, and the folders mods add are spelled as the first one does.
#[derive(Default)]
struct Spelling {
    /// Each folder's key, and its spelling.
    dirs: BTreeMap<String, RelPath>,
}

impl Spelling {
    /// `rel` as spelled on disk, as far as it exists there; the rest as planned before, or as
    /// given. `to_path` places a path; `root` is where one without a named folder starts.
    fn of(&mut self, rel: &RelPath, root: &Path, to_path: &dyn Fn(&RelPath) -> PathBuf) -> RelPath {
        if cfg!(windows) {
            return rel.clone();
        }
        let part = |c: &str| RelPath::parse(c).expect("a path's own component");
        let mut out: Option<RelPath> = None;
        for c in rel.components() {
            let given = match &out {
                Some(o) => o.join(&part(c)),
                None => part(c),
            };
            let key = given.key();
            let next = match self.dirs.get(&key) {
                Some(s) => s.clone(),
                // A named folder's root stays as it is.
                None if out.is_none() && c.starts_with('@') => given,
                None if to_path(&given).exists() => given,
                None => {
                    let lower = c.to_lowercase();
                    let parent = out.as_ref().map_or_else(|| root.to_path_buf(), to_path);
                    let on_disk = fs::read_dir(&parent).ok().and_then(|rd| {
                        rd.flatten()
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .find(|n| n.to_lowercase() == lower)
                    });
                    match (on_disk, &out) {
                        (Some(n), Some(o)) => o.join(&part(&n)),
                        (Some(n), None) => part(&n),
                        (None, _) => given,
                    }
                }
            };
            self.dirs.insert(key, next.clone());
            out = Some(next);
        }
        out.unwrap_or_else(|| rel.clone())
    }
}

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

    // A named folder outside the game folder: absolute, and not a drive's root.
    for dir in plan.folders.values() {
        if !dir.is_absolute() || dir.parent().is_none() {
            return Err(DeployError::UnsafeGameRoot(dir.clone()));
        }
    }
    let to_path = |rel: &RelPath| resolve(&game_root, &plan.folders, rel);
    // Hard links only work within a drive: decided for each place files go.
    let link_ok = |rel: &RelPath| {
        let place = match rel
            .first()
            .and_then(|f| f.strip_prefix('@'))
            .and_then(|n| plan.folders.get(n))
        {
            Some(dir) => dir.clone(),
            None => game_root.clone(),
        };
        let probe = place
            .ancestors()
            .find(|a| a.exists())
            .map(Path::to_path_buf)
            .unwrap_or(place);
        !prefer_copy && same_volume(&game.mods_dir(), &probe)
    };
    let (backup_root, backup_in_game) = if same_volume(game.root(), &game_root) {
        (game.backups_dir(), false)
    } else {
        (game_root.join(IN_GAME_BACKUP_DIR), true)
    };

    let mut new_dirs: BTreeMap<String, RelPath> = BTreeMap::new();
    let mut file_ops = Vec::new();
    let mut spell = Spelling::default();
    for f in &plan.files {
        // As the folders on disk (or those already planned) spell it.
        let target = spell.of(&f.target, &game_root, &to_path);
        for dir in target.ancestors() {
            if new_dirs.contains_key(&dir.key()) {
                continue;
            }
            let p = to_path(&dir);
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

        let method = if f.writable || !link_ok(&target) {
            Method::Copy
        } else {
            Method::Hardlink
        };
        let p = to_path(&target);
        let op = match fs::symlink_metadata(&p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Op::Add {
                path: target.clone(),
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
                return Err(DeployError::LinkInPath(target.to_string()))
            }
            Ok(m) if m.is_dir() => return Err(DeployError::TargetIsDir(target.to_string())),
            Ok(_) => Op::Replace {
                path: target.clone(),
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
    for (i, e) in plan.external.iter().enumerate() {
        if !e.path.is_absolute() {
            return Err(DeployError::Changed(e.path.display().to_string()));
        }
        let orig_hash = match fs::symlink_metadata(&e.path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                return Err(umm_store::fsutil::io_err(
                    err,
                    format!("inspecting {}", e.path.display()),
                )
                .into())
            }
            Ok(m) if !m.is_file() => {
                return Err(DeployError::TargetIsDir(e.path.display().to_string()))
            }
            Ok(_) => Some(hash_file(&e.path).ctx(|| format!("hashing {}", e.path.display()))?),
        };
        let name = e
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ops.push(Op::External {
            path: e.path.clone(),
            src: e.source.clone(),
            hash: e.hash.clone(),
            orig_hash,
            backup: backup_root
                .join(EXTERNAL_BACKUPS)
                .join(format!("{i}-{name}")),
            tolerated: e.tolerated.clone(),
        });
    }
    // Files the settings create: the first setting creates one, the next ones add to it.
    let mut creating: Vec<PathBuf> = Vec::new();
    for s in &plan.ini {
        if !s.path.is_absolute() {
            return Err(DeployError::Changed(s.path.display().to_string()));
        }
        let text = match fs::read(&s.path) {
            Ok(b) => Some(String::from_utf8_lossy(&b).into_owned()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("reading {}", s.path.display())).into(),
                )
            }
        };
        let created = match &text {
            Some(_) => false,
            // A file the game has not written yet is left for it to write, unless it is one
            // of overrides that may be created.
            None if !s.create => continue,
            None if creating.contains(&s.path) => false,
            None => {
                creating.push(s.path.clone());
                true
            }
        };
        let orig = text
            .as_deref()
            .and_then(|t| crate::ini::get(t, &s.section, &s.key));
        if orig.as_deref() == Some(s.value.as_str()) {
            continue;
        }
        ops.push(Op::IniSet {
            section_added: !text
                .as_deref()
                .is_some_and(|t| crate::ini::has_section(t, &s.section)),
            path: s.path.clone(),
            section: s.section.clone(),
            key: s.key.clone(),
            value: s.value.clone(),
            orig,
            created,
        });
    }

    for l in &plan.lists {
        if !l.path.is_absolute() {
            return Err(DeployError::Changed(l.path.display().to_string()));
        }
        let text = match fs::read(&l.path) {
            Ok(b) => Some(l.format.decode(&b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("reading {}", l.path.display())).into(),
                )
            }
        };
        let current = text.as_deref().and_then(|t| l.format.get(t, &l.key));
        let have: Vec<String> = current.iter().flatten().map(|c| l.format.id(c)).collect();
        let added: Vec<String> = l
            .entries
            .iter()
            .filter(|e| !have.contains(&l.format.id(e)))
            .cloned()
            .collect();
        if added.is_empty() {
            continue;
        }
        ops.push(Op::ListAdd {
            path: l.path.clone(),
            key: l.key.clone(),
            format: l.format,
            added,
            created: text.is_none(),
            block_added: current.is_none(),
        });
    }

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
        folders: plan.folders.clone(),
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
            // A named folder's root may lack its parents too (a game not run yet).
            let made = if is_folder_root(&j.folders, path) {
                fs::create_dir_all(&p)
            } else {
                fs::create_dir(&p)
            };
            match made {
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
        Op::External {
            path,
            src,
            orig_hash,
            backup,
            ..
        } => {
            match orig_hash {
                Some(orig) => {
                    let same = path.is_file()
                        && hash_file(path).ctx(|| format!("hashing {}", path.display()))? == *orig;
                    if !same {
                        return Err(DeployError::Changed(path.display().to_string()));
                    }
                    // Copied, not moved: the file may be on another drive than the backups.
                    faults.tick()?;
                    if let Some(parent) = backup.parent() {
                        fs::create_dir_all(parent)
                            .ctx(|| format!("creating {}", parent.display()))?;
                    }
                    place(path, backup, Method::Copy, faults)?;
                }
                None => {
                    if fs::symlink_metadata(path).is_ok() {
                        return Err(DeployError::Changed(path.display().to_string()));
                    }
                    faults.tick()?;
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)
                            .ctx(|| format!("creating {}", parent.display()))?;
                    }
                }
            }
            place(src, path, Method::Copy, faults)
        }
        Op::IniSet {
            path,
            section,
            key,
            value,
            orig,
            created: true,
            ..
        } => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(DeployError::Changed(op_label(path, section, key)));
            }
            faults.tick()?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
            }
            debug_assert!(orig.is_none());
            write_text(
                path,
                &crate::ini::set("", section, key, Some(value)),
                faults,
            )
        }
        Op::IniSet {
            path,
            section,
            key,
            value,
            orig,
            created: false,
            ..
        } => {
            let text = read_text(path)?;
            if crate::ini::get(&text, section, key) != *orig {
                return Err(DeployError::Changed(op_label(path, section, key)));
            }
            let new = crate::ini::set(&text, section, key, Some(value));
            write_text(path, &new, faults)
        }
        Op::ListAdd {
            path,
            key,
            format,
            added,
            created,
            ..
        } => {
            let text = if *created {
                if fs::symlink_metadata(path).is_ok() {
                    return Err(DeployError::Changed(op.label()));
                }
                faults.tick()?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
                }
                String::new()
            } else {
                format.decode(&fs::read(path).ctx(|| format!("reading {}", path.display()))?)
            };
            let mut entries = format.get(&text, key).unwrap_or_default();
            for a in added {
                let id = format.id(a);
                if !entries.iter().any(|e| format.id(e) == id) {
                    entries.push(a.clone());
                }
            }
            // An `.lsx` file whose list is gone: changed since the plan.
            let new = format
                .set(&text, key, Some(&entries))
                .ok_or_else(|| DeployError::Changed(op.label()))?;
            write_bytes(path, &format.encode(&new), faults)
        }
    }
}

pub(crate) fn op_label(path: &Path, section: &str, key: &str) -> String {
    format!("{} [{section}] {key}", path.display())
}

pub(crate) fn read_text(path: &Path) -> Result<String, DeployError> {
    let b = fs::read(path).ctx(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

/// Replaces a file's text through a temporary sibling, so it never holds a partial file.
pub(crate) fn write_text(path: &Path, text: &str, faults: &mut Faults) -> Result<(), DeployError> {
    write_bytes(path, text.as_bytes(), faults)
}

/// [`write_text`] for bytes.
pub(crate) fn write_bytes(
    path: &Path,
    bytes: &[u8],
    faults: &mut Faults,
) -> Result<(), DeployError> {
    let tmp = tmp_path(path);
    faults.tick()?;
    fs::write(&tmp, bytes).ctx(|| format!("writing {}", tmp.display()))?;
    faults.tick()?;
    fs::rename(&tmp, path).ctx(|| format!("replacing {}", path.display()))?;
    Ok(())
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

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn targets_take_the_spelling_on_disk_then_the_first_planned() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("data/textures")).unwrap();
        let to_path = |r: &RelPath| r.to_path(root);
        let mut s = Spelling::default();
        let of = |s: &mut Spelling, p: &str| {
            s.of(&RelPath::parse(p).unwrap(), root, &to_path)
                .to_string()
        };
        assert_eq!(
            of(&mut s, "Data/Textures/Rock.dds"),
            "data/textures/Rock.dds"
        );
        assert_eq!(of(&mut s, "DATA/Meshes/a.nif"), "data/Meshes/a.nif");
        // A folder no mod had before: as the first spelled it.
        assert_eq!(of(&mut s, "data/meshes/b.nif"), "data/Meshes/b.nif");
        assert_eq!(of(&mut s, "@user/Mods/x"), "@user/Mods/x");
    }
}
