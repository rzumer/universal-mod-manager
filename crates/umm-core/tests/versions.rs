//! Installing another version of a Thunderstore package, older or newer, no older than the
//! other installed packages need.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{LoaderSetup, Session, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

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

const LISTING: &str = r#"[
  {"name": "lovely", "full_name": "Thunderstore-lovely", "owner": "Thunderstore", "categories": [],
   "versions": [{"version_number": "0.10.0", "dependencies": []}]},
  {"name": "Core", "full_name": "Lib-Core", "owner": "Lib", "categories": [],
   "versions": [{"version_number": "2.0.0", "dependencies": [], "file_size": 200},
                {"version_number": "1.5.0", "dependencies": [], "file_size": 150},
                {"version_number": "1.0.0", "dependencies": [], "file_size": 100}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": [],
   "versions": [{"version_number": "1.0.0", "dependencies": ["Lib-Core-1.5.0"]}]}
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

fn core(version: &str) -> Vec<u8> {
    zip_of(&[
        (
            "manifest.json",
            &format!(r#"{{"name":"Core","version_number":"{version}","dependencies":[]}}"#),
        ),
        ("core.lua", &format!("-- core {version}")),
    ])
}

#[test]
fn a_package_goes_back_no_further_than_its_dependents_need() {
    let mut http = FakeHttp::default()
        .with(ECOSYSTEM_URL, ECOSYSTEM)
        .with(
            "https://thunderstore.io/c/jokers/api/v1/package-listing-index/",
            gz(r#"["https://cdn/chunk1"]"#),
        )
        .with("https://cdn/chunk1", gz(LISTING))
        .with(
            "https://thunderstore.io/package/download/Thunderstore/lovely/0.10.0/",
            zip_of(&[("manifest.json", "{}"), ("winmm.dll", "proxy")]),
        )
        .with(
            "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
            zip_of(&[
                (
                    "manifest.json",
                    r#"{"name":"Mine","version_number":"1.0.0","dependencies":["Lib-Core-1.5.0"]}"#,
                ),
                ("mine.lua", "-- mine"),
            ]),
        );
    for v in ["2.0.0", "1.5.0", "1.0.0"] {
        http = http.with(
            &format!("https://thunderstore.io/package/download/Lib/Core/{v}/"),
            core(v),
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Jokers.exe"), "game").unwrap();
    let s = Session::open_with(Some(tmp.path().join("data")), Arc::new(http)).unwrap();
    let p = s.data().game("jokers").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Jokers\"\n[detect]\nsteam_app_id = 6262\n\
         [launch]\nexe = \"Jokers.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("jokers", &root).unwrap();
    s.set_loader("jokers", LoaderSetup::Thunderstore).unwrap();

    // Mine names Core 1.5.0; Core comes at its newest.
    s.ts_install(
        "jokers",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    let versions = s.ts_versions("jokers", "Lib-Core").unwrap();
    let summary: Vec<(&str, bool, usize)> = versions
        .iter()
        .map(|v| (v.version.as_str(), v.installed, v.needed_by.len()))
        .collect();
    assert_eq!(
        summary,
        [("2.0.0", true, 0), ("1.5.0", false, 0), ("1.0.0", false, 1)]
    );
    assert_eq!(
        versions[2].needed_by,
        [("Me-Mine".to_string(), "1.5.0".to_string())]
    );

    // Back to 1.5.0: installed in its place, and the update is offered again.
    let r = s
        .ts_install_version("jokers", "Lib-Core", "1.5.0", DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    assert_eq!(
        r.installed,
        [(
            "Lib-Core".to_string(),
            "1.5.0".to_string(),
            Some("2.0.0".to_string())
        )]
    );
    assert!(s
        .ts_updates("jokers")
        .unwrap()
        .iter()
        .any(|(n, have, latest)| n == "Lib-Core" && have == "1.5.0" && latest == "2.0.0"));
    let files = s.game_dir("jokers").mod_dir("lib-core").join("files");
    let lua = walk(&files)
        .into_iter()
        .find(|p| p.ends_with("core.lua"))
        .unwrap();
    assert_eq!(fs::read_to_string(lua).unwrap(), "-- core 1.5.0");

    // Older than Mine needs: refused, saying why.
    let err = s
        .ts_install_version("jokers", "Lib-Core", "1.0.0", DEFAULT_PROFILE, &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(err.contains("Me-Mine needs 1.5.0 or later"), "{err}");
    assert!(s
        .ts_install_version("jokers", "Lib-Core", "9.9.9", DEFAULT_PROFILE, &mut |_| {})
        .is_err());
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
