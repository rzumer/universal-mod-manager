//! A game's plugin list (Bethesda's `plugins.txt`): written while mods are deployed, the
//! user's own file put back afterwards, from any crash.

use std::fs;
use std::path::{Path, PathBuf};
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "skyrim";
const ORIGINAL: &str = "# This file is used by Skyrim to keep track of your downloaded content.\r\n*Existing.esp\r\nOff.esp\r\n*Overwritten.esp\r\n";

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

struct Fx {
    _tmp: tempfile::TempDir,
    s: Session,
    list: PathBuf,
}

fn setup(original: Option<&str>) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("SkyrimSE.exe"), "exe");
    write(&root.join("Data/Skyrim.esm"), "esm");
    let list = tmp
        .path()
        .join("appdata/Skyrim Special Edition/plugins.txt");
    if let Some(o) = original {
        write(&list, o);
    }
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        &format!(
            "schema = 1\nname = \"Skyrim\"\n[detect]\nmarker = \"SkyrimSE.exe\"\n\
             [mod_list]\nlist = '{}'\nformat = \"asterisk\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
            list.display()
        ),
    );
    s.set_install_path(GAME, &root).unwrap();
    let import = |id: &str, files: &[&str]| {
        let src = tmp.path().join("src").join(id);
        for f in files {
            write(&src.join(f), f);
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
    import("a", &["Data/A.esp", "Data/A.esm", "Data/meshes/a.nif"]);
    import("b", &["Data/B.esp", "Data/Overwritten.esp"]);
    import("c", &["Data/C.esp"]);
    // c is turned off.
    let mut p = umm_store::load_profile(&s.data().game(GAME), DEFAULT_PROFILE).unwrap();
    p.mods.iter_mut().find(|m| m.id == "c").unwrap().enabled = false;
    umm_store::save_profile(&s.data().game(GAME), &p).unwrap();
    Fx { _tmp: tmp, s, list }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

#[test]
fn the_list_enables_the_mods_plugins_while_deployed() {
    let fx = setup(Some(ORIGINAL));
    // Shown before deploying: the user's entries, then the mods' in load order.
    let preview =
        fx.s.plugin_list_preview(GAME, DEFAULT_PROFILE)
            .unwrap()
            .unwrap();
    let names: Vec<&str> = preview.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Existing.esp",
            "Off.esp",
            "A.esm",
            "A.esp",
            "B.esp",
            "Overwritten.esp"
        ]
    );
    assert_eq!(preview.entries[4].owner.as_deref(), Some("b"));

    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        read(&fx.list),
        "# This file is used by Skyrim to keep track of your downloaded content.\r\n\
         *Existing.esp\r\nOff.esp\r\n*A.esm\r\n*A.esp\r\n*B.esp\r\n*Overwritten.esp\r\n"
    );
    fx.s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(
        read(&fx.list),
        ORIGINAL,
        "the user's own file is back, byte for byte"
    );
}

#[test]
fn oblivion_keeps_its_list_in_its_folder_when_told_to() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Oblivion.exe"), "exe");
    write(&root.join("Data/Oblivion.esm"), "esm");
    let appdata = tmp.path().join("appdata/Oblivion/plugins.txt");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("oblivion").def_path(),
        &format!(
            "schema = 1\nname = \"Oblivion\"\n[detect]\nmarker = \"Oblivion.exe\"\n\
             [mod_list]\nlist = '{}'\nformat = \"plain\"\n\
             [mod_list.in_game_folder_when]\nini = \"Oblivion.ini\"\nsection = \"General\"\nkey = \"bUseMyGamesDirectory\"\nvalue = \"0\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
            appdata.display()
        ),
    );
    s.set_install_path("oblivion", &root).unwrap();
    let src = tmp.path().join("src/o");
    write(&src.join("Data/Mine.esp"), "esp");
    s.import(
        "oblivion",
        &src,
        ImportOptions {
            id: Some("o".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    let where_ = || {
        s.plugin_list_preview("oblivion", DEFAULT_PROFILE)
            .unwrap()
            .unwrap()
            .path
    };

    // No Oblivion.ini, or My Games in use: the list is in local app data.
    assert_eq!(where_(), appdata);
    write(
        &root.join("Oblivion.ini"),
        "[General]\r\nbUseMyGamesDirectory=1\r\n",
    );
    assert_eq!(where_(), appdata);

    // Set not to use My Games: in the game folder, checked again at each deploy.
    write(
        &root.join("Oblivion.ini"),
        "[General]\r\nbUseMyGamesDirectory=0\r\n",
    );
    let in_game = root.join("plugins.txt");
    assert_eq!(where_(), in_game);
    s.deploy("oblivion", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(read(&in_game), "Mine.esp\r\n");
    assert!(!appdata.exists());
    s.revert("oblivion", false, &mut Faults::none()).unwrap();
    assert!(!in_game.exists());
}

#[test]
fn a_list_that_did_not_exist_is_removed_again() {
    let fx = setup(None);
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        read(&fx.list),
        "*A.esm\r\n*A.esp\r\n*B.esp\r\n*Overwritten.esp\r\n"
    );
    fx.s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!fx.list.exists());
}

#[test]
fn a_list_changed_while_playing_is_set_aside() {
    let fx = setup(Some(ORIGINAL));
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    write(&fx.list, "*Something.esp\r\n");
    let r =
        fx.s.revert(GAME, false, &mut Faults::none())
            .unwrap()
            .unwrap();
    assert_eq!(read(&fx.list), ORIGINAL);
    let quarantined: Vec<std::path::PathBuf> = r.quarantined.iter().map(Into::into).collect();
    assert_eq!(quarantined, [fx.list.clone()]);
}

#[test]
fn crashes_at_every_step_leave_the_users_list() {
    for original in [Some(ORIGINAL), None] {
        let fx = setup(original);
        let mut count = Faults::none();
        fx.s.deploy(GAME, DEFAULT_PROFILE, &mut count).unwrap();
        let mut rcount = Faults::none();
        fx.s.revert(GAME, false, &mut rcount).unwrap();
        for n in 0..count.steps() {
            assert!(fx
                .s
                .deploy(GAME, DEFAULT_PROFILE, &mut Faults::crash_at(n))
                .is_err());
            for (_, r) in fx.s.recover_all(false) {
                r.unwrap();
            }
            assert_eq!(
                fs::read_to_string(&fx.list).ok().as_deref(),
                original,
                "deploy step {n}"
            );
        }
        for n in 0..rcount.steps() {
            fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
                .unwrap();
            assert!(fx.s.revert(GAME, false, &mut Faults::crash_at(n)).is_err());
            fx.s.revert(GAME, false, &mut Faults::none()).unwrap();
            assert_eq!(
                fs::read_to_string(&fx.list).ok().as_deref(),
                original,
                "revert step {n}"
            );
        }
    }
}

/// A game with a list, set up in a temp folder with mods imported from `mods` (id, files).
struct Game {
    tmp: tempfile::TempDir,
    s: Session,
    root: PathBuf,
}

fn game(id: &str, files: &[(&str, &[u8])], plugins: &str, install: &str) -> Game {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Game.exe"), "exe");
    for (f, b) in files {
        fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
        fs::write(root.join(f), b).unwrap();
    }
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(id).def_path(),
        &format!(
            "schema = 1\nname = \"G\"\n[detect]\nmarker = \"Game.exe\"\n[mod_list]\n{plugins}\n\
             [[install]]\nmatch = \"**\"\ntarget = \"{install}\"\n"
        ),
    );
    s.set_install_path(id, &root).unwrap();
    Game { tmp, s, root }
}

impl Game {
    fn import(&self, game: &str, id: &str, files: &[(&str, &str)]) {
        let src = self.tmp.path().join("src").join(id);
        for (f, text) in files {
            write(&src.join(f), text);
        }
        self.s
            .import(
                game,
                &src,
                ImportOptions {
                    id: Some(id.into()),
                    unwrap: true,
                    ..Default::default()
                },
                DEFAULT_PROFILE,
            )
            .unwrap();
    }

    /// Deploys and reverts, crashing at every step of each: the list is the user's again.
    fn survives_crashes(&self, game: &str, list: &Path) {
        let original = fs::read(list).ok();
        let mut count = Faults::none();
        self.s.deploy(game, DEFAULT_PROFILE, &mut count).unwrap();
        let mut rcount = Faults::none();
        self.s.revert(game, false, &mut rcount).unwrap();
        for n in 0..count.steps() {
            assert!(self
                .s
                .deploy(game, DEFAULT_PROFILE, &mut Faults::crash_at(n))
                .is_err());
            for (_, r) in self.s.recover_all(false) {
                r.unwrap();
            }
            assert_eq!(fs::read(list).ok(), original, "deploy step {n}");
        }
        for n in 0..rcount.steps() {
            self.s
                .deploy(game, DEFAULT_PROFILE, &mut Faults::none())
                .unwrap();
            assert!(self
                .s
                .revert(game, false, &mut Faults::crash_at(n))
                .is_err());
            self.s.revert(game, false, &mut Faults::none()).unwrap();
            assert_eq!(fs::read(list).ok(), original, "revert step {n}");
        }
    }
}

#[test]
fn morrowind_enables_plugins_in_its_ini() {
    // Its ini is in the system code page: the rest of it comes back byte for byte.
    let ini: &[u8] = b"[General]\r\nScreenshot Enable=0\r\nName=Caf\xe9\r\n\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n";
    let g = game(
        "morrowind",
        &[("Morrowind.ini", ini), ("Data Files/Morrowind.esm", b"esm")],
        "list = \"Morrowind.ini\"\nformat = \"morrowind_ini\"\nfolder = \"Data Files\"",
        "",
    );
    g.import(
        "morrowind",
        "m",
        &[
            ("Data Files/Mine.esp", "esp"),
            ("Data Files/Mine.esm", "esm"),
            // A mod's own copy of the game's ini is left out: UMM writes the list.
            ("Morrowind.ini", "[Game Files]\r\nGameFile0=Other.esp\r\n"),
        ],
    );
    let list = g.root.join("Morrowind.ini");
    g.s.deploy("morrowind", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(&list).unwrap(),
        b"[General]\r\nScreenshot Enable=0\r\nName=Caf\xe9\r\n\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=Mine.esm\r\nGameFile3=Mine.esp\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n"
    );
    assert!(g.root.join("Data Files/Mine.esp").is_file());
    g.s.revert("morrowind", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(&list).unwrap(), ini);
    g.survives_crashes("morrowind", &list);
}

#[test]
fn oblivion_remastered_keeps_its_list_with_its_plugins() {
    let data = "OblivionRemastered/Content/Dev/ObvData/Data";
    let list_rel = format!("{data}/Plugins.txt");
    let original = b"Oblivion.esm\r\nDLCBattlehornCastle.esp\r\nKnights.esp\r\n";
    let g = game(
        "obr",
        &[(list_rel.as_str(), original)],
        &format!("list = \"{list_rel}\"\nformat = \"plain\"\nfolder = \"{data}\""),
        "",
    );
    g.import("obr", "m", &[(&format!("{data}/Mine.esp"), "esp")]);
    let list = g.root.join(&list_rel);
    g.s.deploy("obr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        read(&list),
        "Oblivion.esm\r\nDLCBattlehornCastle.esp\r\nKnights.esp\r\nMine.esp\r\n"
    );
    g.s.revert("obr", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(&list).unwrap(), original);
    g.survives_crashes("obr", &list);
}

#[test]
fn rimworld_activates_mods_by_package_id() {
    let tmp_list = tempfile::tempdir().unwrap();
    let list = tmp_list.path().join("Config/ModsConfig.xml");
    let original = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.5.4104 rev435</version>\n  <activeMods>\n    <li>ludeon.rimworld</li>\n  </activeMods>\n  <knownExpansions />\n</ModsConfigData>\n";
    write(&list, original);
    let g = game(
        "rimworld",
        &[],
        &format!(
            "list = '{}'\nformat = \"rimworld_xml\"\nfolder = \"Mods\"",
            list.display()
        ),
        "Mods/{mod}/",
    );
    let about = |id: &str| {
        format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModMetaData>\n  <name>X</name>\n  <packageId>{id}</packageId>\n</ModMetaData>\n")
    };
    g.import(
        "rimworld",
        "harmony",
        &[("Harmony/About/About.xml", &about("brrainz.harmony"))],
    );
    g.import(
        "rimworld",
        "mine",
        &[
            ("About/About.xml", &about("Me.Mine")),
            ("Defs/x.xml", "<Defs />"),
        ],
    );
    // No package id: nothing to list it by.
    g.import("rimworld", "old", &[("About/About.xml", "<ModMetaData />")]);
    let preview =
        g.s.plugin_list_preview("rimworld", DEFAULT_PROFILE)
            .unwrap()
            .unwrap();
    assert_eq!(
        preview
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.owner.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("ludeon.rimworld", None),
            ("brrainz.harmony", Some("harmony")),
            ("me.mine", Some("mine"))
        ]
    );
    g.s.deploy("rimworld", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(g.root.join("Mods/mine/Defs/x.xml").is_file());
    assert_eq!(
        read(&list),
        original.replace(
            "    <li>ludeon.rimworld</li>\n",
            "    <li>ludeon.rimworld</li>\n    <li>brrainz.harmony</li>\n    <li>me.mine</li>\n"
        )
    );
    g.s.revert("rimworld", false, &mut Faults::none()).unwrap();
    assert_eq!(read(&list), original);
    g.survives_crashes("rimworld", &list);
}

#[test]
fn the_game_rewriting_its_list_is_not_set_aside() {
    // Fallout 3 rewrites the list on every start: its header, the mods' entries kept, and
    // every master in its data folder (one of the mods' a second time).
    for original in [Some(ORIGINAL), None] {
        let fx = setup(original);
        fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
            .unwrap();
        write(
            &fx.list,
            "# This file is used by the game to keep track of your downloaded content.\r\n\
             A.esm\r\nSkyrim.esm\r\nA.esm\r\n*B.esp\r\n",
        );
        let r =
            fx.s.revert(GAME, false, &mut Faults::none())
                .unwrap()
                .unwrap();
        assert!(r.quarantined.is_empty(), "{:?}", r.quarantined);
        match original {
            Some(o) => assert_eq!(read(&fx.list), o),
            None => assert!(!fx.list.exists()),
        }
    }
    // A plugin the user enabled by hand while playing still sets the list aside.
    let fx = setup(Some(ORIGINAL));
    fx.s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    write(&fx.list, "*Skyrim.esm\r\n*Unknown.esp\r\n");
    let r =
        fx.s.revert(GAME, false, &mut Faults::none())
            .unwrap()
            .unwrap();
    assert_eq!(r.quarantined.len(), 1);
}
