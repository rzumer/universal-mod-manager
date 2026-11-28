//! A Windows game run through Proton on Linux: the folders it keeps in Windows' user folders
//! (its plugin list, its ini files) are those in its prefix, not the Linux user's.
#![cfg(not(windows))]

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};
use umm_gamedef::steam::SteamApp;

const GAME: &str = "skyrim";

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn a_proton_games_list_and_ini_are_in_its_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("SteamLibrary");
    let root = lib.join("steamapps/common/Skyrim Special Edition");
    write(&root.join("SkyrimSE.exe"), "exe");
    write(&root.join("Data/Skyrim.esm"), "esm");
    let user = lib.join("steamapps/compatdata/489830/pfx/drive_c/users/steamuser");
    let list = user.join("AppData/Local/Skyrim Special Edition/plugins.txt");
    let ini = user.join("Documents/My Games/Skyrim Special Edition/Skyrim.ini");
    write(&list, "*Theirs.esp\n");
    write(&ini, "[Archive]\nbInvalidateOlderFiles=0\n");

    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        "schema = 1\nname = \"Skyrim\"\n[detect]\nsteam_app_id = 489830\nmarker = \"SkyrimSE.exe\"\n\
         [mod_list]\nlist = \"%LOCALAPPDATA%/Skyrim Special Edition/plugins.txt\"\nformat = \"asterisk\"\n\
         [[ini]]\nfile = \"%DOCUMENTS%/My Games/Skyrim Special Edition/Skyrim.ini\"\n\
         section = \"Archive\"\nkey = \"bInvalidateOlderFiles\"\nvalue = \"1\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path(GAME, &root).unwrap();
    let def = s.game_def(GAME).unwrap();
    assert_eq!(
        s.path_vars(&def).prefix_user.as_deref(),
        Some(user.as_path())
    );

    let src = tmp.path().join("src/a");
    // As made on Windows: `data` for the game's `Data`.
    write(&src.join("data/A.esp"), "esp");
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some("a".into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(root.join("Data/A.esp").is_file());
    assert!(!root.join("data").exists());
    let text = fs::read_to_string(&list).unwrap();
    assert!(
        text.contains("*Theirs.esp") && text.contains("*A.esp"),
        "{text}"
    );
    assert!(fs::read_to_string(&ini)
        .unwrap()
        .contains("bInvalidateOlderFiles=1"));

    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&list).unwrap(), "*Theirs.esp\n");
    assert_eq!(
        fs::read_to_string(&ini).unwrap(),
        "[Archive]\nbInvalidateOlderFiles=0\n"
    );
    assert!(!root.join("Data/A.esp").exists());
}

#[test]
fn a_game_steam_does_not_run_through_proton_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("lib/steamapps/common/Native");
    write(&root.join("native.x86_64"), "elf");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let app = SteamApp {
        app_id: 4321,
        name: "Native".into(),
        install_dir: "Native".into(),
        path: root.clone(),
    };
    let err = s.add_steam_app(&app, None).unwrap_err().to_string();
    assert!(err.contains("not run through Proton"), "{err}");

    // A Windows game it no longer runs through Proton: nothing is deployed.
    write(
        &s.data().game("win").def_path(),
        "schema = 1\nname = \"Win\"\n[detect]\nsteam_app_id = 4321\nmarker = \"Game.exe\"\n\
         [launch]\nexe = \"Game.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    write(&root.join("Game.exe"), "MZ\x7f");
    s.set_install_path("win", &root).unwrap();
    let err = s
        .launch("win", DEFAULT_PROFILE, &|| false, &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(err.contains("not run through Proton"), "{err}");
    assert!(!s.data().game("win").journal_path().exists());
}

#[test]
fn a_folder_whose_game_keeps_files_in_windows_folders_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let plain = tmp.path().join("Plain");
    write(&plain.join("Plain.exe"), "exe");
    s.add_manual_game(&plain, "Plain", "Plain.exe", None)
        .unwrap();

    let ck3 = tmp.path().join("CK3");
    write(&ck3.join("binaries/ck3.exe"), "exe");
    write(&ck3.join("launcher/launcher-settings.json"), "{}");
    let err = s
        .add_manual_game(&ck3, "CK3", "binaries/ck3.exe", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Windows' user folders"), "{err}");
}
