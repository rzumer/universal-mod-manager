//! Picking up a file the user downloads in their browser. Free Nexus Mods accounts can only
//! download through the website, so UMM opens the file's page and waits for a new archive
//! to appear in the Downloads folder. Only file names and sizes there are looked at, and
//! only the new archive is read.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Waits for a new archive in one folder.
pub struct DownloadWatch {
    dir: PathBuf,
    since: SystemTime,
    /// What was there when the watch began, with each file's modification time.
    before: HashMap<String, Option<SystemTime>>,
    /// Sizes seen at the last poll: a file counts once its size holds between two polls.
    sizes: Mutex<HashMap<PathBuf, u64>>,
    /// New archives found not to be the download awaited.
    ignored: Mutex<Vec<PathBuf>>,
    /// The awaited file's size, when Nexus says: a file of another size is not it, or not
    /// finished (a resumable download saves its progress in steps).
    size: Option<u64>,
}

/// The user's Downloads folder.
pub fn downloads_dir() -> Option<PathBuf> {
    dirs::download_dir().filter(|d| d.is_dir())
}

fn modified(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl DownloadWatch {
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        let mut before = HashMap::new();
        for e in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
            before.insert(
                e.file_name().to_string_lossy().into_owned(),
                modified(&e.path()),
            );
        }
        Ok(DownloadWatch {
            dir: dir.to_path_buf(),
            since: SystemTime::now(),
            before,
            sizes: Mutex::new(HashMap::new()),
            ignored: Mutex::new(Vec::new()),
            size: None,
        })
    }

    /// Waits for a file of this size only.
    pub fn with_size(mut self, size: Option<u64>) -> Self {
        self.size = size.filter(|&s| s > 0);
        self
    }

    /// Passes over this archive from now on: it turned out to be another download.
    pub fn ignore(&self, path: &Path) {
        self.ignored
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(path.to_path_buf());
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new, finished archive, if one has appeared: a .zip, .7z or .rar that was not there
    /// (or has changed) since the watch began, whose size held since the last poll. Browsers
    /// write downloads under another name (`.crdownload`, `.part`) until they finish.
    pub fn poll(&self) -> Option<PathBuf> {
        let mut sizes = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
        let ignored = self.ignored.lock().unwrap_or_else(|e| e.into_inner());
        let mut found = None;
        for e in fs::read_dir(&self.dir).ok()?.filter_map(|e| e.ok()) {
            let path = e.path();
            if umm_store::archive_kind(&path).is_none()
                || !path.is_file()
                || ignored.contains(&path)
            {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let changed = match self.before.get(&name) {
                None => true,
                Some(old) => modified(&path) != *old && modified(&path) >= Some(self.since),
            };
            if !changed {
                continue;
            }
            let Ok(size) = e.metadata().map(|m| m.len()) else {
                continue;
            };
            let steady = sizes.get(&path) == Some(&size)
                && size > 0
                && self.size.is_none_or(|want| want == size);
            sizes.insert(path.clone(), size);
            if steady && found.is_none() {
                found = Some(path);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_a_new_finished_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("old.zip"), "old").unwrap();
        fs::write(d.join("notes.txt"), "x").unwrap();
        let w = DownloadWatch::new(d).unwrap();
        assert!(w.poll().is_none());

        // In progress under the browser's own name, then renamed when done.
        fs::write(d.join("Glow-14-3-1-300.zip.crdownload"), "partial").unwrap();
        assert!(w.poll().is_none());
        fs::rename(
            d.join("Glow-14-3-1-300.zip.crdownload"),
            d.join("Glow-14-3-1-300.zip"),
        )
        .unwrap();
        assert!(w.poll().is_none(), "the first sight only notes its size");
        assert_eq!(w.poll().unwrap(), d.join("Glow-14-3-1-300.zip"));
        // Files that were already there are never picked up.
        assert!(!w.poll().unwrap().ends_with("old.zip"));
        // One found to be another download is passed over.
        w.ignore(&d.join("Glow-14-3-1-300.zip"));
        assert!(w.poll().is_none());

        // Of a known size: a resumable download's saved progress is not taken for the file.
        let w = DownloadWatch::new(d).unwrap().with_size(Some(10));
        fs::write(d.join("Big.7z"), "12345").unwrap();
        assert!(w.poll().is_none());
        assert!(w.poll().is_none(), "steady, but only half there");
        fs::write(d.join("Big.7z"), "1234567890").unwrap();
        assert!(w.poll().is_none());
        assert_eq!(w.poll().unwrap(), d.join("Big.7z"));
    }
}
