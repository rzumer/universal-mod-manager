//! Brotato-style games with the Godot Mod Loader built in, from Thunderstore, against fake
//! Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/potato/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for Brotato: mods in `mods/` as whole package zips, and the loader package.
const ECOSYSTEM: &str = r#"{
  "games": {
    "potato": {"label": "potato",
      "distributions": [{"platform": "steam", "identifier": "6264"}],
      "r2modman": [{"packageLoader": "godotml", "exeNames": ["Potato.exe"],
        "dataFolderName": "",
        "packageIndex": "https://thunderstore.io/c/potato/api/v1/package-listing-index/",
        "installRules": [{"route": "mods", "defaultFileExtensions": [], "trackingMethod": "package-zip", "subRoutes": [], "isDefaultLocation": true}]}]}
  },
  "modloaderPackages": [{"packageId": "GodotModding-GodotModLoader", "rootFolder": "", "loader": "godotml"}]
}"#;

const LISTING: &str = r#"[
  {"name": "GodotModLoader", "full_name": "GodotModding-GodotModLoader", "owner": "GodotModding",
   "is_pinned": true, "categories": ["Libraries"],
   "versions": [{"version_number": "6.3.0", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["GodotModding-GodotModLoader-6.3.0"]}]}
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

fn mine() -> Vec<u8> {
    zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["GodotModding-GodotModLoader-6.3.0"]}"#,
        ),
        ("mods-unpacked/Me-Mine/mod_main.gd", "extends Node"),
    ])
}

fn fake() -> Arc<FakeHttp> {
    let pack = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"GodotModLoader","version_number":"6.3.0","dependencies":[]}"#,
        ),
        ("addons/mod_loader/mod_loader.gd", "extends Node"),
        ("addons/mod_loader/mod_loader_setup.gd", "extends SceneTree"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/GodotModding/GodotModLoader/6.3.0/",
                pack,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine(),
            ),
    )
}

#[test]
fn godotml_games_load_mod_zips_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Potato.exe"), "game").unwrap();
    // The game's pack without the loader in its project settings.
    let pck = root.join("Potato.pck");
    fs::write(&pck, "GDPC res://main.tscn").unwrap();
    let p = s.data().game("potato").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Potato\"\n[detect]\nsteam_app_id = 6264\n\
         [launch]\nexe = \"Potato.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("potato", &root).unwrap();

    let c = s.loader_choice("potato").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("godotml"));
    s.set_loader("potato", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("potato").unwrap().setup_label(),
        "Godot Mod Loader from Thunderstore"
    );
    let page = s.ts_search("potato", &TsQuery::default()).unwrap();
    assert_eq!(
        page.loader_package.as_deref(),
        Some("GodotModding-GodotModLoader")
    );

    s.ts_install(
        "potato",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    // Not built in: UMM runs the loader's setup in a scratch copy of the game, once, and
    // deploys what it wrote with the loader's files.
    let runs = Arc::new(AtomicUsize::new(0));
    let r = runs.clone();
    let game_root = root.clone();
    s.set_godotml_runner(Arc::new(move |exe: &Path, args: &[String]| {
        r.fetch_add(1, Ordering::SeqCst);
        let dir = exe.parent().unwrap();
        assert_ne!(dir, game_root, "never in the game folder");
        assert_eq!(args, umm_core::GODOTML_SETUP_ARGS);
        assert!(dir.join("addons/mod_loader/mod_loader_setup.gd").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("steam_appid.txt")).unwrap(),
            "6264"
        );
        fs::write(
            dir.join("override.cfg"),
            "[autoload]
ModLoader=\"*res://addons/mod_loader/mod_loader.gd\"
",
        )?;
        fs::create_dir_all(dir.join("godot"))?;
        fs::write(dir.join("godot/global_script_class_cache.cfg"), "list=[]")
    }));
    let before = listing(&root);
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    for f in [
        "override.cfg",
        "godot/global_script_class_cache.cfg",
        "addons/mod_loader/mod_loader.gd",
    ] {
        assert!(root.join(f).is_file(), "{f}");
    }
    let rt = s.game_dir("potato").runtime_dir(DEFAULT_PROFILE);
    assert_eq!(fs::read(rt.join("mods/Me-Mine.ts.zip")).unwrap(), mine());
    assert_eq!(fs::read_to_string(&pck).unwrap(), "GDPC res://main.tscn");
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(listing(&root), before, "the game folder as it was");

    // Kept: not run again until the game's exe changes.
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    fs::write(root.join("Potato.exe"), "game, updated").unwrap();
    let before = listing(&root);
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(listing(&root), before);

    // A setup that writes nothing is reported, and nothing is deployed.
    s.set_godotml_runner(Arc::new(|_: &Path, _: &[String]| Ok(())));
    fs::write(root.join("Potato.exe"), "game, updated again").unwrap();
    let before = listing(&root);
    let err = s
        .deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(err.to_string().contains("did not finish"), "{err}");
    assert_eq!(listing(&root), before);

    // Built in: the mod's package goes to the runtime folder as one zip; nothing in the game.
    fs::write(
        &pck,
        "GDPC autoload/ModLoader *res://addons/mod_loader/mod_loader.gd",
    )
    .unwrap();
    let before = listing(&root);
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let rt = s.game_dir("potato").runtime_dir(DEFAULT_PROFILE);
    assert_eq!(fs::read(rt.join("mods/Me-Mine.ts.zip")).unwrap(), mine());
    assert!(rt.join("addons/mod_loader/mod_loader.gd").is_file());
    assert!(!root.join("mods").exists() && !root.join("addons").exists());
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(listing(&root), before, "the game folder as it was");
}

/// Every path in a folder, with its contents.
fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = walkdir(dir)
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(dir).unwrap().display().to_string();
            let body = if p.is_file() {
                fs::read(&p).unwrap()
            } else {
                Vec::new()
            };
            (rel, body)
        })
        .collect();
    out.sort();
    out
}

fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        }
        out.push(p);
    }
    out
}

#[test]
fn the_hook_pack_stays_with_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Potato.exe"), "game").unwrap();
    // Built in, to keep the test about the hook pack.
    fs::write(
        root.join("Potato.pck"),
        "GDPC autoload/ModLoader *res://addons/mod_loader/mod_loader.gd",
    )
    .unwrap();
    let p = s.data().game("potato").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Potato\"\n[detect]\nsteam_app_id = 6264\n\
         [launch]\nexe = \"Potato.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("potato", &root).unwrap();
    s.set_loader("potato", LoaderSetup::Thunderstore).unwrap();
    s.ts_install(
        "potato",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    let hooks = root.join("mod-hooks.zip");
    let kept = s
        .game_dir("potato")
        .runtime_dir(DEFAULT_PROFILE)
        .join("mod-hooks.zip");
    let before = listing(&root);

    // The loader writes it during a session: it moves to the profile afterwards.
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(!hooks.exists());
    fs::write(&hooks, "hooks 1").unwrap();
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(listing(&root), before);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "hooks 1");

    // Back for the next session, and kept as the loader leaves it, even after a crash.
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read_to_string(&hooks).unwrap(), "hooks 1");
    fs::write(&hooks, "hooks 2").unwrap();
    for (_, r) in s.recover_all(false) {
        r.unwrap();
    }
    assert_eq!(listing(&root), before);
    assert_eq!(fs::read_to_string(&kept).unwrap(), "hooks 2");

    // A crash at any step of a deploy leaves the game folder and the profile's copy.
    let mut count = Faults::none();
    s.deploy("potato", DEFAULT_PROFILE, &mut count).unwrap();
    s.revert("potato", false, &mut Faults::none()).unwrap();
    for n in 0..count.steps() {
        assert!(s
            .deploy("potato", DEFAULT_PROFILE, &mut Faults::crash_at(n))
            .is_err());
        for (_, r) in s.recover_all(false) {
            r.unwrap();
        }
        assert_eq!(listing(&root), before, "deploy step {n}");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "hooks 2");
    }

    // A hook pack of the user's own is left alone.
    fs::write(&hooks, "theirs").unwrap();
    s.deploy("potato", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read_to_string(&hooks).unwrap(), "theirs");
    s.revert("potato", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&hooks).unwrap(), "theirs");
    assert_eq!(fs::read_to_string(&kept).unwrap(), "hooks 2");
}
