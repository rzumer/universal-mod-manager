//! Mods' config files in a profile's runtime folder: listed, read (BepInEx's setting by
//! setting) and edited from UMM.

use std::fs;
use std::path::Path;
use umm_core::{ConfigChange, Session, DEFAULT_PROFILE};

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

const CFG: &str = "## Settings file was created by plugin Better UI v2.1.0\n\
## Plugin GUID: com.example.betterui\n\n[General]\n\n\
## Show the clock.\n# Setting type: Boolean\n# Default value: true\nShow Clock = true\n";

#[test]
fn a_profiles_configs_are_listed_and_edited() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("g").def_path(),
        "schema = 1\nname = \"G\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n\
         [writable]\npaths = [\"BepInEx/config/**\", \"**/*.json\"]\n",
    );
    // Before the game has run with the profile: nothing yet.
    assert!(s.config_files("g", DEFAULT_PROFILE).unwrap().is_empty());

    let rt = s.config_dir("g", DEFAULT_PROFILE);
    write(&rt.join("BepInEx/config/com.example.betterui.cfg"), CFG);
    write(&rt.join("BepInEx/config/plain.txt"), "x");
    // Dumps and logs in a config folder (Lovely's) are not settings.
    write(&rt.join("BepInEx/config/dump/@game.lua"), "-- patched");
    write(&rt.join("BepInEx/config/log/lovely.log"), "log");
    write(&rt.join("BepInEx/config/run.log"), "log");
    // A package's manifest matches only the catch-all pattern: not a config.
    write(&rt.join("BepInEx/plugins/Better UI/manifest.json"), "{}");
    let files = s.config_files("g", DEFAULT_PROFILE).unwrap();
    let names: Vec<(&str, bool)> = files.iter().map(|f| (f.name.as_str(), f.bepinex)).collect();
    assert_eq!(names, [("Better UI", true), ("plain.txt", false)]);
    assert_eq!(files[0].guid.as_deref(), Some("com.example.betterui"));

    let v = s
        .set_config_values(
            "g",
            DEFAULT_PROFILE,
            "BepInEx/config/com.example.betterui.cfg",
            &[ConfigChange {
                section: "General".into(),
                key: "Show Clock".into(),
                value: "false".into(),
            }],
        )
        .unwrap();
    assert_eq!(v.sections.unwrap()[0].entries[0].value, "false");
    assert_eq!(
        fs::read_to_string(rt.join("BepInEx/config/com.example.betterui.cfg")).unwrap(),
        CFG.replace("Show Clock = true", "Show Clock = false")
    );
    let v = s
        .set_config_text("g", DEFAULT_PROFILE, "BepInEx/config/plain.txt", "y")
        .unwrap();
    assert_eq!(v.text, "y");

    // Only its configs, inside the runtime folder.
    for bad in [
        "../game.toml",
        "BepInEx/plugins/Better UI/manifest.json",
        "BepInEx/config/missing.cfg",
    ] {
        assert!(
            s.set_config_text("g", DEFAULT_PROFILE, bad, "z").is_err(),
            "{bad}"
        );
    }
}
