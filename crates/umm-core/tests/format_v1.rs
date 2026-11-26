//! Data written by UMM in format 1, frozen in `fixtures/v1`: every later version must still
//! read it, and undo the deployment it was in the middle of when it crashed.
//!
//! The fixture holds a data folder (a game file, mods, a mod with a pinned older version,
//! profiles, and the journal of a deploy that crashed partway), the game folder and a folder
//! outside it in that state, and both folders as they were before the deploy. Absolute paths
//! are written as placeholders and filled in with a temporary folder.
//!
//! When the data format changes, generate the fixture of the new format with
//! `cargo test -p umm-core --test format_v1 -- --ignored`: it writes `fixtures/v<format>`
//! only if that folder does not exist, and never touches an older one.
//!
//! Windows only: the fixture's paths are Windows paths, as UMM wrote them there.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "fixture";

/// The placeholder for the temporary folder holding `data`, `game` and `user`, in each way
/// paths are written: as they are, escaped in JSON, and with forward slashes.
const ROOT: &str = "@@ROOT@@";
const ROOT_JSON: &str = "@@ROOT_JSON@@";
const ROOT_FWD: &str = "@@ROOT_FWD@@";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// Every file under `dir`, by `/`-separated path, with its bytes.
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for e in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let e = e.unwrap();
        if e.file_type().is_file() {
            let rel = e.path().strip_prefix(dir).unwrap();
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((rel, fs::read(e.path()).unwrap()));
        }
    }
    out
}

/// Copies a tree, replacing one root path by another in its text files.
fn copy_tree(from: &Path, to: &Path, subst: &dyn Fn(&str) -> String) {
    for (rel, bytes) in tree(from) {
        let dest = to.join(&rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        match String::from_utf8(bytes) {
            Ok(text) => fs::write(&dest, subst(&text)).unwrap(),
            Err(e) => fs::write(&dest, e.into_bytes()).unwrap(),
        }
    }
}

fn forms(root: &Path) -> [String; 3] {
    let plain = root.display().to_string();
    [
        plain.clone(),
        serde_json::to_string(&plain)
            .unwrap()
            .trim_matches('"')
            .to_string(),
        plain.replace('\\', "/"),
    ]
}

/// The scenario, in `root`: returns the session, with the default profile deployed up to a
/// crash.
fn scenario(root: &Path) -> Session {
    let game = root.join("game");
    let user = root.join("user");
    write(&game.join("Game.exe"), "exe");
    write(&game.join("Data/Base.esm"), "base");
    write(&game.join("Data/textures/rock.dds"), "original rock");
    write(&user.join("plugins.txt"), "*Base.esm\r\n");
    write(&user.join("Prefs.ini"), "[Display]\r\nWidth=1920\r\n");
    let s = Session::open(Some(root.join("data"))).unwrap();
    let u = user.display().to_string();
    write(
        &s.data().game(GAME).def_path(),
        &format!(
            "schema = 1\nname = \"Fixture\"\nprotect = [\"*.exe\"]\nwritable = [\"**/*.ini\"]\n\
             [detect]\nmarker = \"Game.exe\"\n\
             [launch]\nexe = \"Game.exe\"\n\
             [mod_list]\nlist = '{u}\\plugins.txt'\nformat = \"asterisk\"\nfolder = \"Data\"\n\
             [[ini]]\nfile = '{u}\\Prefs.ini'\nsection = \"Display\"\nkey = \"Mods\"\nvalue = \"1\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"\"\n\
             [folders]\nuser = '{u}'\n"
        ),
    );
    s.set_install_path(GAME, &game).unwrap();
    let src = |id: &str, files: &[(&str, &str)]| {
        let dir = root.join("src").join(id);
        let _ = fs::remove_dir_all(&dir);
        for (f, body) in files {
            write(&dir.join(f), body);
        }
        dir
    };
    let opts = |id: &str, version: &str| ImportOptions {
        id: Some(id.into()),
        version: Some(version.into()),
        ..Default::default()
    };
    s.import(
        GAME,
        &src(
            "alpha",
            &[
                ("Data/textures/rock.dds", "alpha rock"),
                ("Data/Alpha.esp", "alpha plugin"),
                ("@user/alpha.txt", "alpha notes"),
            ],
        ),
        opts("alpha", "1.0"),
        DEFAULT_PROFILE,
    )
    .unwrap();
    // Beta 1.0, then 2.0 replacing it, with 1.0 kept for a profile pinned to it.
    s.import(
        GAME,
        &src("beta", &[("Data/Beta.esp", "beta 1.0")]),
        opts("beta", "1.0"),
        DEFAULT_PROFILE,
    )
    .unwrap();
    s.create_profile(GAME, "Pinned", Some(DEFAULT_PROFILE))
        .unwrap();
    let mut p = s.profile(GAME, "Pinned").unwrap();
    p.mods[0].enabled = false;
    p.mods[1].version = Some("1.0".into());
    s.save_profile(GAME, &p).unwrap();
    umm_store::import_mod(
        &s.game_dir(GAME),
        &src("beta", &[("Data/Beta.esp", "beta 2.0")]),
        &ImportOptions {
            replace: true,
            keep_replaced: true,
            ..opts("beta", "2.0")
        },
    )
    .unwrap();

    // A deploy that crashes two steps before the end.
    let mut count = Faults::none();
    s.deploy(GAME, DEFAULT_PROFILE, &mut count).unwrap();
    let steps = count.steps();
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    let crashed = s
        .deploy(GAME, DEFAULT_PROFILE, &mut Faults::crash_at(steps - 2))
        .unwrap_err();
    assert!(crashed.is_injected_crash(), "{crashed}");
    s
}

/// Writes the fixture of the current format, if it has none yet.
#[test]
#[ignore = "generates a fixture; run when the data format changes"]
fn generate_fixture() {
    let out = fixtures().join(format!("v{}", umm_store::DATA_FORMAT));
    if out.exists() {
        eprintln!("{} exists; left as it is", out.display());
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // The folders before the deploy, from a run of the scenario without it.
    {
        let before = tempfile::tempdir().unwrap();
        let s = scenario(before.path());
        s.revert(GAME, false, &mut Faults::none()).unwrap();
        for d in ["game", "user"] {
            copy_tree(&before.path().join(d), &out.join("before").join(d), &|t| {
                t.to_string()
            });
        }
    }
    let _s = scenario(root);
    let [plain, json, fwd] = forms(root);
    let subst = |t: &str| {
        t.replace(&json, ROOT_JSON)
            .replace(&plain, ROOT)
            .replace(&fwd, ROOT_FWD)
    };
    for d in ["data", "game", "user"] {
        copy_tree(&root.join(d), &out.join("crashed").join(d), &subst);
    }
    // Locks and scratch folders are not part of the format.
    for e in walkdir::WalkDir::new(out.join("crashed/data")) {
        let e = e.unwrap();
        if e.file_name() == "state.lock" {
            fs::remove_file(e.path()).unwrap();
        }
    }
}

/// The fixture's crashed state, in a temporary folder.
fn crashed(version: u32) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let [plain, json, fwd] = forms(&root);
    let subst = |t: &str| {
        t.replace(ROOT_JSON, &json)
            .replace(ROOT_FWD, &fwd)
            .replace(ROOT, &plain)
    };
    copy_tree(
        &fixtures().join(format!("v{version}/crashed")),
        &root,
        &subst,
    );
    (tmp, root)
}

#[test]
fn format_1_data_is_read_and_its_crashed_deploy_undone() {
    let (_tmp, root) = crashed(1);
    let before = fixtures().join("v1/before");
    assert_ne!(
        tree(&root.join("game")),
        tree(&before.join("game")),
        "mid-deploy"
    );

    let s = Session::open(Some(root.join("data"))).unwrap();
    let def = s.game_def(GAME).unwrap();
    assert_eq!(def.name, "Fixture");
    assert_eq!(def.protect, ["*.exe"]);
    assert_eq!(def.writable, ["**/*.ini"]);
    assert!(def.plugins.is_some() && def.ini.len() == 1);
    let mods = s.mods(GAME).unwrap();
    assert_eq!(
        mods.iter()
            .map(|m| (m.id.as_str(), m.version.as_deref()))
            .collect::<Vec<_>>(),
        [("alpha", Some("1.0")), ("beta", Some("2.0"))]
    );
    let pinned = s.profile(GAME, "Pinned").unwrap();
    assert_eq!(pinned.pin("beta"), Some("1.0"));
    assert!(!pinned.mods[0].enabled);

    // The crashed deploy is undone, as at startup.
    let recovered = s.recover_all(false);
    assert_eq!(recovered.len(), 1);
    recovered.into_iter().for_each(|(_, r)| {
        r.unwrap();
    });
    for d in ["game", "user"] {
        assert_eq!(tree(&root.join(d)), tree(&before.join(d)), "{d} as before");
    }

    // Each profile deploys, with its version of beta, and reverts cleanly.
    for (profile, beta) in [(DEFAULT_PROFILE, "beta 2.0"), ("Pinned", "beta 1.0")] {
        s.deploy(GAME, profile, &mut Faults::none()).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("game/Data/Beta.esp")).unwrap(),
            beta
        );
        s.revert(GAME, false, &mut Faults::none()).unwrap();
        for d in ["game", "user"] {
            assert_eq!(tree(&root.join(d)), tree(&before.join(d)), "{profile}: {d}");
        }
    }
}
