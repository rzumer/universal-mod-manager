//! Titanfall 2 with Northstar from Thunderstore, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/titan/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for Titanfall 2: mods to `R2Northstar/mods`, and the loader package under `Northstar/`,
// laid out as Northstar's own release.
const ECOSYSTEM: &str = r#"{
  "games": {
    "titan": {"label": "titan",
      "distributions": [{"platform": "steam", "identifier": "6267"}],
      "r2modman": [{"packageLoader": "northstar", "exeNames": ["NorthstarLauncher.exe", "Titan.exe"],
        "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/titan/api/v1/package-listing-index/",
        "installRules": [{"route": "R2Northstar/mods", "defaultFileExtensions": [], "trackingMethod": "state", "subRoutes": [], "isDefaultLocation": false}]}]}
  },
  "modloaderPackages": [{"packageId": "northstar-Northstar", "rootFolder": "Northstar", "loader": "northstar"}]
}"#;

const LISTING: &str = r#"[
  {"name": "Northstar", "full_name": "northstar-Northstar", "owner": "northstar",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "1.31.10", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["northstar-Northstar-1.31.10"]}]}
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
            r#"{"name":"Northstar","version_number":"1.31.10","dependencies":[]}"#,
        ),
        ("icon.png", "png"),
        ("Northstar/NorthstarLauncher.exe", "launcher"),
        ("Northstar/bin/x64_retail/wsock32.dll", "proxy"),
        ("Northstar/r2ds.bat", "bat"),
        ("Northstar/R2Northstar/Northstar.dll", "northstar"),
        ("Northstar/R2Northstar/mods/Northstar.Client/mod.json", "{}"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["northstar-Northstar-1.31.10"]}"#,
        ),
        ("mods/Me.Mine/mod.json", r#"{"Name":"Me.Mine"}"#),
        ("mods/Me.Mine/mod/scripts/vscripts/mine.nut", "// mine"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/northstar/Northstar/1.31.10/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            ),
    )
}

#[test]
fn northstar_runs_its_profile_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Titan.exe"), "game").unwrap();
    let p = s.data().game("titan").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Titan\"\n[detect]\nsteam_app_id = 6267\n\
         [launch]\nexe = \"Titan.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("titan", &root).unwrap();

    let c = s.loader_choice("titan").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("northstar"));
    s.set_loader("titan", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("titan").unwrap().setup_label(),
        "Northstar from Thunderstore"
    );
    let page = s.ts_search("titan", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Northstar");

    let err = s
        .deploy("titan", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Northstar"), "{err}");

    s.ts_install(
        "titan",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("titan", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // The launcher and the proxy go into the game folder; the profile, Northstar.dll and its
    // mods with the others', to the runtime folder.
    for f in [
        "NorthstarLauncher.exe",
        "bin/x64_retail/wsock32.dll",
        "r2ds.bat",
    ] {
        assert!(root.join(f).is_file(), "{f}");
    }
    assert!(!root.join("R2Northstar").exists());
    let rt = s.game_dir("titan").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "R2Northstar/Northstar.dll",
        "R2Northstar/mods/Northstar.Client/mod.json",
        "R2Northstar/mods/Me.Mine/mod.json",
        "R2Northstar/mods/Me.Mine/mod/scripts/vscripts/mine.nut",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("titan", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}
