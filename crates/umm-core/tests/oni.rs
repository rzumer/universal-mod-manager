//! Oxygen Not Included: each mod goes in its own folder in the user folder's `mods/Local`, and
//! is enabled in `mods/mods.json` there; the user's list comes back afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, LoaderSetup, Session, DEFAULT_PROFILE};

const GAME: &str = "oni";

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// As the game writes it: a Steam Workshop mod, enabled for Spaced Out.
const ORIGINAL: &str = "{\r\n  \"version\": 1,\r\n  \"mods\": [\r\n    {\r\n      \"label\": {\r\n        \"distribution_platform\": 1,\r\n        \"id\": \"1838445101\",\r\n        \"title\": \"Theirs\",\r\n        \"version\": 1700000000\r\n      },\r\n      \"status\": 1,\r\n      \"enabled\": true,\r\n      \"enabledForDlc\": [\r\n        \"EXPANSION1_ID\"\r\n      ],\r\n      \"crash_count\": 0,\r\n      \"reinstall_path\": null,\r\n      \"staticID\": \"Theirs\"\r\n    }\r\n  ],\r\n  \"mod_load_in_progress\": false\r\n}";

#[test]
fn oni_mods_get_a_folder_each_and_are_enabled_in_its_mods_json() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("OxygenNotIncluded.exe"), b"exe");
    write(&root.join("UnityPlayer.dll"), b"dll");
    write(
        &root.join("OxygenNotIncluded_Data/globalgamemanagers"),
        b"x",
    );
    let user = tmp.path().join("docs/Klei/OxygenNotIncluded");
    let list = user.join("mods/mods.json");
    write(&list, ORIGINAL.as_bytes());
    write(&user.join("mods/Steam/1838445101/Theirs.dll"), b"theirs");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let user_toml = user.display().to_string().replace('\\', "/");
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"Oxygen Not Included\"\n[detect]\nmarker = \"OxygenNotIncluded.exe\"\n\
             [launch]\nexe = \"OxygenNotIncluded.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mods/Local/{{mod}}/\"\n\
             [folders]\nuser = \"{user_toml}\"\n\
             [mod_list]\nlist = \"{user_toml}/mods/mods.json\"\nformat = \"oni_json\"\nfolder = \"@user/mods/Local\"\n"
        )
        .as_bytes(),
    );
    s.set_install_path(GAME, &root).unwrap();
    assert_eq!(
        s.loader_choice(GAME).unwrap().available,
        [LoaderSetup::Generic],
        "the game has its own mod loader"
    );
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
    // Its folder beside a readme, with an older version kept for older game builds.
    import(
        "tweaks",
        &[
            ("README.txt", b"read me"),
            ("Tweaks/mod.yaml", b"title: Tweaks\nstaticID: Tweaks\n"),
            (
                "Tweaks/mod_info.yaml",
                b"supportedContent: ALL\nAPIVersion: 2\n",
            ),
            ("Tweaks/Tweaks.dll", b"dll"),
            ("Tweaks/preview.png", b"png"),
            ("Tweaks/LICENSE.md", b"license"),
            (
                "Tweaks/archived_versions/old/mod_info.yaml",
                b"APIVersion: 2\n",
            ),
            ("Tweaks/archived_versions/old/Tweaks.dll", b"old dll"),
        ],
    );
    // Only its code.
    import("fast", &[("Fast.dll", b"fast")]);

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
            ("1838445101.Steam", true, None),
            ("tweaks", true, Some("tweaks")),
            ("fast", true, Some("fast")),
        ]
    );

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let local = user.join("mods/Local");
    assert_eq!(fs::read(local.join("tweaks/Tweaks.dll")).unwrap(), b"dll");
    assert!(local.join("tweaks/mod.yaml").is_file());
    assert!(
        local.join("tweaks/preview.png").is_file(),
        "the game shows it"
    );
    assert_eq!(
        fs::read(local.join("tweaks/archived_versions/old/Tweaks.dll")).unwrap(),
        b"old dll"
    );
    assert!(!local.join("tweaks/LICENSE.md").exists());
    assert!(!local.join("tweaks/README.txt").exists());
    assert!(!local.join("tweaks/Tweaks").exists());
    assert_eq!(fs::read(local.join("fast/Fast.dll")).unwrap(), b"fast");
    assert!(!root.join("mods").exists(), "nothing in the game folder");

    let text = fs::read_to_string(&list).unwrap();
    assert!(
        text.starts_with(&ORIGINAL[..ORIGINAL.find("\r\n  ],").unwrap()]),
        "their entry as it was: {text}"
    );
    assert!(text.ends_with("\r\n  ],\r\n  \"mod_load_in_progress\": false\r\n}"));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mods = v["mods"].as_array().unwrap();
    assert_eq!(mods.len(), 3);
    for (m, id) in mods[1..].iter().zip(["tweaks", "fast"]) {
        assert_eq!(m["label"]["distribution_platform"], 0);
        assert_eq!(m["label"]["id"], id);
        assert_eq!(m["status"], 1);
        assert_eq!(m["enabledForDlc"], serde_json::json!(["", "EXPANSION1_ID"]));
    }

    // The game rewrites the list while playing: the user's comes back all the same.
    write(
        &list,
        text.replace("\"crash_count\": 0", "\"crash_count\": 1")
            .as_bytes(),
    );
    write(&local.join("tweaks/config.json"), b"{}");
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert!(!local.join("fast").exists());
    assert!(!local.join("tweaks/Tweaks.dll").exists());
    assert!(user.join("mods/Steam/1838445101/Theirs.dll").is_file());
    assert_eq!(fs::read_to_string(&list).unwrap(), ORIGINAL);
}

/// Steam Workshop items in a profile: their entries, which the game lists, on or off as the
/// profile says while mods are deployed; the user's list comes back afterwards.
#[test]
fn a_profile_turns_oni_workshop_items_on_and_off() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("OxygenNotIncluded.exe"), b"exe");
    let user = tmp.path().join("docs/Klei/OxygenNotIncluded");
    let list = user.join("mods/mods.json");
    // Theirs (1838445101) is on; another item (42) is off.
    let original = ORIGINAL.replace(
        "\r\n  ],\r\n  \"mod_load_in_progress\"",
        ",\r\n    {\r\n      \"label\": {\r\n        \"distribution_platform\": 1,\r\n        \"id\": \"42\"\r\n      },\r\n      \"enabled\": false,\r\n      \"enabledForDlc\": []\r\n    }\r\n  ],\r\n  \"mod_load_in_progress\"",
    );
    write(&list, original.as_bytes());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let user_toml = user.display().to_string().replace('\\', "/");
    write(
        &s.data().game(GAME).def_path(),
        format!(
            "schema = 1\nname = \"Oxygen Not Included\"\n[detect]\nmarker = \"OxygenNotIncluded.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mods/Local/{{mod}}/\"\n\
             [folders]\nuser = \"{user_toml}\"\n\
             [mod_list]\nlist = \"{user_toml}/mods/mods.json\"\nformat = \"oni_json\"\nfolder = \"@user/mods/Local\"\n"
        )
        .as_bytes(),
    );
    s.set_install_path(GAME, &root).unwrap();
    let mut p = s.profile(GAME, DEFAULT_PROFILE).unwrap();
    p.mods = vec![
        umm_core::ProfileEntry::new("workshop:1838445101", false),
        umm_core::ProfileEntry::new("workshop:42", true),
    ];
    s.save_profile(GAME, &p).unwrap();

    s.deploy(GAME, DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&list).unwrap()).unwrap();
    assert_eq!(v["mods"][0]["enabled"], false);
    assert_eq!(v["mods"][0]["enabledForDlc"], serde_json::json!([]));
    assert_eq!(
        v["mods"][0]["staticID"], "Theirs",
        "the rest as the game wrote it"
    );
    assert_eq!(v["mods"][1]["enabled"], true);
    assert_eq!(
        v["mods"][1]["enabledForDlc"],
        serde_json::json!(["", "EXPANSION1_ID"])
    );
    s.revert(GAME, false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&list).unwrap(), original);
}
