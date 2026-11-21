//! Small filesystem helpers shared by the store and the deployment engine.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// An I/O error with a note of what was being done, so messages name the file involved.
#[derive(Debug, thiserror::Error)]
#[error("{context}: {source}")]
pub struct IoError {
    pub context: String,
    #[source]
    pub source: io::Error,
}

pub trait IoCtx<T> {
    fn ctx<F: FnOnce() -> String>(self, f: F) -> Result<T, IoError>;
}

impl<T> IoCtx<T> for io::Result<T> {
    fn ctx<F: FnOnce() -> String>(self, f: F) -> Result<T, IoError> {
        self.map_err(|source| IoError {
            context: f(),
            source,
        })
    }
}

pub fn io_err(source: io::Error, context: impl Into<String>) -> IoError {
    IoError {
        context: context.into(),
        source,
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

/// BLAKE3 hash of a file's contents, as `b3:<hex>`.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("b3:{}", h.finalize().to_hex()))
}

/// Writes a file so that readers see either the old or the new contents, never a mix:
/// write a temporary sibling, flush it to disk, then rename it over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", unique_suffix()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn clear_readonly(path: &Path) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    let mut perms = meta.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Removes a file, clearing the read-only flag first if Windows refuses.
pub fn remove_file_force(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            clear_readonly(path)?;
            fs::remove_file(path)
        }
        r => r,
    }
}

/// Moves a file, creating the destination's parent folders. Falls back to copy-then-delete
/// when a rename is impossible (different drives).
pub fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            if !src.is_file() {
                return Err(rename_err);
            }
            fs::copy(src, dst)?;
            remove_file_force(src)
        }
    }
}

/// Removes every empty folder under `root`, deepest first, then `root` itself if empty.
/// Non-empty folders are left alone.
pub fn remove_empty_dirs(root: &Path) {
    if !root.is_dir() {
        return;
    }
    let dirs: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .contents_first(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    for d in dirs {
        let _ = fs::remove_dir(&d);
    }
}

/// True if `dir` does not exist or holds no entries.
pub fn dir_is_empty(dir: &Path) -> io::Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut it) => Ok(it.next().is_none()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

pub fn count_files(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}
