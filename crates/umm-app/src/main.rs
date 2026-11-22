#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The UMM desktop app: a thin layer over `umm_core::Session`. Every filesystem change goes
//! through the same engine as the CLI; the webview only calls the commands in `commands.rs`.

mod commands;
mod dto;
mod nexus;
#[cfg(test)]
mod tests;

/// Every command the webview may call. A macro so tests can build the same handler.
macro_rules! umm_handlers {
    () => {
        tauri::generate_handler![
            commands::startup,
            commands::list_games,
            commands::game_details,
            commands::get_profile,
            commands::save_profile,
            commands::create_profile,
            commands::delete_profile,
            commands::preview_plan,
            commands::pick_path,
            commands::import_mod,
            commands::remove_mod,
            commands::placement_view,
            commands::set_placement,
            commands::set_install_path,
            commands::launch,
            commands::stop_waiting,
            commands::revert,
            commands::recover_all,
            commands::doctor,
            commands::delete_game_data,
            commands::open_folder,
            commands::open_package_page,
            commands::quit_anyway,
            commands::steam_games,
            commands::add_steam_games,
            commands::loader_choice,
            commands::set_loader,
            commands::pick_game_folder,
            commands::add_manual_game,
            commands::ts_search,
            commands::ts_refresh,
            commands::ts_install,
            commands::ts_update,
            commands::ts_lookup,
            commands::ts_install_unlisted,
            commands::ts_dependents,
            commands::profile_export_preview,
            commands::profile_export_code,
            commands::profile_export_file,
            commands::profile_import_code,
            commands::profile_import_file,
            commands::link_handler,
            commands::set_link_handler,
            commands::take_pending_link,
            commands::resolve_link,
            crate::nexus::nexus_game,
            crate::nexus::nexus_find_games,
            crate::nexus::set_nexus_domain,
            crate::nexus::nexus_search,
            crate::nexus::nexus_files,
            crate::nexus::nexus_download_start,
            crate::nexus::nexus_download_poll,
            crate::nexus::nexus_download_cancel,
            crate::nexus::nexus_download_in_browser,
            crate::nexus::nexus_download_reopen,
            crate::nexus::nexus_download_install,
            crate::nexus::nexus_resolve_link,
            crate::nexus::nexus_updates,
            crate::nexus::open_nexus_page,
            crate::nexus::open_web_page,
        ]
    };
}
pub(crate) use umm_handlers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, WindowEvent};

pub struct AppState {
    pub session: Arc<umm_core::Session>,
    /// Game id of the launch in progress, if any. One at a time.
    pub active: Arc<Mutex<Option<String>>>,
    /// Set by "stop waiting"; read by the launch thread.
    pub stop: Arc<AtomicBool>,
    /// A `ror2mm://` or `nxm://` link UMM was started with, until the window asks for it.
    pub pending_link: Arc<Mutex<Option<String>>>,
    /// A Nexus Mods download the user is making on the website.
    pub nexus_download: Arc<Mutex<Option<nexus::PendingDownload>>>,
}

fn link_in(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter().find(|a| {
        umm_core::LINK_SCHEMES.iter().any(|s| {
            a.get(..s.len() + 3)
                .is_some_and(|p| p.eq_ignore_ascii_case(&format!("{s}://")))
        })
    })
}

fn main() {
    let session = match umm_core::Session::open(None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open the UMM data folder: {e}");
            std::process::exit(1);
        }
    };
    let state = AppState {
        session: Arc::new(session),
        active: Arc::new(Mutex::new(None)),
        stop: Arc::new(AtomicBool::new(false)),
        pending_link: Arc::new(Mutex::new(link_in(std::env::args()))),
        nexus_download: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        // A second start (say, from a link on a mod site) hands its link to the window
        // already open instead of opening another.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            if let Some(link) = link_in(argv) {
                let _ = app.emit("install-link", link);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .on_window_event(|window, event| {
            // Closing while a game runs would leave its mods deployed until the next start.
            // Ask first; the UI calls `quit_anyway` if the user confirms.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<AppState>();
                let busy = state.active.lock().map(|a| a.is_some()).unwrap_or(false);
                if busy && !state.stop.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.emit("close-blocked", ());
                }
            }
        })
        .invoke_handler(umm_handlers!())
        .run(tauri::generate_context!())
        .expect("error while running the UMM app");
}
