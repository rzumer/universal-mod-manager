//! The data directory: game definitions, per-game mod store, profiles, backups and journal.
//!
//! Nothing here ever lives inside a game folder, so uninstalling or reinstalling a game leaves
//! mods, profiles and settings untouched. Game data is deleted only on an explicit request.

pub mod fsutil;
mod mods;
mod placement;
mod profile;
pub mod versions;

pub use fsutil::{IoCtx, IoError};
pub use mods::{
    archive_kind, import_mod, list_mods, load_mod, name_archive, remove_mod, set_fomod,
    set_placement, slugify, sniff_archive, verify_mod, ArchiveKind, FileIssue, FileRecord,
    FomodChoice, FomodFile, FomodMeta, ImportOptions, Layout, ModMeta, NexusMeta, ThunderstoreMeta,
    MOD_META_FILE,
};
pub use placement::{LeftOut, Placement};
pub use profile::{
    list_profiles, load_profile, save_profile, Profile, ProfileEntry, DEFAULT_PROFILE,
};

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::RelPathError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("{path}: {source}")]
    TomlDe {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("serializing TOML: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("mod `{0}` already exists")]
    ModExists(String),
    #[error("mod `{0}` not found")]
    ModNotFound(String),
    #[error("profile `{0}` not found")]
    ProfileNotFound(String),
    #[error("invalid id or name `{0}`")]
    InvalidId(String),
    #[error("the mod contains no files")]
    EmptyMod,
    #[error("unsupported mod source {0} (expected a folder, or a .zip, .7z or .rar file)")]
    Unsupported(PathBuf),
    #[error("{}: {message}", path.display())]
    Archive { path: PathBuf, message: String },
    #[error("{0} is a symlink; symlinks are not imported")]
    Symlink(PathBuf),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("archive entry `{0}` would be extracted outside the mod folder")]
    UnsafeEntry(String),
    #[error(transparent)]
    Path(#[from] RelPathError),
    #[error("placement: {0}")]
    Placement(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

pub(crate) fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).ctx(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).map_err(|source| StoreError::TomlDe {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

pub(crate) fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value)?;
    fsutil::write_atomic(path, text.as_bytes()).ctx(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// `UMM_DATA` if set, otherwise `%APPDATA%\umm` (or the platform's config folder).
    pub fn default_root() -> PathBuf {
        if let Some(p) = std::env::var_os("UMM_DATA") {
            return PathBuf::from(p);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("umm")
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let games = root.join("games");
        fs::create_dir_all(&games).ctx(|| format!("creating {}", games.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn games_dir(&self) -> PathBuf {
        self.root.join("games")
    }

    pub fn game(&self, id: &str) -> GameDir {
        GameDir {
            id: id.to_string(),
            root: self.games_dir().join(id),
        }
    }

    /// Ids of every game that has data on disk, installed or not.
    pub fn games_with_data(&self) -> Vec<String> {
        let Ok(rd) = fs::read_dir(self.games_dir()) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .collect();
        ids.sort();
        ids
    }
}

/// Per-game folder inside the data directory.
#[derive(Debug, Clone)]
pub struct GameDir {
    pub id: String,
    root: PathBuf,
}

impl GameDir {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }
    pub fn ensure(&self) -> Result<()> {
        for p in [self.mods_dir(), self.profiles_dir()] {
            fs::create_dir_all(&p).ctx(|| format!("creating {}", p.display()))?;
        }
        Ok(())
    }
    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }
    pub fn mod_dir(&self, id: &str) -> PathBuf {
        self.mods_dir().join(id)
    }
    /// The mod's content, laid out as it was in the archive.
    pub fn mod_files_dir(&self, id: &str) -> PathBuf {
        self.mod_dir(id).join("files")
    }
    /// A Thunderstore package's own files (manifest, icon, readme), kept for the interface
    /// and never deployed.
    pub fn mod_package_dir(&self, id: &str) -> PathBuf {
        self.mod_dir(id).join("package")
    }
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn quarantine_dir(&self) -> PathBuf {
        self.root.join("quarantine")
    }
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join(".staging")
    }
    pub fn journal_path(&self) -> PathBuf {
        self.root.join("journal.json")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.root.join("state.lock")
    }
    /// The game's definition, which also holds what UMM sets for it (install path, mod
    /// loader).
    pub fn def_path(&self) -> PathBuf {
        self.root.join(umm_gamedef::GAME_FILE)
    }
    /// Per-profile folder a Doorstop-style loader is pointed at. Kept between launches, so
    /// configs and logs the game writes there survive.
    pub fn runtime_dir(&self, profile: &str) -> PathBuf {
        self.root.join("runtime").join(profile)
    }
    /// Files UMM generates itself (e.g. `steam_appid.txt`).
    pub fn generated_dir(&self) -> PathBuf {
        self.root.join("generated")
    }
}
