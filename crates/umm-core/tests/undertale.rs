//! Delta patches (Undertale's mods): a mod's `.xdelta` files patch the game's `data.win` while
//! mods are deployed, stacked in the profile's order, and the original comes back afterwards.

use std::fs;
use std::path::Path;
use umm_core::vcdiff::adler32;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "undertale";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn int(v: usize) -> Vec<u8> {
    let mut digits = vec![(v & 0x7F) as u8];
    let mut v = v >> 7;
    while v > 0 {
        digits.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    digits.reverse();
    digits
}

/// A patch as xdelta3 would write it: one window copying what `target` shares with the
/// start of `source`, then adding the rest; `app` its application header.
fn xdelta(source: &[u8], target: &[u8], app: Option<&str>) -> Vec<u8> {
    let common = source
        .iter()
        .zip(target)
        .take_while(|(a, b)| a == b)
        .count();
    let (mut inst, mut addr) = (Vec::new(), Vec::new());
    if common > 0 {
        // COPY, size given, address mode 0 (from 0).
        inst.push(19);
        inst.extend(int(common));
        addr.extend(int(0));
    }
    let data = &target[common..];
    if !data.is_empty() {
        // ADD, size given.
        inst.push(1);
        inst.extend(int(data.len()));
    }
    let mut p = vec![0xD6, 0xC3, 0xC4, 0x00];
    match app {
        Some(a) => {
            p.push(0x04);
            p.extend(int(a.len()));
            p.extend(a.as_bytes());
        }
        None => p.push(0),
    }
    p.push(0x01 | 0x04);
    p.extend(int(source.len()));
    p.extend(int(0));
    let mut enc = int(target.len());
    enc.push(0);
    enc.extend(int(data.len()));
    enc.extend(int(inst.len()));
    enc.extend(int(addr.len()));
    enc.extend(adler32(target).to_be_bytes());
    enc.extend(data);
    enc.extend(&inst);
    enc.extend(&addr);
    p.extend(int(enc.len()));
    p.extend(enc);
    p
}

const VANILLA: &[u8] = b"GEN8 vanilla Undertale data";
const KINDER: &[u8] = b"GEN8 vanilla Undertale data + kinder";
const BOTH: &[u8] = b"GEN8 vanilla Undertale data + kinder + bigger";

#[test]
fn xdelta_mods_patch_data_win_in_profile_order_while_deployed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("UNDERTALE.exe"), b"exe");
    write(&root.join("data.win"), VANILLA);
    write(&root.join("mus_x.ogg"), b"music");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"Undertale\"\n[detect]\nmarker = \"UNDERTALE.exe\"\n\
          [launch]\nexe = \"UNDERTALE.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path(GAME, &root).unwrap();
    let import = |id: &str, files: &[(&str, Vec<u8>)]| {
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
    // Named in its header (as Delta Patcher writes it), with a replaced song beside it.
    import(
        "kinder",
        &[
            (
                "Kinder/kinder.xdelta",
                xdelta(VANILLA, KINDER, Some("data_kinder.win//data.win/")),
            ),
            ("Kinder/mus_x.ogg", b"kinder music".to_vec()),
            // For applying the patch by hand: not a patcher for UMM to run.
            ("Kinder/DeltaPatcher.exe", b"exe".to_vec()),
        ],
    );
    // One patch per game version, naming no file: data.win, as a GameMaker game's. Made
    // to go on top of the first mod.
    import(
        "bigger",
        &[
            (
                "bigger v1.00.xdelta",
                xdelta(b"GEN8 old Undertale", b"GEN8 old Undertale + bigger", None),
            ),
            ("bigger v1.08.xdelta", xdelta(KINDER, BOTH, None)),
        ],
    );

    // A preview applies nothing yet, and says so.
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert!(
        plan.skipped
            .iter()
            .any(|k| k.file == "kinder.xdelta" && k.reason.contains("at the next deploy")),
        "{:?}",
        plan.skipped
    );
    assert!(plan
        .files
        .iter()
        .all(|f| !f.target.as_str().ends_with(".xdelta")));

    assert!(s.patcher_status(GAME, "kinder").unwrap().is_none());
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(!root.join("DeltaPatcher.exe").exists());
    assert_eq!(fs::read(root.join("data.win")).unwrap(), BOTH);
    assert_eq!(fs::read(root.join("mus_x.ogg")).unwrap(), b"kinder music");
    assert!(!root.join("kinder.xdelta").exists());
    assert!(!root.join("bigger v1.08.xdelta").exists());
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read(root.join("data.win")).unwrap(), VANILLA);
    assert_eq!(fs::read(root.join("mus_x.ogg")).unwrap(), b"music");

    // Kept: a preview now shows the patched file, and which patch went unused.
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    let data = plan
        .files
        .iter()
        .find(|f| f.target.as_str() == "data.win")
        .unwrap();
    assert_eq!(data.owner, "bigger");
    assert_eq!(fs::read(&data.source).unwrap(), BOTH);
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.file == "bigger v1.00.xdelta"
            && k.reason.contains("fits none of the game's files")));

    // A game update: the patches no longer fit, and the game's own data.win stays.
    write(&root.join("data.win"), b"GEN8 Undertale 1.09");
    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("data.win")).unwrap(),
        b"GEN8 Undertale 1.09"
    );
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    let plan = s.plan(GAME, DEFAULT_PROFILE).unwrap();
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.file == "kinder.xdelta" && k.reason.contains("fits none")));
}

/// A GameMaker data file: `FORM`, its size, `GEN8`, then `content`.
fn gm(content: &[u8]) -> Vec<u8> {
    let mut body = b"GEN8".to_vec();
    body.extend((content.len() as u32).to_le_bytes());
    body.extend(content);
    let mut f = b"FORM".to_vec();
    f.extend((body.len() as u32).to_le_bytes());
    f.extend(body);
    f
}

/// A game with a data file per chapter (as DELTARUNE): a patch naming no file goes to the
/// one it was made from, found by trying it. Two mods' patches for the same file, both made
/// for the game's own, clash: the preview says so, and nothing deploys until one is off.
#[test]
fn patches_find_their_data_file_and_clashing_ones_stop_the_deploy() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let menu = gm(b"the chapter select menu");
    let ch1 = gm(b"chapter one: the dark world");
    let ch2 = gm(b"chapter two: the cyber world");
    write(&root.join("DELTARUNE.exe"), b"exe");
    write(&root.join("data.win"), &menu);
    write(&root.join("chapter1_windows/data.win"), &ch1);
    write(&root.join("chapter2_windows/data.win"), &ch2);
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("dr").def_path(),
        b"schema = 1\nname = \"DELTARUNE\"\n[detect]\nmarker = \"DELTARUNE.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path("dr", &root).unwrap();
    let import = |id: &str, files: &[(&str, Vec<u8>)]| {
        let src = tmp.path().join("src").join(id);
        for (f, b) in files {
            write(&src.join(f), b);
        }
        s.import(
            "dr",
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
    let ch2_hard = [ch2.clone(), b" + hard mode".to_vec()].concat();
    let ch2_easy = [ch2.clone(), b" + easy mode".to_vec()].concat();
    import(
        "hard",
        &[("Hard Mode.xdelta", xdelta(&ch2, &ch2_hard, None))],
    );

    s.deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("chapter2_windows/data.win")).unwrap(),
        ch2_hard
    );
    assert_eq!(fs::read(root.join("data.win")).unwrap(), menu);
    s.revert("dr", false, &mut Faults::none()).unwrap();

    // Another mod for chapter 2, also made for the game's own file.
    import(
        "easy",
        &[("easy.xdelta", xdelta(&ch2, &ch2_easy, Some("x//data.win/")))],
    );
    let plan = s.plan("dr", DEFAULT_PROFILE).unwrap();
    assert!(
        plan.skipped
            .iter()
            .any(|k| k.owner == "easy" && k.reason.contains("clashes with hard")),
        "{:?}",
        plan.skipped
    );
    let e = s
        .deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("both patch chapter2_windows/data.win"),
        "{e}"
    );
    // Nothing was deployed.
    assert_eq!(
        fs::read(root.join("chapter2_windows/data.win")).unwrap(),
        ch2
    );
    assert!(!s.data().game("dr").journal_path().exists());

    let mut p = s.profile("dr", DEFAULT_PROFILE).unwrap();
    p.mods.iter_mut().find(|m| m.id == "hard").unwrap().enabled = false;
    s.save_profile("dr", &p).unwrap();
    s.deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("chapter2_windows/data.win")).unwrap(),
        ch2_easy
    );
    s.revert("dr", false, &mut Faults::none()).unwrap();

    // An UndertaleModTool script: left out, and said why.
    import("scripted", &[("fix.csx", b"script".to_vec())]);
    let plan = s.plan("dr", DEFAULT_PROFILE).unwrap();
    assert!(plan.files.iter().all(|f| f.target.as_str() != "fix.csx"));
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.file == "fix.csx" && k.reason.contains("UndertaleModTool")));
}
