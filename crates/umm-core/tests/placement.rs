//! Placing mods whose archives are not laid out like the game folder: the suggestion at
//! import, deploying with it, changing it, and keeping a chosen one across an update.

use std::fs;
use std::path::Path;
use umm_core::{CoreError, Faults, ImportOptions, Placement, Session, DEFAULT_PROFILE};

const GAME: &str = "rocks";

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn import(s: &Session, src: &Path, id: &str) -> umm_core::ModMeta {
    s.import(
        GAME,
        src,
        ImportOptions {
            id: Some(id.into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap()
}

fn targets(s: &Session) -> Vec<String> {
    s.plan(GAME, DEFAULT_PROFILE)
        .unwrap()
        .files
        .iter()
        .map(|f| f.target.to_string())
        .collect()
}

#[test]
fn mods_are_placed_where_the_game_keeps_such_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Rocks.exe"), "exe");
    write(&root.join("Rocks/Data/textures/rock.dds"), "rock");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let def = s.data().game(GAME).def_path();
    write(
        &def,
        "schema = 1\nname = \"Rocks\"\n[detect]\nmarker = \"Rocks.exe\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path(GAME, &root).unwrap();

    // Wrapped, laid out from the data folder, with a readme and a link.
    let src = tmp.path().join("src/Better Rocks");
    write(&src.join("Better Rocks v2/textures/rock.dds"), "better");
    write(&src.join("Better Rocks v2/textures/moss.dds"), "moss");
    write(&src.join("Better Rocks v2/README.txt"), "read me");
    write(
        &src.join("Better Rocks v2/Nexus page.url"),
        "[InternetShortcut]",
    );
    let m = import(&s, &src, "better-rocks");
    let p = m.placement.clone().unwrap();
    assert_eq!(
        (p.root.as_str(), p.target.as_str()),
        ("Better Rocks v2", "Rocks/Data")
    );
    assert!(!p.guessed && !p.chosen);
    assert_eq!(
        targets(&s),
        [
            "Rocks/Data/textures/moss.dds",
            "Rocks/Data/textures/rock.dds"
        ]
    );
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert_eq!(
        plan.skipped.len(),
        2,
        "readme and link left out: {:?}",
        plan.skipped
    );

    // The dialog's view: what each file does, and the choices.
    let v = s.placement_view(GAME, "better-rocks", None).unwrap();
    assert_eq!(v.suggested, p);
    assert!(v.folders.contains(&"Better Rocks v2/textures".to_string()));
    assert!(v.game_folders.contains(&"Rocks/Data".to_string()));
    let readme = v
        .files
        .iter()
        .find(|f| f.file.ends_with("README.txt"))
        .unwrap();
    assert!(readme.skipped && readme.target.is_none());

    // The user wants the readme too, and everything in the game folder itself.
    let mine = Placement {
        root: "Better Rocks v2".into(),
        target: String::new(),
        skip: vec!["Better Rocks v2/Nexus page.url".into()],
        ..Default::default()
    };
    let preview = s
        .placement_view(GAME, "better-rocks", Some(mine.clone()))
        .unwrap();
    assert!(preview
        .files
        .iter()
        .any(|f| f.target.as_deref() == Some("README.txt")));
    let m = s.set_placement(GAME, "better-rocks", Some(mine)).unwrap();
    assert!(m.placement.as_ref().unwrap().chosen);
    assert_eq!(
        targets(&s),
        ["README.txt", "textures/moss.dds", "textures/rock.dds"]
    );

    // Refused while deployed.
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(matches!(
        s.set_placement(GAME, "better-rocks", None),
        Err(CoreError::Deployed(_))
    ));
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // An update keeps the user's choice when it still fits.
    let v3 = tmp.path().join("src/v3");
    write(&v3.join("Better Rocks v2/textures/rock.dds"), "best");
    write(&v3.join("Better Rocks v2/README.txt"), "read me");
    s.import(
        GAME,
        &v3,
        ImportOptions {
            id: Some("better-rocks".into()),
            unwrap: true,
            replace: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    assert_eq!(targets(&s), ["README.txt", "textures/rock.dds"]);

    // Back to UMM's suggestion.
    s.set_placement(GAME, "better-rocks", None).unwrap();
    assert_eq!(targets(&s), ["Rocks/Data/textures/rock.dds"]);

    // Nothing to go by: a guess, flagged for the user to check.
    let odd = tmp.path().join("src/odd");
    write(&odd.join("Odd Mod/payload/blob.bin"), "?");
    let m = import(&s, &odd, "odd");
    let p = m.placement.unwrap();
    assert!(p.guessed);
    assert_eq!(p.root, "Odd Mod/payload");
}
