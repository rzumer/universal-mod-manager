//! End-to-end tests of deploy / revert / recovery, including a simulated crash at every
//! filesystem step of both apply and revert.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use umm_core::{CoreError, DeployError, Faults, ImportOptions, Session, DEFAULT_PROFILE};

/// The game's `games/<id>/game.toml`, its folder created.
fn def_file(s: &Session, id: &str) -> std::path::PathBuf {
    let p = s.data().game(id).def_path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

const GAME: &str = "test-game";

const DEF: &str = r#"
schema = 1
name = "Test Game"
protect = ["*.exe"]
writable = ["**/*.cfg"]

[detect]
marker = "Game.exe"

[loader]
kind = "generic"
source = "mod:loader"

[[install]]
match = "plugins/**"
target = "Mods/{mod}/"

[[install]]
match = "**"
target = ""
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    session: Session,
    game_root: PathBuf,
    original: BTreeMap<String, Option<String>>,
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// Every file (with contents) and folder under `root`.
fn snapshot(root: &Path) -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    for e in walkdir(root) {
        let rel = e
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let v = if e.is_file() {
            Some(fs::read_to_string(&e).unwrap())
        } else {
            None
        };
        out.insert(rel, v);
    }
    out
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p.clone());
            }
            out.push(p);
        }
    }
    out
}

fn mod_dir(base: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = base.join("src").join(name);
    for (p, text) in files {
        write(&d.join(p), text);
    }
    d
}

fn setup() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let session = Session::open(Some(base.join("data"))).unwrap();
    fs::write(def_file(&session, "test-game"), DEF).unwrap();

    let game_root = base.join("game");
    write(&game_root.join("Game.exe"), "exe");
    write(&game_root.join("Data/ui.bundle"), "orig ui");
    write(&game_root.join("Data/level1"), "orig level");
    write(&game_root.join("readme.txt"), "readme");
    session.set_install_path(GAME, &game_root).unwrap();

    let opts = |id: &str| ImportOptions {
        id: Some(id.into()),
        ..Default::default()
    };
    let loader = mod_dir(
        base,
        "loader",
        &[("proxy.dll", "proxy"), ("loader/core.dll", "core")],
    );
    session
        .import(GAME, &loader, opts("loader"), DEFAULT_PROFILE)
        .unwrap();
    let a = mod_dir(
        base,
        "a",
        &[
            ("Data/ui.bundle", "A ui"),
            ("plugins/a.dll", "a"),
            ("config/a.cfg", "a cfg"),
            ("Game.exe", "evil exe"),
        ],
    );
    session
        .import(GAME, &a, opts("mod-a"), DEFAULT_PROFILE)
        .unwrap();
    let b = mod_dir(
        base,
        "b",
        &[
            ("Data/ui.bundle", "B ui"),
            ("Data/level1", "B level"),
            ("New/Deep/b.txt", "b"),
        ],
    );
    session
        .import(GAME, &b, opts("mod-b"), DEFAULT_PROFILE)
        .unwrap();

    let original = snapshot(&game_root);
    Fixture {
        _tmp: tmp,
        session,
        game_root,
        original,
    }
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

impl Fixture {
    fn assert_clean(&self) {
        assert_eq!(
            snapshot(&self.game_root),
            self.original,
            "game folder not restored"
        );
        let game = self.session.game_dir(GAME);
        assert!(!game.journal_path().exists(), "journal left behind");
        assert!(!game.backups_dir().exists(), "backups left behind");
        for m in self.session.mods(GAME).unwrap() {
            assert!(
                umm_store::verify_mod(&game, &m).unwrap().is_empty(),
                "store changed: {}",
                m.id
            );
        }
    }

    fn deploy(&self, faults: &mut Faults) -> Result<umm_core::Journal, CoreError> {
        self.session.deploy(GAME, DEFAULT_PROFILE, faults)
    }

    fn revert(&self, faults: &mut Faults) -> Result<Option<umm_core::RevertReport>, CoreError> {
        self.session.revert(GAME, false, faults)
    }
}

#[test]
fn plan_resolves_priority_rules_and_protection() {
    let fx = setup();
    let plan = fx.session.plan(GAME, DEFAULT_PROFILE).unwrap();
    let targets: Vec<(String, String)> = plan
        .files
        .iter()
        .map(|f| (f.target.to_string(), f.owner.clone()))
        .collect();
    let has = |t: &str, o: &str| targets.iter().any(|(a, b)| a == t && b == o);
    assert!(
        has("Data/ui.bundle", "mod-b"),
        "later mod wins: {targets:?}"
    );
    assert!(
        has("Mods/mod-a/a.dll", "mod-a"),
        "install rule applied: {targets:?}"
    );
    assert!(
        has("proxy.dll", "loader"),
        "loader deployed as-is: {targets:?}"
    );
    assert!(has("loader/core.dll", "loader"));
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].overridden, ["mod-a"]);
    assert!(plan
        .skipped
        .iter()
        .any(|s| s.file == "Game.exe" && s.owner == "mod-a"));
}

#[test]
fn deploy_then_revert_restores_everything() {
    let fx = setup();
    let j = fx.deploy(&mut Faults::none()).unwrap();
    let r = &fx.game_root;
    assert_eq!(read(r, "Data/ui.bundle"), "B ui");
    assert_eq!(read(r, "Data/level1"), "B level");
    assert_eq!(read(r, "Mods/mod-a/a.dll"), "a");
    assert_eq!(read(r, "config/a.cfg"), "a cfg");
    assert_eq!(read(r, "Game.exe"), "exe", "protected file replaced");
    assert!(j.ops.iter().any(|o| matches!(o, umm_deploy::Op::Add { method: umm_deploy::Method::Copy, path, .. } if path.as_str() == "config/a.cfg")));

    // A second deploy is refused while the first is active.
    assert!(matches!(
        fx.deploy(&mut Faults::none()),
        Err(CoreError::Deployed(_))
    ));

    let report = fx.revert(&mut Faults::none()).unwrap().unwrap();
    assert_eq!(report.restored, 2);
    assert!(report.quarantined.is_empty() && report.errors.is_empty());
    fx.assert_clean();
    assert!(
        fx.revert(&mut Faults::none()).unwrap().is_none(),
        "nothing left to revert"
    );
}

#[test]
fn crash_at_every_apply_step_is_recoverable() {
    let fx = setup();
    let mut count = Faults::none();
    fx.deploy(&mut count).unwrap();
    let steps = count.steps();
    fx.revert(&mut Faults::none()).unwrap();
    fx.assert_clean();
    assert!(
        steps > 10,
        "expected a meaningful number of steps, got {steps}"
    );

    for n in 0..steps {
        let err = fx.deploy(&mut Faults::crash_at(n)).unwrap_err();
        assert!(err.is_injected_crash(), "step {n}: unexpected error {err}");
        // Recovery after the "crash", as the launcher would do at startup.
        let results = fx.session.recover_all(false);
        for (_, r) in &results {
            assert!(
                r.is_ok(),
                "step {n}: recovery failed: {:?}",
                r.as_ref().err()
            );
        }
        fx.assert_clean();
    }
}

#[test]
fn crash_at_every_revert_step_is_recoverable() {
    let fx = setup();
    fx.deploy(&mut Faults::none()).unwrap();
    let mut count = Faults::none();
    fx.revert(&mut count).unwrap();
    let steps = count.steps();

    for n in 0..steps {
        fx.deploy(&mut Faults::none()).unwrap();
        let err = fx.revert(&mut Faults::crash_at(n)).unwrap_err();
        assert!(err.is_injected_crash(), "step {n}: unexpected error {err}");
        fx.revert(&mut Faults::none()).unwrap();
        fx.assert_clean();
    }
}

#[test]
fn files_changed_during_play_are_quarantined_not_lost() {
    let fx = setup();
    fx.deploy(&mut Faults::none()).unwrap();
    // The game rewrites a mod config, and a game update replaces a file a mod also replaced.
    fs::write(fx.game_root.join("config/a.cfg"), "edited by game").unwrap();
    fs::remove_file(fx.game_root.join("Data/level1")).unwrap();
    fs::write(fx.game_root.join("Data/level1"), "patched level").unwrap();

    let report = fx.revert(&mut Faults::none()).unwrap().unwrap();
    assert_eq!(report.quarantined, ["config/a.cfg"]);
    assert_eq!(report.kept_game_versions, ["Data/level1"]);
    assert_eq!(
        read(&fx.game_root, "Data/level1"),
        "patched level",
        "game update kept"
    );
    assert_eq!(read(&fx.game_root, "Data/ui.bundle"), "orig ui");
    assert!(!fx.game_root.join("config").exists());

    let q = fx.session.game_dir(GAME).quarantine_dir();
    let stamp = fs::read_dir(&q).unwrap().next().unwrap().unwrap().path();
    assert_eq!(read(&stamp, "config/a.cfg"), "edited by game");
    assert_eq!(
        read(&stamp, "Data/level1"),
        "orig level",
        "outdated original kept aside"
    );
}

#[test]
fn files_created_by_game_keep_their_folders() {
    let fx = setup();
    fx.deploy(&mut Faults::none()).unwrap();
    write(&fx.game_root.join("Mods/mod-a/settings.json"), "{}");
    let report = fx.revert(&mut Faults::none()).unwrap().unwrap();
    assert!(report.dirs_left.iter().any(|d| d == "Mods/mod-a"));
    assert_eq!(read(&fx.game_root, "Mods/mod-a/settings.json"), "{}");
    assert!(!fx.game_root.join("Mods/mod-a/a.dll").exists());
}

#[test]
fn failed_apply_rolls_back_immediately() {
    let fx = setup();
    // A folder where a mod wants a file: prepare catches this before any change.
    fs::create_dir_all(fx.game_root.join("New/Deep/b.txt")).unwrap();
    let before = snapshot(&fx.game_root);
    let err = fx.deploy(&mut Faults::none()).unwrap_err();
    assert!(
        matches!(err, CoreError::Deploy(DeployError::TargetIsDir(_))),
        "{err}"
    );
    assert_eq!(snapshot(&fx.game_root), before);
    assert!(!fx.session.game_dir(GAME).journal_path().exists());
}

#[test]
fn uninstalling_the_game_never_touches_mods() {
    let fx = setup();
    let mods_before: Vec<String> = fx
        .session
        .mods(GAME)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();

    // Uninstalled while deployed: recovery refuses to guess...
    fx.deploy(&mut Faults::none()).unwrap();
    fs::remove_dir_all(&fx.game_root).unwrap();
    let err = fx.revert(&mut Faults::none()).unwrap_err();
    assert!(
        matches!(err, CoreError::Deploy(DeployError::GameRootMissing(_))),
        "{err}"
    );
    assert!(fx.session.game_dir(GAME).journal_path().exists());

    // ...until told to discard the deployment. Mods and profiles survive either way.
    fx.session.revert(GAME, true, &mut Faults::none()).unwrap();
    assert!(!fx.session.game_dir(GAME).journal_path().exists());
    let mods_after: Vec<String> = fx
        .session
        .mods(GAME)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(mods_before, mods_after);
    assert_eq!(
        fx.session
            .profile(GAME, DEFAULT_PROFILE)
            .unwrap()
            .mods
            .len(),
        3
    );

    let findings = fx.session.doctor(GAME).unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains("may be uninstalled")),
        "{findings:?}"
    );

    // Explicit deletion is the only way the data goes away.
    fx.session.delete_game_data(GAME).unwrap();
    assert!(fx.session.mods(GAME).unwrap().is_empty());
    assert!(fx.session.def(GAME).is_err(), "the game leaves UMM too");
    assert!(fx.session.status().unwrap().is_empty());
}

#[test]
fn doctor_is_clean_after_a_round_trip() {
    let fx = setup();
    fx.deploy(&mut Faults::none()).unwrap();
    fx.revert(&mut Faults::none()).unwrap();
    let findings = fx.session.doctor(GAME).unwrap();
    let bad: Vec<_> = findings
        .iter()
        .filter(|f| f.level >= umm_core::Level::Warn)
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}
