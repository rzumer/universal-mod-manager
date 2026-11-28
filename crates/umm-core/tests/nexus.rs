//! Nexus Mods: browsing without an account, downloads made on the website and checked by
//! hash, and updates by file group. Against fake Nexus answers.

use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{
    CollectionFileState, CoreError, DownloadWatch, ExpectedFile, NexusSearch, NexusUpdateState,
    Session, DEFAULT_PROFILE,
};
use umm_sources::net::FakeHttp;

const GQL: &str = "https://api.nexusmods.com/v2/graphql";

const GAME: &str = r#"{"data": {"game": {"id": 1539, "name": "Glow Valley", "domainName": "glowvalley", "modCount": 96}}}"#;
const GAMES: &str = r#"{"data": {"games": {"nodes": [
  {"id": 1539, "name": "Glow Valley", "domainName": "glowvalley", "modCount": 96},
  {"id": 1540, "name": "Glow Valley 2", "domainName": "glowvalley2", "modCount": 3}]}}}"#;
const MODS: &str = r#"{"data": {"mods": {"totalCount": 2, "nodes": [
  {"modId": 14, "name": "Glow", "summary": "Brighter", "author": "Ann", "version": "3.1",
   "endorsements": 900, "downloads": 20000, "updatedAt": "2023-01-03T22:17:02Z", "category": "Visuals"},
  {"modId": 29, "name": "Dark", "summary": "Darker", "author": "Bob", "version": "1",
   "endorsements": 5, "downloads": 50, "updatedAt": "2020-01-01T00:00:00Z", "category": "Visuals"}]}}}"#;
const REQS: &str = r#"{"data": {"mod": {"modRequirements": {"nexusRequirements": {"nodes": [
  {"modId": "29", "gameId": "1539", "modName": "Dark", "notes": "Needed for the night", "url": "", "externalRequirement": false},
  {"modId": "0", "gameId": "1539", "modName": "Script Extender", "notes": "", "url": "https://example.org/se", "externalRequirement": true},
  {"modId": "0", "gameId": "0", "modName": "Signs", "notes": "", "url": "https://www.nexusmods.com/glowvalley/mods/31", "externalRequirement": true},
  {"modId": "0", "gameId": "0", "modName": "Other Game Thing", "notes": "", "url": "https://www.nexusmods.com/skyrim/mods/5", "externalRequirement": true}]}}}}}"#;
const MOD: &str =
    r#"{"data": {"mod": {"modId": 14, "name": "Glow", "summary": "Brighter", "version": "3.1"}}}"#;
const FILES: &str = r#"{"data": {"modFiles": [
  {"fileId": 491, "name": "Glow 3.0", "version": "3.0", "category": "OLD_VERSION", "date": 200, "uri": "Glow 3.0-14-3-0-200.zip", "groupId": 9},
  {"fileId": 507, "name": "Glow 3.1", "version": "3.1", "category": "MAIN", "date": 300, "uri": "Glow 3.1-14-3-1-300.zip", "groupId": 9},
  {"fileId": 733, "name": "Extras", "version": "1", "category": "OPTIONAL", "date": 250, "uri": "e5/de/59/e5de594b", "groupId": 11}
]}}"#;

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn md5_hex(b: &[u8]) -> String {
    use md5::Digest;
    md5::Md5::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn hash_hit(file_id: u64, version: &str, category: &str, date: u64) -> String {
    format!(
        r#"{{"data": {{"fileHash": [{{"gameId": 1539, "modFile": {{"modId": 14, "fileId": {file_id},
           "name": "Glow {version}", "version": "{version}", "category": "{category}", "date": {date},
           "uri": "x.zip", "groupId": 9, "mod": {{"name": "Glow"}}, "game": {{"domainName": "glowvalley"}}}}}}]}}}}"#
    )
}

fn fake(old: &[u8], new: &[u8]) -> Arc<FakeHttp> {
    Arc::new(
        FakeHttp::default()
            .with_post(GQL, "game(domainName", GAME)
            .with_post(GQL, "games(filter", GAMES)
            .with_post(GQL, "mods(filter", MODS)
            .with_post(GQL, "modRequirements", REQS)
            .with_post(GQL, "mod(modId", MOD)
            .with_post(GQL, "modFiles", FILES)
            .with_post(GQL, &md5_hex(old), hash_hit(491, "3.0", "OLD_VERSION", 200))
            .with_post(GQL, &md5_hex(new), hash_hit(507, "3.1", "MAIN", 300))
            .with_post(GQL, "fileHash", r#"{"data": {"fileHash": []}}"#),
    )
}

fn game(tmp: &std::path::Path, http: Arc<FakeHttp>) -> Session {
    let s = Session::open_with(Some(tmp.join("data")), http).unwrap();
    let root = tmp.join("game");
    fs::create_dir_all(&root).unwrap();
    let p = s.data().game("glow").def_path();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        "# notes\nschema = 1\nname = \"Glow Valley\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path("glow", &root).unwrap();
    s
}

#[test]
fn browses_and_installs_downloads_without_an_account() {
    let tmp = tempfile::tempdir().unwrap();
    let old = zip_of(&[("Glow 3.0/glow.dll", "old")]);
    let new = zip_of(&[("Glow 3.1/glow.dll", "new")]);
    let http = fake(&old, &new);
    let s = game(tmp.path(), http.clone());

    // The game is found by its name; "Glow Valley 2" is not it.
    let g = s.nexus_game("glow").unwrap().unwrap();
    assert_eq!(
        (g.domain.as_str(), g.id, g.guessed),
        ("glowvalley", Some(1539), true)
    );

    let page = s
        .nexus_search(
            "glow",
            &NexusSearch {
                count: 20,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!((page.total, page.items.len()), (2, 2));
    assert!(page.items.iter().all(|i| i.installed.is_empty()));
    let files = s.nexus_files("glow", 14).unwrap();
    assert_eq!(
        files.files[0].file_id, 507,
        "current files first, newest first"
    );
    // What Glow needs: Dark from Nexus (not installed), and something hosted elsewhere.
    let r = &files.requirements;
    assert_eq!(
        (
            r[0].name.as_str(),
            r[0].mod_id,
            r[0].installed,
            r[0].elsewhere
        ),
        ("Dark", Some(29), false, false)
    );
    assert_eq!(
        r[0].url.as_deref(),
        Some("https://www.nexusmods.com/glowvalley/mods/29")
    );
    assert_eq!(
        (r[1].mod_id, r[1].elsewhere, r[1].url.as_deref()),
        (None, true, Some("https://example.org/se"))
    );
    // Listed as external with a link to its page on Nexus: a mod of this game all the same.
    assert_eq!(
        (r[2].mod_id, r[2].elsewhere, r[2].url.as_deref()),
        (
            Some(31),
            false,
            Some("https://www.nexusmods.com/glowvalley/mods/31")
        )
    );
    assert_eq!((r[3].mod_id, r[3].elsewhere), (None, true));
    assert_eq!(
        Session::nexus_download_page("glowvalley", 14, 491),
        "https://www.nexusmods.com/glowvalley/mods/14?tab=files&file_id=491"
    );
    // No request carried an API key.
    assert!(http
        .headers
        .lock()
        .unwrap()
        .iter()
        .all(|(_, h)| !h.iter().any(|(k, _)| k == "apikey")));

    // The user downloads the old file on the website; UMM picks it up and checks its hash.
    let downloads = tmp.path().join("Downloads");
    fs::create_dir_all(&downloads).unwrap();
    let watch = DownloadWatch::new(&downloads).unwrap();
    fs::write(downloads.join("Glow 3.0-14-3-0-200.zip"), &old).unwrap();
    assert!(watch.poll().is_none());
    let got = watch.poll().unwrap();
    let want = ExpectedFile {
        domain: "glowvalley".into(),
        mod_id: 14,
        file_id: 491,
    };
    let r = s
        .nexus_install_download(
            "glow",
            &got,
            Some(&want),
            false,
            DEFAULT_PROFILE,
            &mut |_| {},
        )
        .unwrap();
    assert!(!r.replaced && r.domain_set);
    assert_eq!(r.meta.id, "glow");
    // Stored as extracted; the placement installs from inside the wrapper folder.
    assert_eq!(
        r.meta.files.keys().collect::<Vec<_>>(),
        ["Glow 3.0/glow.dll"]
    );
    assert_eq!(r.meta.placement.as_ref().unwrap().root, "Glow 3.0");
    let n = r.meta.nexus.as_ref().unwrap();
    assert_eq!((n.mod_id, n.file_id, n.game_id), (14, 491, Some(1539)));
    assert!(got.is_file(), "the user's download is left where it is");
    let text = fs::read_to_string(s.data().game("glow").def_path()).unwrap();
    assert!(text.contains("# notes") && text.contains("domain = \"glowvalley\""));
    let page = s
        .nexus_search(
            "glow",
            &NexusSearch {
                count: 20,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(
        page.items[0].installed,
        [("glow".to_string(), Some("3.0".to_string()))]
    );
    // Only what is installed: Glow, not Dark.
    let only = s
        .nexus_search(
            "glow",
            &NexusSearch {
                count: 20,
                ..Default::default()
            },
            true,
        )
        .unwrap();
    assert_eq!((only.total, only.items[0].summary.mod_id), (1, 14));
    let none = NexusSearch {
        text: "dark".into(),
        count: 20,
        ..Default::default()
    };
    assert_eq!(s.nexus_search("glow", &none, true).unwrap().total, 0);

    // Its newer version in the same file group, renamed, is an update.
    let u = s.nexus_updates("glow").unwrap();
    assert!(matches!(
        &u[0].state,
        NexusUpdateState::Newer { file_id: 507, .. }
    ));
    let p = downloads.join("Glow 3.1-14-3-1-300.zip");
    fs::write(&p, &new).unwrap();
    let r = s
        .nexus_install_download("glow", &p, None, false, DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    assert!(r.replaced && !r.domain_set);
    assert_eq!(
        (r.meta.id.as_str(), r.meta.nexus.as_ref().unwrap().file_id),
        ("glow", 507)
    );
    let stored = s
        .data()
        .game("glow")
        .mod_files_dir("glow")
        .join("Glow 3.1/glow.dll");
    assert_eq!(fs::read_to_string(stored).unwrap(), "new");
    // UMM's own placement follows the new version's wrapper folder.
    assert_eq!(r.meta.placement.as_ref().unwrap().root, "Glow 3.1");
    assert!(matches!(
        s.nexus_updates("glow").unwrap()[0].state,
        NexusUpdateState::Current
    ));

    // An archive Nexus does not know: refused, unless the user says it is the file they
    // downloaded. An optional file is a mod of its own.
    let repacked = downloads.join("extras.zip");
    fs::write(&repacked, zip_of(&[("extras.txt", "x")])).unwrap();
    let extras = ExpectedFile {
        domain: "glowvalley".into(),
        mod_id: 14,
        file_id: 733,
    };
    assert!(matches!(
        s.nexus_install_download(
            "glow",
            &repacked,
            Some(&extras),
            false,
            DEFAULT_PROFILE,
            &mut |_| {}
        ),
        Err(CoreError::NexusUnrecognised)
    ));
    assert!(s
        .nexus_install_download("glow", &repacked, None, false, DEFAULT_PROFILE, &mut |_| {})
        .is_err());
    let r = s
        .nexus_install_download(
            "glow",
            &repacked,
            Some(&extras),
            true,
            DEFAULT_PROFILE,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        (r.meta.id.as_str(), r.meta.name.as_str()),
        ("glow-extras", "Glow - Extras")
    );

    // Another Nexus game's file is refused once the game is set.
    let other = ExpectedFile {
        domain: "glowvalley2".into(),
        mod_id: 1,
        file_id: 1,
    };
    let p2 = downloads.join("other.zip");
    fs::write(&p2, zip_of(&[("o.txt", "o")])).unwrap();
    let err = s
        .nexus_install_download(
            "glow",
            &p2,
            Some(&other),
            true,
            DEFAULT_PROFILE,
            &mut |_| {},
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("set up for the Nexus Mods game"),
        "{err}"
    );
}

#[test]
fn a_collection_is_checked_against_what_is_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let old = zip_of(&[("Glow 3.0/glow.dll", "old")]);
    let new = zip_of(&[("Glow 3.1/glow.dll", "new")]);
    let revision = |domain: &str| {
        format!(
            r#"{{"data": {{"collectionRevision": {{"revisionNumber": 4,
              "collection": {{"name": "Nights", "slug": "abc123", "game": {{"domainName": "{domain}"}}}},
              "modFiles": [
                {{"fileId": 733, "optional": true, "file": {{"modId": 14, "name": "Extras", "version": "1", "mod": {{"name": "Glow"}}}}}},
                {{"fileId": 507, "optional": false, "file": {{"modId": 14, "name": "Glow 3.1", "version": "3.1", "mod": {{"name": "Glow"}}}}}},
                {{"fileId": 900, "optional": false, "file": {{"modId": 29, "name": "Dark", "version": "1", "mod": {{"name": "Dark"}}}}}},
                {{"fileId": 1000, "optional": false, "file": {{"modId": 30, "name": "Moon", "version": "1", "mod": {{"name": "Moon"}}}}}}
              ]}}}}}}"#
        )
    };
    let http = Arc::new(
        FakeHttp::default()
            .with_post(GQL, "game(domainName", GAME)
            .with_post(GQL, "games(filter", GAMES)
            .with_post(GQL, "\"abc123\"", revision("glowvalley"))
            .with_post(GQL, "\"other1\"", revision("skyrim"))
            .with_post(
                GQL,
                "\"same01\"",
                r#"{"data": {"collectionRevision": {"revisionNumber": 1,
                  "collection": {"name": "Same", "slug": "same01", "game": {"domainName": "glowvalley"}},
                  "modFiles": [
                    {"fileId": 733, "optional": true, "file": {"modId": 14, "name": "Extras", "version": "1"}},
                    {"fileId": 491, "optional": false, "file": {"modId": 14, "name": "Glow 3.0", "version": "3.0"}}
                  ]}}}"#,
            )
            .with_post(
                GQL,
                &md5_hex(&old),
                hash_hit(491, "3.0", "OLD_VERSION", 200),
            )
            .with_post(GQL, &md5_hex(&new), hash_hit(507, "3.1", "MAIN", 300))
            .with_post(GQL, "modFiles", FILES)
            .with_post(GQL, "mod(modId", MOD),
    );
    let s = game(tmp.path(), http);
    let archive = tmp.path().join("glow.zip");
    fs::write(&archive, &old).unwrap();
    s.nexus_install_download("glow", &archive, None, false, DEFAULT_PROFILE, &mut |_| {})
        .unwrap();

    // Glow 3.0 stands for Glow 3.1 (another version), not for Glow's optional Extras.
    let c = s.nexus_collection("glow", "abc123", None).unwrap();
    assert_eq!(c.collection.name, "Nights");
    assert_eq!(
        c.states,
        [
            CollectionFileState::Missing,
            CollectionFileState::OtherFile {
                id: "glow".into(),
                version: Some("3.0".into())
            },
            CollectionFileState::Missing,
            CollectionFileState::Missing,
        ]
    );
    assert_eq!(c.missing(), 3);

    // A collection listing Glow 3.0 itself: installed.
    let c = s.nexus_collection("glow", "same01", None).unwrap();
    assert_eq!(
        c.states,
        [
            CollectionFileState::Missing,
            CollectionFileState::Installed { id: "glow".into() },
        ]
    );

    let err = s
        .nexus_collection("glow", "other1", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("nexusmods.com/skyrim"), "{err}");
}

#[test]
fn another_mods_file_is_not_installed_as_the_one_awaited() {
    let tmp = tempfile::tempdir().unwrap();
    let old = zip_of(&[("Glow 3.0/glow.dll", "old")]);
    let new = zip_of(&[("Glow 3.1/glow.dll", "new")]);
    let s = game(tmp.path(), fake(&old, &new));
    let archive = tmp.path().join("Glow 3.0-14-3-0-200.zip");
    fs::write(&archive, &old).unwrap();
    // Waiting for Dark's file, the Downloads folder gets Glow's: refused, even when trusted.
    let dark = ExpectedFile {
        domain: "glowvalley".into(),
        mod_id: 29,
        file_id: 900,
    };
    for trust in [false, true] {
        match s.nexus_install_download(
            "glow",
            &archive,
            Some(&dark),
            trust,
            DEFAULT_PROFILE,
            &mut |_| {},
        ) {
            Err(CoreError::NexusOtherMod(name)) => assert_eq!(name, "Glow"),
            other => panic!("{other:?}"),
        }
    }
    assert!(
        s.nexus_updates("glow").unwrap().is_empty(),
        "nothing installed"
    );
}
