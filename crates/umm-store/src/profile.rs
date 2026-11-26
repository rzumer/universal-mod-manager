//! Profiles: named, ordered lists of mods. Later entries win file conflicts.

use crate::{read_toml, write_toml, GameDir, Result, StoreError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use umm_gamedef::RelPath;

pub const DEFAULT_PROFILE: &str = "Default";

/// How a profile names a Steam Workshop item: `workshop:<item id>`. These entries are not
/// mods in the store (Steam keeps their files), so deploys never place them; for a game whose
/// mod list UMM writes, an entry turns the item on or off there.
pub const WORKSHOP_PREFIX: &str = "workshop:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub mods: Vec<ProfileEntry>,
    /// Extra arguments passed to the game on launch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The version this profile is pinned to (Thunderstore packages); `None` follows the
    /// latest installed version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl ProfileEntry {
    /// The Workshop item this entry names, if it is one.
    pub fn workshop_item(&self) -> Option<&str> {
        self.id
            .strip_prefix(WORKSHOP_PREFIX)
            .filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()))
    }

    /// An entry following the latest installed version.
    pub fn new(id: impl Into<String>, enabled: bool) -> Self {
        Self {
            id: id.into(),
            enabled,
            version: None,
        }
    }
}

fn yes() -> bool {
    true
}

impl Profile {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            mods: Vec::new(),
            launch_args: Vec::new(),
        }
    }

    /// The enabled mods in the store, in order (not Workshop items).
    pub fn enabled_ids(&self) -> impl Iterator<Item = &str> {
        self.enabled().map(|m| m.id.as_str())
    }

    /// The enabled entries for mods in the store, in order (not Workshop items).
    pub fn enabled(&self) -> impl Iterator<Item = &ProfileEntry> {
        self.mods
            .iter()
            .filter(|m| m.enabled && m.workshop_item().is_none())
    }

    /// The Workshop items the profile has, each with whether it is enabled, in order.
    pub fn workshop(&self) -> impl Iterator<Item = (&str, bool)> {
        self.mods
            .iter()
            .filter_map(|m| Some((m.workshop_item()?, m.enabled)))
    }

    /// The version the profile is pinned to for this mod, if any.
    pub fn pin(&self, id: &str) -> Option<&str> {
        self.mods
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.version.as_deref())
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.mods.iter().position(|m| m.id == id)
    }
}

fn profile_path(game: &GameDir, name: &str) -> Result<PathBuf> {
    // The name becomes a file name, so it must be one safe path component.
    let rel = RelPath::parse(name).map_err(|_| StoreError::InvalidId(name.to_string()))?;
    if rel.depth() != 1 {
        return Err(StoreError::InvalidId(name.to_string()));
    }
    Ok(game.profiles_dir().join(format!("{name}.toml")))
}

/// Loads a profile. The default profile always exists, even before it is first saved.
pub fn load_profile(game: &GameDir, name: &str) -> Result<Profile> {
    let p = profile_path(game, name)?;
    if !p.exists() {
        if name == DEFAULT_PROFILE {
            return Ok(Profile::new(name));
        }
        return Err(StoreError::ProfileNotFound(name.to_string()));
    }
    read_toml(&p)
}

pub fn save_profile(game: &GameDir, profile: &Profile) -> Result<()> {
    write_toml(&profile_path(game, &profile.name)?, profile)
}

pub fn list_profiles(game: &GameDir) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(game.profiles_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(String::from))?
                })
                .collect()
        })
        .unwrap_or_default();
    if !names.iter().any(|n| n == DEFAULT_PROFILE) {
        names.push(DEFAULT_PROFILE.to_string());
    }
    names.sort();
    names
}
