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
/// one it was made from, found by trying it. Two mods' patches for the same data file, both
/// made for the game's own, are to be merged (with G3MTool): when that fails, nothing
/// deploys until one is off. For a file that is not a data file, they clash outright.
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
    // No network: G3MTool cannot be downloaded, so merging fails.
    let s = Session::open_with(
        Some(tmp.path().join("data")),
        std::sync::Arc::new(umm_thunderstore::http::FakeHttp::default()),
    )
    .unwrap();
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
            .any(|k| k.owner == "easy" && k.reason.contains("merged with hard's changes")),
        "{:?}",
        plan.skipped
    );
    let e = s
        .deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(
        e.to_string().contains("could not merge") && e.to_string().contains("Disable one of them"),
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

    // Two mods patching a file that is not a GameMaker data file: they clash outright.
    write(&root.join("extras.dat"), b"original extras");
    p.mods.iter_mut().for_each(|m| m.enabled = false);
    s.save_profile("dr", &p).unwrap();
    import(
        "x1",
        &[(
            "extras.xdelta",
            xdelta(
                b"original extras",
                b"original extras + one",
                Some("x//extras.dat/"),
            ),
        )],
    );
    import(
        "x2",
        &[(
            "extras.xdelta",
            xdelta(
                b"original extras",
                b"original extras + two",
                Some("x//extras.dat/"),
            ),
        )],
    );
    let e = s
        .deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err();
    assert!(e.to_string().contains("both patch extras.dat"), "{e}");
    let mut p = s.profile("dr", DEFAULT_PROFILE).unwrap();
    p.mods
        .iter_mut()
        .for_each(|m| m.enabled = !m.id.starts_with('x'));
    s.save_profile("dr", &p).unwrap();

    // An UndertaleModTool script: left out, and said why.
    import("scripted", &[("fix.csx", b"script".to_vec())]);
    let plan = s.plan("dr", DEFAULT_PROFILE).unwrap();
    assert!(plan.files.iter().all(|f| f.target.as_str() != "fix.csx"));
    assert!(plan
        .skipped
        .iter()
        .any(|k| k.file == "fix.csx" && k.reason.contains("UndertaleModTool")));
}

/// A GameMaker data file whose `GEN8` chunk names the game: its display name at offset 100,
/// as DELTARUNE's chapters have it.
fn gm_named(display: &str, content: &[u8]) -> Vec<u8> {
    let mut gen = vec![0u8; 120];
    // FORM(8) + GEN8 and its size(8) + the chunk, then the string's length and characters.
    let ptr = (8 + 8 + 120 + 4) as u32;
    gen[100..104].copy_from_slice(&ptr.to_le_bytes());
    let mut body = b"GEN8".to_vec();
    body.extend((gen.len() as u32).to_le_bytes());
    body.extend(gen);
    body.extend((display.len() as u32).to_le_bytes());
    body.extend(display.as_bytes());
    body.push(0);
    body.extend(content);
    let mut f = b"FORM".to_vec();
    f.extend((body.len() as u32).to_le_bytes());
    f.extend(body);
    f
}

/// A DELTAMOD package: its modding.xml names each patch's file and where a copied file goes,
/// its own files stay out of the game folder, and a patch for another game version says the
/// version the package names. A mod's replacement data file goes to the game's data file of
/// the same name, wherever the mod has it.
#[test]
fn deltamod_packages_follow_their_list_and_data_files_go_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    let menu = gm_named("DELTARUNE", b"menu");
    let ch1 = gm_named("DELTARUNE Chapter 1", b"chapter one");
    let ch2 = gm_named("DELTARUNE Chapter 2", b"chapter two");
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
    let ch1_fast = [ch1.clone(), b" + 60 fps".to_vec()].concat();
    import(
        "fps",
        &[
            (
                "modding.xml",
                b"<patch type=\"xdelta\" patch=\"./c1.xdelta\" to=\"./chapter1_windows/data.win\" />\n\
                  <patch type=\"xdelta\" patch=\"./old.xdelta\" to=\"./chapter2_windows/data.win\" />\n\
                  <patch type=\"override\" patch=\"./song.ogg\" to=\"./mus/song.ogg\" />"
                    .to_vec(),
            ),
            (
                "meta.toml",
                b"deltaruneTargetVersion = \"1.05\"\n[metadata]\nname = \"60 FPS\"\n".to_vec(),
            ),
            ("icon.png", b"png".to_vec()),
            ("c1.xdelta", xdelta(&ch1, &ch1_fast, None)),
            ("old.xdelta", xdelta(b"an older chapter two", b"an older chapter two!", None)),
            ("song.ogg", b"ogg".to_vec()),
        ],
    );
    let plan = s.plan("dr", DEFAULT_PROFILE).unwrap();
    let reason = |file: &str| {
        plan.skipped
            .iter()
            .find(|k| k.file == file)
            .map(|k| k.reason.clone())
            .unwrap_or_default()
    };
    assert!(reason("modding.xml").contains("DELTAMOD"));
    assert!(reason("meta.toml").contains("DELTAMOD"));
    assert!(reason("icon.png").contains("DELTAMOD"));
    assert!(reason("c1.xdelta").contains("chapter1_windows/data.win"));
    assert!(
        reason("old.xdelta").contains("version 1.05"),
        "{}",
        reason("old.xdelta")
    );
    assert!(plan
        .files
        .iter()
        .any(|f| f.target.as_str() == "mus/song.ogg"));

    s.deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("chapter1_windows/data.win")).unwrap(),
        ch1_fast
    );
    assert_eq!(fs::read(root.join("mus/song.ogg")).unwrap(), b"ogg");
    assert!(!root.join("modding.xml").exists() && !root.join("song.ogg").exists());
    s.revert("dr", false, &mut Faults::none()).unwrap();

    // A chapter 2 data file at the top of a mod: it goes to chapter 2, not the menu.
    let ch2_new = gm_named("DELTARUNE Chapter 2", b"chapter two, remade");
    import("remade", &[("data.win", ch2_new.clone())]);
    s.deploy("dr", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read(root.join("chapter2_windows/data.win")).unwrap(),
        ch2_new
    );
    assert_eq!(fs::read(root.join("data.win")).unwrap(), menu);
    s.revert("dr", false, &mut Faults::none()).unwrap();
}
