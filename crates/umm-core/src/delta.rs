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
//!
//! Which file a patch is for is found by trying it: xdelta3 checks each part of what it
//! rebuilds, so decoding the patch's first part against a file tells whether it was made from
//! it. The files tried, in order: the one it was applied to before, the one its header names
//! or named like it, then every GameMaker data file of the game (DELTARUNE has one per
//! chapter). A patch made for the game's own file, when another mod already replaced or
//! patched that file, clashes with it: deploying stops until one of them is disabled.

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

/// Where a patch goes, as found by trying it.
enum Found {
    /// The file to patch, as it is in the plan (or the game's own).
    Target(RelPath),
    /// It was made for the game's own file, which another mod (named) replaced or patched.
    Clash(RelPath, String),
    /// It fits none of the files tried.
    NoFit,
    /// No file to try.
    Unknown,
}

/// Whether a file (a path inside the mod, as placed) is a delta patch.
pub fn is_delta_patch(path: &str) -> bool {
    let p = path.to_lowercase();
    EXTENSIONS
        .iter()
        .any(|e| p.len() > e.len() && p.ends_with(e))
}

/// A mod file UMM cannot apply, and why: GameMaker modding formats that need another tool.
pub fn unsupported_patch(path: &str) -> Option<&'static str> {
    let p = path.to_lowercase();
    if p.ends_with(".csx") {
        Some("an UndertaleModTool script (.csx), which UMM cannot run")
    } else if p.ends_with(".g3mpatch") {
        Some("a G3M patch (.g3mpatch), which UMM cannot apply")
    } else {
        None
    }
}

/// Whether a file is a program for applying delta patches by hand (`DeltaPatcher.exe`,
/// `xdelta3.exe`, `xdeltaUI.exe`), shipped beside the patches.
fn is_patch_tool(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    name.ends_with(".exe") && (name.contains("xdelta") || name.contains("deltapatcher"))
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

/// The game files a patch (placed at `patch`) may rebuild, likeliest first: the one its
/// header names, one named like the patch (`data.xdelta` for `data.win`), a GameMaker game's
/// data file, next to the patch first, then at the top of the game folder; then every
/// GameMaker data file in the game and the plan.
fn candidates(root: &Path, plan: &Plan, patch: &RelPath, bytes: &[u8]) -> Vec<RelPath> {
    let mut out: Vec<RelPath> = named_targets(root, plan, patch, bytes);
    out.extend(
        umm_gamedef::gamemaker::data_files(root)
            .iter()
            .filter_map(|p| RelPath::parse(p).ok()),
    );
    out.extend(plan.files.iter().map(|f| f.target.clone()).filter(|t| {
        t.as_str().rsplit_once('.').is_some_and(|(_, e)| {
            umm_gamedef::gamemaker::DATA_EXTENSIONS
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
    }));
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.key()));
    out
}

/// The files a patch names, or is named like (see [`candidates`]).
fn named_targets(root: &Path, plan: &Plan, patch: &RelPath, bytes: &[u8]) -> Vec<RelPath> {
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
        .flat_map(|n| dirs.iter().filter_map(|d| find(root, plan, *d, n)))
        .collect()
}

/// The game's own file at `target`, and its state.
fn game_file(root: &Path, target: &RelPath) -> Option<(PathBuf, String)> {
    let p = target.to_path(root);
    let m = fs::metadata(&p).ok().filter(|m| m.is_file())?;
    let modified = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some((p, format!("game {} {modified}", m.len())))
}

/// The first `n` bytes of a file (all of it, when shorter).
fn read_prefix(path: &Path, n: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    fs::File::open(path)?.take(n).read_to_end(&mut buf)?;
    Ok(buf)
}

/// What a game file to patch is now: in the plan (another mod's, or an earlier patch's
/// result) or the game's own.
fn input_of(root: &Path, plan: &Plan, target: &RelPath) -> Option<(PathBuf, String)> {
    if let Some(f) = plan.files.iter().find(|f| f.target.key() == target.key()) {
        return Some((f.source.clone(), format!("file {}", f.hash)));
    }
    game_file(root, target)
}

/// Whether the game folder holds a GameMaker data file at its top.
pub(crate) fn is_gamemaker(root: &Path) -> bool {
    GAMEMAKER_DATA
        .iter()
        .any(|n| umm_gamedef::gamemaker::is_data_file(&root.join(n)))
}

/// For a GameMaker game: leaves out of the plan the files UMM cannot apply (UndertaleModTool
/// scripts, G3M patches), with the reason. They would do nothing in the game folder.
pub(crate) fn skip_unsupported(root: &Path, plan: &mut Plan) {
    if !plan
        .files
        .iter()
        .any(|f| unsupported_patch(f.target.as_str()).is_some())
        || !is_gamemaker(root)
    {
        return;
    }
    let mut kept = Vec::new();
    for f in std::mem::take(&mut plan.files) {
        match unsupported_patch(f.target.as_str()) {
            Some(why) => plan.skipped.push(Skipped {
                owner: f.owner.clone(),
                file: f.target.as_str().to_string(),
                reason: format!("{why}; the mod may not work without it"),
            }),
            None => kept.push(f),
        }
    }
    plan.files = kept;
}

impl Session {
    /// Whether the patch fits the file at `path` (whose state is `key`): `None` when it
    /// cannot tell. Remembered for the session.
    fn delta_fits(&self, patch: &[u8], patch_hash: &str, path: &Path, key: &str) -> Option<bool> {
        // The file's path too: two files can share their size and time.
        let id = format!("{patch_hash} {} {key}", path.display());
        if let Some(r) = self
            .delta_fits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
        {
            return *r;
        }
        let r = (|| {
            let len = fs::metadata(path).ok()?.len();
            if len < vcdiff::source_needed(patch).ok()? {
                return Some(false);
            }
            let (end, checked) = vcdiff::first_source_window(patch).ok()??;
            if !checked {
                return None;
            }
            let source = read_prefix(path, end).ok()?;
            vcdiff::fits(&source, patch).ok().flatten()
        })();
        self.delta_fits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, r);
        r
    }

    /// Where a patch goes: a clash when it fits the game's own file that another mod changed
    /// (applied to that mod's version, it would rebuild the game's file with its own changes
    /// only, dropping the other mod's: a patch always rebuilds the whole file it was made
    /// to); else the first file it fits as it is in the plan (another mod's version, which
    /// the patch was made on top of, or the game's own); else the likeliest named file when
    /// the patch cannot tell; else no fit. `hint`: the file it was applied to before.
    #[allow(clippy::too_many_arguments)]
    fn find_target(
        &self,
        game_id: &str,
        root: &Path,
        plan: &Plan,
        patch: &RelPath,
        bytes: &[u8],
        hash: &str,
        hint: Option<RelPath>,
    ) -> Found {
        let mut named = named_targets(root, plan, patch, bytes);
        named.extend(hint.clone());
        let mut tried: Vec<RelPath> = hint.into_iter().collect();
        tried.extend(candidates(root, plan, patch, bytes));
        let mut seen = std::collections::HashSet::new();
        tried.retain(|t| seen.insert(t.key()));
        // Made for the game's own file, which another mod changed.
        for t in &tried {
            let Some(other) = plan.files.iter().find(|f| f.target.key() == t.key()) else {
                continue;
            };
            let Some((own, key)) = game_file(root, t) else {
                continue;
            };
            if self.delta_fits(bytes, hash, &own, &key) == Some(true) {
                let name = umm_store::load_mod(&self.data.game(game_id), &other.owner)
                    .map(|m| m.name)
                    .unwrap_or_else(|_| other.owner.clone());
                return Found::Clash(t.clone(), name);
            }
        }
        let mut unsure: Option<RelPath> = None;
        let mut any = false;
        for t in &tried {
            let Some((input, key)) = input_of(root, plan, t) else {
                continue;
            };
            any = true;
            match self.delta_fits(bytes, hash, &input, &key) {
                Some(true) => return Found::Target(t.clone()),
                None if unsure.is_none() && named.iter().any(|n| n.key() == t.key()) => {
                    unsure = Some(t.clone())
                }
                _ => {}
            }
        }
        match unsure {
            Some(t) => Found::Target(t),
            None if any => Found::NoFit,
            None => Found::Unknown,
        }
    }

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
        manifest: Option<&crate::deltamod::Manifest>,
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
        // The tools mods ship for applying their patches by hand (Delta Patcher, xdelta3)
        // have nothing to do in the game folder.
        let tools: Vec<RelPath> = plan
            .files
            .iter()
            .filter(|f| f.owner == meta.id && is_patch_tool(f.target.as_str()))
            .map(|f| f.target.clone())
            .collect();
        plan.files
            .retain(|f| !(f.owner == meta.id && is_patch_tool(f.target.as_str())));
        for t in tools {
            skip(
                plan,
                &t,
                "a tool for applying the mod's patch by hand; UMM applies it itself".into(),
            );
        }

        let dir = patch_dir(&self.data.game(&def.def.id), &meta.id);
        let old = load_record(&dir);
        // Each patch by the file it patches.
        let mut by_target: BTreeMap<String, (RelPath, Vec<Patch>)> = BTreeMap::new();
        let mut clashes = Vec::new();
        for (patch, stored, hash) in patches {
            let bytes = fs::read(&stored).ctx(|| format!("reading {}", stored.display()))?;
            if !vcdiff::is_vcdiff(&bytes) {
                skip(plan, &patch, "not an xdelta (VCDIFF) patch".into());
                continue;
            }
            // The file its package's list names, else the one it was applied to before.
            let hint = manifest
                .and_then(|m| m.targets.get(&patch.key()).cloned())
                .or_else(|| {
                    old.applied
                        .iter()
                        .find(|a| a.patch == patch.as_str() && a.patch_hash == hash)
                        .and_then(|a| RelPath::parse(&a.target).ok())
                });
            let target = match self.find_target(
                &def.def.id,
                root,
                plan,
                &patch,
                &bytes,
                &hash,
                hint,
            ) {
                Found::Target(t) => t,
                Found::Clash(t, other) => {
                    clashes.push((other.clone(), t.as_str().to_string()));
                    skip(
                        plan,
                        &patch,
                        format!(
                            "clashes with {other}: both patch {}, each made for the game's own file; disable one of them",
                            t.as_str()
                        ),
                    );
                    continue;
                }
                Found::NoFit => {
                    let made_for = manifest
                        .and_then(|m| m.game_version.as_deref())
                        .map(|v| format!(" (the mod says it is for version {v})"))
                        .unwrap_or_default();
                    skip(
                        plan,
                        &patch,
                        format!(
                            "fits none of the game's files: made for another version of the game?{made_for}"
                        ),
                    );
                    continue;
                }
                Found::Unknown => {
                    skip(
                        plan,
                        &patch,
                        "UMM cannot tell which of the game's files this patch is for".into(),
                    );
                    continue;
                }
            };
            by_target
                .entry(target.key())
                .or_insert_with(|| (target, Vec::new()))
                .1
                .push((patch, stored, hash));
        }

        if run && !clashes.is_empty() {
            // By the other mod: "A and B both patch x, y and z".
            let mut others: Vec<(String, Vec<String>)> = Vec::new();
            for (other, file) in clashes {
                match others.iter_mut().find(|(o, _)| *o == other) {
                    Some((_, files)) => files.push(file),
                    None => others.push((other, vec![file])),
                }
            }
            let list = |files: &[String]| match files {
                [one] => one.clone(),
                [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                [] => String::new(),
            };
            let what: Vec<String> = others
                .iter()
                .map(|(other, files)| {
                    format!(
                        "{} and {other} both patch {}, each made for the game's own files",
                        meta.name,
                        list(files)
                    )
                })
                .collect();
            return Err(crate::CoreError::PatchClash(format!(
                "{}. UMM cannot use them together: disable one of them to launch.",
                what.join("; ")
            )));
        }
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
