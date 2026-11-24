//! Deploying a Larian game's mods: `.pak` packages go in its user folder's `Mods/`, and
//! those with a module of their own are enabled in `modsettings.lsx` after the game's and the
//! user's own; only UMM's entries come back out afterwards.

use std::fs;
use std::path::Path;
use umm_core::{Faults, ImportOptions, Session, DEFAULT_PROFILE};

fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// A version 18 package of these files, each LZ4-compressed as the game's are.
fn pak(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"LSPK".to_vec();
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&[0; 32]);
    let mut list = Vec::new();
    for (name, content) in files {
        let packed = lz4_flex::block::compress(content);
        let offset = out.len() as u32;
        out.extend_from_slice(&packed);
        let mut e = vec![0u8; 272];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[256..260].copy_from_slice(&offset.to_le_bytes());
        e[263] = 2;
        e[264..268].copy_from_slice(&(packed.len() as u32).to_le_bytes());
        e[268..272].copy_from_slice(&(content.len() as u32).to_le_bytes());
        list.extend_from_slice(&e);
    }
    let list_offset = out.len() as u64;
    let packed = lz4_flex::block::compress(&list);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
    out.extend_from_slice(&packed);
    out[8..16].copy_from_slice(&list_offset.to_le_bytes());
    out
}

fn meta(folder: &str, name: &str, uuid: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<save>\n<region id=\"Config\"><node id=\"root\"><children>\
         <node id=\"ModuleInfo\">\
         <attribute id=\"Folder\" type=\"LSString\" value=\"{folder}\"/>\
         <attribute id=\"Name\" type=\"LSString\" value=\"{name}\"/>\
         <attribute id=\"UUID\" type=\"FixedString\" value=\"{uuid}\"/>\
         <attribute id=\"Version64\" type=\"int64\" value=\"36028797018963970\"/>\
         </node></children></node></region></save>"
    )
}

const GUSTAV: &str = "                        <node id=\"ModuleShortDesc\">\r\n                            <attribute id=\"Folder\" type=\"LSString\" value=\"GustavX\"/>\r\n                            <attribute id=\"MD5\" type=\"LSString\" value=\"\"/>\r\n                            <attribute id=\"Name\" type=\"LSString\" value=\"GustavX\"/>\r\n                            <attribute id=\"PublishHandle\" type=\"uint64\" value=\"0\"/>\r\n                            <attribute id=\"UUID\" type=\"guid\" value=\"cb555efe-2d9e-131f-8195-a89329d218ea\"/>\r\n                            <attribute id=\"Version64\" type=\"int64\" value=\"36028797018963968\"/>\r\n                        </node>\r\n";
const THEIRS: &str = "                        <node id=\"ModuleShortDesc\">\r\n                            <attribute id=\"Folder\" type=\"LSString\" value=\"Theirs\"/>\r\n                            <attribute id=\"UUID\" type=\"guid\" value=\"00000000-0000-0000-0000-00000000aaaa\"/>\r\n                        </node>\r\n";

fn modsettings(entries: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<save>\r\n    <version major=\"4\" minor=\"8\" revision=\"0\" build=\"500\"/>\r\n    <region id=\"ModuleSettings\">\r\n        <node id=\"root\">\r\n            <children>\r\n                <node id=\"Mods\">\r\n                    <children>\r\n{entries}                    </children>\r\n                </node>\r\n            </children>\r\n        </node>\r\n    </region>\r\n</save>\r\n"
    )
}

const UUID: &str = "1a2b3c4d-0000-0000-0000-000000000001";

#[test]
fn a_larian_games_modules_are_enabled_in_its_modsettings() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("bin/bg3.exe"), b"exe");
    let user = tmp.path().join("local/Larian Studios/Baldur's Gate 3");
    let original = modsettings(&format!("{GUSTAV}{THEIRS}"));
    let settings = user.join("PlayerProfiles/Public/modsettings.lsx");
    write(&settings, original.as_bytes());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("bg3").def_path(),
        format!(
            "schema = 1\nname = \"BG3\"\n[detect]\nmarker = \"bin/bg3.exe\"\n\
             [[install]]\nmatch = \"*.pak\"\ntarget = \"@user/Mods/\"\n\
             [[install]]\nmatch = \"bin/**\"\ntarget = \"bin/\"\n\
             [folders]\nuser = \"{}\"\n[larian]\nfolder = \"user\"\n",
            user.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    s.set_install_path("bg3", &root).unwrap();
    let src = tmp.path().join("src/bags");
    let m = meta("BetterBags", "Better Bags", UUID);
    write(
        &src.join("BetterBags.pak"),
        &pak(&[
            ("Public/BetterBags/Stats/a.txt", b"stats".as_slice()),
            ("Mods/BetterBags/meta.lsx", m.as_bytes()),
        ]),
    );
    // An override, with no module: it loads without being listed.
    write(
        &src.join("BagsOverride.pak"),
        &pak(&[("Public/Shared/x.txt", b"x".as_slice())]),
    );
    write(&src.join("info.json"), b"{}");
    s.import(
        "bg3",
        &src,
        ImportOptions {
            id: Some("bags".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();

    s.deploy("bg3", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert!(user.join("Mods/BetterBags.pak").is_file());
    assert!(user.join("Mods/BagsOverride.pak").is_file());
    assert!(!user.join("Mods/info.json").exists());
    let ours = format!(
        "                        <node id=\"ModuleShortDesc\"><attribute id=\"Folder\" type=\"LSString\" value=\"BetterBags\"/><attribute id=\"MD5\" type=\"LSString\" value=\"\"/><attribute id=\"Name\" type=\"LSString\" value=\"Better Bags\"/><attribute id=\"PublishHandle\" type=\"uint64\" value=\"0\"/><attribute id=\"UUID\" type=\"guid\" value=\"{UUID}\"/><attribute id=\"Version64\" type=\"int64\" value=\"36028797018963970\"/></node>\r\n"
    );
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        modsettings(&format!("{GUSTAV}{THEIRS}{ours}"))
    );

    // The game rewrites the file its own way while playing, the user enabling another mod:
    // only UMM's module goes.
    let other = "<node id=\"ModuleShortDesc\"><attribute id=\"UUID\" type=\"guid\" value=\"bbbb\"/></node>\n";
    let rewritten = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<save><region id=\"ModuleSettings\"><node id=\"root\"><children><node id=\"Mods\"><children>\n\
         <node id=\"ModuleShortDesc\">\n<attribute id=\"UUID\" type=\"guid\" value=\"{}\"/>\n<attribute id=\"Folder\" type=\"LSString\" value=\"BetterBags\"/>\n</node>\n{other}\
         </children></node></children></node></region></save>\n",
        UUID.to_uppercase()
    );
    write(&settings, rewritten.as_bytes());
    s.revert("bg3", false, &mut Faults::none()).unwrap();
    assert!(!user.join("Mods/BetterBags.pak").exists());
    assert!(!user.join("Mods/BagsOverride.pak").exists());
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<save><region id=\"ModuleSettings\"><node id=\"root\"><children><node id=\"Mods\"><children>\n\
             {other}</children></node></children></node></region></save>\n"
        )
    );
}

/// Before the game has written its list: deploying says to start it once, and changes nothing.
#[test]
fn a_larian_game_must_have_written_its_modsettings() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("game");
    write(&root.join("bin/bg3.exe"), b"exe");
    let user = tmp.path().join("local/Baldur's Gate 3");
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("bg3").def_path(),
        format!(
            "schema = 1\nname = \"BG3\"\n[[install]]\nmatch = \"*.pak\"\ntarget = \"@user/Mods/\"\n\
             [folders]\nuser = \"{}\"\n[larian]\nfolder = \"user\"\n",
            user.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    s.set_install_path("bg3", &root).unwrap();
    let src = tmp.path().join("src/m");
    let m = meta("M", "M", UUID);
    write(
        &src.join("M.pak"),
        &pak(&[("Mods/M/meta.lsx", m.as_bytes())]),
    );
    s.import(
        "bg3",
        &src,
        ImportOptions {
            id: Some("m".into()),
            unwrap: true,
            ..Default::default()
        },
        DEFAULT_PROFILE,
    )
    .unwrap();
    let err = s
        .deploy("bg3", DEFAULT_PROFILE, &mut Faults::none())
        .unwrap_err()
        .to_string();
    assert!(err.contains("start the game once"), "{err}");
    assert!(!user.join("Mods").exists());
}
