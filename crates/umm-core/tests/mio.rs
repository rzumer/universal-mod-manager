//! MIO: Memories in Orbit-style games with the MIO Mod Loader from Thunderstore, against fake
//! Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/orbit/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for MIO: no install rules, and the loader package under `MioModLoader/`.
const ECOSYSTEM: &str = r#"{
  "games": {
    "orbit": {"label": "orbit",
      "distributions": [{"platform": "steam", "identifier": "6268"}],
      "r2modman": [{"packageLoader": "mio", "exeNames": ["orbit.exe"],
        "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/orbit/api/v1/package-listing-index/",
        "installRules": []}]}
  },
  "modloaderPackages": [{"packageId": "MIO_Modding-MioModLoader", "rootFolder": "MioModLoader", "loader": "mio"}]
}"#;

const LISTING: &str = r#"[
  {"name": "MioModLoader", "full_name": "MIO_Modding-MioModLoader", "owner": "MIO_Modding",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "1.0.0", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["MIO_Modding-MioModLoader-1.0.0"]}]}
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
            r#"{"name":"MioModLoader","version_number":"1.0.0","dependencies":[]}"#,
        ),
        ("MioModLoader/winhttp.dll", "proxy"),
        ("MioModLoader/mio-mod-loader/MioModLoader.dll", "loader"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["MIO_Modding-MioModLoader-1.0.0"]}"#,
        ),
        ("Mine/mod.json", r#"{"id":"mine"}"#),
        ("Mine/Mine.dll", "dll"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/MIO_Modding/MioModLoader/1.0.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            ),
    )
}

#[test]
fn mio_runs_mods_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("orbit.exe"), "game").unwrap();
    let p = s.data().game("orbit").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Orbit\"\n[detect]\nsteam_app_id = 6268\n\
         [launch]\nexe = \"orbit.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("orbit", &root).unwrap();

    let c = s.loader_choice("orbit").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("mio"));
    s.set_loader("orbit", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("orbit").unwrap().setup_label(),
        "MIO Mod Loader from Thunderstore"
    );
    let page = s.ts_search("orbit", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "MIO Mod Loader");

    let err = s
        .deploy("orbit", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no MIO Mod Loader"), "{err}");

    s.ts_install(
        "orbit",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("orbit", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // The proxy and the loader's folder go into the game folder (it loads them from there);
    // the mods to the runtime folder.
    for f in ["winhttp.dll", "mio-mod-loader/MioModLoader.dll"] {
        assert!(root.join(f).is_file(), "{f}");
    }
    assert!(!root.join("mods").exists());
    let rt = s.game_dir("orbit").runtime_dir(DEFAULT_PROFILE);
    for f in ["mods/Me-Mine/mod.json", "mods/Me-Mine/Mine.dll"] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("orbit", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}
