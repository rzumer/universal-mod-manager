//! Unity games with MelonLoader from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/melon-quest/api/v1/package-listing-index/";

// As for Schedule I: MelonLoader 0.7's layout, no install rules, and the MelonLoader package
// listed once for each MelonLoader kind.
const ECOSYSTEM: &str = r#"{
  "games": {
    "melon-quest": {"label": "melon-quest",
      "distributions": [{"platform": "steam", "identifier": "5151"}],
      "r2modman": [{"packageLoader": "recursive-melonloader", "exeNames": ["Melon Quest.exe"],
        "dataFolderName": "Melon Quest_Data",
        "packageIndex": "https://thunderstore.io/c/melon-quest/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [
    {"packageId": "LavaGang-MelonLoader", "rootFolder": "", "loader": "melonloader"},
    {"packageId": "LavaGang-MelonLoader", "rootFolder": "", "loader": "recursive-melonloader"}
  ]
}"#;

const LISTING: &str = r#"[
  {"name": "MelonLoader", "full_name": "LavaGang-MelonLoader", "owner": "LavaGang",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "0.7.3", "dependencies": []}, {"version_number": "0.6.6", "dependencies": []}]},
  {"name": "Sled", "full_name": "Bob-Sled", "owner": "Bob", "categories": ["Mods"],
   "versions": [{"version_number": "2.0.0", "dependencies": ["LavaGang-MelonLoader-0.6.6"]}]},
  {"name": "Seeds", "full_name": "Ann-Seeds", "owner": "Ann", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["LavaGang-MelonLoader-0.7.3"]}]}
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
        ("manifest.json", r#"{"name":"MelonLoader","version_number":"0.7.3","dependencies":[]}"#),
        ("README.md", "readme"),
        ("icon.png", "png"),
        ("version.dll", "proxy"),
        ("MelonLoader/net6/MelonLoader.dll", "ml"),
        ("MelonLoader/net35/MelonLoader.dll", "ml35"),
        ("MelonLoader/Documentation/LICENSE.md", "license"),
    ]);
    let seeds = zip_of(&[
        ("manifest.json", r#"{"name":"Seeds","version_number":"1.0.0","dependencies":["LavaGang-MelonLoader-0.7.3"]}"#),
        ("icon.png", "png"),
        ("Seeds.dll", "seeds"),
        ("Assets/seeds.bundle", "bundle"),
        ("UserData/seeds.cfg", "fast=true"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/LavaGang/MelonLoader/0.7.3/",
                pack,
            )
            .with("https://thunderstore.io/package/download/Ann/Seeds/1.0.0/", seeds)
            .with(
                "https://thunderstore.io/package/download/Bob/Sled/2.0.0/",
                zip_of(&[
                    ("manifest.json", r#"{"name":"Sled","version_number":"2.0.0","dependencies":["LavaGang-MelonLoader-0.6.6"]}"#),
                    ("Sled.dll", "sled"),
                ]),
            ),
    )
}

/// A generic Unity game, as Add from Steam writes it without Thunderstore.
fn game(data: &Path, root: &Path) -> Session {
    let s = Session::open_with(Some(data.to_path_buf()), fake()).unwrap();
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("Melon Quest.exe"), "game").unwrap();
    fs::write(root.join("UnityPlayer.dll"), "unity").unwrap();
    let p = s.data().game("melon-quest").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Melon Quest\"\n\
         [detect]\nsteam_app_id = 5151\n\
         [launch]\nexe = \"Melon Quest.exe\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("melon-quest", root).unwrap();
    s
}

#[test]
fn melonloader_games_run_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = game(&tmp.path().join("data"), &root);

    let c = s.loader_choice("melon-quest").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("recursive-melonloader"));
    assert_eq!(c.available, [LoaderSetup::Thunderstore, LoaderSetup::Generic]);
    s.set_loader("melon-quest", LoaderSetup::Thunderstore).unwrap();
    let def = s.game_def("melon-quest").unwrap();
    assert_eq!(def.loader.as_ref().unwrap().kind, "recursive-melonloader");
    assert_eq!(def.setup_label(), "MelonLoader from Thunderstore");

    // The package listed for both kinds is still the game's loader.
    let page = s.ts_search("melon-quest", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "MelonLoader");
    assert_eq!(page.loader_package.as_deref(), Some("LavaGang-MelonLoader"));

    let err = s.deploy("melon-quest", DEFAULT_PROFILE, &mut Faults::none()).unwrap_err();
    assert!(err.to_string().contains("no MelonLoader"), "{err}");

    s.ts_install("melon-quest", &[("Ann-Seeds".into(), None)], DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    let page = s.ts_search("melon-quest", &TsQuery::default()).unwrap();
    assert!(page.loader_installed);

    s.deploy("melon-quest", DEFAULT_PROFILE, &mut Faults::none()).unwrap();
    // Only the proxy goes into the game folder.
    assert_eq!(fs::read_to_string(root.join("version.dll")).unwrap(), "proxy");
    for d in ["MelonLoader", "Mods", "UserData"] {
        assert!(!root.join(d).exists(), "{d}");
    }
    assert!(!root.join("README.md").exists());

    // MelonLoader and each mod, in its own folder, go to the runtime folder.
    let rt = s.game_dir("melon-quest").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "MelonLoader/net6/MelonLoader.dll",
        "Mods/Ann-Seeds/Seeds.dll",
        "Mods/Ann-Seeds/Assets/seeds.bundle",
        "UserData/Ann-Seeds/seeds.cfg",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }
    // MelonLoader only loads a mod folder with the package's manifest in it; the package's
    // files are also kept beside the mod, as for every package.
    assert!(rt.join("Mods/Ann-Seeds/manifest.json").is_file());
    let package = s.game_dir("melon-quest").mod_package_dir("ann-seeds");
    assert!(package.join("icon.png").is_file() && package.join("manifest.json").is_file());

    s.revert("melon-quest", false, &mut Faults::none()).unwrap();
    assert!(!root.join("version.dll").exists());
    assert!(root.join("UnityPlayer.dll").is_file());
}

#[test]
fn a_mod_naming_an_old_melonloader_gets_a_newer_one() {
    let tmp = tempfile::tempdir().unwrap();
    let s = game(&tmp.path().join("data"), &tmp.path().join("game"));
    s.set_loader("melon-quest", LoaderSetup::Thunderstore).unwrap();

    // MelonLoader 0.6.6 cannot load mods from their own folders, so 0.7.3 comes instead.
    let r = s
        .ts_install("melon-quest", &[("Bob-Sled".into(), None)], DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    let installed: Vec<(&str, &str)> = r.installed.iter().map(|(n, v, _)| (n.as_str(), v.as_str())).collect();
    assert!(installed.contains(&("LavaGang-MelonLoader", "0.7.3")), "{installed:?}");
    assert!(r.notes.iter().any(|n| n.contains("0.7.3 instead of 0.6.6")), "{:?}", r.notes);
    s.deploy("melon-quest", DEFAULT_PROFILE, &mut Faults::none()).unwrap();
}
