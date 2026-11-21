use crate::DeployError;
use std::fs::{self, File, OpenOptions};
#[cfg(not(windows))]
use std::io;
use std::io::Write;
use std::path::PathBuf;
use umm_store::fsutil::IoCtx;
use umm_store::GameDir;

/// Stops two UMM processes from changing one game at the same time. Held until dropped.
#[derive(Debug)]
pub struct GameLock {
    _file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl GameLock {
    pub fn acquire(game: &GameDir) -> Result<Self, DeployError> {
        fs::create_dir_all(game.root()).ctx(|| format!("creating {}", game.root().display()))?;
        let path = game.lock_path();
        let mut file = open_exclusive(&path)?;
        let _ = write!(file, "{}", std::process::id());
        Ok(Self { _file: file, path })
    }
}

/// On Windows, opening with no sharing allowed fails while another process holds the file,
/// and the OS releases it if the process dies.
#[cfg(windows)]
fn open_exclusive(path: &std::path::Path) -> Result<File, DeployError> {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(path)
    {
        Ok(f) => Ok(f),
        Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Err(DeployError::Locked),
        Err(e) => Err(umm_store::fsutil::io_err(e, format!("locking {}", path.display())).into()),
    }
}

/// Elsewhere: a lock file holding the owner's pid; a lock whose process is gone is stale.
#[cfg(not(windows))]
fn open_exclusive(path: &std::path::Path) -> Result<File, DeployError> {
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(f) => return Ok(f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let pid = fs::read_to_string(path).unwrap_or_default();
                let alive = !pid.trim().is_empty()
                    && std::path::Path::new("/proc").join(pid.trim()).exists();
                if alive {
                    return Err(DeployError::Locked);
                }
                let _ = fs::remove_file(path);
            }
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("locking {}", path.display())).into(),
                )
            }
        }
    }
    Err(DeployError::Locked)
}

#[cfg(not(windows))]
impl Drop for GameLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
