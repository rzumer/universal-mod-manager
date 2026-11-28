//! Balatro-style games with Lovely from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_sources::net::FakeHttp;
use umm_sources::thunderstore::ecosystem::ECOSYSTEM_URL;

const INDEX: &str = "https://thunderstore.io/c/jokers/api/v1/package-listing-index/";

// As for Balatro: no install rules, and Lovely as the loader package.
const ECOSYSTEM: &str = r#"{
  "games": {
    "jokers": {"label": "jokers",
      "distributions": [{"platform": "steam", "identifier": "6262"}],
      "r2modman": [{"packageLoader": "lovely", "exeNames": ["Jokers.exe"],
        "dataFolderName": "Jokers_Data",
        "packageIndex": "https://thunderstore.io/c/jokers/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "Thunderstore-lovely", "rootFolder": "", "loader": "lovely"}]
}"#;

const LISTING: &str = r#"[
  {"name": "lovely", "full_name": "Thunderstore-lovely", "owner": "Thunderstore",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "0.10.0", "dependencies": []}]},
  {"name": "smods", "full_name": "Steamodded-smods", "owner": "Steamodded", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["Thunderstore-lovely-0.10.0"]}]}
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
            r#"{"name":"lovely","version_number":"0.10.0","dependencies":[]}"#,
        ),
        ("README.md", "readme"),
        ("icon.png", "png"),
        ("winmm.dll", "proxy"),
        ("lovely/config.toml", "[workspace]"),
    ]);
    let smods = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"smods","version_number":"1.0.0","dependencies":["Thunderstore-lovely-0.10.0"]}"#,
        ),
        ("icon.png", "png"),
        ("lovely.toml", "[manifest]"),
        ("src/core.lua", "-- core"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/Thunderstore/lovely/0.10.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Steamodded/smods/1.0.0/",
                smods,
            ),
    )
}

#[test]
fn lovely_games_run_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Jokers.exe"), "game").unwrap();
    let p = s.data().game("jokers").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Jokers\"\n[detect]\nsteam_app_id = 6262\n\
         [launch]\nexe = \"Jokers.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("jokers", &root).unwrap();

    let c = s.loader_choice("jokers").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("lovely"));
    assert_eq!(
        c.available,
        [LoaderSetup::Thunderstore, LoaderSetup::Generic]
    );
    s.set_loader("jokers", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("jokers").unwrap().setup_label(),
        "Lovely from Thunderstore"
    );

    let page = s.ts_search("jokers", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Lovely");
    assert_eq!(page.loader_package.as_deref(), Some("Thunderstore-lovely"));

    let err = s
        .deploy("jokers", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Lovely"), "{err}");

    s.ts_install(
        "jokers",
        &[("Steamodded-smods".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("jokers", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // Only the proxy goes into the game folder; Lovely's folder and the mods to the runtime one.
    assert_eq!(fs::read_to_string(root.join("winmm.dll")).unwrap(), "proxy");
    assert!(!root.join("mods").exists() && !root.join("lovely").exists());
    let rt = s.game_dir("jokers").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "mods/lovely/config.toml",
        "mods/Steamodded-smods/lovely.toml",
        "mods/Steamodded-smods/src/core.lua",
        "mods/Steamodded-smods/manifest.json",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("jokers", false, &mut Faults::none()).unwrap();
    assert!(!root.join("winmm.dll").exists());
}
