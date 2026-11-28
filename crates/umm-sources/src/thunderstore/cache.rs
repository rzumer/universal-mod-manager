//! Downloaded data kept on disk. A copy younger than its maximum age is used without asking
//! the network; an older one is refreshed, and kept if the network fails.

use crate::thunderstore::{io_err, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Downloaded just now.
    Fetched,
    /// A cached copy young enough to use.
    Cached,
    /// The network failed; this cached copy is older than wanted.
    Stale,
}

pub fn age(path: &Path) -> Option<Duration> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default(),
    )
}

/// Returns the bytes at `path`, refreshing them with `fetch` when missing, older than
/// `max_age`, or when `force` is set.
pub fn cached(
    path: &Path,
    max_age: Duration,
    force: bool,
    fetch: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<(Vec<u8>, Freshness)> {
    let fresh = !force && age(path).is_some_and(|a| a <= max_age);
    if fresh {
        if let Ok(b) = fs::read(path) {
            return Ok((b, Freshness::Cached));
        }
    }
    match fetch() {
        Ok(bytes) => {
            write_atomic(path, &bytes)?;
            Ok((bytes, Freshness::Fetched))
        }
        Err(e) => match fs::read(path) {
            Ok(b) => Ok((b, Freshness::Stale)),
            Err(_) => Err(e),
        },
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(io_err(format!("creating {}", dir.display())))?;
    let tmp: PathBuf = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(io_err(format!("writing {}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(io_err(format!("replacing {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thunderstore::SourceError;

    #[test]
    fn uses_fresh_copies_and_falls_back_when_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x.json");
        let (b, f) = cached(&p, Duration::from_secs(60), false, || Ok(b"1".to_vec())).unwrap();
        assert_eq!((b.as_slice(), f), (&b"1"[..], Freshness::Fetched));
        let (_, f) = cached(&p, Duration::from_secs(60), false, || panic!("no fetch")).unwrap();
        assert_eq!(f, Freshness::Cached);
        let offline = || Err(SourceError::Invalid("offline".into()));
        let (b, f) = cached(&p, Duration::from_secs(60), true, offline).unwrap();
        assert_eq!((b.as_slice(), f), (&b"1"[..], Freshness::Stale));
        let missing = tmp.path().join("none.json");
        assert!(cached(&missing, Duration::ZERO, false, offline).is_err());
    }
}
