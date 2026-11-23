//! Tauri commands for patchers: mods whose program patches the game's own files.

use crate::commands::{blocking, Res};
use crate::AppState;
use serde::Serialize;
use tauri::State;
use umm_core::{PatchState, PatcherStatus};

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
