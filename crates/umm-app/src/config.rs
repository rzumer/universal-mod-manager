//! Tauri commands for mods' config files in a profile's runtime folder.

use crate::commands::{blocking, Res};
use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
use umm_core::{ConfigChange, ConfigFile, ConfigView};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFileDto {
    pub path: String,
    pub name: String,
    pub version: Option<String>,
    pub guid: Option<String>,
    pub size: u64,
    pub modified_unix: u64,
    pub bepinex: bool,
}

impl From<ConfigFile> for ConfigFileDto {
    fn from(f: ConfigFile) -> Self {
        ConfigFileDto {
            path: f.path,
            name: f.name,
            version: f.version,
            guid: f.guid,
            size: f.size,
            modified_unix: f.modified_unix,
            bepinex: f.bepinex,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigListDto {
    /// The profile's runtime folder, where they are.
    pub dir: String,
    pub files: Vec<ConfigFileDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntryDto {
    pub key: String,
    pub value: String,
    pub description: String,
    pub kind: Option<String>,
    pub default: Option<String>,
    pub acceptable: Vec<String>,
    pub range: Option<(String, String)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSectionDto {
    pub name: String,
    pub entries: Vec<ConfigEntryDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigViewDto {
    pub file: ConfigFileDto,
    pub text: String,
    /// A BepInEx file's settings.
    pub sections: Option<Vec<ConfigSectionDto>>,
}

impl From<ConfigView> for ConfigViewDto {
    fn from(v: ConfigView) -> Self {
        ConfigViewDto {
            file: v.file.into(),
            text: v.text,
            sections: v.sections.map(|s| {
                s.into_iter()
                    .map(|s| ConfigSectionDto {
                        name: s.name,
                        entries: s
                            .entries
                            .into_iter()
                            .map(|e| ConfigEntryDto {
                                key: e.key,
                                value: e.value,
                                description: e.description,
                                kind: e.kind,
                                default: e.default,
                                acceptable: e.acceptable,
                                range: e.range,
                            })
                            .collect(),
                    })
                    .collect()
            }),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChangeDto {
    pub section: String,
    pub key: String,
    pub value: String,
}

/// The profile's config files.
#[tauri::command]
pub async fn config_files(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<ConfigListDto> {
    blocking(&state, move |s| {
        Ok(ConfigListDto {
            dir: s.config_dir(&game, &profile).display().to_string(),
            files: s
                .config_files(&game, &profile)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    })
    .await
}

#[tauri::command]
pub async fn read_config(
    state: State<'_, AppState>,
    game: String,
    profile: String,
    path: String,
) -> Res<ConfigViewDto> {
    blocking(&state, move |s| {
        Ok(s.read_config(&game, &profile, &path)?.into())
    })
    .await
}

/// Sets settings of a BepInEx config file.
#[tauri::command]
pub async fn set_config_values(
    state: State<'_, AppState>,
    game: String,
    profile: String,
    path: String,
    changes: Vec<ConfigChangeDto>,
) -> Res<ConfigViewDto> {
    blocking(&state, move |s| {
        let changes: Vec<ConfigChange> = changes
            .into_iter()
            .map(|c| ConfigChange {
                section: c.section,
                key: c.key,
                value: c.value,
            })
            .collect();
        Ok(s.set_config_values(&game, &profile, &path, &changes)?
            .into())
    })
    .await
}

/// Replaces a config file's text.
#[tauri::command]
pub async fn set_config_text(
    state: State<'_, AppState>,
    game: String,
    profile: String,
    path: String,
    text: String,
) -> Res<ConfigViewDto> {
    blocking(&state, move |s| {
        Ok(s.set_config_text(&game, &profile, &path, &text)?.into())
    })
    .await
}
