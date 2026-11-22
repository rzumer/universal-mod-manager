//! Scrap Mechanic-style games with Rivet from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/scrap/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for Scrap Mechanic: no install rules, and the loader package's files under `RivetPack/`
// (Thunderstore's data names `Rivet` as its root folder; r2modman reads `RivetPack`).
const ECOSYSTEM: &str = r#"{
  "games": {
    "scrap": {"label": "scrap",
      "distributions": [{"platform": "steam", "identifier": "6269"}],
      "r2modman": [{"packageLoader": "rivet", "exeNames": ["Scrap.exe"],
        "dataFolderName": "Data",
        "packageIndex": "https://thunderstore.io/c/scrap/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "ReDoIngMods-Rivet", "rootFolder": "Rivet", "loader": "rivet"}]
}"#;

const LISTING: &str = r#"[
  {"name": "Rivet", "full_name": "ReDoIngMods-Rivet", "owner": "ReDoIngMods",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "1.0.0", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["ReDoIngMods-Rivet-1.0.0"]}]}
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
            r#"{"name":"Rivet","version_number":"1.0.0","dependencies":[]}"#,
        ),
        ("RivetPack/version.dll", "proxy"),
        ("RivetPack/Rivet.ini", "[Doorstop]\nenable=false\n"),
        ("RivetPack/Rivet/Loader.dll", "loader"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["ReDoIngMods-Rivet-1.0.0"]}"#,
        ),
        ("Mine.dll", "dll"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/ReDoIngMods/Rivet/1.0.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            ),
    )
}

#[test]
fn rivet_runs_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    // Its exe is in a subfolder, as Scrap Mechanic's is in `Release/`.
    fs::create_dir_all(root.join("Release")).unwrap();
    fs::write(root.join("Release/Scrap.exe"), "game").unwrap();
    let p = s.data().game("scrap").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Scrap\"\n[detect]\nsteam_app_id = 6269\n\
         [launch]\nexe = \"Release/Scrap.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("scrap", &root).unwrap();

    let c = s.loader_choice("scrap").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("rivet"));
    s.set_loader("scrap", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("scrap").unwrap().setup_label(),
        "Rivet from Thunderstore"
    );
    let page = s.ts_search("scrap", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Rivet");

    let err = s
        .deploy("scrap", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Rivet"), "{err}");

    s.ts_install(
        "scrap",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("scrap", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // The proxy and its ini go next to the game's exe; the loader and the mods to the runtime
    // folder.
    for f in ["Release/version.dll", "Release/Rivet.ini"] {
        assert!(root.join(f).is_file(), "{f}");
    }
    assert!(!root.join("version.dll").exists());
    assert!(!root.join("Rivet").exists());
    let rt = s.game_dir("scrap").runtime_dir(DEFAULT_PROFILE);
    for f in ["Rivet/Loader.dll", "Rivet/Mods/Me-Mine/Mine.dll"] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("scrap", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_dir(root.join("Release")).unwrap().count(), 1);
}
