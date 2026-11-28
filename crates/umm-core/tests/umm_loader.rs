//! A Dance of Fire and Ice-style games with Unity Mod Manager from Thunderstore, started
//! through Unity Doorstop, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_sources::net::FakeHttp;
use umm_sources::thunderstore::ecosystem::ECOSYSTEM_URL;

const INDEX: &str = "https://thunderstore.io/c/rhythm/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for A Dance of Fire and Ice: mods to `UMM/Mods`, one folder each.
const ECOSYSTEM: &str = r#"{
  "games": {
    "rhythm": {"label": "rhythm",
      "distributions": [{"platform": "steam", "identifier": "6266"}],
      "r2modman": [{"packageLoader": "umm", "exeNames": ["Rhythm.exe"],
        "dataFolderName": "Rhythm_Data",
        "packageIndex": "https://thunderstore.io/c/rhythm/api/v1/package-listing-index/",
        "installRules": [{"route": "UMM/Mods", "isDefaultLocation": true, "defaultFileExtensions": [".dll"], "trackingMethod": "subdir", "subRoutes": []}]}]}
  },
  "modloaderPackages": [{"packageId": "UMM-UMM", "rootFolder": "UMM", "loader": "umm"}]
}"#;

const LISTING: &str = r#"[
  {"name": "UMM", "full_name": "UMM-UMM", "owner": "UMM",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "0.32.4", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["UMM-UMM-0.32.4"]}]},
  {"name": "Bare", "full_name": "Me-Bare", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": []}]}
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
            r#"{"name":"UMM","version_number":"0.32.4","dependencies":[]}"#,
        ),
        ("UMM/Core/UnityModManager.dll", "umm"),
        ("winhttp.dll", "proxy"),
        ("doorstop_config.ini", "[General]\nenabled=true"),
        (".doorstop_version", "4.3.0"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["UMM-UMM-0.32.4"]}"#,
        ),
        ("Mine/Info.json", r#"{"Id":"Mine"}"#),
        ("Mine/Mine.dll", "dll"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/UMM/UMM/0.32.4/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Bare/1.0.0/",
                zip_of(&[
                    (
                        "manifest.json",
                        r#"{"name":"Bare","version_number":"1.0.0","dependencies":[]}"#,
                    ),
                    ("Bare/Info.json", r#"{"Id":"Bare"}"#),
                    ("Bare/Bare.dll", "dll"),
                ]),
            ),
    )
}

#[test]
fn unity_mod_manager_runs_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(root.join("Rhythm_Data")).unwrap();
    fs::write(root.join("Rhythm.exe"), "game").unwrap();
    let p = s.data().game("rhythm").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Rhythm\"\n[detect]\nsteam_app_id = 6266\n\
         [launch]\nexe = \"Rhythm.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("rhythm", &root).unwrap();

    let c = s.loader_choice("rhythm").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("umm"));
    s.set_loader("rhythm", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("rhythm").unwrap().setup_label(),
        "Unity Mod Manager from Thunderstore"
    );
    let page = s.ts_search("rhythm", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "Unity Mod Manager");

    let err = s
        .deploy("rhythm", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no Unity Mod Manager"), "{err}");

    s.ts_install(
        "rhythm",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("rhythm", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // Doorstop's files go into the game folder; Unity Mod Manager and the mods to the runtime
    // one, each mod in its own folder with its Info.json.
    for f in ["winhttp.dll", "doorstop_config.ini", ".doorstop_version"] {
        assert!(root.join(f).is_file(), "{f}");
    }
    assert!(!root.join("UMM").exists());
    let rt = s.game_dir("rhythm").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "UMM/Core/UnityModManager.dll",
        "UMM/Mods/Me-Mine/Info.json",
        "UMM/Mods/Me-Mine/Mine.dll",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("rhythm", false, &mut Faults::none()).unwrap();
    assert!(!root.join("winhttp.dll").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn the_loader_comes_with_a_mod_that_does_not_name_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(root.join("Rhythm_Data")).unwrap();
    fs::write(root.join("Rhythm.exe"), "game").unwrap();
    let p = s.data().game("rhythm").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1
name = \"Rhythm\"
[detect]
steam_app_id = 6266
         [launch]
exe = \"Rhythm.exe\"
[[install]]
match = \"**\"
target = \"\"
",
    )
    .unwrap();
    s.set_install_path("rhythm", &root).unwrap();
    s.set_loader("rhythm", LoaderSetup::Thunderstore).unwrap();

    let r = s
        .ts_install(
            "rhythm",
            &[("Me-Bare".into(), None)],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert!(
        r.installed.iter().any(|(n, _, _)| n == "UMM-UMM"),
        "{:?}",
        r.installed
    );
    assert!(
        r.notes.iter().any(|n| n.contains("UMM-UMM too")),
        "{:?}",
        r.notes
    );
    s.deploy("rhythm", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    s.revert("rhythm", false, &mut Faults::none()).unwrap();

    // Once there, it is not added again.
    let r = s
        .ts_install(
            "rhythm",
            &[("Me-Mine".into(), None)],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}
