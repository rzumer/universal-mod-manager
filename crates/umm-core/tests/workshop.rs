//! A Steam game's Workshop items: read from Steam's record in the game's library, with their
//! details from Steam, kept for the next time and used when Steam cannot be reached.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use umm_core::workshop::DETAILS_URL;
use umm_core::Session;
use umm_thunderstore::http::FakeHttp;

const RECORD: &str = r#""AppWorkshop"
{
	"appid"		"4242"
	"WorkshopItemsInstalled"
	{
		"111"
		{
			"size"		"2048"
			"timeupdated"		"1700000000"
			"manifest"		"1"
		}
		"222"
		{
			"size"		"10"
			"timeupdated"		"1600000000"
			"manifest"		"2"
		}
	}
	"WorkshopItemDetails"
	{
		"111"
		{
			"manifest"		"1"
			"timeupdated"		"1700000000"
			"subscribedby"		"99"
			"latest_timeupdated"		"1700000000"
		}
		"222"
		{
			"manifest"		"2"
			"timeupdated"		"1600000000"
			"subscribedby"		"99"
			"latest_timeupdated"		"1650000000"
		}
		"333"
		{
			"manifest"		"3"
			"timeupdated"		"0"
			"subscribedby"		"99"
		}
	}
}"#;

const DETAILS: &str = r#"{"response":{"result":1,"resultcount":3,"publishedfiledetails":[
  {"publishedfileid":"111","result":1,"title":"Zebra Mod","description":"Stripes","file_size":"2048","time_updated":1700000000,"tags":[{"tag":"Mod"}]},
  {"publishedfileid":"222","result":1,"title":"Apple Mod","description":"","file_size":10,"time_updated":1650000000,"tags":[]},
  {"publishedfileid":"333","result":9}]}}"#;

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn workshop_items_come_from_steams_record_with_their_details() {
    let tmp = tempfile::tempdir().unwrap();
    let steamapps = tmp.path().join("lib/steamapps");
    let root = steamapps.join("common/Game");
    write(&root.join("Game.exe"), "exe");
    write(&steamapps.join("workshop/appworkshop_4242.acf"), RECORD);
    write(&steamapps.join("workshop/content/4242/111/mod.txt"), "a");
    write(&steamapps.join("workshop/content/4242/222/mod.txt"), "b");
    let http = Arc::new(FakeHttp::default().with_post(DETAILS_URL, "itemcount=3", DETAILS));
    let s = Session::open_with(Some(tmp.path().join("data")), http.clone()).unwrap();
    write(
        &s.data().game("g").def_path(),
        "schema = 1\nname = \"G\"\n[detect]\nsteam_app_id = 4242\nmarker = \"Game.exe\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    s.set_install_path("g", &root).unwrap();

    let w = s.workshop_items("g", false).unwrap();
    assert!(!w.stale);
    let summary: Vec<(&str, Option<&str>, bool, bool, bool)> = w
        .items
        .iter()
        .map(|i| {
            (
                i.id.as_str(),
                i.details.as_ref().map(|d| d.title.as_str()),
                i.subscribed,
                i.folder.is_some(),
                i.update_pending(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("222", Some("Apple Mod"), true, true, true),
            ("111", Some("Zebra Mod"), true, true, false),
            // Removed from the Workshop, never downloaded: listed, without details.
            ("333", None, true, false, false),
        ]
    );
    let posts = http.posts.lock().unwrap().clone();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].1.contains("publishedfileids%5B2%5D=333"),
        "{}",
        posts[0].1
    );
    assert_eq!(
        w.content_dir.as_deref(),
        Some(
            steamapps
                .join("workshop")
                .join("content")
                .join("4242")
                .as_path()
        )
    );

    // Kept: not asked again until refreshed, and an old copy serves when Steam is down.
    s.workshop_items("g", false).unwrap();
    assert_eq!(http.posts.lock().unwrap().len(), 1, "cached");
    http.post_rules.lock().unwrap().clear();
    let w = s.workshop_items("g", true).unwrap();
    assert!(w.stale);
    assert_eq!(
        w.items[0].details.as_ref().map(|d| d.title.as_str()),
        Some("Apple Mod")
    );

    // A game not on Steam has no Workshop.
    write(
        &s.data().game("other").def_path(),
        "schema = 1\nname = \"Other\"\n[[install]]\nmatch = \"**\"\ntarget = \"\"\n",
    );
    assert!(s.workshop_items("other", false).is_err());
}

/// A Steam library with the game, and Workshop items 1, 2 and 3 downloaded and subscribed,
/// each with `files` in its folder.
fn library(
    tmp: &Path,
    app: u32,
    files: &dyn Fn(&str) -> Vec<(String, String)>,
) -> std::path::PathBuf {
    let steamapps = tmp.join("lib/steamapps");
    let root = steamapps.join("common/Game");
    write(&root.join("Game.exe"), "exe");
    let entry = |i: &str| {
        format!("\t\t\"{i}\"\n\t\t{{\n\t\t\t\"size\"\t\t\"1\"\n\t\t\t\"timeupdated\"\t\t\"1\"\n\t\t\t\"subscribedby\"\t\t\"9\"\n\t\t}}\n")
    };
    let items: String = ["1", "2", "3"].iter().map(|i| entry(i)).collect();
    write(
        &steamapps.join(format!("workshop/appworkshop_{app}.acf")),
        &format!("\"AppWorkshop\"\n{{\n\t\"WorkshopItemsInstalled\"\n\t{{\n{items}\t}}\n\t\"WorkshopItemDetails\"\n\t{{\n{items}\t}}\n}}\n"),
    );
    for i in ["1", "2", "3"] {
        for (f, body) in files(i) {
            write(
                &steamapps.join(format!("workshop/content/{app}/{i}/{f}")),
                &body,
            );
        }
    }
    root
}

fn profile_with(s: &Session, game: &str, entries: &[(&str, bool)]) {
    let mut p = s.profile(game, umm_core::DEFAULT_PROFILE).unwrap();
    p.mods = entries
        .iter()
        .map(|(id, on)| umm_core::ProfileEntry::new(*id, *on))
        .collect();
    s.save_profile(game, &p).unwrap();
}

#[test]
fn a_profile_turns_rimworld_workshop_items_on_and_off() {
    use umm_core::Faults;
    let tmp = tempfile::tempdir().unwrap();
    let pid = |i: &str| ["a.one", "b.two", "c.three"][i.parse::<usize>().unwrap() - 1];
    let root = library(tmp.path(), 294100, &|i| {
        vec![(
            "About/About.xml".into(),
            format!(
                "<ModMetaData><packageId>{}</packageId></ModMetaData>",
                pid(i)
            ),
        )]
    });
    let list = tmp.path().join("config/ModsConfig.xml");
    let original = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.5</version>\n  <activeMods>\n    <li>ludeon.rimworld</li>\n    <li>b.two</li>\n    <li>c.three</li>\n  </activeMods>\n</ModsConfigData>\n";
    write(&list, original);
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("rw").def_path(),
        &format!(
            "schema = 1\nname = \"RimWorld\"\n[detect]\nsteam_app_id = 294100\nmarker = \"Game.exe\"\n\
             [mod_list]\nlist = '{}'\nformat = \"rimworld_xml\"\nfolder = \"Mods\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"Mods/{{mod}}/\"\n",
            list.display()
        ),
    );
    s.set_install_path("rw", &root).unwrap();
    // 1 on, 2 off; 3 is not in the profile, so it stays as the game has it.
    profile_with(&s, "rw", &[("workshop:1", true), ("workshop:2", false)]);
    assert!(s
        .plan("rw", umm_core::DEFAULT_PROFILE)
        .unwrap()
        .files
        .is_empty());
    s.deploy("rw", umm_core::DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let text = fs::read_to_string(&list).unwrap();
    let active: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("<li>")?.strip_suffix("</li>"))
        .collect();
    assert_eq!(active, ["ludeon.rimworld", "c.three", "a.one"]);
    s.revert("rw", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&list).unwrap(), original);
}

#[test]
fn a_profile_turns_paradox_workshop_items_on_and_off() {
    use umm_core::Faults;
    let tmp = tempfile::tempdir().unwrap();
    let root = library(tmp.path(), 1158310, &|i| {
        vec![
            (
                "descriptor.mod".into(),
                format!("name=\"Item {i}\"\nremote_file_id=\"{i}\"\n"),
            ),
            ("common/x.txt".into(), "x".into()),
        ]
    });
    let user = tmp.path().join("docs");
    let original =
        r#"{"enabled_mods":["mod/ugc_1.mod","mod/ugc_2.mod","mod/ugc_3.mod"],"disabled_dlcs":[]}"#;
    write(&user.join("dlc_load.json"), original);
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    let u = user.display().to_string();
    write(
        &s.data().game("ck3").def_path(),
        &format!(
            "schema = 1\nname = \"CK3\"\n[detect]\nsteam_app_id = 1158310\nmarker = \"Game.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mod/{{mod}}/\"\n\
             [folders]\nuser = '{u}'\n[paradox]\nfolder = \"user\"\n"
        ),
    );
    s.set_install_path("ck3", &root).unwrap();
    profile_with(&s, "ck3", &[("workshop:1", true), ("workshop:2", false)]);
    s.deploy("ck3", umm_core::DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(user.join("dlc_load.json")).unwrap()).unwrap();
    assert_eq!(
        v["enabled_mods"],
        serde_json::json!(["mod/ugc_3.mod", "mod/umm-workshop-1.mod"])
    );
    let d = fs::read_to_string(user.join("mod/umm-workshop-1.mod")).unwrap();
    assert!(d.contains("name=\"Item 1\""), "{d}");
    let folder = tmp
        .path()
        .join("lib/steamapps/workshop/content/1158310/1")
        .display()
        .to_string()
        .replace('\\', "/");
    assert!(d.contains(&format!("path=\"{folder}\"")), "{d}");
    s.revert("ck3", false, &mut Faults::none()).unwrap();
    assert_eq!(
        fs::read_to_string(user.join("dlc_load.json")).unwrap(),
        original
    );
    assert!(!user.join("mod/umm-workshop-1.mod").exists());
}

/// Crusader Kings II, on Steam's older Workshop: the game downloads its items itself, each
/// with a descriptor named after its file. A profile turns them on in `last_mods`, and the
/// game is not given those it has off, even ticked in its launcher.
#[test]
fn a_profile_turns_crusader_kings_ii_workshop_items_on_and_off() {
    use umm_core::Faults;
    let tmp = tempfile::tempdir().unwrap();
    let root = library(tmp.path(), 203770, &|_| Vec::new());
    let user = tmp.path().join("docs");
    let original = "last_mods={\n\t\"mod/two.mod.mod\"\n}\n";
    write(&user.join("settings.txt"), original);
    write(
        &user.join("mod/one.mod.mod"),
        "name=\"One\"\narchive=\"mod/one.zip\"\n",
    );
    write(
        &user.join("mod/two.mod.mod"),
        "name=\"Two\"\narchive=\"mod/two.zip\"\n",
    );
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    // Steam's details as UMM last had them, which name each item's file.
    write(
        &tmp.path().join("data/steam/workshop/203770.json"),
        r#"{"fetched_unix":0,"items":{"1":{"title":"One","filename":"mod/one.zip"},"2":{"title":"Two","filename":"mod/two.zip"}}}"#,
    );
    let u = user.display().to_string();
    write(
        &s.data().game("ck2").def_path(),
        &format!(
            "schema = 1\nname = \"CK2\"\n[detect]\nsteam_app_id = 203770\nmarker = \"Game.exe\"\n\
             [launch]\nexe = \"Game.exe\"\n\
             [[install]]\nmatch = \"**\"\ntarget = \"@user/mod/{{mod}}/\"\n\
             [folders]\nuser = '{u}'\n[paradox]\nfolder = \"user\"\nlist = \"settings\"\n"
        ),
    );
    s.set_install_path("ck2", &root).unwrap();
    profile_with(&s, "ck2", &[("workshop:1", true), ("workshop:2", false)]);
    s.deploy("ck2", umm_core::DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(user.join("settings.txt")).unwrap(),
        "last_mods={\n\t\"mod/two.mod.mod\"\n\t\"mod/one.mod.mod\"\n}\n"
    );
    let def = s.def("ck2").unwrap();
    let profile = s.profile("ck2", umm_core::DEFAULT_PROFILE).unwrap();
    assert_eq!(s.paradox_mod_args(&def, &profile), ["-mod=mod/one.mod.mod"]);
    s.revert("ck2", false, &mut Faults::none()).unwrap();
    assert_eq!(
        fs::read_to_string(user.join("settings.txt")).unwrap(),
        original
    );
}

/// Health: a profile's Workshop items are Steam's, not installed in UMM; only one Steam does
/// not record as subscribed to is worth a word.
#[test]
fn health_checks_a_profiles_workshop_items_against_steam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = library(tmp.path(), 1158310, &|_| Vec::new());
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("ck3").def_path(),
        "schema = 1\nname = \"CK3\"\n[detect]\nsteam_app_id = 1158310\nmarker = \"Game.exe\"\n",
    );
    s.set_install_path("ck3", &root).unwrap();
    profile_with(
        &s,
        "ck3",
        &[
            ("workshop:1", true),
            ("workshop:2", false),
            ("workshop:9", true),
        ],
    );
    let warnings: Vec<String> = s
        .doctor("ck3")
        .unwrap()
        .into_iter()
        .filter(|f| f.level == umm_core::Level::Warn)
        .map(|f| f.message)
        .collect();
    assert_eq!(
        warnings,
        ["profile `Default` lists Workshop item 9, which is not subscribed to in Steam"]
    );
}

/// Kenshi's Workshop items in a profile: enabled in `data/mods.cfg` by their `.mod` files, or
/// taken out of it, as the profile says; the user's list comes back afterwards.
#[test]
fn a_profile_turns_kenshi_workshop_items_on_and_off() {
    use umm_core::Faults;
    let tmp = tempfile::tempdir().unwrap();
    let root = library(tmp.path(), 233860, &|i| {
        vec![(format!("Item {i}.mod"), "mod".into())]
    });
    let list = root.join("data/mods.cfg");
    let original = "Item 2.mod\r\nItem 3.mod\r\n";
    write(&list, original);
    let s = Session::open(Some(tmp.path().join("data"))).unwrap();
    write(
        &s.data().game("kenshi").def_path(),
        "schema = 1\nname = \"Kenshi\"\n[detect]\nsteam_app_id = 233860\nmarker = \"Game.exe\"\n\
         [[install]]\nmatch = \"**\"\ntarget = \"mods/{mod}/\"\n\
         [mod_list]\nlist = \"data/mods.cfg\"\nformat = \"kenshi_cfg\"\nfolder = \"mods\"\n",
    );
    s.set_install_path("kenshi", &root).unwrap();
    profile_with(&s, "kenshi", &[("workshop:1", true), ("workshop:2", false)]);
    s.deploy("kenshi", umm_core::DEFAULT_PROFILE, &mut Faults::none())
        .unwrap();
    assert_eq!(
        fs::read_to_string(&list).unwrap(),
        "Item 3.mod\r\nItem 1.mod\r\n"
    );
    s.revert("kenshi", false, &mut Faults::none()).unwrap();
    assert_eq!(fs::read_to_string(&list).unwrap(), original);
}
