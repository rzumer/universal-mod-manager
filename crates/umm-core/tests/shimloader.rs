//! Unreal games with Shimloader from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/trip/api/v1/package-listing-index/";

// Thunderstore's entry for an Unreal game, as for Palworld.
const ECOSYSTEM: &str = r#"{
  "games": {
    "trip": {"label": "trip",
      "distributions": [{"platform": "steam", "identifier": "7777"}],
      "r2modman": [{"packageLoader": "shimloader", "exeNames": ["Trip.exe"], "dataFolderName": "Trip",
        "packageIndex": "https://thunderstore.io/c/trip/api/v1/package-listing-index/",
        "installRules": [
          {"route": "shimloader/mod", "trackingMethod": "subdir", "isDefaultLocation": true},
          {"route": "shimloader/pak", "trackingMethod": "subdir"},
          {"route": "shimloader/cfg", "trackingMethod": "none"},
          {"route": "shimloader/overlay", "trackingMethod": "subdir"}
        ]}]}
  },
  "modloaderPackages": [{"packageId": "Thunderstore-unreal_shimloader", "rootFolder": "", "loader": "shimloader"}]
}"#;

const LISTING: &str = r#"[
  {"name": "unreal_shimloader", "full_name": "Thunderstore-unreal_shimloader", "owner": "Thunderstore",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "1.1.7", "dependencies": []}]},
  {"name": "Glow", "full_name": "Ann-Glow", "owner": "Ann", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["Thunderstore-unreal_shimloader-1.1.7"]}]}
]"#;

fn gz(s: &str) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(s.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn fake() -> Arc<FakeHttp> {
    let pack = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"unreal_shimloader","version_number":"1.1.7","dependencies":[]}"#,
        ),
        ("dwmapi.dll", "shim"),
        ("UE4SS/dwmapi.dll", "ue4ss's own proxy"),
        ("UE4SS/UE4SS.dll", "ue4ss"),
        ("UE4SS/UE4SS-settings.ini", "[General]"),
        ("UE4SS/Mods/mods.txt", "Keybinds : 1"),
        ("UE4SS/Mods/Keybinds/Scripts/main.lua", "-- keys"),
    ]);
    let glow = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Glow","version_number":"1.0.0","dependencies":["Thunderstore-unreal_shimloader-1.1.7"]}"#,
        ),
        ("icon.png", "png"),
        ("mod/Scripts/main.lua", "-- glow"),
        ("pak/Glow_P.pak", "pak"),
        ("cfg/glow.cfg", "bright=true"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/Thunderstore/unreal_shimloader/1.1.7/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Ann/Glow/1.0.0/",
                glow,
            ),
    )
}

/// A generic Unreal game, as Add from Steam writes it without Thunderstore.
fn game(data: &Path, root: &Path) -> Session {
    let s = Session::open_with(Some(data.to_path_buf()), fake()).unwrap();
    fs::create_dir_all(root.join("Trip/Binaries/Win64")).unwrap();
    fs::write(root.join("Trip.exe"), "bootstrap").unwrap();
    fs::write(
        root.join("Trip/Binaries/Win64/Trip-Win64-Shipping.exe"),
        "game",
    )
    .unwrap();
    let p = s.data().game("trip").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Trip\"\n\
         [detect]\nsteam_app_id = 7777\n\
         [launch]\nexe = \"Trip.exe\"\nsteam_relaunch = \"appid_file\"\n\
         [[install]]\nmatch = \"*.pak\"\ntarget = \"Trip/Content/Paks/~mods/\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("trip", root).unwrap();
    s
}

#[test]
fn shimloader_games_run_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = game(&tmp.path().join("data"), &root);

    // Thunderstore's loader for the game is offered; BepInEx is not.
    let c = s.loader_choice("trip").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("shimloader"));
    assert_eq!(
        c.available,
        [LoaderSetup::Thunderstore, LoaderSetup::Generic]
    );
    s.set_loader("trip", LoaderSetup::Thunderstore).unwrap();
    let def = s.game_def("trip").unwrap();
    let loader = def.loader.as_ref().unwrap();
    assert_eq!(
        (loader.kind.as_str(), def.community()),
        ("shimloader", Some("trip"))
    );
    assert_eq!(def.setup_label(), "Shimloader from Thunderstore");

    let page = s.ts_search("trip", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Shimloader");
    assert_eq!(
        page.loader_package.as_deref(),
        Some("Thunderstore-unreal_shimloader")
    );
    assert!(!page.loader_installed);

    let err = s
        .deploy("trip", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Shimloader"), "{err}");

    // The mod brings Shimloader with it.
    s.ts_install(
        "trip",
        &[("Ann-Glow".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();

    s.deploy("trip", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let bin = root.join("Trip/Binaries/Win64");
    // Only Shimloader, UE4SS and its settings go next to the real exe, plus steam_appid.txt
    // there and at the root so the game does not restart through Steam without its arguments.
    for f in [
        "dwmapi.dll",
        "ue4ss.dll",
        "UE4SS-settings.ini",
        "steam_appid.txt",
    ] {
        assert!(bin.join(f).is_file(), "{f}");
    }
    assert_eq!(fs::read_to_string(bin.join("dwmapi.dll")).unwrap(), "shim");
    assert!(root.join("steam_appid.txt").is_file());
    assert!(!root.join("shimloader").exists());
    assert!(
        !root.join("Trip/Content").exists(),
        "the generic pak rule is gone"
    );

    // Mods, paks and configs go to the profile's runtime folder.
    let rt = s.game_dir("trip").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "shimloader/mod/Ann-Glow/Scripts/main.lua",
        "shimloader/mod/Keybinds/Scripts/main.lua",
        "shimloader/mod/mods.txt",
        "shimloader/pak/Ann-Glow/Glow_P.pak",
        "shimloader/cfg/glow.cfg",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }
    assert!(
        rt.join("shimloader/overlay").is_dir(),
        "every folder Shimloader is given exists"
    );

    // The package's icon and manifest are kept beside the mod, and deployed nowhere.
    let package = s.game_dir("trip").mod_package_dir("ann-glow");
    assert!(package.join("icon.png").is_file() && package.join("manifest.json").is_file());
    assert!(!rt.join("shimloader/mod/Ann-Glow/icon.png").exists());

    s.revert("trip", false, &mut Faults::none()).unwrap();
    for f in [
        "dwmapi.dll",
        "ue4ss.dll",
        "UE4SS-settings.ini",
        "steam_appid.txt",
    ] {
        assert!(!bin.join(f).exists(), "{f} reverted");
    }
    assert!(bin.join("Trip-Win64-Shipping.exe").is_file());
}
