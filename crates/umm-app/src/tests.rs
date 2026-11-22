//! IPC tests: each call goes through Tauri's real command dispatch with the same JSON
//! (camelCase argument names) that `ui/api.js` sends, against a real data folder.

use crate::{commands, AppState};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Listener, WebviewWindow, WebviewWindowBuilder};

/// The game's `games/<id>/game.toml`, its folder created.
fn def_file(s: &umm_core::Session, id: &str) -> std::path::PathBuf {
    let p = s.data().game(id).def_path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

struct Fx {
    _tmp: tempfile::TempDir,
    base: std::path::PathBuf,
    _app: tauri::App<tauri::test::MockRuntime>,
    win: WebviewWindow<tauri::test::MockRuntime>,
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn setup(exe: &str) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let game_root = base.join("game");
    write(&game_root.join("Data/ui.bundle"), "orig");
    write(&game_root.join(exe), "placeholder exe");
    let session = umm_core::Session::open(Some(base.join("data"))).unwrap();
    fs::write(
        def_file(&session, "ipc-game"),
        format!(
            "schema = 1\nid = \"ipc-game\"\nname = \"IPC Game\"\n[detect]\nmarker = \"{exe}\"\n\
             [launch]\nexe = \"{exe}\"\n"
        ),
    )
    .unwrap();
    session.set_install_path("ipc-game", &game_root).unwrap();
    write(&base.join("src/my-mod/Data/ui.bundle"), "modded");
    write(&base.join("src/my-mod/readme.txt"), "hi");

    let state = AppState {
        session: Arc::new(session),
        active: Arc::new(Mutex::new(None)),
        stop: Arc::new(AtomicBool::new(false)),
        pending_link: Arc::new(Mutex::new(None)),
        nexus_download: Arc::new(Mutex::new(None)),
    };
    let app = mock_builder()
        .manage(state)
        .invoke_handler(crate::umm_handlers!())
        .build(mock_context(noop_assets()))
        .unwrap();
    let win = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    Fx {
        _tmp: tmp,
        base,
        _app: app,
        win,
    }
}

fn call(fx: &Fx, cmd: &str, args: Value) -> Result<Value, Value> {
    let req = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "http://tauri.localhost".parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    get_ipc_response(&fx.win, req).map(|b| b.deserialize::<Value>().unwrap())
}

fn ok(fx: &Fx, cmd: &str, args: Value) -> Value {
    call(fx, cmd, args.clone()).unwrap_or_else(|e| panic!("{cmd}({args}) failed: {e}"))
}

#[test]
fn profile_and_import_round_trip() {
    let fx = setup("Game.exe");
    let st = ok(&fx, "startup", json!({}));
    assert!(st["recovery"].as_array().unwrap().is_empty());

    let games = ok(&fx, "list_games", json!({}));
    assert_eq!(games[0]["id"], "ipc-game");
    assert_eq!(games[0]["canLaunch"], true);

    let src = fx.base.join("src/my-mod").display().to_string();
    let m = ok(
        &fx,
        "import_mod",
        json!({"game": "ipc-game", "path": src, "profile": "Default", "id": null}),
    );
    assert_eq!(m["id"], "my-mod");
    assert_eq!(m["files"], 2);

    let d = ok(&fx, "game_details", json!({"game": "ipc-game"}));
    assert_eq!(d["mods"].as_array().unwrap().len(), 1);
    assert_eq!(d["profiles"], json!(["Default"]));

    let p = ok(
        &fx,
        "create_profile",
        json!({"game": "ipc-game", "name": "Test", "copyFrom": "Default"}),
    );
    assert_eq!(p["mods"][0]["id"], "my-mod");
    let mut p = ok(
        &fx,
        "get_profile",
        json!({"game": "ipc-game", "profile": "Test"}),
    );
    p["mods"][0]["enabled"] = json!(false);
    ok(
        &fx,
        "save_profile",
        json!({"game": "ipc-game", "profile": p}),
    );
    let plan = ok(
        &fx,
        "preview_plan",
        json!({"game": "ipc-game", "profile": "Test"}),
    );
    assert!(plan["files"].as_array().unwrap().is_empty());
    let plan = ok(
        &fx,
        "preview_plan",
        json!({"game": "ipc-game", "profile": "Default"}),
    );
    assert_eq!(plan["files"].as_array().unwrap().len(), 2);

    ok(
        &fx,
        "delete_profile",
        json!({"game": "ipc-game", "profile": "Test"}),
    );
    let err = call(
        &fx,
        "delete_profile",
        json!({"game": "ipc-game", "profile": "Default"}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("cannot be deleted"), "{err}");

    let findings = ok(&fx, "doctor", json!({"game": "ipc-game"}));
    assert!(findings
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["level"] != "error"));
    assert_eq!(
        ok(
            &fx,
            "revert",
            json!({"game": "ipc-game", "discardIfMissing": false})
        ),
        Value::Null
    );

    let err = call(
        &fx,
        "open_folder",
        json!({"path": fx.base.join("game/Game.exe").display().to_string()}),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("not a folder"),
        "files must never be opened: {err}"
    );

    ok(
        &fx,
        "remove_mod",
        json!({"game": "ipc-game", "id": "my-mod"}),
    );
    ok(&fx, "delete_game_data", json!({"game": "ipc-game"}));
    // The game leaves UMM entirely; its own folder stays.
    let games = ok(&fx, "list_games", json!({}));
    assert!(games.as_array().unwrap().is_empty(), "{games}");
    assert!(fx.base.join("game").is_dir());
}

/// A full launch through IPC, with a real short-lived Windows program standing in for the game.
#[cfg(windows)]
#[test]
fn launch_emits_session_events_and_restores_the_folder() {
    let fx = setup("Game.exe");
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    fs::copy(
        Path::new(&sys).join("System32").join("whoami.exe"),
        fx.base.join("game/Game.exe"),
    )
    .unwrap();
    let src = fx.base.join("src/my-mod").display().to_string();
    ok(
        &fx,
        "import_mod",
        json!({"game": "ipc-game", "path": src, "profile": "Default", "id": null}),
    );

    let events: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&events);
    fx.win.listen_any("session", move |e| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(e.payload()).unwrap());
    });

    ok(
        &fx,
        "launch",
        json!({"game": "ipc-game", "profile": "Default"}),
    );
    let err = call(
        &fx,
        "launch",
        json!({"game": "ipc-game", "profile": "Default"}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("already running"), "{err}");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let done = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "finished" || e["kind"] == "failed");
        if done {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no finished event: {:?}",
            events.lock().unwrap()
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let kinds: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        kinds,
        ["deployed", "started", "exited", "reverted", "finished"]
    );
    assert_eq!(
        fs::read_to_string(fx.base.join("game/Data/ui.bundle")).unwrap(),
        "orig"
    );
    assert!(!fx.base.join("game/readme.txt").exists());
}
