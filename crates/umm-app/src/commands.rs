//! Tauri commands. Each maps onto one `Session` call and runs on a blocking worker thread,
//! so file hashing and copying never freeze the window.

use crate::dto::*;
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use umm_core::{Faults, ImportOptions, Profile, Session};

type Res<T> = Result<T, String>;

async fn blocking<T, F>(state: &State<'_, AppState>, f: F) -> Res<T>
where
    T: Send + 'static,
    F: FnOnce(&Session) -> Result<T, umm_core::CoreError> + Send + 'static,
{
    let session = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || f(&session).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

fn recovery_results(s: &Session) -> Vec<RecoveryDto> {
    s.recover_all(false)
        .into_iter()
        .map(|(game, r)| match r {
            Ok(Some(rep)) => RecoveryDto {
                game,
                ok: true,
                message: "restored the game folder after an interrupted session".into(),
                report: Some(rep.into()),
            },
            Ok(None) => RecoveryDto {
                game,
                ok: true,
                message: "nothing to recover".into(),
                report: None,
            },
            Err(e) => RecoveryDto {
                game,
                ok: false,
                message: e.to_string(),
                report: None,
            },
        })
        .collect()
}

/// Called once when the window loads: reverts interrupted deployments (games still running are
/// skipped and reported) and lists invalid game definitions.
#[tauri::command]
pub async fn startup(state: State<'_, AppState>) -> Res<StartupDto> {
    blocking(&state, |s| {
        let (_, errors) = s.defs();
        Ok(StartupDto {
            data_dir: s.data().root().display().to_string(),
            gamedefs_dir: s.data().gamedefs_dir().display().to_string(),
            recovery: recovery_results(s),
            def_errors: errors.into_iter().map(|e| e.to_string()).collect(),
        })
    })
    .await
}

#[tauri::command]
pub async fn recover_all(state: State<'_, AppState>) -> Res<Vec<RecoveryDto>> {
    blocking(&state, |s| Ok(recovery_results(s))).await
}

fn game_dtos(s: &Session) -> Result<Vec<GameDto>, umm_core::CoreError> {
    Ok(s.status()?
        .into_iter()
        .map(|st| {
            let running = s.is_running(&st.def).unwrap_or(false);
            GameDto::new(st, running)
        })
        .collect())
}

#[tauri::command]
pub async fn list_games(state: State<'_, AppState>) -> Res<Vec<GameDto>> {
    blocking(&state, game_dtos).await
}

#[tauri::command]
pub async fn game_details(state: State<'_, AppState>, game: String) -> Res<GameDetails> {
    blocking(&state, move |s| {
        let g = game_dtos(s)?
            .into_iter()
            .find(|g| g.id == game)
            .ok_or_else(|| umm_core::CoreError::UnknownGame(game.clone()))?;
        Ok(GameDetails {
            game: g,
            profiles: s.profiles(&game),
            mods: s.mods(&game)?.into_iter().map(Into::into).collect(),
            data_dir: s.game_dir(&game).root().display().to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn get_profile(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<Profile> {
    blocking(&state, move |s| s.profile(&game, &profile)).await
}

#[tauri::command]
pub async fn save_profile(state: State<'_, AppState>, game: String, profile: Profile) -> Res<()> {
    blocking(&state, move |s| s.save_profile(&game, &profile)).await
}

#[tauri::command]
pub async fn create_profile(
    state: State<'_, AppState>,
    game: String,
    name: String,
    copy_from: Option<String>,
) -> Res<Profile> {
    blocking(&state, move |s| {
        s.create_profile(&game, &name, copy_from.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, game: String, profile: String) -> Res<()> {
    blocking(&state, move |s| s.delete_profile(&game, &profile)).await
}

#[tauri::command]
pub async fn preview_plan(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<PlanDto> {
    blocking(&state, move |s| s.plan(&game, &profile).map(Into::into)).await
}

/// Opens a native picker. `kind` is "archive" (a .zip) or "folder".
#[tauri::command]
pub async fn pick_path<R: Runtime>(app: AppHandle<R>, kind: String) -> Res<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let d = app.dialog().file();
        let picked = if kind == "folder" {
            d.blocking_pick_folder()
        } else {
            d.add_filter("Mod archive", &["zip"]).blocking_pick_file()
        };
        picked
            .and_then(|p| p.into_path().ok())
            .map(|p| p.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_mod(
    state: State<'_, AppState>,
    game: String,
    path: String,
    profile: String,
    id: Option<String>,
) -> Res<ModDto> {
    blocking(&state, move |s| {
        let opts = ImportOptions {
            id: id.filter(|i| !i.trim().is_empty()),
            unwrap: true,
            ..Default::default()
        };
        s.import(&game, Path::new(&path), opts, &profile)
            .map(Into::into)
    })
    .await
}

#[tauri::command]
pub async fn remove_mod(state: State<'_, AppState>, game: String, id: String) -> Res<()> {
    blocking(&state, move |s| s.remove_mod(&game, &id)).await
}

#[tauri::command]
pub async fn set_install_path(state: State<'_, AppState>, game: String, path: String) -> Res<()> {
    blocking(&state, move |s| s.set_install_path(&game, Path::new(&path))).await
}

/// Starts a launch on its own thread and returns at once. Progress arrives as `session`
/// events; the last one is `finished` or `failed`.
#[tauri::command]
pub async fn launch<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<()> {
    {
        let mut active = state.active.lock().map_err(|e| e.to_string())?;
        if let Some(g) = active.as_ref() {
            return Err(format!("`{g}` is already running through UMM"));
        }
        *active = Some(game.clone());
    }
    state.stop.store(false, Ordering::SeqCst);
    let session = Arc::clone(&state.session);
    let active = Arc::clone(&state.active);
    let stop = Arc::clone(&state.stop);
    std::thread::spawn(move || {
        let emit = |e: SessionEvent| {
            let _ = app.emit("session", e);
        };
        let stop_fn = || stop.load(Ordering::SeqCst);
        let result = session.launch(&game, &profile, &stop_fn, &mut |e| {
            emit(SessionEvent::from_launch(&game, e))
        });
        if let Ok(mut a) = active.lock() {
            *a = None;
        }
        match result {
            Ok(o) => emit(SessionEvent::Finished {
                game: game.clone(),
                abandoned: o.abandoned,
                appeared: o.game_appeared,
                seconds: o.played.as_secs(),
            }),
            Err(e) => emit(SessionEvent::Failed {
                game: game.clone(),
                message: e.to_string(),
            }),
        }
    });
    Ok(())
}

/// Stops waiting for the game. Its mods stay deployed until it exits and recovery runs.
#[tauri::command]
pub fn stop_waiting(state: State<'_, AppState>) {
    state.stop.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub async fn revert(
    state: State<'_, AppState>,
    game: String,
    discard_if_missing: bool,
) -> Res<Option<ReportDto>> {
    blocking(&state, move |s| {
        Ok(s.revert(&game, discard_if_missing, &mut Faults::none())?
            .map(Into::into))
    })
    .await
}

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>, game: String) -> Res<Vec<FindingDto>> {
    blocking(&state, move |s| {
        Ok(s.doctor(&game)?.into_iter().map(Into::into).collect())
    })
    .await
}

#[tauri::command]
pub async fn delete_game_data(state: State<'_, AppState>, game: String) -> Res<()> {
    blocking(&state, move |s| s.delete_game_data(&game)).await
}

/// Shows a folder in the system file manager. Folders only: passing a file to the shell
/// could run it.
#[tauri::command]
pub fn open_folder(path: String) -> Res<()> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(&p)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Closes the app even though a game is running; its mods are reverted on the next start.
#[tauri::command]
pub fn quit_anyway<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) {
    state.stop.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// Installed Steam games, with their guessed engine and whether UMM already has each.
#[tauri::command]
pub async fn steam_games(state: State<'_, AppState>) -> Res<Vec<SteamGameDto>> {
    blocking(&state, |s| {
        Ok(s.steam_games().into_iter().map(Into::into).collect())
    })
    .await
}

/// Writes a drafted game definition for each app. One failure does not stop the others.
#[tauri::command]
pub async fn add_steam_games(
    state: State<'_, AppState>,
    app_ids: Vec<u32>,
) -> Res<Vec<AddedGameDto>> {
    blocking(&state, move |s| {
        Ok(app_ids
            .into_iter()
            .map(|app_id| match s.add_steam_game(app_id) {
                Ok(d) => AddedGameDto {
                    app_id,
                    ok: true,
                    message: format!("Added {} ({})", d.def.name, d.engine.label()),
                    id: Some(d.def.id),
                    name: Some(d.def.name),
                    notes: d.notes,
                },
                Err(e) => AddedGameDto {
                    app_id,
                    ok: false,
                    id: None,
                    name: None,
                    message: e.to_string(),
                    notes: Vec::new(),
                },
            })
            .collect())
    })
    .await
}
