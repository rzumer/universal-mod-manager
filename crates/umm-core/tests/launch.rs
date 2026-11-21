//! Launch tests against a real process: `umm-fake-game`, copied into a fake game folder under
//! a name unique to each test (so parallel tests don't see each other as "running").

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use umm_core::{CoreError, Faults, ImportOptions, LaunchEvent, Session, DEFAULT_PROFILE};

/// The game's `games/<id>/game.toml`, its folder created.
fn def_file(s: &Session, id: &str) -> std::path::PathBuf {
    let p = s.data().game(id).def_path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

const GAME: &str = "launch-game";

struct Fx {
    _tmp: tempfile::TempDir,
    session: Session,
    root: PathBuf,
    out: PathBuf,
    exe: String,
    base: PathBuf,
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn listing(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            out.push(
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            if p.is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

/// `extra` is appended to the game definition; `args` go in `[launch] args`.
fn setup(test: &str, extra: &str, args: &[&str]) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let root = base.join("game");
    let out = base.join("out");
    let exe = format!("FakeGame-{test}.exe");
    fs::create_dir_all(&root).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_umm-fake-game"), root.join(&exe)).unwrap();
    write(&root.join("Data/ui.bundle"), "orig");

    let mut all_args = vec!["--out".to_string(), out.display().to_string()];
    all_args.extend(args.iter().map(|a| a.to_string()));
    let args_toml: Vec<String> = all_args.iter().map(|a| format!("'{a}'")).collect();
    let def = format!(
        "schema = 1\nid = \"{GAME}\"\nname = \"Launch Game\"\n{extra}\n\
         [launch]\nexe = \"{exe}\"\nargs = [{}]\n\
         [[install]]\nmatch = \"plugins/**\"\ntarget = \"BepInEx/plugins/{{mod}}/\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
        args_toml.join(", ")
    );
    let session = Session::open(Some(base.join("data"))).unwrap();
    fs::write(def_file(&session, GAME), &def).unwrap();
    session.set_install_path(GAME, &root).unwrap();

    let m = base.join("src/ui");
    write(&m.join("Data/ui.bundle"), "modded");
    write(&m.join("plugins/p.dll"), "plugin");
    session
        .import(
            GAME,
            &m,
            ImportOptions {
                id: Some("ui".into()),
                ..Default::default()
            },
            DEFAULT_PROFILE,
        )
        .unwrap();
    Fx {
        _tmp: tmp,
        session,
        root,
        out,
        exe,
        base,
    }
}

fn launch(fx: &Fx) -> (umm_core::LaunchOutcome, Vec<LaunchEvent>) {
    let mut events = Vec::new();
    let o = fx
        .session
        .launch(GAME, DEFAULT_PROFILE, &|| false, &mut |e| events.push(e))
        .unwrap();
    (o, events)
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn waits_for_the_whole_process_tree_then_reverts() {
    let fx = setup("tree", "", &["--child-ms", "1500"]);
    let before = listing(&fx.root);
    let t0 = Instant::now();
    let (o, events) = launch(&fx);

    assert!(
        t0.elapsed() >= Duration::from_millis(1500),
        "returned before the child exited"
    );
    assert!(
        fx.out.join("child-done.txt").exists(),
        "child never finished"
    );
    assert_eq!(o.exit_code, Some(0));
    assert!(!o.abandoned && o.game_appeared);
    // Mods were in place for both the launcher process and the process it handed off to.
    for who in ["main", "child"] {
        let seen = read(&fx.out.join(format!("{who}-seen.txt")));
        assert!(
            seen.contains("BepInEx/plugins/ui/p.dll"),
            "{who} saw: {seen}"
        );
    }
    assert_eq!(listing(&fx.root), before, "game folder not restored");
    assert_eq!(read(&fx.root.join("Data/ui.bundle")), "orig");
    assert!(matches!(events.first(), Some(LaunchEvent::Deployed { .. })));
    assert!(matches!(events.last(), Some(LaunchEvent::Reverted(_))));
}

#[test]
fn doorstop_keeps_the_loader_tree_out_of_the_game_folder() {
    let fx = setup(
        "doorstop",
        "[loader]\nkind = \"bepinex\"\nsource = \"mod:bepinex\"\ninject = \"doorstop\"\n",
        &[],
    );
    let pack = fx.base.join("src/bepinex");
    write(&pack.join("winhttp.dll"), "proxy");
    write(&pack.join("doorstop_config.ini"), "[General]\nenabled=true");
    write(
        &pack.join("BepInEx/core/BepInEx.Preloader.dll"),
        "preloader",
    );
    fx.session
        .import(
            GAME,
            &pack,
            ImportOptions {
                id: Some("bepinex".into()),
                unwrap: false,
                ..Default::default()
            },
            DEFAULT_PROFILE,
        )
        .unwrap();
    let before = listing(&fx.root);

    launch(&fx);

    let seen = read(&fx.out.join("main-seen.txt"));
    assert!(
        seen.contains("winhttp.dll") && seen.contains("Data/ui.bundle"),
        "{seen}"
    );
    assert!(
        !seen.contains("BepInEx/"),
        "BepInEx tree leaked into the game folder: {seen}"
    );
    let runtime = fx.session.game_dir(GAME).runtime_dir(DEFAULT_PROFILE);
    let args = read(&fx.out.join("args.txt"));
    let target = runtime
        .join("BepInEx")
        .join("core")
        .join("BepInEx.Preloader.dll");
    assert!(
        args.contains(&format!("--doorstop-target-assembly\n{}", target.display())),
        "args: {args}"
    );
    assert!(runtime.join("BepInEx/plugins/ui/p.dll").exists());
    assert_eq!(listing(&fx.root), before);

    // Files the game writes in the runtime folder survive; a disabled mod's files go.
    write(&runtime.join("BepInEx/config/ui.cfg"), "user setting");
    let mut p = fx.session.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods.iter_mut().find(|m| m.id == "ui").unwrap().enabled = false;
    fx.session.save_profile(GAME, &p).unwrap();
    launch(&fx);
    assert_eq!(read(&runtime.join("BepInEx/config/ui.cfg")), "user setting");
    assert!(!runtime.join("BepInEx/plugins/ui/p.dll").exists());
    assert!(runtime.join("BepInEx/core/BepInEx.Preloader.dll").exists());
}

#[test]
fn appid_file_is_present_only_while_playing() {
    let fx = setup("appid", "", &[]);
    // Add the Steam settings to the definition.
    let p = fx.session.data().game(GAME).def_path();
    let def = read(&p)
        .replace("[launch]\n", "[launch]\nsteam_relaunch = \"appid_file\"\n")
        // set_install_path added a [detect] table; the app id goes in it.
        .replace("[detect]\n", "[detect]\nsteam_app_id = 480\n");
    fs::write(&p, def).unwrap();

    launch(&fx);
    assert_eq!(read(&fx.out.join("appid.txt")), "480");
    assert!(!fx.root.join("steam_appid.txt").exists());
}

#[test]
fn refuses_to_touch_files_while_the_game_runs() {
    let fx = setup("running", "", &[]);
    let mut game = Command::new(fx.root.join(&fx.exe))
        .args(["--sleep-ms", "4000", "--out"])
        .arg(&fx.out)
        .current_dir(&fx.root)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let err = fx
        .session
        .deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(matches!(err, CoreError::GameRunning(_)), "{err}");
    let err = fx
        .session
        .launch(GAME, DEFAULT_PROFILE, &|| false, &mut |_| {})
        .unwrap_err();
    assert!(matches!(err, CoreError::GameRunning(_)), "{err}");
    game.kill().unwrap();
    game.wait().unwrap();

    // A deployment left behind while the game ran is reverted only after it exits.
    fx.session
        .deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let mut game = Command::new(fx.root.join(&fx.exe))
        .args(["--sleep-ms", "4000", "--out"])
        .arg(&fx.out)
        .current_dir(&fx.root)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let results = fx.session.recover_all(false);
    assert!(matches!(results[0].1, Err(CoreError::GameRunning(_))));
    game.kill().unwrap();
    game.wait().unwrap();
    assert!(fx.session.recover_all(false)[0].1.is_ok());
    assert_eq!(read(&fx.root.join("Data/ui.bundle")), "orig");
}

#[test]
fn abandoning_the_wait_leaves_mods_for_recovery() {
    let fx = setup("abandon", "", &["--sleep-ms", "1500"]);
    let o = fx
        .session
        .launch(GAME, DEFAULT_PROFILE, &|| true, &mut |_| {})
        .unwrap();
    assert!(o.abandoned && o.revert.is_none());
    assert!(fx.session.game_dir(GAME).journal_path().exists());
    // The next launch waits for nothing it doesn't own: the game is still running.
    let err = fx
        .session
        .launch(GAME, DEFAULT_PROFILE, &|| false, &mut |_| {})
        .unwrap_err();
    assert!(matches!(err, CoreError::GameRunning(_)), "{err}");
    std::thread::sleep(Duration::from_millis(2000));
    let (_, events) = launch(&fx);
    assert!(
        matches!(events.first(), Some(LaunchEvent::Recovered(_))),
        "{events:?}"
    );
    assert_eq!(read(&fx.root.join("Data/ui.bundle")), "orig");
}
