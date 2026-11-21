//! The per-profile runtime folder used by Doorstop-style loaders.
//!
//! It is UMM's own folder, so it is synced rather than journaled: files the plan no longer
//! needs are removed, missing or outdated ones are placed, and files the game created or
//! edited there (configs, logs) are left alone.

use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use umm_deploy::PlannedFile;
use umm_gamedef::RelPath;
use umm_store::fsutil::{clear_readonly, hash_file, remove_file_force, write_atomic, IoCtx};

const MANIFEST: &str = ".umm-runtime.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    /// Path → hash of what UMM last placed there.
    files: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub placed: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// Files the game edited; UMM's copy was not written over them.
    pub kept_edited: Vec<String>,
}

fn hash_of(p: &Path) -> Option<String> {
    hash_file(p).ok()
}

/// Removes the folders above `file` that are now empty (a removed mod's own folder, say),
/// stopping at the first that is not, and never removing `root` itself.
fn remove_empty_parents(file: &Path, root: &Path) {
    let mut d = file.parent();
    while let Some(p) = d {
        if p == root || !p.starts_with(root) {
            break;
        }
        // Fails, and so stops, once a folder still holds something.
        if fs::remove_dir(p).is_err() {
            break;
        }
        d = p.parent();
    }
}

pub fn sync(dir: &Path, files: &[PlannedFile]) -> Result<SyncReport> {
    fs::create_dir_all(dir).ctx(|| format!("creating {}", dir.display()))?;
    let mpath = dir.join(MANIFEST);
    let old: Manifest = fs::read(&mpath)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut new = Manifest::default();
    let mut report = SyncReport::default();

    let wanted: BTreeSet<String> = files
        .iter()
        .map(|f| String::from(f.target.clone()))
        .collect();
    for (path, placed_hash) in &old.files {
        if wanted.contains(path) {
            continue;
        }
        let p = RelPath::parse(path)?.to_path(dir);
        if p.is_file() && hash_of(&p).as_deref() == Some(placed_hash.as_str()) {
            remove_file_force(&p).ctx(|| format!("removing {}", p.display()))?;
            report.removed += 1;
            remove_empty_parents(&p, dir);
        }
    }

    for f in files {
        let key = String::from(f.target.clone());
        let p = f.target.to_path(dir);
        if p.is_file() {
            if same_file::is_same_file(&p, &f.source).unwrap_or(false) {
                new.files.insert(key, f.hash.clone());
                report.unchanged += 1;
                continue;
            }
            let current = hash_of(&p);
            if current.as_deref() == Some(f.hash.as_str()) {
                new.files.insert(key, f.hash.clone());
                report.unchanged += 1;
                continue;
            }
            let ours = old.files.get(&key);
            if f.writable && current.as_deref() != ours.map(String::as_str) {
                // The game created or edited this file: its version wins.
                if let Some(h) = ours {
                    new.files.insert(key.clone(), h.clone());
                }
                report.kept_edited.push(key);
                continue;
            }
            remove_file_force(&p).ctx(|| format!("replacing {}", p.display()))?;
        }
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        }
        if f.writable || fs::hard_link(&f.source, &p).is_err() {
            fs::copy(&f.source, &p).ctx(|| format!("copying {}", f.source.display()))?;
            clear_readonly(&p).ctx(|| format!("updating {}", p.display()))?;
        }
        new.files.insert(key, f.hash.clone());
        report.placed += 1;
    }

    let bytes = serde_json::to_vec_pretty(&new).expect("manifest serializes");
    write_atomic(&mpath, &bytes).ctx(|| format!("writing {}", mpath.display()))?;
    Ok(report)
}
