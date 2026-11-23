//! Tauri commands for patchers: mods whose program patches the game's own files.

use crate::commands::{blocking, Res};
use crate::AppState;
use serde::Serialize;
use tauri::State;
use umm_core::{PatchState, PatcherStatus, WeiduState, WeiduStatus};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatcherDto {
    /// The patcher program, in the game folder's terms.
    pub exe: String,
    pub approved: bool,
    /// `notRun`, `current`, `outdated` or `failed`.
    pub state: &'static str,
    /// The game files it patches.
    pub files: Vec<String>,
    pub output: String,
    pub error: Option<String>,
    pub ran_unix: u64,
}

impl From<PatcherStatus> for PatcherDto {
    fn from(s: PatcherStatus) -> Self {
        PatcherDto {
            exe: s.exe,
            approved: s.approved,
            state: match s.state {
                PatchState::NotRun => "notRun",
                PatchState::Current => "current",
                PatchState::Outdated => "outdated",
                PatchState::Failed => "failed",
            },
            files: s.files.into_iter().map(|f| f.target).collect(),
            output: s.output,
            error: s.error,
            ran_unix: s.ran_unix,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeiduModDto {
    pub mod_id: String,
    pub setup: String,
    /// Components installed, by number and name; empty: none chosen (yet).
    pub components: Vec<(u32, String)>,
    pub chosen: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeiduDto {
    /// `notRun`, `current`, `outdated`, `needsChoices` or `failed`.
    pub state: &'static str,
    /// A mod comes without its installer, and UMM has no WeiDU to stand in for it.
    pub needs_weidu: bool,
    pub mods: Vec<WeiduModDto>,
    pub files: usize,
    pub output: String,
    pub error: Option<String>,
    pub ran_unix: u64,
    /// The mods the last successful install included.
    pub installed: Vec<String>,
}

impl From<WeiduStatus> for WeiduDto {
    fn from(s: WeiduStatus) -> Self {
        WeiduDto {
            needs_weidu: s.needs_weidu,
            state: match s.state {
                WeiduState::NotRun => "notRun",
                WeiduState::Current => "current",
                WeiduState::Outdated => "outdated",
                WeiduState::NeedsChoices => "needsChoices",
                WeiduState::Failed => "failed",
            },
            mods: s
                .mods
                .into_iter()
                .map(|c| WeiduModDto {
                    chosen: c.language.is_some(),
                    mod_id: c.mod_id,
                    setup: c.setup,
                    components: c.components,
                })
                .collect(),
            files: s.files,
            output: s.output,
            error: s.error,
            ran_unix: s.ran_unix,
            installed: s.installed,
        }
    }
}

/// How the profile's WeiDU mods stand; `None` when it has none.
#[tauri::command]
pub async fn weidu_status(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<Option<WeiduDto>> {
    blocking(&state, move |s| {
        Ok(s.weidu_status(&game, &profile)?.map(Into::into))
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeiduChosenDto {
    pub language: Option<u32>,
    pub components: Vec<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeiduComponentsDto {
    pub languages: Vec<(u32, String)>,
    pub language: u32,
    pub components: Vec<(u32, String)>,
    pub chosen: Option<WeiduChosenDto>,
}

/// A WeiDU mod's languages and components, as WeiDU lists them, in `language` (else the
/// one chosen, else the first).
#[tauri::command]
pub async fn weidu_components(
    state: State<'_, AppState>,
    game: String,
    profile: String,
    id: String,
    language: Option<u32>,
) -> Res<WeiduComponentsDto> {
    blocking(&state, move |s| {
        let c = s.weidu_components(&game, &profile, &id, language)?;
        Ok(WeiduComponentsDto {
            languages: c.languages,
            language: c.language,
            components: c.components,
            chosen: c.chosen.map(|(language, components)| WeiduChosenDto {
                language,
                components,
            }),
        })
    })
    .await
}

/// Sets a WeiDU mod's components for the profile.
#[tauri::command]
pub async fn set_weidu_choice(
    state: State<'_, AppState>,
    game: String,
    profile: String,
    id: String,
    language: u32,
    components: Vec<(u32, String)>,
) -> Res<WeiduDto> {
    blocking(&state, move |s| {
        Ok(
            s.set_weidu_choice(&game, &profile, &id, language, components)?
                .into(),
        )
    })
    .await
}

/// Keeps a copy of WeiDU (`weidu.exe`) for mods that come without their installer.
#[tauri::command]
pub async fn set_weidu_exe(state: State<'_, AppState>, path: String) -> Res<()> {
    blocking(&state, move |s| {
        s.set_weidu_exe(std::path::Path::new(&path))
    })
    .await
}

/// Installs the profile's WeiDU mods on a copy of the game, with their components chosen.
#[tauri::command]
pub async fn run_weidu(state: State<'_, AppState>, game: String, profile: String) -> Res<WeiduDto> {
    blocking(&state, move |s| Ok(s.run_weidu(&game, &profile)?.into())).await
}

/// The mod's patcher and how its run stands; `None` when the mod is not a patcher.
#[tauri::command]
pub async fn patcher_status(
    state: State<'_, AppState>,
    game: String,
    id: String,
) -> Res<Option<PatcherDto>> {
    blocking(&state, move |s| {
        Ok(s.patcher_status(&game, &id)?.map(Into::into))
    })
    .await
}

/// Runs the mod's patcher on a copy of the game's files (the user's approval to run it again
/// after a game update), keeping the files it patched for deploys.
#[tauri::command]
pub async fn run_patcher(state: State<'_, AppState>, game: String, id: String) -> Res<PatcherDto> {
    blocking(&state, move |s| Ok(s.run_patcher(&game, &id)?.into())).await
}
