//! Hades II-style games with Return of Modding (as Hell2Modding) from Thunderstore, against
//! fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/hell/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for Hades II: no install rules, the loader package in `ReturnOfModdingPack/`, and the
// game's exe in `Ship/`.
const ECOSYSTEM: &str = r#"{
  "games": {
    "hell": {"label": "hell",
      "distributions": [{"platform": "steam", "identifier": "6265"}],
      "r2modman": [{"packageLoader": "return-of-modding", "exeNames": ["Hell.exe"],
        "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/hell/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "Hell2Modding-Hell2Modding", "rootFolder": "ReturnOfModdingPack", "loader": "return-of-modding"}]
}"#;

const LISTING: &str = r#"[
  {"name": "Hell2Modding", "full_name": "Hell2Modding-Hell2Modding", "owner": "Hell2Modding",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "1.1.0", "dependencies": []},
                {"version_number": "1.0.0", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["Hell2Modding-Hell2Modding-1.0.0"]}]}
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
            r#"{"name":"Hell2Modding","version_number":"1.1.0","dependencies":[]}"#,
        ),
        ("icon.png", "png"),
        ("ReturnOfModdingPack/d3d12.dll", "proxy"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["Hell2Modding-Hell2Modding-1.0.0"]}"#,
        ),
        ("main.lua", "-- mine"),
        ("plugins_data/sprite.png", "png"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/Hell2Modding/Hell2Modding/1.1.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            ),
    )
}

#[test]
fn return_of_modding_runs_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(root.join("Ship")).unwrap();
    fs::write(root.join("Ship/Hell.exe"), "game").unwrap();
    let p = s.data().game("hell").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Hell\"\n[detect]\nsteam_app_id = 6265\n\
         [launch]\nexe = \"Ship/Hell.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("hell", &root).unwrap();

    let c = s.loader_choice("hell").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("return-of-modding"));
    s.set_loader("hell", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("hell").unwrap().setup_label(),
        "Return of Modding from Thunderstore"
    );
    let page = s.ts_search("hell", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Return of Modding");

    let err = s
        .deploy("hell", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Return of Modding"), "{err}");

    // The mod names the loader it was built against (1.0.0); the newest is installed, as for
    // any dependency.
    let r = s
        .ts_install(
            "hell",
            &[("Me-Mine".into(), None)],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert!(
        r.installed
            .iter()
            .any(|(n, v, _)| n == "Hell2Modding-Hell2Modding" && v == "1.1.0"),
        "{:?}",
        r.installed
    );
    s.deploy("hell", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // Only the proxy goes into the game folder, next to the exe; the mods to the runtime one,
    // each under its GUID with its manifest.
    assert_eq!(
        fs::read_to_string(root.join("Ship/d3d12.dll")).unwrap(),
        "proxy"
    );
    assert!(!root.join("d3d12.dll").exists());
    assert!(!root.join("ReturnOfModding").exists() && !root.join("Ship/ReturnOfModding").exists());
    let rt = s.game_dir("hell").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "ReturnOfModding/plugins/Me-Mine/main.lua",
        "ReturnOfModding/plugins/Me-Mine/manifest.json",
        "ReturnOfModding/plugins_data/Me-Mine/sprite.png",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("hell", false, &mut Faults::none()).unwrap();
    assert!(!root.join("Ship/d3d12.dll").exists());
    assert_eq!(fs::read_dir(root.join("Ship")).unwrap().count(), 1);
}
