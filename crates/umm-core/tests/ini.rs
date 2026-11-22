//! Ini settings set while mods are deployed (Fallout 4's `bEnableFileSelection`): only the
//! key is put back afterwards, so the game's own changes to the file are kept.

use std::fs;
use std::path::{Path, PathBuf};
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "fo4";
const PREFS: &str = "[Display]\r\niSize W=1920\r\n\r\n[Launcher]\r\nbEnableFileSelection=0\r\nuLastAspectRatio=1\r\n";

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

struct Fx {
    _tmp: tempfile::TempDir,
    s: Session,
    prefs: PathBuf,
}

fn setup(prefs: Option<&str>) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Fallout4.exe"), "exe");
    let ini = tmp.path().join("docs/My Games/Fallout4/Fallout4Prefs.ini");
    if let Some(p) = prefs {
        write(&ini, p);
    }
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        &format!(
            "schema = 1\nname = \"Fallout 4\"\n[detect]\nmarker = \"Fallout4.exe\"\n\
             [[ini]]\nfile = '{}'\nsection = \"Launcher\"\nkey = \"bEnableFileSelection\"\nvalue = \"1\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
            ini.display()
        ),
    );
    s.set_install_path(GAME, &root).unwrap();
    let src = tmp.path().join("src/m");
    write(&src.join("Data/M.esp"), "esp");
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some("m".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    Fx {
        _tmp: tmp,
        s,
        prefs: ini,
    }
}

/// A game whose `Custom.ini` of overrides gets two `[Archive]` settings, created if missing.
fn setup_custom(custom: Option<&str>) -> Fx {
    let fx = setup(None);
    let ini = fx.prefs.with_file_name("Fallout4Custom.ini");
    if let Some(c) = custom {
        write(&ini, c);
    }
    let setting = |key: &str, value: &str| {
        format!(
            "[[ini]]\nfile = '{}'\nsection = \"Archive\"\nkey = \"{key}\"\nvalue = \"{value}\"\ncreate = true\n",
            ini.display()
        )
    };
    write(
        &fx.s.data().game(GAME).def_path(),
        &format!(
            "schema = 1\nname = \"Fallout 4\"\n[detect]\nmarker = \"Fallout4.exe\"\n{}{}\
             [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
            setting("bInvalidateOlderFiles", "1"),
            setting("sResourceDataDirsFinal", "")
        ),
    );
    // The definition was rewritten: its install folder too.
    fx.s.set_install_path(GAME, &fx._tmp.path().join("game"))
        .unwrap();
    Fx { prefs: ini, ..fx }
}

#[test]
fn a_custom_ini_is_created_and_deleted_again() {
    let fx = setup_custom(None);
    assert!(fx.s.ini_preview(GAME).unwrap()[0].create);
    deploy(&fx);
    assert_eq!(
        read(&fx.prefs),
        "[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n"
    );
    revert(&fx);
    assert!(
        !fx.prefs.exists(),
        "UMM made it, and nothing else was put in it"
    );
}

#[test]
fn a_custom_ini_keeps_what_else_it_holds() {
    let mine = "[Display]\r\nfShadowDistance=3000\r\n";
    let fx = setup_custom(Some(mine));
    deploy(&fx);
    assert!(read(&fx.prefs).contains("sResourceDataDirsFinal=\r\n"));
    revert(&fx);
    assert_eq!(read(&fx.prefs), mine);

    // Created by UMM, but the user added a line of their own while playing: kept.
    let fx = setup_custom(None);
    deploy(&fx);
    write(
        &fx.prefs,
        &format!("{}[General]\nbSkipSplash=1\n", read(&fx.prefs)),
    );
    revert(&fx);
    assert_eq!(read(&fx.prefs), "[General]\nbSkipSplash=1\n");
}

#[test]
fn crashes_at_every_step_leave_no_custom_ini() {
    let fx = setup_custom(None);
    let mut count = Faults::none();
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut count).unwrap();
    let mut rcount = Faults::none();
    fx.s.revert(GAME, false, &mut rcount).unwrap();
    for n in 0..count.steps() {
        assert!(fx
            .s
            .deploy(GAME, DEFAULT_PROFILE, &mut Faults::crash_at(n))
            .is_err());
        for (_, r) in fx.s.recover_all(false) {
            r.unwrap();
        }
        assert!(!fx.prefs.exists(), "deploy step {n}");
    }
    for n in 0..rcount.steps() {
        deploy(&fx);
        assert!(fx.s.revert(GAME, false, &mut Faults::crash_at(n)).is_err());
        fx.s.revert(GAME, false, &mut Faults::none()).unwrap();
        assert!(!fx.prefs.exists(), "revert step {n}");
    }
}

fn deploy(fx: &Fx) {
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
}

fn revert(fx: &Fx) -> umm_core::RevertReport {
    fx.s.revert(GAME, false, &mut Faults::none())
        .unwrap()
        .unwrap()
}

#[test]
fn the_key_is_set_while_deployed_and_put_back() {
    let fx = setup(Some(PREFS));
    let p = fx.s.ini_preview(GAME).unwrap();
    assert_eq!((p[0].current.as_deref(), p[0].exists), (Some("0"), true));
    deploy(&fx);
    assert_eq!(
        read(&fx.prefs),
        PREFS.replace("bEnableFileSelection=0", "bEnableFileSelection=1")
    );
    // The launcher changes another setting while playing: that is kept.
    let played = read(&fx.prefs).replace("iSize W=1920", "iSize W=2560");
    write(&fx.prefs, &played);
    revert(&fx);
    assert_eq!(
        read(&fx.prefs),
        PREFS.replace("iSize W=1920", "iSize W=2560")
    );
}

#[test]
fn a_key_the_file_did_not_set_is_removed_again() {
    let no_key = "[Launcher]\r\nuLastAspectRatio=1\r\n";
    let fx = setup(Some(no_key));
    deploy(&fx);
    assert!(read(&fx.prefs).contains("bEnableFileSelection=1"));
    revert(&fx);
    assert_eq!(read(&fx.prefs), no_key);
}

#[test]
fn already_set_or_missing_files_are_left_alone() {
    let on = PREFS.replace("bEnableFileSelection=0", "bEnableFileSelection=1");
    let fx = setup(Some(&on));
    deploy(&fx);
    revert(&fx);
    assert_eq!(read(&fx.prefs), on);

    let fx = setup(None);
    deploy(&fx);
    assert!(
        !fx.prefs.exists(),
        "the game writes its own prefs on first run"
    );
    revert(&fx);
    assert!(!fx.prefs.exists());
}

#[test]
fn a_value_the_user_changed_while_deployed_is_kept() {
    let fx = setup(Some(PREFS));
    deploy(&fx);
    write(
        &fx.prefs,
        &PREFS.replace("bEnableFileSelection=0", "bEnableFileSelection=2"),
    );
    let r = revert(&fx);
    assert!(read(&fx.prefs).contains("bEnableFileSelection=2"));
    assert_eq!(r.kept_game_versions.len(), 1);
}

#[test]
fn crashes_at_every_step_leave_the_users_prefs() {
    let fx = setup(Some(PREFS));
    let mut count = Faults::none();
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut count).unwrap();
    let mut rcount = Faults::none();
    fx.s.revert(GAME, false, &mut rcount).unwrap();
    for n in 0..count.steps() {
        assert!(fx
            .s
            .deploy(GAME, DEFAULT_PROFILE, &mut Faults::crash_at(n))
            .is_err());
        for (_, r) in fx.s.recover_all(false) {
            r.unwrap();
        }
        assert_eq!(read(&fx.prefs), PREFS, "deploy step {n}");
    }
    for n in 0..rcount.steps() {
        deploy(&fx);
        assert!(fx.s.revert(GAME, false, &mut Faults::crash_at(n)).is_err());
        fx.s.revert(GAME, false, &mut Faults::none()).unwrap();
        assert_eq!(read(&fx.prefs), PREFS, "revert step {n}");
    }
}
