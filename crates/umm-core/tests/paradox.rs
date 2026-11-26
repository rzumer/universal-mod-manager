//! Deploying outside the game folder: a Paradox game's mods go in its user folder, each with
//! a descriptor UMM writes, enabled in `dlc_load.json` after the user's own; all of it
//! comes back out afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "ck3";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

#[test]
fn a_paradox_games_mods_go_in_its_user_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("binaries/ck3.exe"), b"exe");
    let user = tmp
        .path()
        .join("docs/Paradox Interactive/Crusader Kings III");
    let original = r#"{"enabled_mods":["mod/ugc_42.mod"],"disabled_dlcs":["dlc/dlc001.dlc"]}"#;
    write(&user.join("dlc_load.json"), original.as_bytes());
    write(&user.join("mod/ugc_42.mod"), b"name=\"Theirs\"");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let user_toml = user.display().to_string().replace('\\', "/");
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"CK3\"\n[detect]\nmarker = \"binaries/ck3.exe\"\n\
             [launch]\nexe = \"binaries/ck3.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mod/{{mod}}/\"\n\
             [folders]\nuser = \"{user_toml}\"\n[paradox]\nfolder = \"user\"\n"
        )
        .as_bytes(),
    );
    s.set_install_path(GAME, &root).unwrap();
    // As archives have it: the mod's folder with its descriptor, the launcher's `.mod` beside.
    let src = tmp.path().join("src/barons");
    write(
        &src.join("Better Barons/descriptor.mod"),
        b"version=\"1.2\"\nname=\"Better Barons\"\npath=\"C:/somewhere/else\"\n",
    );
    write(&src.join("Better Barons/common/barons.txt"), b"barons");
    write(
        &src.join("Better Barons.mod"),
        b"path=\"mod/Better Barons\"",
    );
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some("barons".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    let targets: Vec<&str> = plan.files.iter().map(|f| f.target.as_str()).collect();
    assert_eq!(
        targets,
        [
            "@user/mod/barons/common/barons.txt",
            "@user/mod/barons/descriptor.mod"
        ]
    );

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(user.join("mod/barons/common/barons.txt")).unwrap(),
        b"barons"
    );
    assert!(!root.join("mod").exists(), "nothing in the game folder");
    let d = fs::read_to_string(user.join("mod/umm-barons.mod")).unwrap();
    assert!(d.contains("name=\"Better Barons\""), "{d}");
    let folder = user
        .join("mod/barons")
        .display()
        .to_string()
        .replace('\\', "/");
    assert!(d.contains(&format!("path=\"{folder}\"")), "{d}");
    assert!(!d.contains("somewhere/else"));
    let list: serde_json::Value =
        serde_json::from_slice(&fs::read(user.join("dlc_load.json")).unwrap()).unwrap();
    assert_eq!(
        list["enabled_mods"],
        serde_json::json!(["mod/ugc_42.mod", "mod/umm-barons.mod"])
    );
    assert_eq!(list["disabled_dlcs"], serde_json::json!(["dlc/dlc001.dlc"]));

    // Afterwards: the user folder as it was.
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!user.join("mod/barons").exists());
    assert!(!user.join("mod/umm-barons.mod").exists());
    assert!(user.join("mod/ugc_42.mod").is_file());
    assert_eq!(
        fs::read_to_string(user.join("dlc_load.json")).unwrap(),
        original
    );
}

/// Crusader Kings II: mods come as a launcher `.mod` beside their folder, and are enabled in
/// `settings.txt`'s `last_mods`, the rest of which is the game's: kept when it changes.
#[test]
fn crusader_kings_ii_mods_are_enabled_in_its_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("CK2game.exe"), b"exe");
    let user = tmp
        .path()
        .join("docs/Paradox Interactive/Crusader Kings II");
    let original = "language=\"l_english\"\r\nlast_mods={\r\n\t\"mod/theirs.mod\"\r\n}\r\nsize={\r\n\tx=1920\r\n}\r\n";
    write(&user.join("settings.txt"), original.as_bytes());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let user_toml = user.display().to_string().replace('\\', "/");
    write(
        &s.data().game("ck2").def_path(),
        format!(
            "schema = 1\nname = \"CK2\"\n[detect]\nmarker = \"CK2game.exe\"\n\
             [launch]\nexe = \"CK2game.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mod/{{mod}}/\"\n\
             [folders]\nuser = \"{user_toml}\"\n[paradox]\nfolder = \"user\"\nlist = \"settings\"\n"
        )
        .as_bytes(),
    );
    s.set_install_path("ck2", &root).unwrap();
    let src = tmp.path().join("src/agot");
    write(
        &src.join("A Game of Thrones.mod"),
        b"name=\"A Game of Thrones\"\npath=\"mod/A Game of Thrones\"\nreplace_path=\"history/titles\"\nuser_dir=\"AGOT\"\n",
    );
    write(&src.join("A Game of Thrones/common/houses.txt"), b"houses");
    write(&src.join("readme.txt"), b"read me");
    s.import(
        "ck2",
        &src,
        ImportOptions {
            id: Some("agot".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    let plan = s.plan("ck2", DEFAULT_PROFILE).unwrap();
    let targets: Vec<&str> = plan.files.iter().map(|f| f.target.as_str()).collect();
    assert_eq!(
        targets,
        ["@user/mod/agot/common/houses.txt"],
        "the folder beside the launcher's .mod, the .mod and readme left out"
    );

    s.deploy("ck2", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(user.join("mod/agot/common/houses.txt")).unwrap(),
        b"houses"
    );
    let d = fs::read_to_string(user.join("mod/umm_agot.mod")).unwrap();
    assert!(d.contains("replace_path=\"history/titles\""), "{d}");
    assert!(d.contains("user_dir=\"AGOT\""), "{d}");
    assert!(
        d.ends_with("path=\"mod/agot\"\n"),
        "relative, as the game reads it: {d}"
    );
    let settings = fs::read_to_string(user.join("settings.txt")).unwrap();
    assert_eq!(
        settings,
        "language=\"l_english\"\r\nlast_mods={\r\n\t\"mod/theirs.mod\"\r\n\t\"mod/umm_agot.mod\"\r\n}\r\nsize={\r\n\tx=1920\r\n}\r\n"
    );

    // Started past its launcher, the game is given them as arguments, the user's own too.
    let def = s.def("ck2").unwrap();
    assert_eq!(
        s.paradox_mod_args(&def, &s.profile("ck2", DEFAULT_PROFILE).unwrap()),
        ["-mod=mod/theirs.mod", "-mod=mod/umm_agot.mod"]
    );

    // The game rewrites its settings while playing: those are kept, UMM's entry goes.
    write(
        &user.join("settings.txt"),
        b"language=\"l_english\"\nsize={\n\tx=2560\n}\nlast_mods={\n\t\"mod/theirs.mod\"\n\t\"mod/umm_agot.mod\"\n}\n",
    );
    s.revert("ck2", false, &mut Faults::none()).unwrap();
    assert!(!user.join("mod/agot").exists());
    assert!(!user.join("mod/umm_agot.mod").exists());
    assert_eq!(
        fs::read_to_string(user.join("settings.txt")).unwrap(),
        "language=\"l_english\"\nsize={\n\tx=2560\n}\nlast_mods={\n\t\"mod/theirs.mod\"\n}\n"
    );
}

/// Before the game has written its settings: UMM writes the list alone, and takes the file
/// away again if the game added nothing to it.
#[test]
fn crusader_kings_ii_settings_made_for_the_list_go_again() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("CK2game.exe"), b"exe");
    let user = tmp
        .path()
        .join("docs/Paradox Interactive/Crusader Kings II");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("ck2").def_path(),
        format!(
            "schema = 1\nname = \"CK2\"\n[[install]]\nmatch = \"**\"\ntarget = \"@user/mod/{{mod}}/\"\n\
             [folders]\nuser = \"{}\"\n[paradox]\nfolder = \"user\"\nlist = \"settings\"\n",
            user.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    s.set_install_path("ck2", &root).unwrap();
    let src = tmp.path().join("src/m");
    write(
        &src.join("Plain.mod"),
        b"name=\"Plain\"\npath=\"mod/Plain\"\n",
    );
    write(&src.join("Plain/events/e.txt"), b"e");
    s.import(
        "ck2",
        &src,
        ImportOptions {
            id: Some("plain-old".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    s.deploy("ck2", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(user.join("settings.txt")).unwrap(),
        "last_mods={\n\t\"mod/umm_plain_old.mod\"\n}\n"
    );
    s.revert("ck2", false, &mut Faults::none()).unwrap();
    assert!(!user.join("settings.txt").exists());
}

#[test]
fn a_named_folder_that_does_not_exist_yet_is_made_and_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Game.exe"), b"exe");
    let docs = tmp.path().join("docs/Studio/Game/mods");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("g").def_path(),
        format!(
            "schema = 1\nname = \"G\"\n[[install]]\nmatch = \"**\"\ntarget = \"@mods/{{mod}}/\"\n\
             [folders]\nmods = \"{}\"\n",
            docs.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    s.set_install_path("g", &root).unwrap();
    let src = tmp.path().join("src/m");
    write(&src.join("a.txt"), b"a");
    s.import(
        "g",
        &src,
        ImportOptions {
            id: Some("m".into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    s.deploy("g", DEFAULT_PROFILE, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(docs.join("m/a.txt")).unwrap(), b"a");
    s.revert("g", false, &mut Faults::none()).unwrap();
    assert!(!docs.exists(), "the folder UMM made is gone again");
}
