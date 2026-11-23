//! WeiDU mods: a profile's installers run in order on a copy of the game, the components
//! chosen the first time are remembered, and what they changed is deployed only while mods
//! are.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, WeiduState, DEFAULT_PROFILE, WEIDU_SCRATCH};

const GAME: &str = "bgee";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn weidu_mod(s: &Session, tmp: &Path, m: &str, answers: &str) {
    weidu_mod_with(s, tmp, m, answers, true);
}

/// A WeiDU mod as archives have it: its installer beside its folder (the installer left out
/// with `!setup`), wrapped in a folder of its own.
fn weidu_mod_with(s: &Session, tmp: &Path, m: &str, answers: &str, setup: bool) {
    let src = tmp.join("src").join(m);
    fs::create_dir_all(&src).unwrap();
    if setup {
        fs::copy(
            env!("CARGO_BIN_EXE_umm-fake-game"),
            src.join(format!("setup-{m}.exe")),
        )
        .unwrap();
    }
    write(&src.join(m).join(format!("{m}.tp2")), b"BACKUP ~x~");
    write(&src.join(m).join("answers.txt"), answers.as_bytes());
    s.import(
        GAME,
        &src,
        ImportOptions {
            id: Some(m.into()),
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
}

#[test]
fn a_profiles_weidu_mods_install_on_a_copy_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("Baldur.exe"), b"exe");
    write(&root.join("chitin.key"), b"key");
    write(&root.join("lang/en_US/dialog.tlk"), b"tlk");
    write(&root.join("data/a.bif"), b"bif");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"BG\"\n[detect]\nmarker = \"Baldur.exe\"\n\
          [launch]\nexe = \"Baldur.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"\"\n[protect]\npaths = [\"*.exe\"]\n",
    );
    s.set_install_path(GAME, &root).unwrap();
    weidu_mod(&s, tmp.path(), "alpha", "0 2");
    weidu_mod(&s, tmp.path(), "beta", "1");

    // Nothing chosen yet: nothing of them deploys (the game still can), and there is
    // nothing to install.
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert_eq!(st.state, WeiduState::NeedsChoices);
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert!(plan.files.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.reason.contains("not chosen yet")));
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(!root.join("override").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(s
        .run_weidu(GAME, DEFAULT_PROFILE)
        .unwrap_err()
        .to_string()
        .contains("components first"));

    // One mod chosen: installed on its own, the other left out until its turn.
    let comps = |ns: &[u32]| {
        ns.iter()
            .map(|n| (*n, format!("Component {n}")))
            .collect::<Vec<_>>()
    };
    let st = s
        .set_weidu_choice(GAME, DEFAULT_PROFILE, "alpha", 0, comps(&[0, 2]))
        .unwrap();
    assert_eq!(st.state, WeiduState::NotRun);
    let st = s.run_weidu(GAME, DEFAULT_PROFILE).unwrap();
    assert_eq!(
        (st.state, st.installed.as_slice()),
        (WeiduState::Current, &["alpha".to_string()][..])
    );
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("lang/en_US/dialog.tlk")).unwrap(),
        "tlk+alpha0+alpha2"
    );
    assert!(!root.join("override/beta_1.itm").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // The other chosen: both installed again, in order; the game is untouched.
    let st = s
        .set_weidu_choice(GAME, DEFAULT_PROFILE, "beta", 0, comps(&[1]))
        .unwrap();
    assert_eq!(st.state, WeiduState::Outdated);
    let st = s.run_weidu(GAME, DEFAULT_PROFILE).unwrap();
    assert_eq!(st.state, WeiduState::Current);
    assert_eq!(st.installed, ["alpha", "beta"]);
    let chosen: Vec<(&str, Vec<u32>)> = st
        .mods
        .iter()
        .map(|c| {
            (
                c.mod_id.as_str(),
                c.components.iter().map(|(n, _)| *n).collect(),
            )
        })
        .collect();
    assert_eq!(chosen, [("alpha", vec![0, 2]), ("beta", vec![1])]);
    assert_eq!(st.mods[0].components[0].1, "Component 0");
    assert_eq!(
        fs::read(root.join("lang/en_US/dialog.tlk")).unwrap(),
        b"tlk"
    );
    assert!(!root.join(WEIDU_SCRATCH).exists());
    assert!(!root.join("override").exists());

    // Deployed: what they changed, not the installers or their folders; then put back.
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("lang/en_US/dialog.tlk")).unwrap(),
        "tlk+alpha0+alpha2+beta1",
        "beta built on alpha's result"
    );
    assert_eq!(
        fs::read_to_string(root.join("override/beta_1.itm")).unwrap(),
        "beta 1"
    );
    assert!(root.join("WeiDU.log").is_file());
    assert!(!root.join("setup-alpha.exe").exists());
    assert!(!root.join("alpha").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(
        fs::read(root.join("lang/en_US/dialog.tlk")).unwrap(),
        b"tlk"
    );
    assert!(!root.join("override").exists());
    assert_eq!(fs::read(root.join("data/a.bif")).unwrap(), b"bif");

    // A game update: installed again at the next deploy, with the components remembered.
    write(&root.join("chitin.key"), b"key v2");
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert_eq!(st.state, WeiduState::Outdated);
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let args = fs::read_to_string(root.join("override/alpha_args.txt")).unwrap();
    assert!(args.contains("--force-install-list 0 2"), "{args}");
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // A mod that comes without its installer: another WeiDU mod's (WeiDU itself) stands in.
    weidu_mod_with(&s, tmp.path(), "gamma", "4", false);
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert!(!st.needs_weidu);
    s.set_weidu_choice(GAME, DEFAULT_PROFILE, "gamma", 0, comps(&[4]))
        .unwrap();
    s.run_weidu(GAME, DEFAULT_PROFILE).unwrap();
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("override/gamma_4.itm")).unwrap(),
        "gamma 4"
    );
    assert!(!root.join("setup-gamma.exe").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // Components chosen in UMM: listed by WeiDU, in the language asked for, and installed
    // with them, without asking.
    let list = s
        .weidu_components(GAME, DEFAULT_PROFILE, "beta", None)
        .unwrap();
    assert_eq!(
        list.languages,
        [(0, "English".into()), (1, "Francais".into())]
    );
    assert_eq!(list.chosen, Some((Some(0), vec![1])));
    let names: Vec<&str> = list.components.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Core", "Portraits -> Original", "Portraits -> New"]);
    let fr = s
        .weidu_components(GAME, DEFAULT_PROFILE, "beta", Some(1))
        .unwrap();
    assert_eq!((fr.language, fr.components[0].1.as_str()), (1, "Core (fr)"));
    let st = s
        .set_weidu_choice(
            GAME,
            DEFAULT_PROFILE,
            "beta",
            0,
            vec![(0, "Core".into()), (2, "Portraits -> New".into())],
        )
        .unwrap();
    assert_eq!(st.state, WeiduState::Outdated);
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let args = fs::read_to_string(root.join("override/beta_args.txt")).unwrap();
    assert!(args.contains("--force-install-list 0 2"), "{args}");
    assert!(root.join("override/beta_2.itm").is_file());
    s.revert(GAME, false, &mut Faults::none()).unwrap();

    // A deleted mod's components are forgotten: imported again, it starts over.
    s.remove_mod(GAME, "gamma").unwrap();
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert!(st.mods.iter().all(|c| c.mod_id != "gamma"));
    weidu_mod_with(&s, tmp.path(), "gamma", "4", false);
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert!(!st.installed.contains(&"gamma".to_string()));
    let gamma = st.mods.iter().find(|c| c.mod_id == "gamma").unwrap();
    assert!(gamma.language.is_none() && gamma.components.is_empty());
    s.set_weidu_choice(GAME, DEFAULT_PROFILE, "gamma", 0, vec![(4, "Four".into())])
        .unwrap();

    // An installer that fails: reported, with its log.
    let m = s.data().game(GAME).mod_files_dir("beta");
    fs::write(m.join("beta/answers.txt"), "fail").unwrap();
    let err = s.run_weidu(GAME, DEFAULT_PROFILE).unwrap_err().to_string();
    assert!(err.contains("setup-beta.exe failed"), "{err}");
    let st = s.weidu_status(GAME, DEFAULT_PROFILE).unwrap().unwrap();
    assert!(st.output.contains("ERROR: it failed"), "{}", st.output);
}
