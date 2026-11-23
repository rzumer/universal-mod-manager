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
