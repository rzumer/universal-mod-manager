//! Nexus Mods: browsing without an account, free downloads picked up from the Downloads
//! folder, updates by file group, and Premium downloads. Against fake Nexus answers.

use std::fs;
use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{
    CoreError, DownloadWatch, ExpectedFile, NexusError, NexusFile, NexusSearch, NexusUpdateState,
    Session, DEFAULT_PROFILE,
};
use umm_thunderstore::http::FakeHttp;

const GQL: &str = "https://api.nexusmods.com/v2/graphql";
const API: &str = "https://api.nexusmods.com/v1";

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
  {"modId": "0", "gameId": "1539", "modName": "Script Extender", "notes": "", "url": "https://example.org/se", "externalRequirement": true}]}}}}}"#;
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
    assert!(s.nexus_account().is_none());

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
    assert_eq!(
        r.meta.files.keys().collect::<Vec<_>>(),
        ["glow.dll"],
        "the wrapper folder is unwrapped"
    );
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
    // Free accounts cannot download it from inside UMM.
    assert!(matches!(
        s.nexus_update("glow", "glow", DEFAULT_PROFILE, &mut |_| {}),
        Err(CoreError::Nexus(NexusError::PremiumOnly))
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
    let stored = s.data().game("glow").mod_files_dir("glow").join("glow.dll");
    assert_eq!(fs::read_to_string(stored).unwrap(), "new");
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
fn premium_accounts_download_directly() {
    let tmp = tempfile::tempdir().unwrap();
    let old = zip_of(&[("glow.dll", "old")]);
    let new = zip_of(&[("glow.dll", "new")]);
    let http = fake(&old, &new);
    http.set(
        &format!("{API}/users/validate.json"),
        r#"{"user_id": 42, "name": "Ann", "is_premium": true, "is_supporter": false}"#,
    );
    http.set(
        &format!("{API}/games/glowvalley/mods/14/files/491/download_link.json"),
        r#"[{"name": "CDN", "URI": "https://cdn/old"}]"#,
    );
    http.set(
        &format!("{API}/games/glowvalley/mods/14/files/507/download_link.json"),
        r#"[{"name": "CDN", "URI": "https://cdn/new"}]"#,
    );
    http.set("https://cdn/old", old);
    http.set("https://cdn/new", new);
    let s = game(tmp.path(), http);

    let file = NexusFile {
        domain: "glowvalley".into(),
        mod_id: 14,
        file_id: 491,
        credentials: None,
    };
    // Signed out: no direct downloads.
    assert!(matches!(
        s.nexus_install("glow", &file, DEFAULT_PROFILE, &mut |_| {}),
        Err(CoreError::Nexus(NexusError::PremiumOnly))
    ));
    assert!(s.nexus_sign_in("key").unwrap().is_premium);
    s.nexus_install("glow", &file, DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    let r = s
        .nexus_update("glow", "glow", DEFAULT_PROFILE, &mut |_| {})
        .unwrap()
        .unwrap();
    assert!(r.replaced);
    assert_eq!(r.meta.nexus.unwrap().file_id, 507);
    assert!(s
        .nexus_update("glow", "glow", DEFAULT_PROFILE, &mut |_| {})
        .unwrap()
        .is_none());
    assert!(
        fs::read_dir(tmp.path().join("data/nexus/downloads"))
            .unwrap()
            .next()
            .is_none(),
        "UMM's own downloads are removed once stored"
    );

    // An expired link from the website is refused before anything is downloaded.
    let link = "nxm://glowvalley/mods/14/files/507?key=k&expires=1";
    assert!(matches!(
        s.nexus_install_link("glow", link, DEFAULT_PROFILE, &mut |_| {}),
        Err(CoreError::Nexus(NexusError::LinkExpired))
    ));
}

#[test]
fn free_accounts_install_from_website_links() {
    let tmp = tempfile::tempdir().unwrap();
    let old = zip_of(&[("glow.dll", "old")]);
    let new = zip_of(&[("glow.dll", "new")]);
    let http = fake(&old, &new);
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    http.set(
        &format!("{API}/users/validate.json"),
        r#"{"user_id": 42, "name": "Ann", "is_premium": false, "is_supporter": false}"#,
    );
    http.set(
        &format!(
            "{API}/games/glowvalley/mods/14/files/507/download_link.json?key=k1&expires={expires}"
        ),
        r#"[{"name": "CDN", "URI": "https://cdn/new"}]"#,
    );
    http.set("https://cdn/new", new);
    let s = game(tmp.path(), http);
    assert_eq!(
        s.nexus_link_games("glowvalley").unwrap(),
        [("glow".to_string(), "Glow Valley".to_string(), true)]
    );
    let link = format!("nxm://glowvalley/mods/14/files/507?key=k1&expires={expires}&user_id=42");
    let l = umm_core::NxmLink::parse(&link).unwrap();
    let (name, file) = s.nexus_link_info(&l).unwrap();
    assert_eq!(
        (name.as_str(), file.unwrap().version.as_str()),
        ("Glow", "3.1")
    );

    // Nexus needs the user's own key too, even with the link's.
    let err = s
        .nexus_install_link("glow", &link, DEFAULT_PROFILE, &mut |_| {})
        .unwrap_err();
    assert!(err.to_string().contains("sign in"), "{err}");
    assert!(!s.nexus_sign_in("key").unwrap().is_premium);
    let r = s
        .nexus_install_link("glow", &link, DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    assert_eq!(
        (r.meta.id.as_str(), r.meta.nexus.unwrap().file_id),
        ("glow", 507)
    );
}

#[test]
fn a_refused_key_is_not_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let http =
        Arc::new(FakeHttp::default().with_error(&format!("{API}/users/validate.json"), 401, "{}"));
    let s = game(tmp.path(), http);
    assert!(matches!(
        s.nexus_sign_in("bad"),
        Err(CoreError::Nexus(NexusError::BadKey))
    ));
    assert!(s.nexus_account().is_none());
    assert!(s.nexus_sign_in("two words").is_err());
}
