//! Journaled deployment.
//!
//! 1. [`build_plan`] (read-only) decides which stored file ends up at which game path.
//! 2. [`prepare`] inspects the game folder and turns the plan into journal operations.
//! 3. [`apply`] saves the journal to disk *before* the first change, then performs it.
//! 4. [`revert`] undoes the journal in reverse, checking each entry against the disk, so it is
//!    correct after a crash at any point and safe to run more than once.

mod apply;
pub mod block;
mod faults;
mod ini;
mod journal;
pub mod json;
mod list;
mod lock;
pub mod lsx;
mod plan;
mod revert;

pub use apply::{apply, prepare};
pub use faults::Faults;
pub use journal::{Journal, Method, Op, Phase, JOURNAL_VERSION};
pub use list::ListFormat;
pub use lock::GameLock;
pub use plan::{
    build_plan, Conflict, Plan, PlannedExternal, PlannedFile, PlannedIni, PlannedList, Skipped,
};
pub use revert::{revert, RevertReport};

use std::path::PathBuf;
use umm_gamedef::RelPathError;
use umm_store::{IoError, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Path(#[from] RelPathError),
    #[error("mod `{0}` is in the profile but not installed")]
    MissingMod(String),
    #[error("mods are already deployed for this game (journal at {0}); revert first")]
    AlreadyDeployed(PathBuf),
    #[error("game folder {0} does not exist")]
    GameRootMissing(PathBuf),
    #[error("refusing to deploy into {0}: it is a drive root")]
    UnsafeGameRoot(PathBuf),
    #[error("`{0}` passes through a symlink or junction in the game folder")]
    LinkInPath(String),
    #[error("`{0}` is a folder in the game folder, but a mod ships it as a file")]
    TargetIsDir(String),
    #[error("`{0}` is a file in the game folder, but a mod needs a folder there")]
    TargetIsFile(String),
    #[error("`{0}` changed between planning and applying")]
    Changed(String),
    #[error("another UMM process is working on this game")]
    Locked,
    #[error("journal {path}: {message}")]
    Journal { path: PathBuf, message: String },
    #[error("revert incomplete: {} problem(s) remain; the journal was kept so it can be retried", .0.errors.len())]
    RevertIncomplete(RevertReport),
    #[error("deploy failed ({cause}); every change was rolled back")]
    RolledBack {
        cause: Box<DeployError>,
        report: RevertReport,
    },
    #[error("deploy failed ({cause}) and the rollback was incomplete; run recovery")]
    RollbackFailed {
        cause: Box<DeployError>,
        report: RevertReport,
    },
    #[error("injected crash at step {0}")]
    InjectedCrash(usize),
}
