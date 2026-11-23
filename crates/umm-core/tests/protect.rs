//! Protected game paths: a game file matching them is never replaced, but a mod may add a
//! new file that only matches the pattern (a script extender's loader `.exe`).

use std::fs;
use std::path::Path;
use umm_core::{ImportOptions, Session, DEFAULT_PROFILE};

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

#[test]
fn a_new_exe_deploys_but_the_games_own_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Fallout3.exe"), b"game");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("fo3").def_path(),
        b"schema = 1\nname = \"Fallout 3\"\n[detect]\nmarker = \"Fallout3.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"\"\n[protect]\npaths = [\"*.exe\"]\n",
    );
    s.set_install_path("fo3", &root).unwrap();
    let src = tmp.path().join("fose");
    write(&src.join("fose_loader.exe"), b"loader");
    write(&src.join("Fallout3.exe"), b"patched");
    s.import(
        "fo3",
        &src,
        ImportOptions {
            id: Some("fose".into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    let plan = s.plan("fo3", DEFAULT_PROFILE).unwrap();
    let targets: Vec<String> = plan.files.iter().map(|f| f.target.to_string()).collect();
    assert_eq!(targets, ["fose_loader.exe"]);
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.file == "Fallout3.exe" && k.reason.contains("protected")));
}
