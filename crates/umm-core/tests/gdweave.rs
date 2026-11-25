//! WEBFISHING-style games with GDWeave from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/fishing/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
const ECOSYSTEM: &str = r#"{
  "games": {
    "fishing": {"label": "fishing",
      "distributions": [{"platform": "steam", "identifier": "6263"}],
      "r2modman": [{"packageLoader": "gdweave", "exeNames": ["fishing.exe"],
        "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/fishing/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "NotNet-GDWeave", "rootFolder": "", "loader": "gdweave"}]
}"#;

const LISTING: &str = r#"[
  {"name": "GDWeave", "full_name": "NotNet-GDWeave", "owner": "NotNet",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "2.0.0", "dependencies": []}]},
  {"name": "Lure", "full_name": "Sulayre-Lure", "owner": "Sulayre", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["NotNet-GDWeave-2.0.0"]}]}
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
            r#"{"name":"GDWeave","version_number":"2.0.0","dependencies":[]}"#,
        ),
        ("README.md", "readme"),
        ("icon.png", "png"),
        ("winmm.dll", "proxy"),
        ("GDWeave/core/GDWeave.dll", "core"),
    ]);
    let lure = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Lure","version_number":"1.0.0","dependencies":["NotNet-GDWeave-2.0.0"]}"#,
        ),
        ("icon.png", "png"),
        (
            "GDWeave/mods/Sulayre.Lure/manifest.json",
            r#"{"Id":"Sulayre.Lure"}"#,
        ),
        ("GDWeave/mods/Sulayre.Lure/Lure.dll", "dll"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/NotNet/GDWeave/2.0.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Sulayre/Lure/1.0.0/",
                lure,
            ),
    )
}

#[test]
fn gdweave_games_run_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("fishing.exe"), "game").unwrap();
    let p = s.data().game("fishing").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Fishing\"\n[detect]\nsteam_app_id = 6263\n\
         [launch]\nexe = \"fishing.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("fishing", &root).unwrap();

    let c = s.loader_choice("fishing").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("gdweave"));
    assert_eq!(
        c.available,
        [LoaderSetup::Thunderstore, LoaderSetup::Generic]
    );
    s.set_loader("fishing", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("fishing").unwrap().setup_label(),
        "GDWeave from Thunderstore"
    );

    let page = s.ts_search("fishing", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "GDWeave");
    assert_eq!(page.loader_package.as_deref(), Some("NotNet-GDWeave"));

    let err = s
        .deploy("fishing", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no GDWeave"), "{err}");

    s.ts_install(
        "fishing",
        &[("Sulayre-Lure".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("fishing", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // Only the proxy goes into the game folder; GDWeave's folder, with the mods, to the
    // runtime one.
    assert_eq!(fs::read_to_string(root.join("winmm.dll")).unwrap(), "proxy");
    assert!(!root.join("GDWeave").exists());
    let rt = s.game_dir("fishing").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "GDWeave/core/GDWeave.dll",
        "GDWeave/mods/Sulayre.Lure/manifest.json",
        "GDWeave/mods/Sulayre.Lure/Lure.dll",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }
    assert!(!rt.join("GDWeave/mods/Sulayre.Lure/icon.png").exists());

    s.revert("fishing", false, &mut Faults::none()).unwrap();
    assert!(!root.join("winmm.dll").exists());
}
