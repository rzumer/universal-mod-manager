//! Tauri commands. Each maps onto one `Session` call and runs on a blocking worker thread,
//! so file hashing and copying never freeze the window.

use crate::dto::*;
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use umm_core::{Faults, ImportOptions, LoaderSetup, Profile, Session};

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

/// Opens a package's page on thunderstore.io in the default browser. The address is built
/// here from the game's community and the package's `Owner-Name`, both checked, so the
/// webview cannot open anything else.
#[tauri::command]
pub async fn open_package_page(
    state: State<'_, AppState>,
    game: String,
    full_name: String,
) -> Res<()> {
    let url = blocking(&state, move |s| {
        let def = s.def(&game)?.def;
        let community = def.community().ok_or_else(|| {
            umm_core::CoreError::Invalid(format!("{} has no Thunderstore community", def.name))
        })?;
        let plain = |t: &str, extra: char| {
            !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == extra)
        };
        let (owner, name) = full_name.split_once('-').unwrap_or_default();
        if !plain(community, '-') || !plain(owner, '_') || !plain(name, '_') {
            return Err(umm_core::CoreError::Invalid(format!(
                "`{full_name}` is not a Thunderstore package name"
            )));
        }
        Ok(format!(
            "https://thunderstore.io/c/{community}/p/{owner}/{name}/"
        ))
    })
    .await?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(&url)
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

/// Installed Steam games, with their engine, Thunderstore status, setup choices and whether
/// UMM already has each.
#[tauri::command]
pub async fn steam_games(state: State<'_, AppState>) -> Res<SteamListDto> {
    blocking(&state, |s| {
        let list = s.steam_games();
        Ok(SteamListDto {
            games: list.games.into_iter().map(Into::into).collect(),
            thunderstore_error: list.thunderstore_error,
        })
    })
    .await
}

/// Writes a drafted game definition for each pick. One failure does not stop the others.
#[tauri::command]
pub async fn add_steam_games(
    state: State<'_, AppState>,
    picks: Vec<SteamPick>,
) -> Res<Vec<AddedGameDto>> {
    blocking(&state, move |s| {
        Ok(picks
            .into_iter()
            .map(|SteamPick { app_id, setup }| match s.add_steam_game(app_id, setup) {
                Ok(d) => AddedGameDto {
                    app_id,
                    ok: true,
                    message: format!("Added {} ({})", d.def.name, d.setup.label()),
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

/// The game's mod loader setup and what it can switch to.
#[tauri::command]
pub async fn loader_choice(state: State<'_, AppState>, game: String) -> Res<LoaderChoiceDto> {
    blocking(&state, move |s| Ok(s.loader_choice(&game)?.into())).await
}

/// Switches the game's mod loader setup, written into its `game.toml`.
#[tauri::command]
pub async fn set_loader(
    state: State<'_, AppState>,
    game: String,
    setup: LoaderSetup,
) -> Res<LoaderChoiceDto> {
    blocking(&state, move |s| Ok(s.set_loader(&game, setup)?.into())).await
}

/// Opens a folder picker for "Add game manually" and describes the folder picked. `None` if
/// cancelled.
#[tauri::command]
pub async fn pick_game_folder<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Res<Option<FolderInfoDto>> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|p| p.into_path().ok())
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(folder) = folder else { return Ok(None) };
    blocking(&state, move |s| Ok(Some(s.inspect_game_folder(&folder)?.into()))).await
}

/// Adds a game from the "Add game manually" form.
#[tauri::command]
pub async fn add_manual_game(
    state: State<'_, AppState>,
    folder: String,
    name: String,
    exe: String,
    setup: Option<LoaderSetup>,
) -> Res<AddedGameDto> {
    blocking(&state, move |s| {
        let d = s.add_manual_game(Path::new(&folder), &name, &exe, setup)?;
        Ok(AddedGameDto {
            app_id: 0,
            ok: true,
            message: format!("Added {} ({})", d.def.name, d.setup.label()),
            id: Some(d.def.id),
            name: Some(d.def.name),
            notes: d.notes,
        })
    })
    .await
}

/// A page of the game's Thunderstore packages, plus updates for what is installed.
#[tauri::command]
pub async fn ts_search(
    state: State<'_, AppState>,
    game: String,
    query: TsQueryDto,
) -> Res<TsPageDto> {
    blocking(&state, move |s| {
        let page = s.ts_search(&game, &query.into())?;
        Ok(TsPageDto {
            updates: s.ts_updates(&game)?,
            total: page.total,
            items: page.items.into_iter().map(Into::into).collect(),
            categories: page.categories,
            stale: page.stale,
            loader_package: page.loader_package,
            loader_installed: page.loader_installed,
            loader_warning: page.loader_warning,
            loader_suggestion: page.loader_suggestion,
        })
    })
    .await
}

/// Downloads the game's package list again. Returns whether Thunderstore could be reached.
#[tauri::command]
pub async fn ts_refresh(state: State<'_, AppState>, game: String) -> Res<bool> {
    blocking(&state, move |s| Ok(!s.ts_packages(&game, true)?.1)).await
}

fn progress_to<R: Runtime>(app: AppHandle<R>, game: String) -> impl FnMut(&str) {
    move |m: &str| {
        let _ = app.emit(
            "ts-progress",
            TsProgressDto {
                game: game.clone(),
                message: m.to_string(),
            },
        );
    }
}

/// Installs packages (`Owner-Name`, or `Owner-Name@1.2.3`) with their dependencies into the
/// profile. Progress arrives as `ts-progress` events.
#[tauri::command]
pub async fn ts_install<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    packages: Vec<String>,
    profile: String,
) -> Res<TsReportDto> {
    blocking(&state, move |s| {
        let wanted: Vec<(String, Option<String>)> = packages
            .iter()
            .map(|p| match p.split_once('@') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (p.clone(), None),
            })
            .collect();
        let mut progress = progress_to(app, game.clone());
        Ok(s.ts_install(&game, &wanted, &profile, &mut progress)?.into())
    })
    .await
}

/// Looks up a package by `Owner-Name`, including one the game's community does not list.
#[tauri::command]
pub async fn ts_lookup(state: State<'_, AppState>, game: String, name: String) -> Res<TsLookupDto> {
    blocking(&state, move |s| Ok(s.ts_lookup(&game, &name)?.into())).await
}

/// Installs a package the game's community does not list, after the interface has shown why.
#[tauri::command]
pub async fn ts_install_unlisted<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    name: String,
    version: Option<String>,
    profile: String,
) -> Res<TsReportDto> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(s.ts_install_unlisted(&game, &name, version, &profile, &mut progress)?.into())
    })
    .await
}

/// Updates the named packages, or all of them when none are named.
#[tauri::command]
pub async fn ts_update<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    packages: Vec<String>,
    profile: String,
) -> Res<TsReportDto> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(s.ts_update(&game, &packages, &profile, &mut progress)?.into())
    })
    .await
}

/// Installed packages that depend on the mod, to warn before removing it.
#[tauri::command]
pub async fn ts_dependents(
    state: State<'_, AppState>,
    game: String,
    mod_id: String,
) -> Res<Vec<String>> {
    blocking(&state, move |s| s.ts_dependents(&game, &mod_id)).await
}

/// What an export of the profile would hold, shown before anything is saved or uploaded.
#[tauri::command]
pub async fn profile_export_preview(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<ExportPreviewDto> {
    blocking(&state, move |s| {
        let e = s.profile_export(&game, &profile)?;
        Ok(ExportPreviewDto {
            mods: e.mods,
            files: e.files,
            left_out: e.left_out,
            size: e.r2z.len(),
        })
    })
    .await
}

/// Uploads the profile's export to Thunderstore and returns its code. The interface asks
/// first, since this publishes the mod list and configs.
#[tauri::command]
pub async fn profile_export_code(
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<String> {
    blocking(&state, move |s| s.profile_export_code(&game, &profile)).await
}

/// Saves the profile's export as an `.r2z` file the user picks. `None` if cancelled.
#[tauri::command]
pub async fn profile_export_file<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    profile: String,
) -> Res<Option<String>> {
    let export = blocking(&state, {
        let (game, profile) = (game.clone(), profile.clone());
        move |s| s.profile_export(&game, &profile)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("r2modman profile", &["r2z"])
            .set_file_name(format!("{profile}.r2z"))
            .blocking_save_file();
        let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
            return Ok(None);
        };
        std::fs::write(&path, &export.r2z).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Imports a profile code into a new profile.
#[tauri::command]
pub async fn profile_import_code<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    code: String,
) -> Res<ImportDto> {
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(s.profile_import_code(&game, &code, &mut progress)?.into())
    })
    .await
}

/// Imports an `.r2z` file the user picks into a new profile. `None` if cancelled.
#[tauri::command]
pub async fn profile_import_file<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
) -> Res<Option<ImportDto>> {
    let picker = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        picker
            .dialog()
            .file()
            .add_filter("r2modman profile", &["r2z"])
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = path else { return Ok(None) };
    blocking(&state, move |s| {
        let mut progress = progress_to(app, game.clone());
        Ok(Some(s.profile_import_file(&game, &path, &mut progress)?.into()))
    })
    .await
}

fn current_exe() -> Res<PathBuf> {
    std::env::current_exe().map_err(|e| format!("cannot find UMM's own program: {e}"))
}

/// Whether UMM handles Thunderstore's install links, or which program does.
#[tauri::command]
pub async fn link_handler(state: State<'_, AppState>) -> Res<LinkHandlerDto> {
    let exe = current_exe()?;
    blocking(&state, move |s| {
        Ok(match umm_core::system_registry() {
            Some(reg) => {
                let h = s.link_handler(reg.as_ref(), &exe);
                LinkHandlerDto {
                    supported: true,
                    ours: h.ours,
                    other: h.other,
                }
            }
            None => LinkHandlerDto {
                supported: false,
                ours: false,
                other: None,
            },
        })
    })
    .await
}

/// Turns handling of Thunderstore's install links on or off for this user.
#[tauri::command]
pub async fn set_link_handler(state: State<'_, AppState>, on: bool) -> Res<LinkHandlerDto> {
    let exe = current_exe()?;
    blocking(&state, move |s| {
        let reg = umm_core::system_registry()
            .ok_or_else(|| umm_core::CoreError::Invalid("not supported on this system".into()))?;
        let h = s.set_link_handler(reg.as_ref(), &exe, on)?;
        Ok(LinkHandlerDto {
            supported: true,
            ours: h.ours,
            other: h.other,
        })
    })
    .await
}

/// A link UMM was started with, once.
#[tauri::command]
pub fn take_pending_link(state: State<'_, AppState>) -> Option<String> {
    state.pending_link.lock().ok().and_then(|mut l| l.take())
}

/// The package an install link names, and the games it can go into.
#[tauri::command]
pub async fn resolve_link(state: State<'_, AppState>, url: String) -> Res<LinkDto> {
    blocking(&state, move |s| {
        let link = umm_core::parse_link(&url)
            .ok_or_else(|| umm_core::CoreError::Invalid(format!("not a Thunderstore install link: {url}")))?;
        let games = s
            .link_games(&link)?
            .into_iter()
            .map(|(id, name)| LinkGameDto { id, name })
            .collect();
        Ok(LinkDto {
            full_name: link.full_name(),
            name: link.name,
            owner: link.owner,
            version: link.version,
            games,
        })
    })
    .await
}
