//! Mount & Blade II: Bannerlord: each mod goes in `Modules/`, in a folder named after its
//! module's id, and the game is started with the modules to load named, each after the ones
//! it depends on.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "mb2";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn submodule(id: &str, extra: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Module>\n  <Id value=\"{id}\"/>\n{extra}</Module>\n")
}

fn official(id: &str, deps: &[&str]) -> String {
    let deps: String = deps
        .iter()
        .flat_map(|d| ["    <DependedModule Id=\"", d, "\"/>\n"])
        .collect();
    submodule(
        id,
        &format!(
            "  <ModuleType value=\"Official\"/>\n  <ModuleCategory value=\"{}\"/>\n  <DependedModules>\n{deps}  </DependedModules>\n",
            if id == "Multiplayer" { "Multiplayer" } else { "Singleplayer" }
        ),
    )
}

#[test]
fn bannerlord_modules_are_named_after_their_ids_and_loaded_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(
        &root.join("bin/Win64_Shipping_Client/Bannerlord.exe"),
        b"exe",
    );
    for (id, deps) in [
        ("Native", &[][..]),
        ("SandBoxCore", &["Native"][..]),
        ("Sandbox", &["Native", "SandBoxCore"][..]),
        ("StoryMode", &["Sandbox"][..]),
        ("CustomBattle", &["Native"][..]),
        ("Multiplayer", &["Native"][..]),
    ] {
        write(
            &root.join(format!("Modules/{id}/SubModule.xml")),
            official(id, deps).as_bytes(),
        );
    }
    // The user's own modules: one selected in the launcher, one not.
    write(
        &root.join("Modules/Selected/SubModule.xml"),
        submodule("Selected", "").as_bytes(),
    );
    write(
        &root.join("Modules/Unselected/SubModule.xml"),
        submodule("Unselected", "").as_bytes(),
    );
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"Bannerlord\"\n[detect]\nmarker = \"bin/Win64_Shipping_Client/Bannerlord.exe\"\n\
          [launch]\nexe = \"bin/Win64_Shipping_Client/Bannerlord.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"Modules/{mod}/\"\n\
          [bannerlord]\n",
    );
    s.set_install_path(GAME, &root).unwrap();
    let import = |id: &str, files: &[(&str, &[u8])]| {
        let src = tmp.path().join("src").join(id);
        for (f, b) in files {
            write(&src.join(f), b);
        }
        s.import(
            GAME,
            &src,
            ImportOptions {
                id: Some(id.into()),
                unwrap: true,
                ..Default::default()
            },
            DEFAULT_PROFILE,
        )
        .unwrap();
    };
    // In its own folder in the archive; it needs Harmony loaded before it.
    let smithing = submodule(
        "BetterSmithing",
        "  <DependedModules>\n    <DependedModule Id=\"Sandbox\"/>\n  </DependedModules>\n  <DependedModuleMetadatas>\n    <DependedModuleMetadata id=\"Bannerlord.Harmony\" order=\"LoadBeforeThis\"/>\n  </DependedModuleMetadatas>\n",
    );
    import(
        "smithing",
        &[
            ("BetterSmithing/SubModule.xml", smithing.as_bytes()),
            (
                "BetterSmithing/bin/Win64_Shipping_Client/BetterSmithing.dll",
                b"dll",
            ),
        ],
    );
    // Laid out from the module's folder, and after it in the profile.
    let harmony = submodule("Bannerlord.Harmony", "");
    import(
        "harmony",
        &[
            ("SubModule.xml", harmony.as_bytes()),
            ("bin/Win64_Shipping_Client/0Harmony.dll", b"dll"),
        ],
    );

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(root.join("Modules/BetterSmithing/SubModule.xml").is_file());
    assert!(root
        .join("Modules/BetterSmithing/bin/Win64_Shipping_Client/BetterSmithing.dll")
        .is_file());
    assert!(root
        .join("Modules/Bannerlord.Harmony/SubModule.xml")
        .is_file());
    assert!(!root.join("Modules/smithing").exists());
    assert!(!root.join("Modules/harmony").exists());

    let journal = umm_deploy::Journal::load(&s.data().game(GAME).journal_path())
        .unwrap()
        .unwrap();
    let def = s.def(GAME).unwrap();
    let profile = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    let selection = [
        ("CustomBattle".to_string(), false),
        ("Selected".to_string(), true),
        ("Unselected".to_string(), false),
    ];
    assert_eq!(
        s.bannerlord_args_with(&def, &profile, &journal, &selection),
        [
            "/singleplayer",
            "_MODULES_*Native*SandBoxCore*Sandbox*StoryMode*Selected*Bannerlord.Harmony*BetterSmithing*_MODULES_"
        ]
    );

    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!root.join("Modules/BetterSmithing").exists());
    assert!(root.join("Modules/Native/SubModule.xml").is_file());
}
