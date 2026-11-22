//! Resonite-style games with BepisLoader from Thunderstore (BepInEx for the .NET process, and
//! for the Unity renderer from a companion package), against fake Thunderstore data.

use flate2::write::GzEncoder;
use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{Faults, LoaderSetup, Session, TsQuery, DEFAULT_PROFILE};
use umm_thunderstore::ecosystem::ECOSYSTEM_URL;
use umm_thunderstore::http::FakeHttp;

const INDEX: &str = "https://thunderstore.io/c/reso/api/v1/package-listing-index/";

// As for WEBFISHING: no install rules, and GDWeave as the loader package.
// As for Resonite: BepInEx install rules, and two loader packages, both under `BepInExPack/`.
const ECOSYSTEM: &str = r#"{
  "games": {
    "reso": {"label": "reso",
      "distributions": [{"platform": "steam", "identifier": "6270"}],
      "r2modman": [{"packageLoader": "bepisloader", "exeNames": ["Reso.exe"],
        "dataFolderName": "Reso_Data",
        "packageIndex": "https://thunderstore.io/c/reso/api/v1/package-listing-index/",
        "installRules": [
          {"route": "BepInEx/plugins", "isDefaultLocation": true, "defaultFileExtensions": [".dll"], "trackingMethod": "subdir", "subRoutes": []},
          {"route": "Renderer/BepInEx/plugins", "isDefaultLocation": false, "defaultFileExtensions": [], "trackingMethod": "subdir", "subRoutes": []}
        ]}]}
  },
  "modloaderPackages": [
    {"packageId": "ResoniteModding-BepisLoader", "rootFolder": "BepInExPack", "loader": "bepisloader"},
    {"packageId": "ResoniteModding-BepInExRenderer", "rootFolder": "BepInExPack", "loader": "bepisloader"}
  ]
}"#;

const LISTING: &str = r#"[
  {"name": "BepisLoader", "full_name": "ResoniteModding-BepisLoader", "owner": "ResoniteModding",
   "is_pinned": true, "categories": ["Libraries"], "downloads": 100,
   "versions": [{"version_number": "1.6.0", "dependencies": []}]},
  {"name": "BepInExRenderer", "full_name": "ResoniteModding-BepInExRenderer", "owner": "ResoniteModding",
   "categories": ["Libraries"], "downloads": 500,
   "versions": [{"version_number": "1.0.0", "dependencies": []}]},
  {"name": "Mine", "full_name": "Me-Mine", "owner": "Me", "categories": ["Mods"],
   "versions": [{"version_number": "1.0.0", "dependencies": ["ResoniteModding-BepisLoader-1.6.0", "ResoniteModding-BepInExRenderer-1.0.0"]}]}
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
    let bepis = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"BepisLoader","version_number":"1.6.0","dependencies":[]}"#,
        ),
        ("BepInExPack/hostfxr.dll", "hook"),
        ("BepInExPack/hookfxr.ini", "[hookfxr]\nenable=false\n"),
        ("BepInExPack/BepisLoader.dll", "bepis"),
        ("BepInExPack/BepInEx/core/BepInEx.Core.dll", "core"),
    ]);
    let renderer = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"BepInExRenderer","version_number":"1.0.0","dependencies":[]}"#,
        ),
        (
            "BepInExPack/Renderer/BepInEx/core/BepInEx.Unity.Mono.Preloader.dll",
            "preloader",
        ),
        ("BepInExPack/Renderer/winhttp.dll", "proxy"),
    ]);
    let mine = zip_of(&[
        (
            "manifest.json",
            r#"{"name":"Mine","version_number":"1.0.0","dependencies":["ResoniteModding-BepisLoader-1.6.0","ResoniteModding-BepInExRenderer-1.0.0"]}"#,
        ),
        ("plugins/Mine.dll", "main"),
        ("Renderer/BepInEx/plugins/Mine.Renderer.dll", "renderer"),
    ]);
    Arc::new(
        FakeHttp::default()
            .with(ECOSYSTEM_URL, ECOSYSTEM)
            .with(INDEX, gz(r#"["https://cdn/chunk1"]"#))
            .with("https://cdn/chunk1", gz(LISTING))
            .with(
                "https://thunderstore.io/package/download/ResoniteModding/BepisLoader/1.6.0/",
                bepis,
            )
            .with(
                "https://thunderstore.io/package/download/ResoniteModding/BepInExRenderer/1.0.0/",
                renderer,
            )
            .with(
                "https://thunderstore.io/package/download/Me/Mine/1.0.0/",
                mine,
            ),
    )
}

#[test]
fn bepisloader_mods_both_processes_from_the_runtime_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let s = Session::open_with(Some(tmp.path().join("data")), fake()).unwrap();
    fs::create_dir_all(root.join("Renderer")).unwrap();
    fs::write(root.join("Reso.exe"), "game").unwrap();
    fs::write(root.join("Renderer/Renderite.Renderer.exe"), "renderer").unwrap();
    let p = s.data().game("reso").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        "schema = 1\nname = \"Reso\"\n[detect]\nsteam_app_id = 6270\n\
         [launch]\nexe = \"Reso.exe\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("reso", &root).unwrap();

    let c = s.loader_choice("reso").unwrap();
    assert_eq!(c.ts_loader.as_deref(), Some("bepisloader"));
    s.set_loader("reso", LoaderSetup::Thunderstore).unwrap();
    assert_eq!(
        s.game_def("reso").unwrap().setup_label(),
        "BepisLoader from Thunderstore"
    );
    // The banner offers BepisLoader, not its companion (though downloaded more).
    let page = s.ts_search("reso", &TsQuery::default()).unwrap();
    assert_eq!(page.loader_name, "BepisLoader");
    assert_eq!(
        page.loader_package.as_deref(),
        Some("ResoniteModding-BepisLoader")
    );

    s.ts_install(
        "reso",
        &[("Me-Mine".into(), None)],
        DEFAULT_PROFILE,
        &mut |_| {},
    )
    .unwrap();
    // Both loader packages stay installed and enabled: the renderer one comes with BepisLoader.
    let profile = fs::read_to_string(
        s.data()
            .game("reso")
            .root()
            .join("profiles")
            .join("Default.toml"),
    )
    .unwrap();
    for id in [
        "resonitemodding-bepisloader",
        "resonitemodding-bepinexrenderer",
    ] {
        let at = profile.find(&format!("id = \"{id}\"")).expect(id);
        assert!(
            profile[at..].lines().nth(1).unwrap().contains("true"),
            "{profile}"
        );
    }
    s.deploy("reso", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();

    // The hook and BepisLoader's own files go into the game folder; each process's BepInEx,
    // with the mods' plugins, to the runtime folder.
    for f in ["hostfxr.dll", "hookfxr.ini", "BepisLoader.dll"] {
        assert!(root.join(f).is_file(), "{f}");
    }
    assert!(!root.join("BepInEx").exists());
    assert!(!root.join("Renderer/BepInEx").exists() && !root.join("Renderer/winhttp.dll").exists());
    let rt = s.game_dir("reso").runtime_dir(DEFAULT_PROFILE);
    for f in [
        "BepInEx/core/BepInEx.Core.dll",
        "BepInEx/plugins/Me-Mine/Mine.dll",
        "Renderer/BepInEx/core/BepInEx.Unity.Mono.Preloader.dll",
        "Renderer/BepInEx/plugins/Me-Mine/Mine.Renderer.dll",
    ] {
        assert!(rt.join(f).is_file(), "{f}");
    }

    s.revert("reso", false, &mut Faults::none()).unwrap();
    assert!(!root.join("hostfxr.dll").exists());
    assert_eq!(fs::read_dir(root.join("Renderer")).unwrap().count(), 1);
}
