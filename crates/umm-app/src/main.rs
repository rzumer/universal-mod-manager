#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The UMM desktop app: a thin layer over `umm_core::Session`. Every filesystem change goes
//! through the same engine as the CLI; the webview only calls the commands in `commands.rs`.

mod commands;
mod dto;
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
            commands::set_install_path,
            commands::launch,
            commands::stop_waiting,
            commands::revert,
            commands::recover_all,
            commands::doctor,
            commands::delete_game_data,
            commands::open_folder,
            commands::quit_anyway,
            commands::steam_games,
            commands::add_steam_games,
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
    };

    tauri::Builder::default()
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
