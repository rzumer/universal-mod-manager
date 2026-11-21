//! Adding games from a (fake) Steam library.

use std::fs;
use std::path::Path;
use umm_core::{Engine, Session};
use umm_gamedef::steam::installed_games_in;

fn touch(p: &Path) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, b"x").unwrap();
}

#[test]
fn adds_a_steam_game_once_and_finds_its_install() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib");
    fs::create_dir_all(lib.join("steamapps")).unwrap();
    fs::write(
        lib.join("steamapps/appmanifest_4242.acf"),
        "\"AppState\" { \"appid\" \"4242\" \"name\" \"Test Quest\" \"installdir\" \"TestQuest\" }",
    )
    .unwrap();
    let root = lib.join("steamapps/common/TestQuest");
    touch(&root.join("TestQuest.exe"));
    touch(&root.join("UnityPlayer.dll"));
    touch(&root.join("TestQuest_Data/x"));

    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let apps = installed_games_in(&[lib]);
    let c = s.steam_candidates(apps.clone());
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].engine, Engine::UnityMono);
    assert!(c[0].defined_as.is_none());

    let d = s.add_steam_app(&apps[0]).unwrap();
    assert_eq!(d.def.id, "test-quest");
    let file = s.data().gamedefs_dir().join("test-quest.toml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("# Generated from Steam by UMM"), "{text}");

    // Loaded like any other definition, with the Steam ids filled in.
    let def = s.def("test-quest").unwrap();
    assert_eq!(def.def.detect.steam_app_id, Some(4242));
    assert!(def.def.doorstop().is_some());
    assert_eq!(
        s.steam_candidates(apps.clone())[0].defined_as.as_deref(),
        Some("test-quest")
    );

    // A second add is refused and the user's edits are never overwritten.
    fs::write(
        &file,
        text.replace("name = \"Test Quest\"", "name = \"My Quest\""),
    )
    .unwrap();
    let err = s.add_steam_app(&apps[0]).unwrap_err();
    assert!(err.to_string().contains("already set up"), "{err}");
    assert!(fs::read_to_string(&file).unwrap().contains("My Quest"));

    // The install folder is found from the definition (here through a manual path, since this
    // fake library is not registered with a real Steam install).
    s.set_install_path("test-quest", &root).unwrap();
    assert!(s.status().unwrap()[0].install.is_some());
}

#[test]
fn a_taken_id_gets_the_app_id_appended() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib");
    fs::create_dir_all(lib.join("steamapps")).unwrap();
    fs::write(
        lib.join("steamapps/appmanifest_7.acf"),
        "\"AppState\" { \"appid\" \"7\" \"name\" \"Clash\" \"installdir\" \"Clash\" }",
    )
    .unwrap();
    touch(&lib.join("steamapps/common/Clash/clash.exe"));
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    fs::write(
        s.data().gamedefs_dir().join("clash.toml"),
        "schema = 1\nid = \"clash\"\nname = \"Another Clash\"\n",
    )
    .unwrap();
    let apps = installed_games_in(&[lib]);
    assert_eq!(s.add_steam_app(&apps[0]).unwrap().def.id, "clash-7");
}
