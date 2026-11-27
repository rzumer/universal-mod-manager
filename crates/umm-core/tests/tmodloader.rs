//! tModLoader: `.tmod` mods go to its mods folder in its save folder, named after the mod in
//! each, and are enabled in `enabled.json` there while deployed; the player's own list and
//! mods come back afterwards.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use umm_core::{Faults, ImportOptions, LoaderSetup, Session, DEFAULT_PROFILE};
use umm_gamedef::steam::{draft_manual, inspect_folder, Engine};

const GAME: &str = "tmodloader";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// A `.tmod` file as tModLoader writes it, up to its name and version.
fn tmod(name: &str, version: &str) -> Vec<u8> {
    let s = |out: &mut Vec<u8>, v: &str| {
        out.push(v.len() as u8);
        out.extend_from_slice(v.as_bytes());
    };
    let mut out = b"TMOD".to_vec();
    s(&mut out, "2024.9.3.0");
    out.extend_from_slice(&[7u8; 20 + 256 + 4]);
    s(&mut out, name);
    s(&mut out, version);
    out.extend_from_slice(b"compressed files");
    out
}

fn import(s: &Session, tmp: &Path, id: &str, files: &[(&str, Vec<u8>)]) {
    let src = tmp.join("src").join(id);
    for (rel, bytes) in files {
        write(&src.join(rel), bytes);
    }
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some(id.into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
}

/// The game's definition as drafted, with its save folder in `save`.
fn define(s: &Session, root: &Path, save: &Path, extra: &str) {
    let save = save.display().to_string().replace('\\', "/");
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"tModLoader\"\n{extra}\
             [launch]\nexe = \"start-tModLoader.bat\"\n\
             [[install]]\nmatch = \"**/*.tmod\"\ntarget = \"@user/Mods/\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/ResourcePacks/{{mod}}/\"\n\
             [folders]\nuser = '{save}'\n\
             [mod_list]\nlist = '{save}/Mods/enabled.json'\nformat = \"tmodloader_json\"\nfolder = \"@user/Mods\"\n"
        )
        .as_bytes(),
    );
    s.set_install_path(GAME, root).unwrap();
}

#[test]
fn tmod_files_go_to_its_mods_folder_named_after_their_mod() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("start-tModLoader.bat"), b"@echo off");
    write(&root.join("tModLoader.dll"), b"dll");
    write(&root.join("LaunchUtils/busybox64.exe"), b"exe");

    let found = inspect_folder(&root);
    assert_eq!(found.engine, Engine::TModLoader);
    let draft = draft_manual(
        &root,
        "tModLoader",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert!(draft.notes.iter().any(|n| n.key == "tmodloader"));
    let rules: Vec<(&str, &str)> = draft
        .def
        .install
        .iter()
        .map(|r| (r.pattern.as_str(), r.target.as_str()))
        .collect();
    assert_eq!(
        rules,
        [
            ("**/*.tmod", "@user/Mods/"),
            ("**", "@user/ResourcePacks/{mod}/")
        ]
    );

    // The player's own mod and list.
    let save = tmp.path().join("docs/tModLoader");
    let mods = save.join("Mods");
    let list = mods.join("enabled.json");
    write(&mods.join("Theirs.tmod"), &tmod("Theirs", "1.0"));
    let original = "[\r\n  \"Theirs\"\r\n]";
    write(&list, original.as_bytes());

    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    define(&s, &root, &save, "");
    // Shipped in a folder, under a file name of its own, with a readme.
    import(
        &s,
        tmp.path(),
        "calamity",
        &[
            (
                "Calamity v2/CalamityMod_v2.0.4.tmod",
                tmod("CalamityMod", "2.0.4"),
            ),
            ("Calamity v2/README.txt", b"read me".to_vec()),
        ],
    );
    import(
        &s,
        tmp.path(),
        "magic-storage",
        &[("MagicStorage.tmod", tmod("MagicStorage", "0.6"))],
    );

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(mods.join("CalamityMod.tmod")).unwrap(),
        tmod("CalamityMod", "2.0.4")
    );
    assert!(mods.join("MagicStorage.tmod").is_file());
    assert!(!mods.join("Calamity v2").exists());
    assert!(!mods.join("README.txt").exists());
    assert_eq!(
        fs::read_to_string(&list).unwrap(),
        "[\r\n  \"Theirs\",\r\n  \"CalamityMod\",\r\n  \"MagicStorage\"\r\n]"
    );
    // Nothing in the game folder.
    assert!(!root.join("Mods").exists());

    // As tModLoader rewrites it when it starts, in an order of its own: not a change to keep.
    write(
        &list,
        b"[\r\n  \"MagicStorage\",\r\n  \"Theirs\",\r\n  \"CalamityMod\"\r\n]",
    );
    let r = s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(r.is_some_and(|r| r.quarantined.is_empty()));
    assert_eq!(fs::read_to_string(&list).unwrap(), original);
    assert!(!mods.join("CalamityMod.tmod").exists());
    assert!(!mods.join("MagicStorage.tmod").exists());
    assert!(mods.join("Theirs.tmod").is_file());

    // Two mods that are the same tModLoader mod cannot both be deployed.
    import(
        &s,
        tmp.path(),
        "calamity-copy",
        &[("CalamityMod.tmod", tmod("CalamityMod", "2.0.5"))],
    );
    let err = s
        .deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("are both the tModLoader mod `CalamityMod`"),
        "{err}"
    );
}

/// A Steam record listing Workshop items 1, 2 and 3 as downloaded and subscribed.
fn record(app: u32) -> String {
    let entry = |i: &str| {
        format!("\t\t\"{i}\"\n\t\t{{\n\t\t\t\"size\"\t\t\"1\"\n\t\t\t\"timeupdated\"\t\t\"1\"\n\t\t\t\"subscribedby\"\t\t\"9\"\n\t\t}}\n")
    };
    let items: String = ["1", "2", "3"].iter().map(|i| entry(i)).collect();
    format!("\"AppWorkshop\"\n{{\n\t\"appid\"\t\t\"{app}\"\n\t\"WorkshopItemsInstalled\"\n\t{{\n{items}\t}}\n\t\"WorkshopItemDetails\"\n\t{{\n{items}\t}}\n}}\n")
}

#[test]
fn a_profile_turns_tmodloader_workshop_items_on_and_off() {
    let tmp = tempfile::tempdir().unwrap();
    let steamapps = tmp.path().join("lib/steamapps");
    let root = steamapps.join("common/tModLoader");
    write(&root.join("start-tModLoader.bat"), b"@echo off");
    write(&root.join("tModLoader.dll"), b"dll");
    write(
        &steamapps.join("workshop/appworkshop_1281930.acf"),
        record(1281930).as_bytes(),
    );
    for (i, name) in [("1", "One"), ("2", "Two"), ("3", "Three")] {
        write(
            &steamapps.join(format!("workshop/content/1281930/{i}/2024.9/{name}.tmod")),
            &tmod(name, "1.0"),
        );
    }
    // The player's own list has 2 and 3 on.
    let save = tmp.path().join("docs/tModLoader");
    let list = save.join("Mods/enabled.json");
    let original = "[\"Two\", \"Three\", \"Gone\"]";
    write(&list, original.as_bytes());

    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    define(
        &s,
        &root,
        &save,
        "[detect]\nsteam_app_id = 1281930\nmarker = \"start-tModLoader.bat\"\n",
    );
    assert_eq!(
        umm_core::workshop::workshop_control(&s.def(GAME).unwrap().def),
        umm_core::workshop::WorkshopControl::Profile
    );
    // 1 on, 2 off; 3 is not in the profile, so it stays as the player's list has it.
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods = vec![
        umm_core::ProfileEntry::new("workshop:1", true),
        umm_core::ProfileEntry::new("workshop:2", false),
    ];
    s.save_profile(GAME, &p).unwrap();
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(&list).unwrap(),
        "[\r\n  \"Three\",\r\n  \"Gone\",\r\n  \"One\"\r\n]"
    );
    // Workshop items stay where Steam keeps them.
    assert!(!save.join("Mods/One.tmod").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&list).unwrap(), original);
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

/// The game's settings, with a pack of the player's own, as Terraria writes them.
const CONFIG: &str = "{\r\n  \"SmartCursorToggle\": true,\r\n  \"ResourcePacks\": [\r\n    {\r\n      \"FileName\": \"Theirs\",\r\n      \"Enabled\": true,\r\n      \"SortingOrder\": 0\r\n    }\r\n  ],\r\n  \"Language\": \"en-US\"\r\n}";

#[test]
fn resource_packs_go_to_its_packs_folder_and_are_turned_on_in_its_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("start-tModLoader.bat"), b"@echo off");
    write(&root.join("tModLoader.dll"), b"dll");
    let save = tmp.path().join("docs/tModLoader");
    let packs = save.join("ResourcePacks");
    let config = save.join("config.json");
    write(&packs.join("Theirs/pack.json"), b"{}");
    write(&config, CONFIG.as_bytes());

    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    define(&s, &root, &save, "");
    // A pack in a folder of its own, with a readme beside it; a zipped pack.
    import(
        &s,
        tmp.path(),
        "cool-pack",
        &[
            ("Cool Pack v1/pack.json", b"{\"Name\": \"Cool\"}".to_vec()),
            ("Cool Pack v1/Content/Images/Item_1.png", b"png".to_vec()),
            ("README.txt", b"read me".to_vec()),
        ],
    );
    import(
        &s,
        tmp.path(),
        "zipped",
        &[(
            "ZipPack.zip",
            zip_of(&[("pack.json", "{}"), ("Content/Images/Item_2.png", "png")]),
        )],
    );
    // Not a pack: left out.
    import(&s, tmp.path(), "loose", &[("notes.txt", b"x".to_vec())]);

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(packs.join("cool-pack/pack.json").is_file());
    assert!(packs.join("cool-pack/Content/Images/Item_1.png").is_file());
    assert!(packs.join("ZipPack.zip").is_file());
    assert!(!packs.join("cool-pack/README.txt").exists());
    assert!(!packs.join("cool-pack/Cool Pack v1").exists());
    assert!(!packs.join("loose").exists());
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    let names: Vec<(&str, bool)> = v["ResourcePacks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["FileName"].as_str().unwrap(),
                p["Enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [("Theirs", true), ("cool-pack", true), ("ZipPack.zip", true)]
    );
    assert_eq!(v["Language"], "en-US");

    // The game saves its settings as it quits: a setting changed, the packs as they were.
    let mut v = v;
    v["SmartCursorToggle"] = false.into();
    fs::write(&config, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    let back: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(back["SmartCursorToggle"], false);
    let left: Vec<&str> = back["ResourcePacks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["FileName"].as_str().unwrap())
        .collect();
    assert_eq!(left, ["Theirs"]);
    assert!(!packs.join("cool-pack").exists());
    assert!(!packs.join("ZipPack.zip").exists());
    assert!(packs.join("Theirs/pack.json").is_file());
}
