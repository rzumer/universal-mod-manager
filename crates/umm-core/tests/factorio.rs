//! Factorio: each mod goes in the user folder's `mods/`, in a folder named after its
//! `info.json`, and is enabled in `mod-list.json` there; the user's list comes back
//! afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

const GAME: &str = "factorio";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

fn info(name: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"factorio_version\": \"2.0\"\n}}\n")
}

const ORIGINAL: &str = "\n{\n  \"mods\": \n  [\n    \n    {\n      \"name\": \"base\",\n      \"enabled\": true\n    },\n    \n    {\n      \"name\": \"space-age\",\n      \"enabled\": false\n    },\n    \n    {\n      \"name\": \"Krastorio2\",\n      \"enabled\": false,\n      \"version\": \"1.3.0\"\n    }\n  ]\n}\n";

#[test]
fn factorio_mods_are_named_and_enabled_in_its_mod_list() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("bin/x64/factorio.exe"), b"exe");
    let user = tmp.path().join("appdata/Factorio");
    let list = user.join("mods/mod-list.json");
    write(&list, ORIGINAL.as_bytes());
    write(&user.join("mods/theirs_0.1.0.zip"), b"zip");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let user_toml = user.display().to_string().replace('\\', "/");
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"Factorio\"\n[detect]\nmarker = \"bin/x64/factorio.exe\"\n\
             [launch]\nexe = \"bin/x64/factorio.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mods/{{mod}}/\"\n\
             [folders]\nuser = \"{user_toml}\"\n\
             [plugins]\nlist = \"{user_toml}/mods/mod-list.json\"\nformat = \"factorio_json\"\nfolder = \"@user/mods\"\n"
        )
        .as_bytes(),
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
    // As the mod portal's zips have it: a folder named `<name>_<version>`.
    import(
        "k2",
        &[
            ("Krastorio2_1.4.0/info.json", info("Krastorio2").as_bytes()),
            ("Krastorio2_1.4.0/data.lua", b"-- data"),
            ("Krastorio2_1.4.0/changelog.txt", b"Version: 1.4.0"),
            ("Krastorio2_1.4.0/readme.txt", b"read me"),
            ("Krastorio2_1.4.0/LICENSE.md", b"license"),
        ],
    );
    // Laid out flat, under an id that is not its name.
    import(
        "helmod-pack",
        &[
            ("info.json", info("helmod").as_bytes()),
            ("control.lua", b"-- control"),
        ],
    );

    let preview = s
        .plugin_list_preview(GAME, DEFAULT_PROFILE)
        .unwrap()
        .unwrap();
    assert_eq!(
        preview
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.enabled, e.owner.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("base", true, None),
            ("space-age", false, None),
            ("Krastorio2", true, Some("k2")),
            ("helmod", true, Some("helmod-pack")),
        ],
        "the user's entry for a mod UMM deploys is replaced, enabled"
    );

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let mods = user.join("mods");
    assert_eq!(
        fs::read(mods.join("Krastorio2/data.lua")).unwrap(),
        b"-- data"
    );
    assert!(
        mods.join("Krastorio2/changelog.txt").is_file(),
        "the game shows it"
    );
    assert!(!mods.join("Krastorio2/readme.txt").exists());
    assert!(!mods.join("Krastorio2/LICENSE.md").exists());
    assert_eq!(
        fs::read(mods.join("helmod/control.lua")).unwrap(),
        b"-- control"
    );
    assert!(!mods.join("k2").exists() && !mods.join("helmod-pack").exists());
    assert!(!root.join("mods").exists(), "nothing in the game folder");
    assert_eq!(
        fs::read_to_string(&list).unwrap(),
        "\n{\n  \"mods\": \n  [\n    \n    {\n      \"name\": \"base\",\n      \"enabled\": true\n    },\n    \n    {\n      \"name\": \"space-age\",\n      \"enabled\": false\n    },\n    \n    {\n      \"name\": \"Krastorio2\",\n      \"enabled\": true\n    },\n    \n    {\n      \"name\": \"helmod\",\n      \"enabled\": true\n    }\n  ]\n}\n"
    );

    // The game writes its mod settings beside the mods while playing: those stay.
    write(&mods.join("mod-settings.dat"), b"settings");
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!mods.join("Krastorio2").exists() && !mods.join("helmod").exists());
    assert!(mods.join("theirs_0.1.0.zip").is_file());
    assert!(mods.join("mod-settings.dat").is_file());
    assert_eq!(fs::read_to_string(&list).unwrap(), ORIGINAL);

    // Two mods of the same name cannot both be deployed.
    import("k2-again", &[("info.json", info("Krastorio2").as_bytes())]);
    let err = s
        .deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err()
        .to_string();
    assert!(err.contains("both the Factorio mod `Krastorio2`"), "{err}");
}
