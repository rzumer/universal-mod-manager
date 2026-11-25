//! Several stored versions of one mod (Thunderstore packages), one copy per version.
//!
//! A mod's own folder (`mods/<id>/`) holds its latest installed version, which every profile
//! uses unless it is pinned to another. Each other version a profile is pinned to is kept in
//! `mods/<id>/versions/<version>/`, laid out like the mod's folder (`mod.toml`, `files/`,
//! `package/`). Versions no profile is pinned to are pruned.

use crate::fsutil::{self, IoCtx};
use crate::mods::{load_mod, stage_into, ImportOptions, ModMeta, MOD_META_FILE};
use crate::{read_toml, GameDir, Result, StoreError};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folder in a mod's folder holding its other versions.
pub const VERSIONS_DIR: &str = "versions";

/// The parts of a mod's folder that make up one version.
const PARTS: [&str; 3] = [MOD_META_FILE, "files", "package"];

/// Whether `v` can be a folder name: one plain path component.
pub fn is_valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v != "."
        && v != ".."
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn version_dir(game: &GameDir, id: &str, version: &str) -> Result<PathBuf> {
    if !is_valid_version(version) {
        return Err(StoreError::InvalidId(version.to_string()));
    }
    Ok(game.mod_dir(id).join(VERSIONS_DIR).join(version))
}

/// The folder holding a version of a mod (`None`: its latest), with its metadata.
pub fn load_mod_version(
    game: &GameDir,
    id: &str,
    version: Option<&str>,
) -> Result<(ModMeta, PathBuf)> {
    let latest = load_mod(game, id)?;
    match version {
        None => Ok((latest, game.mod_dir(id))),
        Some(v) if latest.version.as_deref() == Some(v) => Ok((latest, game.mod_dir(id))),
        Some(v) => {
            let dir = version_dir(game, id, v)?;
            let p = dir.join(MOD_META_FILE);
            if !p.is_file() {
                return Err(StoreError::ModNotFound(format!("{id} {v}")));
            }
            Ok((read_toml(&p)?, dir))
        }
    }
}

/// The versions of a mod stored besides its latest one.
pub fn other_versions(game: &GameDir, id: &str) -> Vec<String> {
    let Ok(rd) = fs::read_dir(game.mod_dir(id).join(VERSIONS_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().join(MOD_META_FILE).is_file())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|v| is_valid_version(v))
        .collect();
    out.sort();
    out
}

/// Whether this version of the mod is stored, as its latest or another.
pub fn has_version(game: &GameDir, id: &str, version: &str) -> bool {
    load_mod_version(game, id, Some(version)).is_ok()
}

/// Stores a version of a mod that is not its latest, in its `versions/` folder. The mod must
/// already be stored; `opts.version` names the version. A version already stored is kept.
pub fn store_version(game: &GameDir, src: &Path, opts: &ImportOptions) -> Result<ModMeta> {
    let id = opts
        .id
        .clone()
        .ok_or_else(|| StoreError::InvalidId(String::new()))?;
    let version = opts
        .version
        .clone()
        .ok_or_else(|| StoreError::InvalidId(format!("{id} (no version)")))?;
    load_mod(game, &id)?;
    let dest = version_dir(game, &id, &version)?;
    if dest.join(MOD_META_FILE).is_file() {
        return read_toml(&dest.join(MOD_META_FILE));
    }
    let staging = game.staging_dir().join(fsutil::unique_suffix());
    fs::create_dir_all(&staging).ctx(|| format!("creating {}", staging.display()))?;
    let result = (|| -> Result<ModMeta> {
        let new_dir = staging.join("new");
        let meta = stage_into(src, &staging, &new_dir, opts)?;
        if dest.exists() {
            // An interrupted store: no mod.toml, so not a version.
            fs::remove_dir_all(&dest).ctx(|| format!("removing {}", dest.display()))?;
        }
        let parent = dest.parent().expect("under the mod's folder");
        fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        fs::rename(&new_dir, &dest)
            .ctx(|| format!("moving the version into {}", dest.display()))?;
        Ok(meta)
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Removes the stored versions of a mod that are not in `keep` (its latest always stays).
/// Returns the versions removed.
pub fn prune_versions(game: &GameDir, id: &str, keep: &HashSet<String>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for v in other_versions(game, id) {
        if keep.contains(&v) {
            continue;
        }
        let dir = version_dir(game, id, &v)?;
        fs::remove_dir_all(&dir).ctx(|| format!("removing {}", dir.display()))?;
        removed.push(v);
    }
    let versions = game.mod_dir(id).join(VERSIONS_DIR);
    if fs::read_dir(&versions).is_ok_and(|mut rd| rd.next().is_none()) {
        let _ = fs::remove_dir(&versions);
    }
    Ok(removed)
}

/// Makes a stored older version the mod's latest, deleting the latest it replaces (one no
/// profile uses any more). The mod's other versions stay.
pub fn promote_version(game: &GameDir, id: &str, version: &str) -> Result<()> {
    let from = version_dir(game, id, version)?;
    if !from.join(MOD_META_FILE).is_file() {
        return Err(StoreError::ModNotFound(format!("{id} {version}")));
    }
    let dest = game.mod_dir(id);
    let old = game.staging_dir().join(fsutil::unique_suffix());
    fs::create_dir_all(&old).ctx(|| format!("creating {}", old.display()))?;
    // The latest's parts aside, then the version's in their place; put back on failure.
    let mut moved: Vec<&str> = Vec::new();
    let mut placed: Vec<&str> = Vec::new();
    let result = (|| -> Result<()> {
        for part in PARTS {
            let p = dest.join(part);
            if p.exists() {
                fs::rename(&p, old.join(part)).map_err(|e| io_ctx(e, &p))?;
                moved.push(part);
            }
        }
        for part in PARTS {
            let p = from.join(part);
            if p.exists() {
                fs::rename(&p, dest.join(part)).map_err(|e| io_ctx(e, &p))?;
                placed.push(part);
            }
        }
        Ok(())
    })();
    if result.is_err() {
        for part in placed {
            let _ = fs::rename(dest.join(part), from.join(part));
        }
        for part in moved {
            let _ = fs::rename(old.join(part), dest.join(part));
        }
        let _ = fs::remove_dir_all(&old);
        return result;
    }
    let _ = fs::remove_dir(&from);
    let versions = dest.join(VERSIONS_DIR);
    if fs::read_dir(&versions).is_ok_and(|mut rd| rd.next().is_none()) {
        let _ = fs::remove_dir(&versions);
    }
    fs::remove_dir_all(&old).ctx(|| format!("removing {}", old.display()))?;
    Ok(())
}

/// What [`carry_versions`] moved, to move back if the update fails.
#[derive(Debug, Default)]
pub(crate) struct Carried {
    versions: bool,
    replaced: Option<String>,
}

/// Before an update replaces the mod in `dest` with the one staged in `new_dir`: moves its
/// other versions into `new_dir`, and with `keep_replaced` the replaced version too.
pub(crate) fn carry_versions(dest: &Path, new_dir: &Path, keep_replaced: bool) -> Result<Carried> {
    let mut c = Carried::default();
    let from = dest.join(VERSIONS_DIR);
    let to = new_dir.join(VERSIONS_DIR);
    if from.is_dir() {
        fs::rename(&from, &to).ctx(|| format!("moving {}", from.display()))?;
        c.versions = true;
    }
    if !keep_replaced {
        return Ok(c);
    }
    let old: Option<ModMeta> = read_toml(&dest.join(MOD_META_FILE)).ok();
    let new: Option<ModMeta> = read_toml(&new_dir.join(MOD_META_FILE)).ok();
    let Some(v) = old
        .and_then(|m| m.version)
        .filter(|v| is_valid_version(v))
        .filter(|v| new.and_then(|m| m.version).as_deref() != Some(v.as_str()))
    else {
        return Ok(c);
    };
    let at = to.join(&v);
    if at.join(MOD_META_FILE).is_file() {
        return Ok(c);
    }
    fs::create_dir_all(&at).ctx(|| format!("creating {}", at.display()))?;
    c.replaced = Some(v);
    for part in PARTS {
        let p = dest.join(part);
        if p.exists() {
            if let Err(e) = fs::rename(&p, at.join(part)) {
                uncarry_versions(dest, new_dir, &c);
                return Err(io_ctx(e, &p).into());
            }
        }
    }
    Ok(c)
}

fn io_ctx(e: io::Error, p: &Path) -> crate::IoError {
    fsutil::io_err(e, format!("moving {}", p.display()))
}

/// Undoes [`carry_versions`] after a failed update, as far as it can.
pub(crate) fn uncarry_versions(dest: &Path, new_dir: &Path, c: &Carried) {
    let to = new_dir.join(VERSIONS_DIR);
    if let Some(v) = &c.replaced {
        for part in PARTS {
            let p = to.join(v).join(part);
            if p.exists() {
                let _ = fs::rename(&p, dest.join(part));
            }
        }
        let _ = fs::remove_dir(to.join(v));
    }
    if c.versions {
        let _ = fs::rename(&to, dest.join(VERSIONS_DIR));
    }
}
