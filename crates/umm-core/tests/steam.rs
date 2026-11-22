//! Adding games from a (fake) Steam library, with a (fake) Thunderstore game list.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use umm_core::{Engine, LoaderSetup, Session};
use umm_gamedef::steam::{installed_games_in, SteamApp};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

fn touch(p: &Path) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, b"x").unwrap();
}

/// Thunderstore knows app 4242 as the BepInEx community `test-quest`, and app 5150 as a
/// game whose loader (BepisLoader) UMM does not support.
const ECOSYSTEM: &str = r#"{
  "games": {
    "test-quest": {"label": "test-quest", "meta": {"displayName": "Test Quest"},
      "distributions": [{"platform": "steam", "identifier": "4242"}],
      "r2modman": [{"packageLoader": "bepinex", "installRules": [], "exeNames": ["TestQuest.exe"]}]},
    "melon-game": {"label": "melon-game",
      "distributions": [{"platform": "steam", "identifier": "5150"}],
      "r2modman": [{"packageLoader": "bepisloader", "installRules": []}]}
  },
  "modloaderPackages": []
}"#;

fn unity_game(lib: &Path, app_id: u32, name: &str, dir: &str) -> PathBuf {
    fs::create_dir_all(lib.join("steamapps")).unwrap();
    fs::write(
        lib.join(format!("steamapps/appmanifest_{app_id}.acf")),
        format!("\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"{name}\" \"installdir\" \"{dir}\" }}"),
    )
    .unwrap();
    let root = lib.join("steamapps/common").join(dir);
    touch(&root.join(format!("{dir}.exe")));
    touch(&root.join("UnityPlayer.dll"));
    touch(&root.join(format!("{dir}_Data/x")));
    root
}

fn session(data: &Path, online: bool) -> Session {
    let http = if online {
        FakeHttp::default().with(ECOSYSTEM_URL, ECOSYSTEM)
    } else {
        FakeHttp::default()
    };
    Session::open_with(Some(data.to_path_buf()), Arc::new(http)).unwrap()
}

fn app(apps: &[SteamApp], id: u32) -> &SteamApp {
    apps.iter().find(|a| a.app_id == id).unwrap()
}

#[test]
fn thunderstore_games_default_to_bepinex_from_thunderstore() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib");
    let root = unity_game(&lib, 4242, "Test Quest", "TestQuest");
    unity_game(&lib, 7, "Other Unity", "OtherUnity");
    let s = session(&tmp.path().join("data"), true);
    let apps = installed_games_in(&[lib]);

    let list = s.steam_list(apps.clone());
    assert!(list.thunderstore_error.is_none());
    let c = list.games.iter().find(|c| c.app.app_id == 4242).unwrap();
    assert_eq!(c.engine, Engine::UnityMono);
    assert_eq!(c.community.as_deref(), Some("test-quest"));
    assert_eq!(c.default_setup, LoaderSetup::Thunderstore);
    let other = list.games.iter().find(|c| c.app.app_id == 7).unwrap();
    assert_eq!(
        other.default_setup,
        LoaderSetup::Generic,
        "not on Thunderstore"
    );
    assert_eq!(other.setups, [LoaderSetup::Generic, LoaderSetup::Bepinex]);

    let d = s.add_steam_app(app(&apps, 4242), None).unwrap();
    assert_eq!(d.def.id, "test-quest");
    let file = s.data().game("test-quest").def_path();
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("# Generated from Steam by UMM"), "{text}");
    assert!(text.contains("community = \"test-quest\""), "{text}");
    assert!(
        !text.lines().any(|l| l.starts_with("id =")),
        "the folder is the id"
    );

    // Loaded like any other definition, with the Steam ids filled in.
    let def = s.def("test-quest").unwrap();
    assert_eq!(def.def.detect.steam_app_id, Some(4242));
    assert_eq!(def.def.setup(), Some(LoaderSetup::Thunderstore));
    let list = s.steam_list(apps.clone());
    let c = list.games.iter().find(|c| c.app.app_id == 4242).unwrap();
    assert_eq!(c.defined_as.as_deref(), Some("test-quest"));

    // A second add is refused and the user's edits are never overwritten.
    fs::write(
        &file,
        text.replace("name = \"Test Quest\"", "name = \"My Quest\""),
    )
    .unwrap();
    let err = s.add_steam_app(app(&apps, 4242), None).unwrap_err();
    assert!(err.to_string().contains("already set up"), "{err}");
    assert!(fs::read_to_string(&file).unwrap().contains("My Quest"));

    // A folder picked in UMM goes into the same file; the user's edit stays.
    s.set_install_path("test-quest", &root).unwrap();
    assert!(s.status().unwrap()[0].install.is_some());
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("My Quest") && text.contains("path = "),
        "{text}"
    );

    // Other Unity games start generic, and can be added with BepInEx instead.
    let g = s.add_steam_app(app(&apps, 7), None).unwrap();
    assert!(g.def.loader.is_none());
    assert!(s
        .add_steam_app(app(&apps, 7), Some(LoaderSetup::Bepinex))
        .is_err());
}

#[test]
fn switching_the_loader_rewrites_only_its_sections() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib");
    unity_game(&lib, 4242, "Test Quest", "TestQuest");
    unity_game(&lib, 7, "Other Unity", "OtherUnity");
    let s = session(&tmp.path().join("data"), true);
    let apps = installed_games_in(&[lib]);
    s.add_steam_app(app(&apps, 4242), None).unwrap();
    s.add_steam_app(app(&apps, 7), None).unwrap();
    let file = s.data().game("test-quest").def_path();
    let with_note = format!("# my own note\n{}", fs::read_to_string(&file).unwrap());
    fs::write(&file, &with_note).unwrap();

    let c = s.set_loader("test-quest", LoaderSetup::Generic).unwrap();
    assert_eq!(c.current, Some(LoaderSetup::Generic));
    assert!(s.def("test-quest").unwrap().def.loader.is_none());
    assert!(s.status().unwrap()[1].def.loader.is_none());
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("# my own note") && !text.contains("[loader]"),
        "{text}"
    );

    let c = s.set_loader("test-quest", LoaderSetup::Bepinex).unwrap();
    assert_eq!(c.current, Some(LoaderSetup::Bepinex));
    // Back to Thunderstore: the community is found again from the Steam app id.
    let c = s
        .set_loader("test-quest", LoaderSetup::Thunderstore)
        .unwrap();
    assert_eq!(c.current, Some(LoaderSetup::Thunderstore));
    assert_eq!(c.community.as_deref(), Some("test-quest"));
    assert!(fs::read_to_string(&file).unwrap().contains("# my own note"));

    // A generic game off Thunderstore cannot pick Thunderstore, but can pick BepInEx.
    let c = s.loader_choice("other-unity").unwrap();
    assert_eq!(c.available, [LoaderSetup::Bepinex, LoaderSetup::Generic]);
    assert!(s
        .set_loader("other-unity", LoaderSetup::Thunderstore)
        .is_err());
    let c = s.set_loader("other-unity", LoaderSetup::Bepinex).unwrap();
    assert_eq!(c.current, Some(LoaderSetup::Bepinex));
    assert_eq!(
        s.def("other-unity")
            .unwrap()
            .def
            .loader
            .unwrap()
            .source_mod(),
        Some("bepinex")
    );
}

#[test]
fn unsupported_loaders_and_offline_start_generic() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib");
    unity_game(&lib, 5150, "Melon Game", "MelonGame");
    unity_game(&lib, 4242, "Test Quest", "TestQuest");
    let apps = installed_games_in(&[lib]);

    let s = session(&tmp.path().join("online"), true);
    let d = s.add_steam_app(app(&apps, 5150), None).unwrap();
    assert!(d.def.loader.is_none());
    assert!(
        d.notes.iter().any(|n| n.key == "thunderstore-other-loader"),
        "{:?}",
        d.notes
    );

    let offline = session(&tmp.path().join("offline"), false);
    let list = offline.steam_list(apps.clone());
    assert!(list.thunderstore_error.is_some());
    assert!(list
        .games
        .iter()
        .all(|c| c.default_setup == LoaderSetup::Generic));
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
    let s = session(&tmp.path().join("data"), false);
    let taken = s.data().game("clash").def_path();
    fs::create_dir_all(taken.parent().unwrap()).unwrap();
    fs::write(&taken, "schema = 1\nname = \"Another Clash\"\n").unwrap();
    let apps = installed_games_in(&[lib]);
    assert_eq!(s.add_steam_app(&apps[0], None).unwrap().def.id, "clash-7");
}

#[test]
fn a_game_can_be_added_manually_from_its_folder() {
    let tmp = tempfile::tempdir().unwrap();
    // A copy of the Thunderstore game outside Steam: recognised by its exe name.
    let root = tmp.path().join("Games/Test Quest (GOG)");
    touch(&root.join("TestQuest.exe"));
    touch(&root.join("UnityPlayer.dll"));
    touch(&root.join("TestQuest_Data/x"));
    touch(&root.join("unins000.exe"));
    let s = session(&tmp.path().join("data"), true);

    let info = s.inspect_game_folder(&root).unwrap();
    assert_eq!(info.engine, Engine::UnityMono);
    assert_eq!(info.name, "Test Quest (GOG)");
    assert_eq!(info.exes, ["TestQuest.exe"], "uninstaller left out");
    assert_eq!(info.community.as_deref(), Some("test-quest"));
    assert_eq!(info.setups[0], LoaderSetup::Thunderstore);
    assert!(info.steam_app.is_none());

    let d = s
        .add_manual_game(&root, "Test Quest", "TestQuest.exe", None)
        .unwrap();
    assert_eq!(d.def.id, "test-quest");
    let st = &s.status().unwrap()[0];
    assert_eq!(st.found_via, Some("manual"));
    assert_eq!(st.def.setup(), Some(LoaderSetup::Thunderstore));
    let launch = st.def.launch.as_ref().unwrap();
    assert_eq!(launch.exe, "TestQuest.exe");
    assert_eq!(launch.steam_relaunch, umm_gamedef::SteamRelaunch::None);
    let file = std::fs::read_to_string(s.data().game("test-quest").def_path()).unwrap();
    assert!(
        !file.contains("steam_relaunch"),
        "not a Steam game:\n{file}"
    );

    // The same name again gets the folder's name appended; bad input is refused.
    let again = s
        .add_manual_game(&root, "Test Quest", "", Some(LoaderSetup::Generic))
        .unwrap();
    assert_eq!(again.def.id, "test-quest-test-quest-gog");
    assert!(again.def.launch.is_none());
    assert!(s
        .add_manual_game(&root, " ", "TestQuest.exe", None)
        .is_err());
    assert!(s.add_manual_game(&root, "X", "Missing.exe", None).is_err());
    assert!(s.add_manual_game(&root, "X", "../evil.exe", None).is_err());
}
