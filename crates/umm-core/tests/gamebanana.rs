//! GameBanana: a game found by its name, its mods listed and marked as installed, a file
//! downloaded, checked against its checksum and installed, and a newer upload of it offered
//! as an update. Answers are shaped as GameBanana's API gives them.

use std::io::{Cursor, Write};
use std::sync::Arc;
use umm_core::{GbSearch, GbSort, GbUpdateState, Session, DEFAULT_PROFILE};
use umm_sources::net::FakeHttp;

const GAME: &str = "undertale";
const API: &str = "https://gamebanana.com/apiv11";

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn md5(bytes: &[u8]) -> String {
    use md5::Digest;
    use std::fmt::Write;
    md5::Md5::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn record(id: u64, name: &str, version: &str) -> serde_json::Value {
    serde_json::json!({
        "_idRow": id, "_sModelName": "Mod", "_sName": name,
        "_sProfileUrl": format!("https://gamebanana.com/mods/{id}"),
        "_tsDateAdded": 1676930565, "_tsDateModified": 1678916491, "_bHasFiles": true,
        "_aPreviewMedia": {"_aImages": [{"_sType": "screenshot", "_sBaseUrl": "https://images.gamebanana.com/img/ss/mods", "_sFile": "a.jpg", "_sFile220": "220-90_a.jpg"}]},
        "_aSubmitter": {"_idRow": 1, "_sName": "BlueSheep3"},
        "_aGame": {"_idRow": 5506, "_sName": "UNDERTALE"},
        "_aRootCategory": {"_sName": "Game files"},
        "_sVersion": version, "_bIsObsolete": false, "_bHasContentRatings": false,
        "_nLikeCount": 62, "_nViewCount": 199184
    })
}

fn profile(files: serde_json::Value, version: &str) -> String {
    let mut r = record(428457, "Undertale with a Gaster Blaster", version);
    let o = r.as_object_mut().unwrap();
    o.insert(
        "_sText".into(),
        "Use <b>Gaster</b> Blasters<br>patch 1.1".into(),
    );
    o.insert("_nDownloadCount".into(), 80987.into());
    o.insert("_aFiles".into(), files);
    r.to_string()
}

fn file(id: u64, name: &str, added: u64, body: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "_idRow": id, "_sFile": name, "_nFilesize": body.len(), "_tsDateAdded": added,
        "_nDownloadCount": 77916, "_sDownloadUrl": format!("https://gamebanana.com/dl/{id}"),
        "_sMd5Checksum": md5(body), "_sAvResult": "clean",
        "_aAnalysisWarnings": {"contains_exe": ["DeltaPatcher.exe"]}
    })
}

#[test]
fn a_gamebanana_mod_is_found_downloaded_checked_installed_and_updated() {
    let tmp = tempfile::tempdir().unwrap();
    let v1 = zip_of(&[
        ("Gaster/gaster.xdelta", b"patch v1"),
        ("Gaster/DeltaPatcher.exe", b"exe"),
    ]);
    let v2 = zip_of(&[("Gaster/gaster.xdelta", b"patch v2")]);
    let http = Arc::new(
        FakeHttp::default()
            .with(
                &format!("{API}/Util/Search/Results?_sModelName=Game&_sOrder=best_match&_sSearchString=Undertale&_nPage=1"),
                serde_json::json!({"_aMetadata": {"_nRecordCount": 2, "_bIsComplete": true}, "_aRecords": [
                    {"_idRow": 6713, "_sModelName": "Game", "_sName": "UNDERTALE (Switch)"},
                    {"_idRow": 5506, "_sModelName": "Game", "_sName": "UNDERTALE"}
                ]})
                .to_string(),
            )
            .with(
                &format!("{API}/Game/5506/ProfilePage"),
                serde_json::json!({"_idRow": 5506, "_sName": "UNDERTALE", "_aModRootCategories": [{"_nItemCount": 253}, {"_nItemCount": 144}]}).to_string(),
            )
            .with(
                &format!("{API}/Mod/Index?_nPerpage=20&_aFilters%5BGeneric_Game%5D=5506&_sSort=Generic_MostDownloaded&_nPage=1"),
                serde_json::json!({"_aMetadata": {"_nRecordCount": 549, "_bIsComplete": false}, "_aRecords": [
                    record(428457, "Undertale with a Gaster Blaster", "1.2"),
                    record(496958, "Undertale Red & Yellow", "2.0")
                ]})
                .to_string(),
            )
            .with(
                &format!("{API}/Util/Search/Results?_sModelName=Mod&_idGameRow=5506&_sOrder=popularity&_sSearchString=gaster+blaster&_nPerpage=20&_nPage=1"),
                serde_json::json!({"_aMetadata": {"_nRecordCount": 1, "_bIsComplete": true}, "_aRecords": [
                    record(428457, "Undertale with a Gaster Blaster", "1.2")
                ]})
                .to_string(),
            )
            .with(
                &format!("{API}/Mod/428457/ProfilePage"),
                profile(serde_json::json!([file(949854, "undertale_gaster_blaster_v12.zip", 1678916481, &v1)]), "1.2"),
            )
            .with("https://gamebanana.com/dl/949854", v1.clone()),
    );
    let s = Session::open_with(Some(tmp.path().join("data")), http.clone()).unwrap();
    let root = tmp.path().join("game");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("UNDERTALE.exe"), b"exe").unwrap();
    let def_path = s.data().game(GAME).def_path();
    std::fs::create_dir_all(def_path.parent().unwrap()).unwrap();
    std::fs::write(
        &def_path,
        "schema = 1\nname = \"Undertale\"\n[detect]\nmarker = \"UNDERTALE.exe\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    )
    .unwrap();
    s.set_install_path(GAME, &root).unwrap();

    // Found by its name (in any case), not yet saved.
    let g = s.gamebanana_game(GAME).unwrap().unwrap();
    assert_eq!((g.game_id, g.guessed, g.mod_count), (5506, true, Some(397)));

    let q = |text: &str| GbSearch {
        text: text.into(),
        sort: GbSort::Downloads,
        page: 1,
        per_page: 20,
    };
    let page = s.gamebanana_search(GAME, &q(""), false).unwrap();
    assert_eq!(
        (page.total, page.complete, page.items.len()),
        (549, false, 2)
    );
    let first = &page.items[0].summary;
    assert_eq!(first.author.as_deref(), Some("BlueSheep3"));
    assert_eq!(
        first.thumbnail_url.as_deref(),
        Some("https://images.gamebanana.com/img/ss/mods/220-90_a.jpg")
    );
    assert!(page.items[0].installed.is_empty());
    // Text search goes through GameBanana's search, in the game.
    let found = s
        .gamebanana_search(GAME, &q("gaster blaster"), false)
        .unwrap();
    assert_eq!(found.items.len(), 1);

    let files = s.gamebanana_files(GAME, 428457).unwrap();
    assert_eq!(files.details.text, "Use Gaster Blasters\npatch 1.1");
    assert_eq!(files.details.files[0].contains_exe, ["DeltaPatcher.exe"]);

    // Installed: the game's GameBanana game is saved with it.
    let mut lines = Vec::new();
    let r = s
        .gamebanana_install(GAME, 428457, 949854, DEFAULT_PROFILE, &mut |m| {
            lines.push(m.to_string())
        })
        .unwrap();
    assert!(r.game_set && !r.replaced);
    assert_eq!(r.meta.id, "undertale-with-a-gaster-blaster");
    assert_eq!(r.meta.version.as_deref(), Some("1.2"));
    // Stored as unpacked; its wrapper folder is left out where it goes.
    assert!(r.meta.files.contains_key("Gaster/gaster.xdelta"));
    assert_eq!(
        r.meta.placement.as_ref().map(|p| p.root.as_str()),
        Some("Gaster")
    );
    assert!(
        lines.iter().any(|l| l.starts_with("Downloading")),
        "{lines:?}"
    );
    let def = s.game_def(GAME).unwrap();
    assert_eq!(def.gamebanana.unwrap().game, 5506);
    let g = s.gamebanana_game(GAME).unwrap().unwrap();
    assert!(!g.guessed);
    let page = s.gamebanana_search(GAME, &q(""), false).unwrap();
    assert_eq!(
        page.items[0].installed,
        [(
            "undertale-with-a-gaster-blaster".to_string(),
            Some("1.2".to_string())
        )]
    );
    // Only installed ones, looked up one by one.
    let mine = s.gamebanana_search(GAME, &q(""), true).unwrap();
    assert_eq!(mine.items.len(), 1);
    assert_eq!(
        s.gamebanana_updates(GAME).unwrap()[0].state,
        GbUpdateState::Current
    );

    // A newer upload of the same file: an update, which replaces the mod.
    http.set(
        &format!("{API}/Mod/428457/ProfilePage"),
        profile(
            serde_json::json!([
                file(960001, "undertale_gaster_blaster_v13.zip", 1700000000, &v2),
                file(949854, "undertale_gaster_blaster_v12.zip", 1678916481, &v1)
            ]),
            "1.3",
        ),
    );
    http.set("https://gamebanana.com/dl/960001", v2.clone());
    let u = s.gamebanana_updates(GAME).unwrap();
    assert_eq!(
        u[0].state,
        GbUpdateState::Newer {
            file_id: 960001,
            file_name: "undertale_gaster_blaster_v13.zip".into(),
            version: "1.3".into(),
        }
    );
    let r = s
        .gamebanana_install(GAME, 428457, 960001, DEFAULT_PROFILE, &mut |_| {})
        .unwrap();
    assert!(r.replaced && !r.game_set);
    assert_eq!(r.meta.id, "undertale-with-a-gaster-blaster");
    assert_eq!(r.meta.gamebanana.as_ref().unwrap().file_id, 960001);
    assert_eq!(
        s.gamebanana_updates(GAME).unwrap()[0].state,
        GbUpdateState::Current
    );

    // The installed file taken down for a new upload, named otherwise (GameBanana's random
    // suffix): that upload is the update.
    let v3 = zip_of(&[("Gaster/gaster.xdelta", b"patch v3")]);
    http.set(
        &format!("{API}/Mod/428457/ProfilePage"),
        profile(
            serde_json::json!([file(970002, "gaster_blaster_4a1f0.zip", 1710000000, &v3)]),
            "1.4",
        ),
    );
    assert!(matches!(
        s.gamebanana_updates(GAME).unwrap()[0].state,
        GbUpdateState::Newer {
            file_id: 970002,
            ..
        }
    ));

    // A download that does not match its checksum is refused.
    http.set("https://gamebanana.com/dl/970002", b"truncated".to_vec());
    let e = s
        .gamebanana_install(GAME, 428457, 970002, DEFAULT_PROFILE, &mut |_| {})
        .unwrap_err();
    assert!(e.to_string().contains("checksum"), "{e}");
}
