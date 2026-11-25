//! Versions of a Thunderstore package per profile: a profile pinned to a version keeps it
//! while the others follow the latest, one stored copy per version; a pin older than another
//! package of the profile needs is refused until that package is downgraded or the pin
//! removed; versions no profile is pinned to are removed.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{CoreError, DepProblem, LoaderSetup, PinConflictChoice, Session, DEFAULT_PROFILE};
use umm_store::versions::other_versions;
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const GAME: &str = "jokers";

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
   "versions": [{"version_number": "1.0.0", "dependencies": ["Lib-Core-1.5.0"]},
                {"version_number": "0.9.0", "dependencies": ["Lib-Core-1.0.0"]}]}
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

fn package(name: &str, version: &str, deps: &[&str], file: &str) -> Vec<u8> {
    let deps = deps
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(",");
    zip_of(&[
        (
            "manifest.json",
            &format!(r#"{{"name":"{name}","version_number":"{version}","dependencies":[{deps}]}}"#),
        ),
        (file, &format!("-- {name} {version}")),
    ])
}

fn session(tmp: &std::path::Path) -> Session {
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
            package("Mine", "1.0.0", &["Lib-Core-1.5.0"], "mine.lua"),
        )
        .with(
            "https://thunderstore.io/package/download/Me/Mine/0.9.0/",
            package("Mine", "0.9.0", &["Lib-Core-1.0.0"], "mine.lua"),
        );
    for v in ["2.0.0", "1.5.0", "1.0.0"] {
        http = http.with(
            &format!("https://thunderstore.io/package/download/Lib/Core/{v}/"),
            package("Core", v, &[], "core.lua"),
        );
    }
    let root = tmp.join("game");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Jokers.exe"), "game").unwrap();
    let s = Session::open_with(Some(tmp.join("data")), Arc::new(http)).unwrap();
    let p = s.data().game(GAME).def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Jokers\"\n[detect]\nsteam_app_id = 6262\n\
         [launch]\nexe = \"Jokers.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path(GAME, &root).unwrap();
    s.set_loader(GAME, LoaderSetup::Thunderstore).unwrap();
    s
}

/// The version of a package a profile deploys, from its stored file.
fn deployed(s: &Session, profile: &str, file: &str) -> String {
    let plan = s.plan(GAME, profile).unwrap();
    let f = plan
        .files
        .iter()
        .find(|f| f.target.as_str().ends_with(file))
        .unwrap_or_else(|| panic!("{file} is not deployed for {profile}"));
    fs::read_to_string(&f.source).unwrap()
}

fn pin(s: &Session, profile: &str, id: &str) -> Option<String> {
    s.profile(GAME, profile)
        .unwrap()
        .pin(id)
        .map(str::to_string)
}

#[test]
fn a_profile_pinned_to_a_version_keeps_it_while_the_others_follow_the_latest() {
    let tmp = tempfile::tempdir().unwrap();
    let s = session(tmp.path());
    let ask = PinConflictChoice::Ask;

    // Mine names Core 1.5.0; Core comes at its newest.
    s.ts_install(
        GAME,
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        ask,
        &mut |_| {},
    )
    .unwrap();
    s.create_profile(GAME, "Other", Some(DEFAULT_PROFILE))
        .unwrap();
    let versions = s.ts_versions(GAME, "Lib-Core", DEFAULT_PROFILE).unwrap();
    let summary: Vec<(&str, bool, bool, usize)> = versions
        .iter()
        .map(|v| (v.version.as_str(), v.installed, v.pinned, v.needed_by.len()))
        .collect();
    assert_eq!(
        summary,
        [
            ("2.0.0", true, false, 0),
            ("1.5.0", false, false, 0),
            ("1.0.0", false, false, 1)
        ]
    );
    assert_eq!(
        versions[2].needed_by,
        [("Me-Mine".to_string(), "1.5.0".to_string())]
    );

    // Back to 1.5.0 in Default: pinned there and stored beside 2.0.0, which Other keeps.
    let r = s
        .ts_install_version(GAME, "Lib-Core", "1.5.0", DEFAULT_PROFILE, ask, &mut |_| {})
        .unwrap();
    assert_eq!(r.installed.len(), 1);
    assert_eq!(
        pin(&s, DEFAULT_PROFILE, "lib-core").as_deref(),
        Some("1.5.0")
    );
    assert_eq!(pin(&s, "Other", "lib-core"), None);
    let game = s.game_dir(GAME);
    assert_eq!(other_versions(&game, "lib-core"), ["1.5.0"]);
    assert_eq!(deployed(&s, DEFAULT_PROFILE, "core.lua"), "-- Core 1.5.0");
    assert_eq!(deployed(&s, "Other", "core.lua"), "-- Core 2.0.0");
    let v = s.ts_versions(GAME, "Lib-Core", DEFAULT_PROFILE).unwrap();
    assert!(v[1].installed && v[1].pinned && v[1].stored);
    assert!(
        s.ts_updates(GAME).unwrap().is_empty(),
        "the latest is stored"
    );

    // Older than Mine needs: refused, naming the conflict...
    let err = s
        .ts_install_version(GAME, "Lib-Core", "1.0.0", DEFAULT_PROFILE, ask, &mut |_| {})
        .unwrap_err();
    let CoreError::PinConflicts(c) = &err else {
        panic!("{err}")
    };
    assert_eq!(
        (c[0].dependent.as_str(), c[0].needs.as_str(), c[0].can_unpin),
        ("Me-Mine", "1.5.0", false)
    );
    assert!(err
        .to_string()
        .contains("Me-Mine 1.0.0 needs Lib-Core 1.5.0 or later"));
    assert_eq!(
        pin(&s, DEFAULT_PROFILE, "lib-core").as_deref(),
        Some("1.5.0")
    );
    // ...or done, with Mine moved back to the version that works with it.
    let r = s
        .ts_install_version(
            GAME,
            "Lib-Core",
            "1.0.0",
            DEFAULT_PROFILE,
            PinConflictChoice::Downgrade,
            &mut |_| {},
        )
        .unwrap();
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("Pinned Me-Mine at 0.9.0")),
        "{:?}",
        r.notes
    );
    assert_eq!(
        pin(&s, DEFAULT_PROFILE, "lib-core").as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        pin(&s, DEFAULT_PROFILE, "me-mine").as_deref(),
        Some("0.9.0")
    );
    assert_eq!(deployed(&s, DEFAULT_PROFILE, "mine.lua"), "-- Mine 0.9.0");
    assert_eq!(deployed(&s, "Other", "mine.lua"), "-- Mine 1.0.0");
    // 1.5.0 is pinned nowhere now.
    assert_eq!(other_versions(&game, "lib-core"), ["1.0.0"]);
    assert_eq!(other_versions(&game, "me-mine"), ["0.9.0"]);

    // A pin the profile had is in the way of a new install: removed when asked to.
    s.create_profile(GAME, "Third", None).unwrap();
    s.ts_install_version(GAME, "Lib-Core", "1.0.0", "Third", ask, &mut |_| {})
        .unwrap();
    let err = s
        .ts_install(GAME, &[("Me-Mine".into(), None)], "Third", ask, &mut |_| {})
        .unwrap_err();
    let CoreError::PinConflicts(c) = &err else {
        panic!("{err}")
    };
    assert!(c[0].can_unpin);
    s.ts_install(
        GAME,
        &[("Me-Mine".into(), None)],
        "Third",
        PinConflictChoice::Unpin,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(pin(&s, "Third", "lib-core"), None);
    assert_eq!(deployed(&s, "Third", "core.lua"), "-- Core 2.0.0");

    // Following the latest again: Default's pins go when the packages are updated in it.
    s.ts_follow_latest(GAME, "Me-Mine", DEFAULT_PROFILE, ask, &mut |_| {})
        .unwrap_err();
    s.ts_follow_latest(
        GAME,
        "Me-Mine",
        DEFAULT_PROFILE,
        PinConflictChoice::Unpin,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(pin(&s, DEFAULT_PROFILE, "me-mine"), None);
    assert_eq!(pin(&s, DEFAULT_PROFILE, "lib-core"), None);
    assert_eq!(deployed(&s, DEFAULT_PROFILE, "mine.lua"), "-- Mine 1.0.0");
    assert!(other_versions(&game, "me-mine").is_empty());
    assert!(other_versions(&game, "lib-core").is_empty());

    // Picking the latest from the list pins it too; a deleted profile's versions go.
    s.ts_install_version(GAME, "Lib-Core", "2.0.0", "Other", ask, &mut |_| {})
        .unwrap();
    assert_eq!(pin(&s, "Other", "lib-core").as_deref(), Some("2.0.0"));
    s.ts_install_version(GAME, "Lib-Core", "1.5.0", "Third", ask, &mut |_| {})
        .unwrap();
    assert_eq!(other_versions(&game, "lib-core"), ["1.5.0"]);
    s.delete_profile(GAME, "Third").unwrap();
    assert!(other_versions(&game, "lib-core").is_empty());

    // Every profile pinned to older versions: the latest goes, the newest pinned version
    // takes its place, and no update is offered for it.
    s.ts_install_version(GAME, "Lib-Core", "1.5.0", "Other", ask, &mut |_| {})
        .unwrap();
    s.ts_install_version(
        GAME,
        "Lib-Core",
        "1.0.0",
        DEFAULT_PROFILE,
        PinConflictChoice::Downgrade,
        &mut |_| {},
    )
    .unwrap();
    let latest = |s: &Session| {
        s.mods(GAME)
            .unwrap()
            .into_iter()
            .find(|m| m.id == "lib-core")
            .unwrap()
            .version
    };
    assert_eq!(latest(&s).as_deref(), Some("1.5.0"));
    assert_eq!(other_versions(&game, "lib-core"), ["1.0.0"]);
    assert_eq!(deployed(&s, "Other", "core.lua"), "-- Core 1.5.0");
    assert_eq!(deployed(&s, DEFAULT_PROFILE, "core.lua"), "-- Core 1.0.0");
    assert!(!s
        .ts_updates(GAME)
        .unwrap()
        .iter()
        .any(|u| u.0 == "Lib-Core"));
    // Following the latest again downloads it.
    let r = s
        .ts_follow_latest(GAME, "Lib-Core", "Other", ask, &mut |_| {})
        .unwrap();
    assert_eq!(r.installed[0].1, "2.0.0");
    assert_eq!(latest(&s).as_deref(), Some("2.0.0"));
    assert_eq!(
        other_versions(&game, "lib-core"),
        ["1.0.0"],
        "1.5.0 is unused"
    );
    assert_eq!(deployed(&s, "Other", "core.lua"), "-- Core 2.0.0");

    assert!(s
        .ts_install_version(GAME, "Lib-Core", "9.9.9", "Other", ask, &mut |_| {})
        .is_err());
}

#[test]
fn unmet_dependencies_are_reported_but_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let s = session(tmp.path());
    let ask = PinConflictChoice::Ask;
    s.ts_install(
        GAME,
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        ask,
        &mut |_| {},
    )
    .unwrap();
    let issues = || {
        s.ts_dependency_issues(GAME, DEFAULT_PROFILE)
            .unwrap()
            .into_iter()
            .map(|i| (i.mod_id, i.dependency, i.needs, i.problem))
            .collect::<Vec<_>>()
    };
    let core = |p: DepProblem| {
        vec![(
            "me-mine".to_string(),
            "Lib-Core".to_string(),
            "1.5.0".to_string(),
            p,
        )]
    };
    assert!(issues().is_empty());

    // Turned off, then left out of the profile: still deployable, with a warning.
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods
        .iter_mut()
        .find(|e| e.id == "lib-core")
        .unwrap()
        .enabled = false;
    s.save_profile(GAME, &p).unwrap();
    assert_eq!(issues(), core(DepProblem::Disabled));
    s.plan(GAME, DEFAULT_PROFILE).unwrap();
    // The fix the interface offers: installing it enables it again.
    s.ts_install(
        GAME,
        &[("Lib-Core".into(), None)],
        DEFAULT_PROFILE,
        ask,
        &mut |_| {},
    )
    .unwrap();
    assert!(issues().is_empty());
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods.retain(|e| e.id != "lib-core");
    s.save_profile(GAME, &p).unwrap();
    assert_eq!(issues(), core(DepProblem::NotInProfile));

    // Pinned older than named.
    s.create_profile(GAME, "Old", None).unwrap();
    s.ts_install_version(GAME, "Lib-Core", "1.0.0", "Old", ask, &mut |_| {})
        .unwrap();
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods.push(umm_core::ProfileEntry {
        version: Some("1.0.0".into()),
        ..umm_core::ProfileEntry::new("lib-core", true)
    });
    s.save_profile(GAME, &p).unwrap();
    assert_eq!(issues(), core(DepProblem::TooOld("1.0.0".into())));

    // Not installed at all.
    s.remove_mod(GAME, "lib-core").unwrap();
    assert_eq!(issues(), core(DepProblem::NotInstalled));
    // A disabled dependent warns about nothing.
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods
        .iter_mut()
        .find(|e| e.id == "me-mine")
        .unwrap()
        .enabled = false;
    s.save_profile(GAME, &p).unwrap();
    assert!(issues().is_empty());
}
