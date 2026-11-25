//! Icons for the interface: a mod's from its package, and a game's from its exe, kept once
//! found.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;
use umm_core::{ImportOptions, LoaderSetup, Session, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really";

#[test]
fn a_mod_shows_the_icon_its_package_came_with() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Game.exe"), b"exe");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("g").def_path(),
        b"schema = 1\nname = \"G\"\n[detect]\nmarker = \"Game.exe\"\n[launch]\nexe = \"Game.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
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

    // A mod without a package picture, and not from Nexus Mods: none, and no network.
    assert_eq!(s.mod_icon("g", "m").unwrap(), None);
    // Its package's icon.png, as Thunderstore packages keep it.
    write(&s.game_dir("g").mod_package_dir("m").join("icon.png"), PNG);
    let url = s.mod_icon("g", "m").unwrap().unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");

    // The exe here is no Windows exe: no icon, and nothing kept.
    assert_eq!(s.game_icon("g").unwrap(), None);
    assert!(!s.game_dir("g").generated_dir().join("icon.ico").exists());
}

/// A game's exe icon, read once and kept in its data folder.
#[cfg(windows)]
#[test]
fn a_game_shows_its_exe_icon() {
    let explorer =
        std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("explorer.exe");
    if !explorer.is_file() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Game.exe"), &fs::read(&explorer).unwrap());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("g").def_path(),
        b"schema = 1\nname = \"G\"\n[detect]\nmarker = \"Game.exe\"\n[launch]\nexe = \"Game.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path("g", &root).unwrap();
    let url = s.game_icon("g").unwrap().unwrap();
    assert!(url.starts_with("data:image/"), "{}", &url[..40]);
    let kept = s.game_dir("g").generated_dir();
    assert!(kept.join("icon.ico").is_file() || kept.join("icon.png").is_file());
    // From the kept copy now, the exe gone.
    fs::remove_file(root.join("Game.exe")).unwrap();
    assert_eq!(s.game_icon("g").unwrap().unwrap(), url);
}

const ECOSYSTEM: &str = r#"{
  "games": {
    "jokers": {"label": "jokers",
      "distributions": [{"platform": "steam", "identifier": "6262"}],
      "r2modman": [{"packageLoader": "lovely", "exeNames": ["Jokers.exe"], "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/jokers/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "Thunderstore-lovely", "rootFolder": "", "loader": "lovely"}]
}"#;

const ICON_URL: &str = "https://gcdn.thunderstore.io/live/repository/icons/Me-Mine-1.0.0.png";

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

/// A package installed before UMM kept package files (with their icon) gets the icon
/// Thunderstore lists for it, once.
#[test]
fn an_older_thunderstore_install_gets_its_listed_icon() {
    let listing = format!(
        r#"[
  {{"name": "lovely", "full_name": "Thunderstore-lovely", "owner": "Thunderstore", "categories": [],
   "versions": [{{"version_number": "0.10.0", "dependencies": []}}]}},
  {{"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": [],
   "versions": [{{"version_number": "1.0.0", "dependencies": [], "icon": "{ICON_URL}"}}]}}
]"#
    );
    let http = Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(
                "https://thunderstore.io/c/jokers/api/v1/package-listing-index/",
                gz(r#"["https://cdn/chunk1"]"#),
            )
            .with("https://cdn/chunk1", gz(&listing))
            .with(
                "https://thunderstore.io/package/download/Thunderstore/lovely/0.10.0/",
                zip_of(&[("manifest.json", "{}"), ("winmm.dll", "proxy")]),
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                zip_of(&[
                    (
                        "manifest.json",
                        r#"{"name":"Mine","version_number":"1.0.0","dependencies":[]}"#,
                    ),
                    ("mine.lua", "-- mine"),
                ]),
            )
            .with(ICON_URL, PNG.to_vec()),
    );
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Jokers.exe"), b"game");
    let s = Session::open_with(Some(tmp.path().join("data")), http.clone()).unwrap();
    write(
        &s.data().game("jokers").def_path(),
        b"schema = 1\nname = \"Jokers\"\n[detect]\nsteam_app_id = 6262\n[launch]\nexe = \"Jokers.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path("jokers", &root).unwrap();
    s.set_loader("jokers", LoaderSetup::Thunderstore).unwrap();
    s.ts_install(
        "jokers",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    // As installed before package files were kept.
    let package = s.game_dir("jokers").mod_package_dir("me-mine");
    fs::remove_dir_all(&package).unwrap();

    let url = s.mod_icon("jokers", "me-mine").unwrap().unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");
    assert_eq!(fs::read(package.join("icon.png")).unwrap(), PNG, "kept");
    let fetched = || {
        http.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|u| *u == ICON_URL)
            .count()
    };
    assert_eq!(fetched(), 1);
    s.mod_icon("jokers", "me-mine").unwrap().unwrap();
    assert_eq!(fetched(), 1, "from the kept copy after that");

    // A package Thunderstore lists no icon for is not asked about again.
    let lovely = s.game_dir("jokers").mod_package_dir("thunderstore-lovely");
    let _ = fs::remove_dir_all(&lovely);
    assert_eq!(s.mod_icon("jokers", "thunderstore-lovely").unwrap(), None);
    assert!(lovely.join(".no-icon").exists());
}
