//! Drafting the games UMM knows: recognising each in its folder, the definition it gets, and
//! back to its own rules from another setup. (`umm-gamedef` drafts; the games are here.)

#![cfg(test)]

use super::{bannerlord, bethesda, factorio, oni, register, tmodloader, xcom2};
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::steam::{draft_manual, inspect_folder, to_toml, USER_FOLDER};
use umm_gamedef::{GameDef, LoaderSetup, ParadoxList, PluginListFormat};

fn touch(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![0u8; bytes]).unwrap();
}

#[test]
fn paradox_games_are_set_up_for_their_user_folder() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("binaries/ck3.exe"), 100);
    touch(&root.join("launcher/dowser.exe"), 500);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("paradox"), "binaries/ck3.exe")
    );
    let d = draft_manual(root, "CK3", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%DOCUMENTS%/Paradox Interactive/Crusader Kings III")
    );
    assert_eq!(d.def.paradox.as_ref().unwrap().folder, USER_FOLDER);
    assert_eq!(d.def.install.len(), 1);
    assert_eq!(d.def.install[0].target, "@user/mod/{mod}/");
    assert!(d.notes.iter().any(|n| n.key == "paradox"));
    assert_eq!(found.own_mods(), Some(umm_gamedef::BUILT_IN_MODS));
    assert_eq!(d.def.setup_label(), umm_gamedef::BUILT_IN_MODS);
    // Reloads as written, and back to generic keeps the rule.
    let text = to_toml(&d);
    let back = GameDef::parse_game(&text, Path::new("game.toml"), "ck3").unwrap();
    assert_eq!(back.paradox, d.def.paradox);
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(again.install[0].target, "@user/mod/{mod}/");
}

#[test]
fn tmodloader_mods_go_to_its_mods_folder_and_terraria_points_to_it() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tModLoader");
    touch(&root.join(tmodloader::TML_EXE), 10);
    touch(&root.join("tModLoader.dll"), 100);
    touch(&root.join("LaunchUtils/busybox64.exe"), 50);
    let found = inspect_folder(&root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("tmodloader"), tmodloader::TML_EXE)
    );
    let d = draft_manual(
        &root,
        "tModLoader",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert!(d.notes.iter().any(|n| n.key == "tmodloader"));
    // Named after its loader, before and after it is drafted.
    assert_eq!(found.own_mods(), Some("tModLoader"));
    assert_eq!(d.def.setup_label(), "tModLoader");
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%DOCUMENTS%/My Games/Terraria/tModLoader")
    );
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.format, p.folder.as_str()),
        (
            "%DOCUMENTS%/My Games/Terraria/tModLoader/Mods/enabled.json",
            PluginListFormat::TmodloaderJson,
            "@user/Mods"
        )
    );
    let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "tml").unwrap();
    // Set up again (the loader changed back to generic), the rules are the same.
    let again = back.with_setup(LoaderSetup::Thunderstore, None).err();
    assert!(again.is_some());
    assert_eq!(back.install.len(), 2);
    let c = back.compile().unwrap();
    let map = |p: &str| {
        c.map_mod_file("m", &umm_gamedef::RelPath::parse(p).unwrap())
            .unwrap()
            .map(|t| t.to_string())
    };
    assert_eq!(
        map("Wrap/Calamity.tmod").as_deref(),
        Some("@user/Mods/Wrap/Calamity.tmod")
    );
    // Anything else to the mod's folder among the packs, where UMM keeps only packs.
    assert_eq!(
        map("README.md").as_deref(),
        Some("@user/ResourcePacks/m/README.md")
    );

    let terraria = tmp.path().join("Terraria");
    touch(&terraria.join("Terraria.exe"), 100);
    touch(&terraria.join("dotNetFx40_Full_setup.exe"), 10);
    let found = inspect_folder(&terraria);
    let d = draft_manual(
        &terraria,
        "Terraria",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert!(d.notes.iter().any(|n| n.key == "terraria"));
    assert!(d.def.plugins.is_none());
}

#[test]
fn factorio_mods_go_to_its_user_folder_unless_it_keeps_its_own() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("bin/x64/factorio.exe"), 100);
    touch(&root.join("data/base/info.json"), 10);
    fs::write(
        root.join("config-path.cfg"),
        "config-path=__PATH__system-write-data__/config\n\
     # A comment\nuse-system-read-write-data-directories=true\n",
    )
    .unwrap();
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("factorio"), factorio::FACTORIO_EXE)
    );
    let d = draft_manual(
        root,
        "Factorio",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%APPDATA%/Factorio")
    );
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.format, p.folder.as_str()),
        (
            "%APPDATA%/Factorio/mods/mod-list.json",
            PluginListFormat::FactorioJson,
            "@user/mods"
        )
    );
    assert!(d.notes.iter().any(|n| n.key == "factorio"));
    let c = d.def.compile().unwrap();
    let map = |p: &str| {
        c.map_mod_file("k2", &umm_gamedef::RelPath::parse(p).unwrap())
            .unwrap()
            .map(|t| t.to_string())
    };
    assert_eq!(map("info.json").as_deref(), Some("@user/mods/k2/info.json"));
    let text = to_toml(&d);
    let back = GameDef::parse_game(&text, Path::new("game.toml"), "factorio").unwrap();
    assert_eq!(back.plugins, d.def.plugins);
    // Back to generic from another setup: the same rules.
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(again.install[0].target, "@user/mods/{mod}/");

    // A zip copy keeps its mods (and their list) in its own folder.
    fs::write(
        root.join("config-path.cfg"),
        "use-system-read-write-data-directories=false\n",
    )
    .unwrap();
    let d = draft_manual(
        root,
        "Factorio",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert!(d.def.folders.is_empty());
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.folder.as_str()),
        ("mods/mod-list.json", "mods")
    );
    assert_eq!(d.def.install[0].target, "mods/{mod}/");
}

#[test]
fn divinity_original_sin_2_mods_go_to_its_user_folder() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("DefEd/bin/EoCApp.exe"), 100);
    touch(&root.join("DefEd/Data/Arena.pak"), 10);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("larian"), "DefEd/bin/EoCApp.exe")
    );
    let d = draft_manual(root, "DOS2", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%DOCUMENTS%/Larian Studios/Divinity Original Sin 2 Definitive Edition")
    );
    let l = d.def.larian.as_ref().unwrap();
    assert_eq!(
        (l.folder.as_str(), l.format),
        (USER_FOLDER, umm_gamedef::LarianFormat::Dos2)
    );
    assert!(d
        .notes
        .iter()
        .any(|n| n.key == "larian" && n.text.contains("each player profile")));
    let c = d.def.compile().unwrap();
    let map = |p: &str| {
        c.map_mod_file("m", &umm_gamedef::RelPath::parse(p).unwrap())
            .unwrap()
            .map(|t| t.to_string())
    };
    assert_eq!(map("Armory.pak").as_deref(), Some("@user/Mods/Armory.pak"));
    assert_eq!(map("DXGI.dll").as_deref(), Some("DefEd/bin/DXGI.dll"));
    assert_eq!(
        map("bin/OsiExtenderEoCApp.dll").as_deref(),
        Some("DefEd/bin/OsiExtenderEoCApp.dll")
    );
    assert_eq!(map("DefEd/Data/x.pak").as_deref(), Some("DefEd/Data/x.pak"));
    let text = to_toml(&d);
    assert!(text.contains("format = \"dos2\""), "{text}");
    let back = GameDef::parse_game(&text, Path::new("game.toml"), "dos2").unwrap();
    assert_eq!(back.larian, d.def.larian);
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    let targets = |rules: &[umm_gamedef::InstallRule]| {
        rules.iter().map(|r| r.target.clone()).collect::<Vec<_>>()
    };
    assert_eq!(targets(&again.install), targets(&d.def.install));
}

#[test]
fn baldurs_gate_3_mods_go_to_its_user_folder() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("bin/bg3.exe"), 100);
    touch(&root.join("bin/bg3_dx11.exe"), 100);
    touch(&root.join("Launcher/LariLauncher.exe"), 50);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("larian"), "bin/bg3.exe")
    );
    let d = draft_manual(root, "BG3", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%LOCALAPPDATA%/Larian Studios/Baldur's Gate 3")
    );
    assert_eq!(d.def.larian.as_ref().unwrap().folder, USER_FOLDER);
    assert!(d.notes.iter().any(|n| n.key == "larian"));
    let c = d.def.compile().unwrap();
    let map = |p: &str| {
        c.map_mod_file("m", &umm_gamedef::RelPath::parse(p).unwrap())
            .unwrap()
            .map(|t| t.to_string())
    };
    assert_eq!(map("Bags.pak").as_deref(), Some("@user/Mods/Bags.pak"));
    assert_eq!(map("DWrite.dll").as_deref(), Some("bin/DWrite.dll"));
    assert_eq!(
        map("bin/NativeMods/x.dll").as_deref(),
        Some("bin/NativeMods/x.dll")
    );
    assert_eq!(map("info.json"), None, "left out");
    let text = to_toml(&d);
    let back = GameDef::parse_game(&text, Path::new("game.toml"), "bg3").unwrap();
    assert_eq!(back.larian, d.def.larian);
    // Back to generic from another setup: the same rules.
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(again.install[0].target, "@user/Mods/");
}

#[test]
fn oxygen_not_included_mods_go_to_its_local_mods_folder() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join(oni::ONI_EXE), 100);
    touch(&root.join("UnityPlayer.dll"), 10);
    touch(&root.join("OxygenNotIncluded_Data/globalgamemanagers"), 10);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("oni"), oni::ONI_EXE)
    );
    let d = draft_manual(
        root,
        "Oxygen Not Included",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%DOCUMENTS%/Klei/OxygenNotIncluded")
    );
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.format, p.folder.as_str()),
        (
            "%DOCUMENTS%/Klei/OxygenNotIncluded/mods/mods.json",
            PluginListFormat::OniJson,
            "@user/mods/Local"
        )
    );
    assert!(d.notes.iter().any(|n| n.key == "oni"));
    assert!(!d.notes.iter().any(|n| n.key == "generic-unity"));
    assert!(d.def.protect.contains(&"UnityPlayer.dll".to_string()));
    let c = d.def.compile().unwrap();
    let mapped = c
        .map_mod_file(
            "tweaks",
            &umm_gamedef::RelPath::parse("Tweaks.dll").unwrap(),
        )
        .unwrap()
        .map(|t| t.to_string());
    assert_eq!(
        mapped.as_deref(),
        Some("@user/mods/Local/tweaks/Tweaks.dll")
    );
    let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "oni").unwrap();
    assert_eq!(back.plugins, d.def.plugins);
}

#[test]
fn kenshi_mods_get_a_folder_each_and_are_listed_in_its_mods_cfg() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("kenshi_x64.exe"), 100);
    touch(&root.join("data/gamedata.base"), 10);
    let found = inspect_folder(root);
    let d = draft_manual(
        root,
        "Kenshi",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.format, p.folder.as_str()),
        ("data/mods.cfg", PluginListFormat::KenshiCfg, "mods")
    );
    assert!(d
        .notes
        .iter()
        .any(|n| n.key == "plugins" && n.text.starts_with("Kenshi")));
    assert!(!d.notes.iter().any(|n| n.key == "unknown-engine"));
    assert_eq!(d.def.install[0].target, "mods/{mod}/");
    let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "kenshi").unwrap();
    assert_eq!(back.plugins, d.def.plugins);
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(again.install[0].target, "mods/{mod}/");
}

#[test]
fn bannerlord_modules_get_a_folder_each() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join(bannerlord::BANNERLORD_EXE), 100);
    touch(
        &root.join("bin/Win64_Shipping_Client/TaleWorlds.MountAndBlade.Launcher.exe"),
        50,
    );
    touch(&root.join("Modules/Native/SubModule.xml"), 10);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("bannerlord"), bannerlord::BANNERLORD_EXE)
    );
    let d = draft_manual(
        root,
        "Bannerlord",
        &found.exe,
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert_eq!(d.def.bannerlord.as_ref().unwrap().folder, "Modules");
    assert_eq!(d.def.install[0].target, "Modules/{mod}/");
    assert!(d.notes.iter().any(|n| n.key == "bannerlord"));
    let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "mb2").unwrap();
    assert_eq!(back.bannerlord, d.def.bannerlord);
    let again = back
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(again.install[0].target, "Modules/{mod}/");
}

#[test]
fn xcom_2_mods_go_to_war_of_the_chosen_when_it_is_there() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join(xcom2::XCOM2_EXE), 100);
    touch(&root.join("Binaries/Win64/Launcher/ModLauncherWPF.exe"), 50);
    let base = inspect_folder(root);
    assert_eq!(
        (base.game.as_ref().map(|g| g.key), base.exe.as_str()),
        (Some("xcom2"), xcom2::XCOM2_EXE)
    );
    let d = draft_manual(root, "XCOM 2", &base.exe, &base, LoaderSetup::Generic, None).unwrap();
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.format, p.folder.as_str()),
        (
            "%DOCUMENTS%/My Games/XCOM2/XComGame/Config/XComModOptions.ini",
            PluginListFormat::XcomIni,
            "XComGame/Mods"
        )
    );
    assert_eq!(d.def.install[0].target, "XComGame/Mods/{mod}/");

    touch(&root.join(xcom2::XCOM2_WOTC_EXE), 100);
    let wotc = inspect_folder(root);
    assert_eq!(wotc.exe, xcom2::XCOM2_WOTC_EXE);
    let d = draft_manual(root, "XCOM 2", &wotc.exe, &wotc, LoaderSetup::Generic, None).unwrap();
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.list.as_str(), p.folder.as_str()),
        (
            "%DOCUMENTS%/My Games/XCOM2 War of the Chosen/XComGame/Config/XComModOptions.ini",
            "XCom2-WarOfTheChosen/XComGame/Mods"
        )
    );
    assert_eq!(
        d.def.install[0].target,
        "XCom2-WarOfTheChosen/XComGame/Mods/{mod}/"
    );
    assert!(d
        .notes
        .iter()
        .any(|n| n.key == "xcom2" && n.text.contains("War of the Chosen")));
    let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "xcom2").unwrap();
    assert_eq!(back.plugins, d.def.plugins);
}

#[test]
fn crusader_kings_ii_lists_its_mods_in_its_settings() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    touch(&root.join("CK2game.exe"), 100);
    let found = inspect_folder(root);
    assert_eq!(
        (found.game.as_ref().map(|g| g.key), found.exe.as_str()),
        (Some("paradox"), "CK2game.exe")
    );
    let d = draft_manual(root, "CK2", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
    assert_eq!(
        d.def.folders.get(USER_FOLDER).map(String::as_str),
        Some("%DOCUMENTS%/Paradox Interactive/Crusader Kings II")
    );
    assert_eq!(d.def.paradox.as_ref().unwrap().list, ParadoxList::Settings);
    assert_eq!(d.def.launch.as_ref().unwrap().args, ["-skiplauncher"]);
    let note = d.notes.iter().find(|n| n.key == "paradox").unwrap();
    assert!(note.text.contains("settings.txt"), "{}", note.text);
    let text = to_toml(&d);
    assert!(text.contains("list = \"settings\""), "{text}");
    let back = GameDef::parse_game(&text, Path::new("game.toml"), "ck2").unwrap();
    assert_eq!(back.paradox, d.def.paradox);
}

#[test]
fn bethesda_games_get_their_plugin_list_and_data_folder_rules() {
    register();
    // Bethesda games get their plugin list, recognised by their exe.
    let exe = |e: &str| bethesda::list_game(&[e.to_string()]).unwrap();
    let steam = tempfile::tempdir().unwrap();
    let sse = exe("SkyrimSE.exe").plugins(steam.path());
    assert_eq!(
        (sse.list.as_str(), sse.format, sse.folder.as_str()),
        (
            "%LOCALAPPDATA%/Skyrim Special Edition/plugins.txt",
            PluginListFormat::Asterisk,
            "Data"
        )
    );
    // Any edition, regional ones included, has the exe; case does not matter.
    let exes = vec![
        "FalloutNVLauncher.exe".to_string(),
        "falloutnv.exe".to_string(),
    ];
    let nv = bethesda::list_game(&exes).unwrap();
    assert_eq!(
        (nv.list, nv.format),
        (
            bethesda::ListAt::LocalAppData("FalloutNV"),
            PluginListFormat::Plain
        )
    );
    let vr = vec!["Fallout4VR.exe".to_string(), "Fallout4.exe".to_string()];
    assert_eq!(
        bethesda::list_game(&vr).unwrap().list,
        bethesda::ListAt::LocalAppData("Fallout4VR")
    );
    assert!(
        bethesda::list_game(&["Game.exe".to_string(), "bin/SkyrimSE.exe".to_string()]).is_none()
    );
    let fo4 = exe("Fallout4.exe").ini();
    assert_eq!(fo4.len(), 3);
    assert!(
        !fo4[0].create && fo4[1].create,
        "Prefs is the game's own; Custom is overrides"
    );
    assert_eq!(
        fo4[1].file,
        "%DOCUMENTS%/My Games/Fallout4/Fallout4Custom.ini"
    );
    let sf = exe("Starfield.exe").ini();
    assert_eq!(
        (sf[0].file.as_str(), sf[1].value.as_str()),
        ("%DOCUMENTS%/My Games/Starfield/StarfieldCustom.ini", "")
    );
    assert!(exe("SkyrimSE.exe").ini().is_empty());

    // Other stores' editions keep their list elsewhere, told apart by a file of theirs.
    let gog = tempfile::tempdir().unwrap();
    touch(&gog.path().join("Galaxy64.dll"), 1);
    assert_eq!(
        exe("SkyrimSE.exe").plugins(gog.path()).list,
        "%LOCALAPPDATA%/Skyrim Special Edition GOG/plugins.txt"
    );
    let epic = tempfile::tempdir().unwrap();
    touch(&epic.path().join("EOSSDK-Win32-Shipping.dll"), 1);
    assert_eq!(
        exe("FalloutNV.exe").plugins(epic.path()).list,
        "%LOCALAPPDATA%/FalloutNV_Epic/plugins.txt"
    );
    // Oblivion's list moves to its own folder when its ini says not to use My Games.
    let c = exe("Oblivion.exe")
        .plugins(gog.path())
        .in_game_folder_when
        .unwrap();
    assert_eq!(
        (c.ini.as_str(), c.key.as_str(), c.value.as_str()),
        ("Oblivion.ini", "bUseMyGamesDirectory", "0")
    );
    assert!(exe("SkyrimSE.exe")
        .plugins(gog.path())
        .in_game_folder_when
        .is_none());
    // GOG's Fallout 4 uses the same folder as Steam's.
    assert_eq!(
        exe("Fallout4.exe").plugins(gog.path()).list,
        "%LOCALAPPDATA%/Fallout4/plugins.txt"
    );

    // A game added by hand (a GOG install, say) is recognised the same way.
    let manual = tempfile::tempdir().unwrap();
    touch(&manual.path().join("FalloutNV.exe"), 100);
    touch(&manual.path().join("FalloutNVLauncher.exe"), 10);
    touch(&manual.path().join("Data/FalloutNV.esm"), 1);
    let found = inspect_folder(manual.path());
    let d = draft_manual(
        manual.path(),
        "New Vegas",
        "FalloutNV.exe",
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    assert_eq!(
        d.def.plugins.as_ref().unwrap().list,
        "%LOCALAPPDATA%/FalloutNV/plugins.txt"
    );
    // Its launcher, which Steam starts first, is followed too.
    assert_eq!(
        d.def.launch.as_ref().unwrap().process_names,
        ["FalloutNV.exe", "FalloutNVLauncher.exe"]
    );
    // Recognised by its data folder, which plugins, archives and assets go to.
    assert_eq!(found.engine.label(), bethesda::LABEL);
    assert_eq!(
        bethesda::top_data_folder(manual.path()).as_deref(),
        Some("Data")
    );
    let c = d.def.compile().unwrap();
    let map = |p: &str| {
        c.map_mod_file("m", &umm_gamedef::RelPath::parse(p).unwrap())
            .unwrap()
            .unwrap()
            .to_string()
    };
    assert_eq!(map("Mod.esp"), "Data/Mod.esp");
    assert_eq!(map("Mod - Main.bsa"), "Data/Mod - Main.bsa");
    assert_eq!(map("meshes/a/b.nif"), "Data/meshes/a/b.nif");
    assert_eq!(map("nvse/plugins/x.dll"), "Data/nvse/plugins/x.dll");
    assert_eq!(map("src/nvse/x.cpp"), "src/nvse/x.cpp");
    assert_eq!(map("nvse/x.cpp"), "nvse/x.cpp");
    assert_eq!(map("Data/textures/t.dds"), "Data/textures/t.dds");
    assert_eq!(map("nvse_loader.exe"), "nvse_loader.exe");
    assert_eq!(map("sub/Mod.esp"), "sub/Mod.esp");
    assert!(d.notes.iter().any(|n| n.key == "bethesda"));
    // Set to generic again from another setup, the rules come back.
    let other = d
        .def
        .with_setup(LoaderSetup::Bepinex, None)
        .unwrap()
        .with_setup(LoaderSetup::Generic, None)
        .unwrap();
    assert_eq!(other.install.len(), d.def.install.len());
    // Morrowind's is `Data Files`.
    let mw_dir = tempfile::tempdir().unwrap();
    touch(&mw_dir.path().join("Morrowind.exe"), 100);
    touch(&mw_dir.path().join("Data Files/Morrowind.esm"), 1);
    assert_eq!(
        bethesda::top_data_folder(mw_dir.path()).as_deref(),
        Some("Data Files")
    );

    // Other games keeping a list: in the game folder, or with a folder per mod.
    let or = exe("OblivionRemastered.exe").plugins(manual.path());
    assert_eq!(
        (or.list.as_str(), or.folder.as_str(), or.format),
        (
            "OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt",
            "OblivionRemastered/Content/Dev/ObvData/Data",
            PluginListFormat::Plain
        )
    );
    assert_eq!(
        or.list_path(Path::new("C:/g"), &umm_gamedef::PathVars::default()),
        Some(PathBuf::from(
            "C:/g/OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt"
        ))
    );
    let mw_found = inspect_folder(mw_dir.path());
    assert_eq!(mw_found.game.as_ref().map(|g| g.key), Some("morrowind"));
    let mw = draft_manual(
        mw_dir.path(),
        "Morrowind",
        "Morrowind.exe",
        &mw_found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    let mw = mw.def.plugins.unwrap();
    assert_eq!(
        (mw.list.as_str(), mw.folder.as_str(), mw.format),
        (
            "Morrowind.ini",
            "Data Files",
            PluginListFormat::MorrowindIni
        )
    );
    let rim = tempfile::tempdir().unwrap();
    touch(&rim.path().join("RimWorldWin64.exe"), 100);
    touch(&rim.path().join("RimWorldWin64_Data/x"), 1);
    touch(&rim.path().join("UnityPlayer.dll"), 1);
    let found = inspect_folder(rim.path());
    let d = draft_manual(
        rim.path(),
        "RimWorld",
        "RimWorldWin64.exe",
        &found,
        LoaderSetup::Generic,
        None,
    )
    .unwrap();
    let p = d.def.plugins.as_ref().unwrap();
    assert_eq!(
        (p.format, p.folder.as_str()),
        (PluginListFormat::RimworldXml, "Mods")
    );
    assert!(p.list.ends_with("/Config/ModsConfig.xml"));
    assert_eq!(
        d.def.install.last().unwrap().target,
        "Mods/{mod}/",
        "a folder per mod"
    );
    let text = to_toml(&d);
    GameDef::parse_game(&text, Path::new("game.toml"), &d.def.id).unwrap();
}

/// Every game UMM knows, from a folder with the least that makes it that game: recognised as
/// it, drafted, written and read back, and its definition is still that game's.
#[test]
fn every_game_drafts_into_a_definition_that_is_still_that_game() {
    register();
    let tmp = tempfile::tempdir().unwrap();
    let game = |name: &str, files: &[&str]| {
        let root = tmp.path().join(name);
        for f in files {
            touch(&root.join(f), 10);
        }
        root
    };
    let folders = [
        (
            "oni",
            game(
                "oni",
                &[
                    "OxygenNotIncluded.exe",
                    "UnityPlayer.dll",
                    "OxygenNotIncluded_Data/x",
                ],
            ),
        ),
        ("paradox", game("ck3", &["binaries/ck3.exe"])),
        ("larian", game("bg3", &["bin/bg3.exe"])),
        (
            "bannerlord",
            game(
                "mb2",
                &[
                    "bin/Win64_Shipping_Client/Bannerlord.exe",
                    "Modules/Native/SubModule.xml",
                ],
            ),
        ),
        ("xcom2", game("xcom2", &["Binaries/Win64/XCom2.exe"])),
        (
            "factorio",
            game("factorio", &["bin/x64/factorio.exe", "data/base/info.json"]),
        ),
        (
            "tmodloader",
            game("tml", &["start-tModLoader.bat", "tModLoader.dll"]),
        ),
        (
            "morrowind",
            game("mw", &["Morrowind.exe", "Data Files/Morrowind.esm"]),
        ),
        (
            "bethesda",
            game("sse", &["SkyrimSE.exe", "Data/Skyrim.esm"]),
        ),
        (
            "rimworld",
            game(
                "rim",
                &[
                    "RimWorldWin64.exe",
                    "UnityPlayer.dll",
                    "RimWorldWin64_Data/x",
                ],
            ),
        ),
        ("kenshi", game("kenshi", &["kenshi_x64.exe"])),
    ];
    for (key, root) in &folders {
        let found = inspect_folder(root);
        assert_eq!(found.game.as_ref().map(|g| g.key), Some(*key), "{key}");
        let d = draft_manual(root, key, &found.exe, &found, LoaderSetup::Generic, None)
            .unwrap_or_else(|e| panic!("{key}: {e}"));
        let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), key).unwrap();
        assert_eq!(
            super::integration(&back).map(|g| g.key()),
            Some(*key),
            "{key}"
        );
        assert_eq!(back.own_mods(), found.own_mods(), "{key}");
    }
    // Every game but Terraria (which no definition is) has its folder here.
    let tested: Vec<&str> = folders.iter().map(|(k, _)| *k).collect();
    for g in super::GAMES {
        assert!(
            g.key() == "terraria" || tested.contains(&g.key()),
            "{} has no folder in this test",
            g.key()
        );
    }
}
