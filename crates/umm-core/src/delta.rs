//! Delta patches: mods made of `.xdelta` files (VCDIFF, see [`crate::vcdiff`]), each of which
//! rebuilds one of the game's files from the original (Undertale's mods patch its `data.win`).
//! Copied into the game folder, such a file does nothing until someone applies it.
//!
//! UMM applies it at deploy time to the file as it is then (the game's own, or another mod's
//! version of it, or the result of an earlier mod's patch: patches stack in the profile's
//! order), keeps the result, and deploys that in place of the file, so the original comes
//! back afterwards. The result is reused until the file or the patch changes. When a mod has
//! several patches for the same file (one per game version, as some do), the first that fits
//! is used.

use crate::patcher::patch_dir;
use crate::{vcdiff, Result, Session};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use umm_deploy::{Plan, PlannedFile, Skipped};
use umm_gamedef::{CompiledDef, RelPath};
use umm_store::fsutil::{hash_file, IoCtx};
use umm_store::ModMeta;

const RECORD: &str = "delta.json";

/// A patch as placed in the game folder, its stored copy and its hash.
type Patch = (RelPath, PathBuf, String);
const OUT: &str = "delta";

/// The extensions of delta patch files.
const EXTENSIONS: &[&str] = &[".xdelta", ".xdelta3", ".vcdiff", ".vcd"];

/// GameMaker games' data files, which their mods patch when the patch does not name one.
const GAMEMAKER_DATA: &[&str] = &["data.win", "game.win", "game.unx", "game.ios"];

/// Whether a file (a path inside the mod, as placed) is a delta patch.
pub fn is_delta_patch(path: &str) -> bool {
    let p = path.to_lowercase();
    EXTENSIONS
        .iter()
        .any(|e| p.len() > e.len() && p.ends_with(e))
}

/// What UMM keeps of a mod's applied patches.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    applied: Vec<Applied>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Applied {
    /// The game file patched.
    target: String,
    /// The patch used, as placed, and its hash.
    patch: String,
    patch_hash: String,
    /// What it was applied to: the game's file (its size and time) or another mod's (its
    /// hash).
    input: String,
    hash: String,
    size: u64,
}

fn load_record(dir: &Path) -> Record {
    fs::read(dir.join(RECORD))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The file named `name` (in any case) directly in `root`/`dir`, as spelled on disk, or in
/// the plan (another mod's).
fn find(root: &Path, plan: &Plan, dir: Option<&RelPath>, name: &str) -> Option<RelPath> {
    let rel = |n: &str| match dir {
        Some(d) => RelPath::parse(&format!("{}/{n}", d.as_str())).ok(),
        None => RelPath::parse(n).ok(),
    };
    let wanted = rel(name)?;
    if let Some(f) = plan.files.iter().find(|f| f.target.key() == wanted.key()) {
        return Some(f.target.clone());
    }
    let folder = match dir {
        Some(d) => d.to_path(root),
        None => root.to_path_buf(),
    };
    let found = fs::read_dir(folder).ok()?.flatten().find(|e| {
        e.file_type().is_ok_and(|t| t.is_file())
            && e.file_name().to_string_lossy().eq_ignore_ascii_case(name)
    })?;
    rel(&found.file_name().to_string_lossy())
}

/// The game file a patch (placed at `patch`) rebuilds: the one its header names, else one
/// named like the patch (`data.xdelta` for `data.win`), else a GameMaker game's data file;
/// next to the patch first, then at the top of the game folder.
fn target_of(root: &Path, plan: &Plan, patch: &RelPath, bytes: &[u8]) -> Option<RelPath> {
    let parent = patch.parent();
    let dirs: Vec<Option<&RelPath>> = match &parent {
        Some(p) => vec![Some(p), None],
        None => vec![None],
    };
    let mut names: Vec<String> = Vec::new();
    if let Some(n) = vcdiff::source_name(bytes) {
        names.push(n);
    }
    let file = patch.file_name().unwrap_or_default();
    let stem = file
        .rsplit_once('.')
        .map_or(file, |(s, _)| s)
        .to_lowercase();
    for d in &dirs {
        let folder = match d {
            Some(d) => d.to_path(root),
            None => root.to_path_buf(),
        };
        let in_folder = fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned());
        let in_plan = plan
            .files
            .iter()
            .filter(|f| f.target.parent().as_ref().map(|p| p.key()) == d.map(|d| d.key()))
            .map(|f| f.target.file_name().unwrap_or_default().to_string());
        let mut alike: Vec<String> = in_folder
            .chain(in_plan)
            .filter(|n| {
                !is_delta_patch(n)
                    && n.rsplit_once('.')
                        .is_some_and(|(s, _)| s.eq_ignore_ascii_case(&stem))
            })
            .collect();
        alike.sort();
        alike.dedup();
        if alike.len() == 1 {
            names.push(alike.remove(0));
        }
    }
    names.extend(GAMEMAKER_DATA.iter().map(|n| n.to_string()));
    names
        .iter()
        .find_map(|n| dirs.iter().find_map(|d| find(root, plan, *d, n)))
}

/// What a game file to patch is now: in the plan (another mod's, or an earlier patch's
/// result) or the game's own.
fn input_of(root: &Path, plan: &Plan, target: &RelPath) -> Option<(PathBuf, String)> {
    if let Some(f) = plan.files.iter().find(|f| f.target.key() == target.key()) {
        return Some((f.source.clone(), format!("file {}", f.hash)));
    }
    let p = target.to_path(root);
    let m = fs::metadata(&p).ok().filter(|m| m.is_file())?;
    let modified = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some((p, format!("game {} {modified}", m.len())))
}

impl Session {
    /// The mod's delta patches, as placed in the game folder, with their stored copies and
    /// hashes, by path.
    fn deltas_in(&self, def: &CompiledDef, meta: &ModMeta) -> Vec<Patch> {
        let mut v: Vec<_> = self
            .placed_files(def, meta)
            .into_iter()
            .filter(|(t, _, _)| is_delta_patch(t.as_str()))
            .collect();
        v.sort_by_key(|(t, _, _)| t.key());
        v
    }

    /// Puts a mod's patched files in a deploy plan in place of the files they patch, its
    /// patches themselves left out. Patches are applied when `run` (a deploy) and their
    /// results not kept yet; otherwise (a preview) they are left out with the reason.
    /// `None` when the mod has no delta patches; else the game files patched.
    pub(crate) fn add_deltas(
        &self,
        def: &CompiledDef,
        meta: &ModMeta,
        root: &Path,
        plan: &mut Plan,
        run: bool,
    ) -> Result<Option<Vec<RelPath>>> {
        let patches = self.deltas_in(def, meta);
        if patches.is_empty() {
            return Ok(None);
        }
        plan.files
            .retain(|f| !(f.owner == meta.id && is_delta_patch(f.target.as_str())));
        let skip = |plan: &mut Plan, file: &RelPath, reason: String| {
            plan.skipped.push(Skipped {
                owner: meta.id.clone(),
                file: file.as_str().to_string(),
                reason,
            })
        };

        // Each patch by the file it patches.
        let mut by_target: BTreeMap<String, (RelPath, Vec<Patch>)> = BTreeMap::new();
        for (patch, stored, hash) in patches {
            let bytes = fs::read(&stored).ctx(|| format!("reading {}", stored.display()))?;
            if !vcdiff::is_vcdiff(&bytes) {
                skip(plan, &patch, "not an xdelta (VCDIFF) patch".into());
                continue;
            }
            let Some(target) = target_of(root, plan, &patch, &bytes) else {
                skip(
                    plan,
                    &patch,
                    "UMM cannot tell which of the game's files this patch is for".into(),
                );
                continue;
            };
            by_target
                .entry(target.key())
                .or_insert_with(|| (target, Vec::new()))
                .1
                .push((patch, stored, hash));
        }

        let dir = patch_dir(&self.data.game(&def.def.id), &meta.id);
        let old = load_record(&dir);
        let mut record = Record::default();
        let mut patched = Vec::new();
        for (target, list) in by_target.into_values() {
            let Some((input, key)) = input_of(root, plan, &target) else {
                for (p, _, _) in &list {
                    skip(
                        plan,
                        p,
                        format!("the game has no {} to patch", target.as_str()),
                    );
                }
                continue;
            };
            let out = target.to_path(&dir.join(OUT));
            let kept = old.applied.iter().find(|a| {
                a.target == target.as_str()
                    && a.input == key
                    && list
                        .iter()
                        .any(|(p, _, h)| a.patch == p.as_str() && a.patch_hash == *h)
                    && fs::metadata(&out).is_ok_and(|m| m.len() == a.size)
            });
            let applied = match kept {
                Some(a) => a.clone(),
                None if !run => {
                    for (p, _, _) in &list {
                        skip(
                            plan,
                            p,
                            format!("applied to {} at the next deploy", target.as_str()),
                        );
                    }
                    continue;
                }
                None => {
                    let source = fs::read(&input).ctx(|| format!("reading {}", input.display()))?;
                    let mut errors = Vec::new();
                    let mut done = None;
                    for (p, stored, h) in &list {
                        let bytes =
                            fs::read(stored).ctx(|| format!("reading {}", stored.display()))?;
                        match vcdiff::apply(&source, &bytes) {
                            Ok(result) => {
                                if let Some(parent) = out.parent() {
                                    fs::create_dir_all(parent)
                                        .ctx(|| format!("creating {}", parent.display()))?;
                                }
                                umm_store::fsutil::write_atomic(&out, &result)
                                    .ctx(|| format!("writing {}", out.display()))?;
                                done = Some(Applied {
                                    target: target.as_str().to_string(),
                                    patch: p.as_str().to_string(),
                                    patch_hash: h.clone(),
                                    input: key.clone(),
                                    hash: hash_file(&out)
                                        .ctx(|| format!("hashing {}", out.display()))?,
                                    size: result.len() as u64,
                                });
                                break;
                            }
                            Err(e) => errors.push((p.clone(), e)),
                        }
                    }
                    match done {
                        Some(a) => a,
                        None => {
                            for (p, e) in errors {
                                skip(plan, &p, format!("not applied to {}: {e}", target.as_str()));
                            }
                            continue;
                        }
                    }
                }
            };
            for (p, _, _) in &list {
                if p.as_str() != applied.patch {
                    skip(
                        plan,
                        p,
                        format!(
                            "not used: {} patched {} first",
                            applied.patch,
                            target.as_str()
                        ),
                    );
                }
            }
            plan.files.retain(|f| f.target.key() != target.key());
            plan.files.push(PlannedFile {
                source: out,
                owner: meta.id.clone(),
                hash: applied.hash.clone(),
                size: applied.size,
                writable: false,
                target: target.clone(),
            });
            patched.push(target);
            record.applied.push(applied);
        }
        if run && record.applied != old.applied {
            fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
            let text = serde_json::to_vec_pretty(&record)
                .map_err(|e| crate::CoreError::Invalid(e.to_string()))?;
            umm_store::fsutil::write_atomic(&dir.join(RECORD), &text)
                .ctx(|| format!("writing {}", dir.join(RECORD).display()))?;
        }
        Ok(Some(patched))
    }
}
