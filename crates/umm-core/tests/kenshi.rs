//! Kenshi: each mod goes in the game folder's `mods/`, in a folder named after its `.mod`
//! file, and is enabled in `data/mods.cfg`; the user's list comes back afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "kenshi";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// As the game writes it: the user's own mod, enabled in the game's launcher.
const ORIGINAL: &str = "Theirs.mod\r\n";

#[test]
fn kenshi_mods_are_named_after_their_mod_file_and_enabled_in_mods_cfg() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("kenshi_x64.exe"), b"exe");
    let list = root.join("data/mods.cfg");
    write(&list, ORIGINAL.as_bytes());
    write(&root.join("mods/Theirs/Theirs.mod"), b"theirs");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game(GAME).def_path(),
        b"schema = 1\nname = \"Kenshi\"\n[detect]\nmarker = \"kenshi_x64.exe\"\n\
          [launch]\nexe = \"kenshi_x64.exe\"\n\
          [[install]]\nmatch = \"**\"\ntarget = \"mods/{mod}/\"\n\
          [mod_list]\nlist = \"data/mods.cfg\"\nformat = \"kenshi_cfg\"\nfolder = \"mods\"\n",
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
    // As archives come: the mod's folder inside, under its own name.
    import(
        "dark-ui",
        &[
            ("Dark UI/Dark UI.mod", b"mod"),
            ("Dark UI/gui/x.layout", b"layout"),
            ("readme.txt", b"read me"),
        ],
    );
    import("recruit", &[("Recruit Prisoners.mod", b"mod")]);

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(root.join("mods/Dark UI/Dark UI.mod").is_file());
    assert!(root.join("mods/Dark UI/gui/x.layout").is_file());
    assert!(root
        .join("mods/Recruit Prisoners/Recruit Prisoners.mod")
        .is_file());
    assert!(!root.join("mods/dark-ui").exists());
    assert_eq!(
        fs::read_to_string(&list).unwrap(),
        "Theirs.mod\r\nDark UI.mod\r\nRecruit Prisoners.mod\r\n"
    );

    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!root.join("mods/Dark UI").exists());
    assert!(root.join("mods/Theirs/Theirs.mod").is_file());
    assert_eq!(fs::read_to_string(&list).unwrap(), ORIGINAL);
}
