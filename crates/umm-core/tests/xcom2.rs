//! XCOM 2: each mod goes in `XComGame/Mods/` (War of the Chosen's), in a folder named after
//! its `.XComMod` file, and is enabled in the user's `XComModOptions.ini`; the user's list
//! comes back afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "xcom2";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// As the game's launcher writes it.
const ORIGINAL: &str =
    "[Engine.XComModOptions]\r\nActiveMods=\"Theirs\"\r\n\r\n[Other]\r\nKey=1\r\n";

#[test]
fn xcom_2_mods_are_named_after_their_file_and_enabled_in_its_ini() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Binaries/Win64/XCom2.exe"), b"exe");
    write(
        &root.join("XCom2-WarOfTheChosen/Binaries/Win64/XCom2.exe"),
        b"exe",
    );
    let ini = tmp
        .path()
        .join("docs/My Games/XCOM2 War of the Chosen/XComGame/Config/XComModOptions.ini");
    write(&ini, ORIGINAL.as_bytes());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"XCOM 2\"\n[detect]\nmarker = \"XCom2-WarOfTheChosen/Binaries/Win64/XCom2.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"XCom2-WarOfTheChosen/XComGame/Mods/{{mod}}/\"\n\
             [mod_list]\nlist = \"{}\"\nformat = \"xcom_ini\"\nfolder = \"XCom2-WarOfTheChosen/XComGame/Mods\"\n",
            ini.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    s.set_install_path(GAME, &root).unwrap();
    let src = tmp.path().join("src/better-ui");
    write(&src.join("BetterUI/BetterUI.XComMod"), b"[mod]");
    write(&src.join("BetterUI/Script/BetterUI.u"), b"script");
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some("better-ui".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let mods = root.join("XCom2-WarOfTheChosen/XComGame/Mods");
    assert!(mods.join("BetterUI/BetterUI.XComMod").is_file());
    assert!(mods.join("BetterUI/Script/BetterUI.u").is_file());
    assert!(!mods.join("better-ui").exists());
    assert_eq!(
        fs::read_to_string(&ini).unwrap(),
        "[Engine.XComModOptions]\r\nActiveMods=\"Theirs\"\r\nActiveMods=\"BetterUI\"\r\n\r\n[Other]\r\nKey=1\r\n"
    );

    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!mods.join("BetterUI").exists());
    assert_eq!(fs::read_to_string(&ini).unwrap(), ORIGINAL);
}
