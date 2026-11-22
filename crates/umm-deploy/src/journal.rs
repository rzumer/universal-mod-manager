use crate::DeployError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use umm_gamedef::RelPath;
use umm_store::fsutil::{write_atomic, IoCtx};

pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Applying,
    Deployed,
    Reverting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Hardlink,
    Copy,
}

/// One change to the game folder, with enough information to undo it from any state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    /// A folder that did not exist before.
    Mkdir { path: RelPath },
    /// A file that did not exist before.
    Add {
        path: RelPath,
        method: Method,
        src: PathBuf,
        hash: String,
        size: u64,
    },
    /// A game file moved to `backup_root/<path>` and replaced by a mod file.
    Replace {
        path: RelPath,
        method: Method,
        src: PathBuf,
        hash: String,
        size: u64,
        orig_hash: String,
    },
    /// A file outside the game folder that UMM writes while mods are deployed (a game's
    /// plugin list): its original copied to `backup` first, if there was one.
    External {
        path: PathBuf,
        /// UMM's version, in the data folder.
        src: PathBuf,
        hash: String,
        /// `None`: the file did not exist.
        orig_hash: Option<String>,
        backup: PathBuf,
    },
    /// One key of an ini file outside the game folder, set while mods are deployed. Only the
    /// key is undone, so other changes to the file (the game's own settings) are kept.
    IniSet {
        path: PathBuf,
        section: String,
        key: String,
        value: String,
        /// `None`: the file did not set the key.
        orig: Option<String>,
        /// UMM created the file; it is deleted again if nothing else was added to it.
        #[serde(default)]
        created: bool,
        /// The file had no such section; it is removed again once empty.
        #[serde(default)]
        section_added: bool,
    },
}

impl Op {
    /// The path it changes, for messages.
    pub fn label(&self) -> String {
        match self {
            Op::Mkdir { path } | Op::Add { path, .. } | Op::Replace { path, .. } => {
                path.to_string()
            }
            Op::External { path, .. } => path.display().to_string(),
            Op::IniSet {
                path, section, key, ..
            } => format!("{} [{section}] {key}", path.display()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub game: String,
    pub profile: String,
    pub game_root: PathBuf,
    pub backup_root: PathBuf,
    /// True when backups live inside the game folder (data folder on another drive).
    pub backup_in_game: bool,
    pub started_unix: u64,
    pub phase: Phase,
    pub ops: Vec<Op>,
}

impl Journal {
    pub fn load(path: &Path) -> Result<Option<Journal>, DeployError> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("reading {}", path.display())).into(),
                )
            }
        };
        let j: Journal = serde_json::from_slice(&bytes).map_err(|e| DeployError::Journal {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        if j.version != JOURNAL_VERSION {
            return Err(DeployError::Journal {
                path: path.to_path_buf(),
                message: format!("unsupported version {}", j.version),
            });
        }
        Ok(Some(j))
    }

    pub fn save(&self, path: &Path) -> Result<(), DeployError> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| DeployError::Journal {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        write_atomic(path, &bytes).ctx(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn target(&self, rel: &RelPath) -> PathBuf {
        rel.to_path(&self.game_root)
    }

    pub fn backup(&self, rel: &RelPath) -> PathBuf {
        rel.to_path(&self.backup_root)
    }
}
