//! Patchers: a mod whose program patches the game's own files (the Fallout Anniversary
//! Patcher). UMM runs it on a copy, once approved, and swaps the patched files in only while
//! mods are deployed.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, PatchState, Session, DEFAULT_PROFILE};

const GAME: &str = "fo3";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

#[test]
fn a_patcher_runs_on_a_copy_and_its_result_is_swapped_in_while_deployed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Game.exe"), b"game");
    write(&root.join("Data/Game.esm"), b"esm");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"Fallout\"\nprotect = [\"*.exe\"]\n[detect]\nmarker = \"Game.exe\"\n\
          [launch]\nexe = \"Game.exe\"\n\
          [mod_list]\nlist = \"%LOCALAPPDATA%/Fallout3/plugins.txt\"\nfolder = \"Data\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path(GAME, &root).unwrap();
    let src = tmp.path().join("patcher");
    fs::create_dir_all(&src).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_umm-fake-game"), src.join("Patcher.exe")).unwrap();
    write(&src.join("patch.txt"), b"go");
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some("patcher".into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    // Not run yet: nothing of it deploys, and the plan says why.
    let st = s.patcher_status(GAME, "patcher").unwrap().unwrap();
    assert_eq!(
        (st.exe.as_str(), st.state),
        ("Patcher.exe", PatchState::NotRun)
    );
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert!(plan.files.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.reason.contains("is a patcher")));

    // Run: on a copy, so the game's own exe is untouched.
    let st = s.run_patcher(GAME, "patcher").unwrap();
    assert_eq!(st.state, PatchState::Current);
    assert_eq!(st.files[0].target, "Game.exe");
    assert!(st.output.contains("Patched Game.exe"), "{}", st.output);
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game");

    // Deployed: the patched exe replaces the game's (protected) one, and comes back out.
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game+patched");
    assert!(
        !root.join("Patcher.exe").exists(),
        "the patcher itself never deploys"
    );
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game");

    // What the game writes beside its exe while playing (logs) changes nothing.
    write(&root.join("fose.log"), b"log");
    let st = s.patcher_status(GAME, "patcher").unwrap().unwrap();
    assert_eq!(st.state, PatchState::Current);

    // A game update: out of date, and run again at the next deploy.
    write(&root.join("Game.exe"), b"game v2");
    let st = s.patcher_status(GAME, "patcher").unwrap().unwrap();
    assert_eq!(st.state, PatchState::Outdated);
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game v2+patched");
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game v2");

    // A patcher that changes nothing failed, with what it printed.
    let m = s.data().game(GAME).mod_files_dir("patcher");
    fs::write(m.join("patch.txt"), "no").unwrap();
    let err = s.run_patcher(GAME, "patcher").unwrap_err().to_string();
    assert!(err.contains("changed none of the game's files"), "{err}");
    let st = s.patcher_status(GAME, "patcher").unwrap().unwrap();
    assert_eq!(st.state, PatchState::Failed);
    assert!(st.output.contains("Invalid executable"), "{}", st.output);

    // One that patches a master in the data folder.
    fs::write(m.join("patch.txt"), "esm").unwrap();
    let st = s.run_patcher(GAME, "patcher").unwrap();
    assert_eq!(st.files[0].target, "Data/Game.esm");
    assert_eq!(fs::read(root.join("Data/Game.esm")).unwrap(), b"esm");
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("Data/Game.esm")).unwrap(),
        b"esm+patched"
    );
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(root.join("Data/Game.esm")).unwrap(), b"esm");

    // One that finds the real game folder by itself: its patch is kept, the original put back.
    fs::write(m.join("patch.txt"), format!("real:{}", root.display())).unwrap();
    let st = s.run_patcher(GAME, "patcher").unwrap();
    assert_eq!(st.files[0].target, "Game.exe");
    assert!(
        st.output.contains("in the game folder itself"),
        "{}",
        st.output
    );
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game v2");
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read(root.join("Game.exe")).unwrap(), b"game v2+patched");
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // One that also adds a file to the game folder, and changes one UMM had no copy of: the
    // new file is moved into its results, the other reported.
    write(&root.join("Data/meshes/rock.nif"), b"rock");
    fs::write(m.join("patch.txt"), format!("add:{}", root.display())).unwrap();
    let st = s.run_patcher(GAME, "patcher").unwrap();
    let mut targets: Vec<&str> = st.files.iter().map(|f| f.target.as_str()).collect();
    targets.sort();
    assert_eq!(targets, ["Data/New.esp", "Game.exe"]);
    assert!(!root.join("Data/New.esp").exists());
    assert!(
        st.output.contains("changed Data/meshes/rock.nif"),
        "{}",
        st.output
    );
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(fs::read(root.join("Data/New.esp")).unwrap(), b"new");
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!root.join("Data/New.esp").exists());
}
