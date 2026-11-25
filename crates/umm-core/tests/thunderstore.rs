//! Installing, updating and deploying Thunderstore packages, against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

/// The game's `games/<id>/game.toml`, its folder created.
fn def_file(s: &Session, id: &str) -> std::path::PathBuf {
    let p = s.data().game(id).def_path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

const INDEX: &str = "https://thunderstore.io/c/test-quest/api/v1/package-listing-index/";

const ECOSYSTEM: &str = r#"{
  "games": {
    "test-quest": {"label": "test-quest",
      "distributions": [{"platform": "steam", "identifier": "4242"}],
      "r2modman": [{"packageLoader": "bepinex", "exeNames": ["Game.exe"],
        "packageIndex": "https://thunderstore.io/c/test-quest/api/v1/package-listing-index/",
        "installRules": [
          {"route": "BepInEx/plugins", "defaultFileExtensions": [".dll"], "trackingMethod": "subdir", "isDefaultLocation": true},
          {"route": "BepInEx/patchers", "trackingMethod": "subdir"},
          {"route": "BepInEx/config", "trackingMethod": "none"}
        ]}]}
  },
  "modloaderPackages": [{"packageId": "BepInEx-BepInExPack", "rootFolder": "BepInExPack", "loader": "bepinex"}]
}"#;

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

fn listing(mod_version: &str) -> String {
    format!(
        r#"[
      {{"name": "BepInExPack", "full_name": "BepInEx-BepInExPack", "owner": "BepInEx", "is_pinned": true,
        "categories": ["Libraries"], "rating_score": 10,
        "versions": [{{"version_number": "5.4.2305", "dependencies": []}}]}},
      {{"name": "Lantern_Tweaks", "full_name": "Ann-Lantern_Tweaks", "owner": "Ann",
        "categories": ["Mods"], "rating_score": 5,
        "versions": [{{"version_number": "{mod_version}", "description": "Brighter lanterns",
                       "dependencies": ["BepInEx-BepInExPack-5.4.2305"]}},
                     {{"version_number": "1.0.0", "dependencies": ["BepInEx-BepInExPack-5.4.2305"]}}]}},
      {{"name": "Old", "full_name": "Bob-Old", "owner": "Bob", "is_deprecated": true,
        "versions": [{{"version_number": "0.1.0"}}]}},
      {{"name": "r2modman", "full_name": "ebkr-r2modman", "owner": "ebkr", "is_pinned": true,
        "categories": ["Tools"], "rating_score": 99,
        "versions": [{{"version_number": "3.2.21", "description": "A simple and easy to use mod manager"}}]}},
      {{"name": "Compass", "full_name": "squidypal-Compass", "owner": "squidypal",
        "categories": ["Tools"], "rating_score": 1,
        "versions": [{{"version_number": "1.0.0", "description": "Simple compass"}}]}}
    ]"#
    )
}

fn fake() -> Arc<FakeHttp> {
    let pack = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"BepInExPack","version_number":"5.4.2305","dependencies":[]}"#,
        ),
        ("BepInExPack/winhttp.dll", "proxy"),
        (
            "BepInExPack/doorstop_config.ini",
            "[General]\nenabled = true",
        ),
        ("BepInExPack/.doorstop_version", "4.3.0"),
        (
            "BepInExPack/BepInEx/core/BepInEx.Preloader.dll",
            "preloader",
        ),
    ]);
    let tweaks = |v: &str| {
        zip_of(&[
            (
                "manifest.json",
                &format!(
                    r#"{{"name":"Lantern_Tweaks","version_number":"{v}","dependencies":["BepInEx-BepInExPack-5.4.2305"]}}"#
                ),
            ),
            ("icon.png", "png"),
            ("LanternTweaks.dll", v),
            ("config/lanterns.cfg", "bright=true"),
        ])
    };
    let http = FakeHttp::default()
        .with(ECOSYSTEM_URL, ECOSYSTEM)
        .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
        .with("https://cdn/chunk1", gz(&listing("1.1.0")))
        .with(
            "https://thunderstore.io/package/download/BepInEx/BepInExPack/5.4.2305/",
            pack,
        )
        .with(
            "https://thunderstore.io/package/download/Ann/Lantern_Tweaks/1.1.0/",
            tweaks("1.1.0"),
        )
        .with(
            "https://thunderstore.io/package/download/Ann/Lantern_Tweaks/1.2.0/",
            tweaks("1.2.0"),
        );
    Arc::new(http)
}

fn game(data: &Path, root: &Path, http: Arc<FakeHttp>) -> Session {
    let s = Session::open_with(Some(data.to_path_buf()), http).unwrap();
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("Game.exe"), "exe").unwrap();
    fs::write(
        def_file(&s, "test-quest"),
        "schema = 1\nname = \"Test Quest\"\n\
         [detect]\nsteam_app_id = 4242\n[launch]\nexe = \"Game.exe\"\n\
         [loader]\nkind = \"bepinex\"\nsource = \"thunderstore\"\ninject = \"doorstop\"\n\
         [thunderstore]\ncommunity = \"test-quest\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("test-quest", root).unwrap();
    s
}

#[test]
fn search_install_update_and_deploy() {
    let tmp = tempfile::tempdir().unwrap();
    let http = fake();
    let s = game(
        &tmp.path().join("data"),
        &tmp.path().join("game"),
        http.clone(),
    );

    // Search: deprecated packages are hidden unless asked for.
    let page = s.ts_search("test-quest", &TsQuery::default()).unwrap();
    let names: Vec<&str> = page.items.iter().map(|i| i.package.name.as_str()).collect();
    // Mod managers are left out, but not other packages tagged Tools.
    assert_eq!(
        names,
        ["BepInExPack", "Lantern_Tweaks", "Compass"],
        "pinned first"
    );
    assert_eq!(page.categories, ["Libraries", "Mods", "Tools"]);
    let tools = TsQuery {
        category: Some("Tools".into()),
        ..Default::default()
    };
    assert_eq!(
        s.ts_search("test-quest", &tools).unwrap().total,
        2,
        "shown when asked for"
    );
    let q = TsQuery {
        text: "brighter".into(),
        ..Default::default()
    };
    assert_eq!(s.ts_search("test-quest", &q).unwrap().total, 1);

    // Installing a mod brings its dependency, and both land in the profile, pack first.
    let mut steps = Vec::new();
    let report = s
        .ts_install(
            "test-quest",
            &[("Ann-Lantern_Tweaks".into(), Some("1.1.0".into()))],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |m| steps.push(m.to_string()),
        )
        .unwrap();
    assert_eq!(report.installed.len(), 2);
    // The mod is fetched first (its manifest names the dependency), each zip only once.
    let downloads: Vec<&String> = steps
        .iter()
        .filter(|m| m.starts_with("Downloading"))
        .collect();
    assert_eq!(
        downloads,
        [
            "Downloading Lantern_Tweaks 1.1.0",
            "Downloading BepInExPack 5.4.2305"
        ],
        "{steps:?}"
    );
    let p = s.profile("test-quest", DEFAULT_PROFILE).unwrap();
    let ids: Vec<&str> = p.mods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["bepinex-bepinexpack", "ann-lantern-tweaks"]);
    assert_eq!(
        s.ts_dependents("test-quest", "bepinex-bepinexpack")
            .unwrap(),
        ["Ann-Lantern_Tweaks"]
    );

    // The plan places the pack at the game root and the plugin under its own folder.
    let plan = s.plan("test-quest", DEFAULT_PROFILE).unwrap();
    let targets: Vec<String> = plan.files.iter().map(|f| f.target.to_string()).collect();
    assert_eq!(
        targets,
        [
            ".doorstop_version",
            "BepInEx/config/lanterns.cfg",
            "BepInEx/core/BepInEx.Preloader.dll",
            "BepInEx/plugins/Ann-Lantern_Tweaks/LanternTweaks.dll",
            "doorstop_config.ini",
            "winhttp.dll",
        ]
    );

    // Deploying puts only the proxy and its config in the game folder; BepInEx goes to the
    // profile's runtime folder. Then revert.
    let root = tmp.path().join("game");
    s.deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(root.join("winhttp.dll").exists());
    assert!(!root.join("BepInEx").exists());
    let rt = s.game_dir("test-quest").runtime_dir(DEFAULT_PROFILE);
    assert!(rt
        .join("BepInEx/plugins/Ann-Lantern_Tweaks/LanternTweaks.dll")
        .exists());
    s.revert("test-quest", false, &mut Faults::none()).unwrap();
    assert!(!root.join("winhttp.dll").exists());

    // A newer version appears: it shows as an update, and updating keeps the profile.
    http.responses
        .lock()
        .unwrap()
        .insert("https://cdn/chunk1".into(), gz(&listing("1.2.0")));
    s.ts_packages("test-quest", true).unwrap();
    let ups = s.ts_updates("test-quest").unwrap();
    assert_eq!(
        ups,
        [(
            "Ann-Lantern_Tweaks".to_string(),
            "1.1.0".to_string(),
            "1.2.0".to_string()
        )]
    );
    let r = s
        .ts_update(
            "test-quest",
            &[],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        r.installed,
        [(
            "Ann-Lantern_Tweaks".to_string(),
            "1.2.0".to_string(),
            Some("1.1.0".to_string())
        )]
    );
    let stored = s
        .mods("test-quest")
        .unwrap()
        .into_iter()
        .find(|m| m.id == "ann-lantern-tweaks")
        .unwrap();
    assert_eq!(stored.version.as_deref(), Some("1.2.0"));
    assert!(s.ts_updates("test-quest").unwrap().is_empty());
    assert_eq!(
        s.profile("test-quest", DEFAULT_PROFILE).unwrap().mods.len(),
        2
    );

    // Switched to generic, the packages stay stored but are left out of the plan.
    s.set_loader("test-quest", LoaderSetup::Generic).unwrap();
    let plan = s.plan("test-quest", DEFAULT_PROFILE).unwrap();
    assert!(plan.files.is_empty());
    assert_eq!(plan.skipped.len(), 2);
    assert!(s.ts_search("test-quest", &TsQuery::default()).is_err());
}

#[test]
fn a_local_package_zip_is_laid_out_like_a_download() {
    let tmp = tempfile::tempdir().unwrap();
    let s = game(&tmp.path().join("data"), &tmp.path().join("game"), fake());
    let zip = tmp.path().join("Local.zip");
    fs::write(
        &zip,
        zip_of(&[
            (
                "manifest.json",
                "\u{feff}{\"name\":\"Local_Thing\",\"version_number\":\"0.3.0\"}",
            ),
            ("Local.dll", "x"),
        ]),
    )
    .unwrap();
    let m = s
        .import("test-quest", &zip, Default::default(), DEFAULT_PROFILE)
        .unwrap();
    assert_eq!(m.name, "Local Thing");
    assert_eq!(m.version.as_deref(), Some("0.3.0"));
    let plan = s.plan("test-quest", DEFAULT_PROFILE).unwrap();
    assert_eq!(
        plan.files[0].target.to_string(),
        "BepInEx/plugins/Local_Thing/Local.dll"
    );
}

#[test]
fn launch_reports_a_missing_or_unsupported_pack() {
    let tmp = tempfile::tempdir().unwrap();
    let s = game(&tmp.path().join("data"), &tmp.path().join("game"), fake());
    let err = s
        .deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("no BepInEx pack"), "{err}");

    let six = tmp.path().join("six.zip");
    fs::write(
        &six,
        zip_of(&[("BepInEx/core/BepInEx.Unity.IL2CPP.dll", "x")]),
    )
    .unwrap();
    s.import("test-quest", &six, Default::default(), DEFAULT_PROFILE)
        .unwrap();
    let err = s
        .deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("dotnet/coreclr.dll"), "{err}");
}

#[test]
fn bepinex_6_il2cpp_packs_run_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let http = fake();
    let root = tmp.path().join("game");
    // Writes the game's file; the session used below sees the changed Thunderstore data.
    game(&tmp.path().join("data"), &root, http.clone());
    // An IL2CPP build of the game, and Thunderstore's IL2CPP pack in its community.
    fs::write(root.join("GameAssembly.dll"), "il2cpp").unwrap();
    let with_pack = listing("1.1.0")
        .trim_end()
        .trim_end_matches(']')
        .to_string()
        + r#", {"name": "BepInExPack_IL2CPP", "full_name": "BepInEx-BepInExPack_IL2CPP", "owner": "BepInEx",
               "versions": [{"version_number": "6.0.788"}]}]"#;
    http.responses
        .lock()
        .unwrap()
        .insert("https://cdn/chunk1".into(), gz(&with_pack));
    http.responses.lock().unwrap().insert(
        "https://thunderstore.io/package/download/BepInEx/BepInExPack_IL2CPP/6.0.788/".into(),
        zip_of(&[
            (
                "manifest.json",
                r#"{"name":"BepInExPack_IL2CPP","version_number":"6.0.788"}"#,
            ),
            ("BepInExPack/winhttp.dll", "proxy"),
            (
                "BepInExPack/doorstop_config.ini",
                "[General]\nenabled = true",
            ),
            ("BepInExPack/.doorstop_version", "4.0.0"),
            ("BepInExPack/BepInEx/core/BepInEx.Unity.IL2CPP.dll", "bep6"),
            ("BepInExPack/dotnet/coreclr.dll", "clr"),
            ("BepInExPack/dotnet/System.Private.CoreLib.dll", "corlib"),
        ]),
    );
    let mut eco: serde_json::Value = serde_json::from_str(ECOSYSTEM).unwrap();
    eco["modloaderPackages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"packageId": "BepInEx-BepInExPack_IL2CPP", "rootFolder": "BepInExPack", "loader": "bepinex"}));
    http.responses
        .lock()
        .unwrap()
        .insert(ECOSYSTEM_URL.into(), eco.to_string().into_bytes());
    let s = Session::open_with(Some(tmp.path().join("data")), http).unwrap();
    s.ts_packages("test-quest", true).unwrap();

    // The tab offers the IL2CPP pack, not the Mono one.
    let page = s.ts_search("test-quest", &TsQuery::default()).unwrap();
    assert_eq!(
        page.loader_package.as_deref(),
        Some("BepInEx-BepInExPack_IL2CPP")
    );
    s.ts_install(
        "test-quest",
        &[("BepInEx-BepInExPack_IL2CPP".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();

    // BepInEx/ and dotnet/ go to the runtime folder; only the proxy and its config to the game.
    s.deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let rt = s.game_dir("test-quest").runtime_dir(DEFAULT_PROFILE);
    assert!(rt.join("BepInEx/core/BepInEx.Unity.IL2CPP.dll").exists());
    assert!(rt.join("dotnet/coreclr.dll").exists());
    assert!(root.join("winhttp.dll").exists());
    assert!(!root.join("dotnet").exists() && !root.join("BepInEx").exists());
    s.revert("test-quest", false, &mut Faults::none()).unwrap();
    assert!(!root.join("winhttp.dll").exists());

    // Deleting the pack takes its files out of the runtime folder at once; a config the game
    // wrote there stays.
    fs::create_dir_all(rt.join("BepInEx/config")).unwrap();
    fs::write(rt.join("BepInEx/config/BepInEx.cfg"), "game wrote this").unwrap();
    s.remove_mod("test-quest", "bepinex-bepinexpack-il2cpp")
        .unwrap();
    assert!(!rt.join("BepInEx/core/BepInEx.Unity.IL2CPP.dll").exists());
    assert!(!rt.join("dotnet/coreclr.dll").exists());
    assert!(rt.join("BepInEx/config/BepInEx.cfg").exists());
    // Folders left empty go too; one still holding the game's config stays.
    assert!(!rt.join("dotnet").exists());
    assert!(!rt.join("BepInEx/core").exists());
    assert!(rt.join("BepInEx/config").is_dir());
}

#[test]
fn profiles_export_and_import_like_r2modman() {
    let tmp = tempfile::tempdir().unwrap();
    let http = fake();
    let s = game(
        &tmp.path().join("data"),
        &tmp.path().join("game"),
        http.clone(),
    );
    s.ts_install(
        "test-quest",
        &[("Ann-Lantern_Tweaks".into(), Some("1.1.0".into()))],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    // A mod that is not from Thunderstore, disabled packages, and a config the game wrote.
    let local = tmp.path().join("local");
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("x.txt"), "x").unwrap();
    s.import("test-quest", &local, Default::default(), DEFAULT_PROFILE)
        .unwrap();
    let mut p = s.profile("test-quest", DEFAULT_PROFILE).unwrap();
    p.mods[1].enabled = false;
    s.save_profile("test-quest", &p).unwrap();
    let rt = s.game_dir("test-quest").runtime_dir(DEFAULT_PROFILE);
    fs::create_dir_all(rt.join("BepInEx/config")).unwrap();
    fs::write(rt.join("BepInEx/config/lanterns.cfg"), "bright=false").unwrap();
    fs::write(rt.join("BepInEx/LogOutput.log"), "log").unwrap();

    let e = s.profile_export("test-quest", DEFAULT_PROFILE).unwrap();
    assert_eq!(e.mods, 2);
    assert_eq!(e.left_out, ["local"]);
    assert_eq!(e.files, ["BepInEx/config/lanterns.cfg"]);

    let imported = s.profile_import("test-quest", &e.r2z, &mut |_| {}).unwrap();
    assert_eq!(imported.profile, "Default (2)");
    assert!(imported.installed.installed.is_empty(), "already installed");
    let p2 = s.profile("test-quest", "Default (2)").unwrap();
    let state: Vec<(&str, bool)> = p2.mods.iter().map(|m| (m.id.as_str(), m.enabled)).collect();
    assert_eq!(
        state,
        [("bepinex-bepinexpack", true), ("ann-lantern-tweaks", false)]
    );
    assert_eq!(
        p2.pin("ann-lantern-tweaks"),
        Some("1.1.0"),
        "imports are pinned"
    );
    let rt2 = s.game_dir("test-quest").runtime_dir("Default (2)");
    assert_eq!(
        fs::read_to_string(rt2.join("BepInEx/config/lanterns.cfg")).unwrap(),
        "bright=false"
    );

    // Through a code: the export comes back from Thunderstore as base64.
    use base64::Engine as _;
    let body = format!(
        "#r2modman\n{}",
        base64::engine::general_purpose::STANDARD.encode(&e.r2z)
    );
    http.responses.lock().unwrap().insert(
        "https://thunderstore.io/api/experimental/legacyprofile/get/abc-123/".into(),
        body.into_bytes(),
    );
    let via_code = s
        .profile_import_code("test-quest", "abc-123", &mut |_| {})
        .unwrap();
    assert_eq!(via_code.profile, "Default (3)");

    // The game moves on to 1.2.0, and 1.1.0 is gone from Thunderstore. Updating keeps 1.1.0
    // for the imported profiles pinned to it; importing the old export again pins the next
    // newer version instead, and says so.
    http.responses
        .lock()
        .unwrap()
        .insert("https://cdn/chunk1".into(), gz(&listing("1.2.0")));
    s.ts_packages("test-quest", true).unwrap();
    s.ts_update(
        "test-quest",
        &[],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        umm_store::versions::other_versions(&s.game_dir("test-quest"), "ann-lantern-tweaks"),
        ["1.1.0"]
    );
    let again = s.profile_import("test-quest", &e.r2z, &mut |_| {}).unwrap();
    assert!(
        again.installed.installed.is_empty(),
        "1.2.0 is stored already"
    );
    assert_eq!(
        again.notes,
        ["Ann-Lantern_Tweaks 1.1.0 is no longer on Thunderstore; pinned the next newer version, 1.2.0, instead"]
    );
    let v = s
        .ts_installed("test-quest")
        .unwrap()
        .into_iter()
        .find(|p| p.meta.full_name == "Ann-Lantern_Tweaks")
        .unwrap()
        .meta
        .version;
    assert_eq!(v, "1.2.0");
    let p4 = s.profile("test-quest", &again.profile).unwrap();
    assert_eq!(p4.pin("ann-lantern-tweaks"), Some("1.2.0"));
}

#[test]
fn deleting_a_plugin_removes_its_folder_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let s = game(&tmp.path().join("data"), &tmp.path().join("game"), fake());
    s.ts_install(
        "test-quest",
        &[("Ann-Lantern_Tweaks".into(), Some("1.1.0".into()))],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    s.deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    s.revert("test-quest", false, &mut Faults::none()).unwrap();
    let rt = s.game_dir("test-quest").runtime_dir(DEFAULT_PROFILE);
    let folder = rt.join("BepInEx/plugins/Ann-Lantern_Tweaks");
    assert!(folder.join("LanternTweaks.dll").exists());

    s.remove_mod("test-quest", "ann-lantern-tweaks").unwrap();
    assert!(!folder.exists(), "the plugin's own folder goes too");
    assert!(
        rt.join("BepInEx/core/BepInEx.Preloader.dll").exists(),
        "the pack stays"
    );
}

/// A file with just enough of a PE header to tell 32-bit (0x14c) from 64-bit (0x8664).
fn fake_pe(machine: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x40];
    b[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b.extend_from_slice(b"PE\0\0");
    b.extend_from_slice(&machine.to_le_bytes());
    b
}

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn a_32_bit_pack_is_flagged_and_an_unlisted_64_bit_one_replaces_it() {
    let tmp = tempfile::tempdir().unwrap();
    let http = fake();
    let root = tmp.path().join("game");
    let s = game(&tmp.path().join("data"), &root, http.clone());
    fs::write(root.join("Game.exe"), fake_pe(0x8664)).unwrap();

    // The community's own pack ships a 32-bit proxy; the game is 64-bit.
    http.responses.lock().unwrap().insert(
        "https://thunderstore.io/package/download/BepInEx/BepInExPack/5.4.2305/".into(),
        zip_bytes(&[
            (
                "manifest.json",
                br#"{"name":"BepInExPack","version_number":"5.4.2305"}"#,
            ),
            ("BepInExPack/winhttp.dll", &fake_pe(0x14c)),
            ("BepInExPack/BepInEx/core/BepInEx.Preloader.dll", b"old"),
        ]),
    );
    let r = s
        .ts_install(
            "test-quest",
            &[("Ann-Lantern_Tweaks".into(), Some("1.1.0".into()))],
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert!(
        r.warnings[0].contains("is 32-bit, but Test Quest is 64-bit"),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings[0].ends_with("an unlisted BepInEx pack built for 64-bit."),
        "{:?}",
        r.warnings
    );
    let page = s.ts_search("test-quest", &TsQuery::default()).unwrap();
    assert!(page.loader_warning.is_some());
    // The known pack for a 64-bit Mono game is the one installed, so nothing to suggest;
    // for an IL2CPP build, the IL2CPP pack.
    assert_eq!(page.loader_suggestion, None);
    fs::write(root.join("GameAssembly.dll"), "il2cpp").unwrap();
    let page = s.ts_search("test-quest", &TsQuery::default()).unwrap();
    assert_eq!(
        page.loader_suggestion.as_deref(),
        Some("BepInEx-BepInExPack_IL2CPP")
    );
    fs::remove_file(root.join("GameAssembly.dll")).unwrap();
    assert!(s
        .deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err()
        .to_string()
        .contains("Windows would not load it"));

    // A 64-bit pack the community does not list (its listing here was rejected).
    let api = "https://thunderstore.io/api/experimental/package/BepInEx/BepInExPack_Wide/";
    http.responses.lock().unwrap().insert(
        api.into(),
        br#"{"name":"BepInExPack_Wide","full_name":"BepInEx-BepInExPack_Wide","owner":"BepInEx",
             "latest":{"version_number":"6.0.1","description":"64-bit pack"},
             "community_listings":[{"community":"test-quest","review_status":"rejected"}]}"#
            .to_vec(),
    );
    http.responses.lock().unwrap().insert(
        "https://thunderstore.io/package/download/BepInEx/BepInExPack_Wide/6.0.1/".into(),
        zip_bytes(&[
            (
                "manifest.json",
                br#"{"name":"BepInExPack_Wide","version_number":"6.0.1"}"#,
            ),
            ("BepInExPack/winhttp.dll", &fake_pe(0x8664)),
            ("BepInExPack/BepInEx/core/BepInEx.Preloader.dll", b"new"),
        ]),
    );
    let mut eco: serde_json::Value = serde_json::from_str(ECOSYSTEM).unwrap();
    eco["modloaderPackages"].as_array_mut().unwrap().push(
        serde_json::json!({"packageId": "BepInEx-BepInExPack_Wide", "rootFolder": "BepInExPack", "loader": "bepinex"}),
    );
    http.responses
        .lock()
        .unwrap()
        .insert(ECOSYSTEM_URL.into(), eco.to_string().into_bytes());
    let s = Session::open_with(Some(tmp.path().join("data")), http.clone()).unwrap();
    s.ecosystem(true).unwrap();

    let found = s
        .ts_lookup("test-quest", "BepInEx-BepInExPack_Wide")
        .unwrap();
    assert!(!found.listed);
    assert_eq!(found.review_status.as_deref(), Some("rejected"));
    let r = s
        .ts_install_unlisted(
            "test-quest",
            "BepInEx-BepInExPack_Wide",
            None,
            DEFAULT_PROFILE,
            umm_core::PinConflictChoice::Ask,
            &mut |_| {},
        )
        .unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(
        r.notes.len(),
        1,
        "the old pack is turned off: {:?}",
        r.notes
    );
    let p = s.profile("test-quest", DEFAULT_PROFILE).unwrap();
    let old = p
        .mods
        .iter()
        .find(|m| m.id == "bepinex-bepinexpack")
        .unwrap();
    assert!(!old.enabled);
    assert!(s
        .ts_search("test-quest", &TsQuery::default())
        .unwrap()
        .loader_warning
        .is_none());

    // Updating the mod (which names the old pack) does not bring the old pack back.
    s.ts_install(
        "test-quest",
        &[("Ann-Lantern_Tweaks".into(), None)],
        DEFAULT_PROFILE,
        umm_core::PinConflictChoice::Ask,
        &mut |_| {},
    )
    .unwrap();
    let p = s.profile("test-quest", DEFAULT_PROFILE).unwrap();
    assert!(
        !p.mods
            .iter()
            .find(|m| m.id == "bepinex-bepinexpack")
            .unwrap()
            .enabled
    );
    s.deploy("test-quest", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    s.revert("test-quest", false, &mut Faults::none()).unwrap();
}
