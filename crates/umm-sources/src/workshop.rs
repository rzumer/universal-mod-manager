//! Steam Workshop: what Steam records on this machine of a game's Workshop items, Steam's
//! public details of items (`GetPublishedFileDetails`, no key needed), and the Workshop's
//! browse page, which needs no sign-in either.
//!
//! Steam downloads each item into `steamapps/workshop/content/<app id>/<item id>/` in the
//! game's library, and lists them in `steamapps/workshop/appworkshop_<app id>.acf`. UMM only
//! reads these: subscribing happens in the Steam client, which UMM opens on the item's or the
//! game's Workshop page.

use crate::net::Http;
use crate::{Result, Site};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::vdf;

/// Steam's public endpoint for Workshop item details.
pub const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// Items asked about per request.
const PER_REQUEST: usize = 100;

/// What Steam says about an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkshopDetails {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub time_updated: u64,
    #[serde(default)]
    pub time_created: u64,
    #[serde(default)]
    pub subscriptions: u64,
    #[serde(default)]
    pub favorited: u64,
    /// The file it is, on Steam's older Workshop (`mod/<name>.zip`): where the game puts it.
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The `steamapps` folder holding the game's Workshop record: the one above its install
/// folder (`steamapps/common/<game>`), else any Steam library's that has one.
pub fn steamapps_of(app_id: u32, root: Option<&Path>) -> Option<PathBuf> {
    let file = Path::new("workshop").join(format!("appworkshop_{app_id}.acf"));
    let above = root.and_then(|r| {
        let common = r.parent()?;
        common
            .file_name()?
            .eq_ignore_ascii_case("common")
            .then(|| common.parent().map(Path::to_path_buf))?
    });
    above
        .into_iter()
        .chain(
            umm_gamedef::detect::steam_libraries()
                .into_iter()
                .map(|l| l.join("steamapps")),
        )
        .find(|s| s.join(&file).is_file())
}

/// A SteamID64 less this is the account's folder under Steam's `userdata`.
const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

/// Steam's list of the game's items its user (the one last signed in) subscribes to,
/// `userdata/<account>/ugc/<app>_subscriptions.vdf`, which Steam updates as soon as one is
/// subscribed to or not. The only record of them for a game on Steam's older Workshop,
/// whose items the game's own launcher downloads (Crusader Kings II). `None` when no Steam
/// user is known.
pub fn subscriptions_file(app_id: u32) -> Option<PathBuf> {
    umm_gamedef::detect::steam_roots()
        .into_iter()
        .find_map(|root| {
            let users = fs::read_to_string(root.join("config").join("loginusers.vdf")).ok()?;
            let user = root
                .join("userdata")
                .join(recent_account(&users)?.to_string());
            user.is_dir()
                .then(|| user.join("ugc").join(format!("{app_id}_subscriptions.vdf")))
        })
}

/// The account id of the user last signed in to Steam, from `loginusers.vdf` (the only one
/// there when none is marked).
fn recent_account(text: &str) -> Option<u64> {
    let doc = vdf::parse(text)?;
    let Some(vdf::Value::Obj(users)) = vdf::get(&doc, "users") else {
        return None;
    };
    let recent = |v: &vdf::Value| {
        matches!(v, vdf::Value::Obj(o)
            if matches!(vdf::get(o, "MostRecent"), Some(vdf::Value::Str(s)) if s == "1"))
    };
    let (id, _) = users
        .iter()
        .find(|(_, v)| recent(v))
        .or_else(|| (users.len() == 1).then(|| &users[0]))?;
    id.parse::<u64>().ok()?.checked_sub(STEAM_ID64_BASE)
}

/// The items a `<app>_subscriptions.vdf` lists; `None` when it cannot be read.
pub fn read_subscriptions(text: &str) -> Option<Vec<String>> {
    let doc = vdf::parse(text)?;
    let Some(vdf::Value::Obj(root)) = vdf::get(&doc, "subscribedfiles") else {
        return None;
    };
    Some(
        root.iter()
            .filter_map(|(_, v)| match v {
                vdf::Value::Obj(o) => match vdf::get(o, "publishedfileid") {
                    Some(vdf::Value::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            })
            .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
            .collect(),
    )
}

/// Steam's record of the game's items, with its user's subscriptions as `subs` lists them
/// (sooner than the record, and the only list for a game on the older Workshop).
pub fn with_subscriptions(
    mut record: Vec<(String, Recorded)>,
    subs: Option<Vec<String>>,
) -> Vec<(String, Recorded)> {
    let Some(subs) = subs else {
        return record;
    };
    for (id, r) in &mut record {
        r.subscribed = subs.contains(id);
    }
    for id in subs {
        if !record.iter().any(|(i, _)| *i == id) {
            record.push((
                id,
                Recorded {
                    subscribed: true,
                    ..Default::default()
                },
            ));
        }
    }
    record.sort_by(|a, b| a.0.cmp(&b.0));
    record
}

/// An item as `appworkshop_<app>.acf` records it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// Subscribed to by an account on this machine.
    pub subscribed: bool,
    /// Size on disk.
    pub size: u64,
    /// When the downloaded version was published (Unix time).
    pub updated: u64,
    /// When the newest version Steam knows of was published.
    pub latest: u64,
    /// Steam has its files.
    pub downloaded: bool,
}

/// The items in `appworkshop_<app>.acf`, by id: from `WorkshopItemsInstalled` (downloaded)
/// and `WorkshopItemDetails` (`subscribedby`, `latest_timeupdated`).
pub fn read_record(text: &str) -> Vec<(String, Recorded)> {
    let Some(doc) = vdf::parse(text) else {
        return Vec::new();
    };
    let Some(vdf::Value::Obj(root)) = vdf::get(&doc, "AppWorkshop") else {
        return Vec::new();
    };
    let section = |name: &str| match vdf::get(root, name) {
        Some(vdf::Value::Obj(o)) => o.clone(),
        _ => Vec::new(),
    };
    let field = |o: &[(String, vdf::Value)], k: &str| match vdf::get(o, k) {
        Some(vdf::Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let installed = section("WorkshopItemsInstalled");
    let details = section("WorkshopItemDetails");
    let mut out: BTreeMap<String, Recorded> = BTreeMap::new();
    for (id, v) in &installed {
        let vdf::Value::Obj(o) = v else { continue };
        let e = out.entry(id.clone()).or_default();
        e.size = field(o, "size").parse().unwrap_or(0);
        e.updated = field(o, "timeupdated").parse().unwrap_or(0);
        e.downloaded = true;
    }
    for (id, v) in &details {
        let vdf::Value::Obj(o) = v else { continue };
        let by = field(o, "subscribedby");
        let e = out.entry(id.clone()).or_default();
        e.subscribed = !by.is_empty() && by != "0";
        e.latest = field(o, "latest_timeupdated").parse().unwrap_or(0);
    }
    out.into_iter()
        .filter(|(id, _)| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// A JSON number or numeric string (Steam sends both).
fn num(v: &Value) -> u64 {
    match v {
        Value::Number(n) => n.as_u64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// An item's id in a `GetPublishedFileDetails` answer.
fn answer_id(d: &Value) -> Option<String> {
    match &d["publishedfileid"] {
        Value::String(s) => Some(s.clone()),
        n @ Value::Number(_) => Some(num(n).to_string()),
        _ => None,
    }
}

/// Details from a `GetPublishedFileDetails` answer, by id, and the ids of the items Steam
/// does not show (removed, private).
#[allow(clippy::type_complexity)]
pub fn read_details(body: &[u8]) -> Option<(BTreeMap<String, WorkshopDetails>, BTreeSet<String>)> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let list = v.get("response")?.get("publishedfiledetails")?.as_array()?;
    let gone = list
        .iter()
        .filter(|d| num(&d["result"]) != 1)
        .filter_map(answer_id)
        .collect();
    Some((
        list.iter()
            .filter(|d| num(&d["result"]) == 1)
            .filter_map(|d| {
                let id = answer_id(d)?;
                let text = |k: &str| d[k].as_str().unwrap_or_default().to_string();
                Some((
                    id,
                    WorkshopDetails {
                        title: text("title"),
                        description: text("description"),
                        preview_url: Some(text("preview_url"))
                            .filter(|u| u.starts_with("https://")),
                        file_size: num(&d["file_size"]),
                        time_updated: num(&d["time_updated"]),
                        time_created: num(&d["time_created"]),
                        subscriptions: num(&d["subscriptions"]),
                        favorited: num(&d["favorited"]),
                        filename: text("filename"),
                        tags: d["tags"]
                            .as_array()
                            .map(|t| {
                                t.iter()
                                    .filter_map(|x| x["tag"].as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    },
                ))
            })
            .collect(),
        gone,
    ))
}

/// How the Workshop's browse page sorts items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkshopSort {
    /// Most popular lately (Steam's default).
    #[default]
    Trend,
    MostSubscribed,
    TopRated,
    Recent,
    Updated,
}

impl WorkshopSort {
    fn param(self) -> &'static str {
        match self {
            WorkshopSort::Trend => "trend",
            WorkshopSort::MostSubscribed => "totaluniquesubscribers",
            WorkshopSort::TopRated => "toprated",
            WorkshopSort::Recent => "mostrecent",
            WorkshopSort::Updated => "lastupdated",
        }
    }
}

/// A search of the game's Workshop.
#[derive(Debug, Clone, Default)]
pub struct WorkshopQuery {
    pub text: String,
    pub sort: WorkshopSort,
    /// Only items with this tag.
    pub tag: Option<String>,
    /// From 1.
    pub page: u32,
}

/// An item found on the Workshop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopFound {
    pub id: String,
    pub title: String,
    pub description: String,
    pub preview_url: Option<String>,
    pub author: Option<String>,
    pub subscriptions: u64,
    pub favorited: u64,
    pub file_size: u64,
    pub time_updated: u64,
    pub tags: Vec<String>,
    /// Subscribed to on this machine, as Steam's record has it.
    pub subscribed: bool,
}

/// A page of the game's Workshop.
#[derive(Debug, Clone, Default)]
pub struct WorkshopPage {
    pub items: Vec<WorkshopFound>,
    pub page: u32,
    pub total_pages: u32,
    pub total: u64,
    /// The game's Workshop tags, for filtering.
    pub tags: Vec<String>,
}

/// Items per page of the browse page.
const PER_PAGE: u32 = 30;

/// `s` for a URL's query string.
fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The Workshop's browse page for a search.
pub fn browse_url(app_id: u32, q: &WorkshopQuery) -> String {
    let text = q.text.trim();
    let sort = if text.is_empty() {
        q.sort.param()
    } else {
        "textsearch"
    };
    let mut url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={app_id}&section=readytouseitems&days=-1&numperpage={PER_PAGE}&browsesort={sort}&p={}",
        q.page.max(1)
    );
    if !text.is_empty() {
        url.push_str(&format!("&searchtext={}", query_escape(text)));
    }
    if let Some(t) = q.tag.as_deref().filter(|t| !t.is_empty()) {
        url.push_str(&format!("&requiredtags%5B%5D={}", query_escape(t)));
    }
    url
}

/// Calls `f` on every object in `v`, and in the JSON documents its strings hold (the page's
/// data nests documents as strings).
fn each_object(v: &Value, f: &mut dyn FnMut(&serde_json::Map<String, Value>)) {
    match v {
        Value::Object(o) => {
            f(o);
            for x in o.values() {
                each_object(x, f);
            }
        }
        Value::Array(a) => {
            for x in a {
                each_object(x, f);
            }
        }
        Value::String(s) if s.starts_with('{') || s.starts_with('[') => {
            if let Ok(inner) = serde_json::from_str::<Value>(s) {
                each_object(&inner, f);
            }
        }
        _ => {}
    }
}

/// A browse page's results, from the data Steam's page embeds (`valve-ssr-data`): the items,
/// their authors' names, and the game's tags.
pub fn read_browse(html: &str) -> Option<WorkshopPage> {
    let start = html.find("id=\"valve-ssr-data\"")?;
    let body = &html[start..];
    let open = body.find('>')? + 1;
    let close = body[open..].find("</script>")? + open;
    let data: Value = serde_json::from_str(&body[open..close]).ok()?;
    let mut results: Option<serde_json::Map<String, Value>> = None;
    let mut authors: BTreeMap<String, String> = BTreeMap::new();
    let mut tags: Vec<String> = Vec::new();
    each_object(&data, &mut |o| {
        if results.is_none() && o.contains_key("total_pages") && o.contains_key("results") {
            results = Some(o.clone());
        }
        if let Some(p) = o.get("public_data") {
            if let (Some(id), Some(name)) = (p["steamid"].as_str(), p["persona_name"].as_str()) {
                authors.insert(id.to_string(), name.to_string());
            }
        }
        if tags.is_empty() {
            if let Some(groups) = o.get("readytouse_tags").and_then(Value::as_array) {
                for g in groups {
                    for t in g["tags"].as_array().into_iter().flatten() {
                        if let Some(n) = t["name"].as_str() {
                            if !tags.iter().any(|x| x == n) {
                                tags.push(n.to_string());
                            }
                        }
                    }
                }
            }
        }
    });
    let r = results?;
    let items = r
        .get("results")?
        .as_array()?
        .iter()
        .filter_map(|d| {
            let id = answer_id(d)?;
            let text = |k: &str| d[k].as_str().unwrap_or_default().to_string();
            Some(WorkshopFound {
                title: text("title"),
                description: text("short_description"),
                preview_url: Some(text("preview_url")).filter(|u| u.starts_with("https://")),
                author: d["creator"].as_str().and_then(|c| authors.get(c).cloned()),
                subscriptions: num(&d["subscriptions"]),
                favorited: num(&d["favorited"]),
                file_size: num(&d["file_size"]),
                time_updated: num(&d["time_updated"]),
                tags: d["tags"]
                    .as_array()
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| x["tag"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                subscribed: false,
                id,
            })
        })
        .collect();
    Some(WorkshopPage {
        items,
        page: num(&r["current_page"]) as u32,
        total_pages: num(&r["total_pages"]) as u32,
        total: num(&r["total_count"]),
        tags,
    })
}

/// Opens an item's page in the Steam client, where it can be subscribed to.
pub fn item_page(item: &str) -> String {
    format!("steam://url/CommunityFilePage/{item}")
}

/// Opens the game's Workshop in the Steam client.
pub fn workshop_page(app_id: u32) -> String {
    format!("steam://url/SteamWorkshopPage/{app_id}")
}

/// Steam's record of the game's items in a `steamapps` folder.
pub fn record_file(steamapps: &Path, app_id: u32) -> PathBuf {
    steamapps
        .join("workshop")
        .join(format!("appworkshop_{app_id}.acf"))
}

/// Steam's folder of the game's downloaded items in a `steamapps` folder.
pub fn content_dir(steamapps: &Path, app_id: u32) -> PathBuf {
    steamapps
        .join("workshop")
        .join("content")
        .join(app_id.to_string())
}

/// Steam's details of `ids`, asked in batches: the details by id, the ids Steam does not
/// show (removed, private), and whether every batch was answered.
pub fn fetch_details(
    http: &dyn Http,
    ids: &[&str],
) -> (BTreeMap<String, WorkshopDetails>, BTreeSet<String>, bool) {
    let mut fresh = BTreeMap::new();
    let mut gone = BTreeSet::new();
    let mut complete = true;
    for chunk in ids.chunks(PER_REQUEST) {
        let mut body = format!("itemcount={}", chunk.len());
        for (i, item) in chunk.iter().enumerate() {
            body.push_str(&format!("&publishedfileids%5B{i}%5D={item}"));
        }
        match http
            .post(
                DETAILS_URL,
                "application/x-www-form-urlencoded",
                body.as_bytes(),
            )
            .ok()
            .and_then(|b| read_details(&b))
        {
            Some((d, g)) => {
                fresh.extend(d);
                gone.extend(g);
            }
            None => complete = false,
        }
    }
    (fresh, gone, complete)
}

/// A page of the game's Workshop, as its browse page lists it.
pub fn browse(http: &dyn Http, app_id: u32, q: &WorkshopQuery) -> Result<WorkshopPage> {
    let url = browse_url(app_id, q);
    let body = http.get(&url).map_err(|e| {
        if e.status == Some(429) {
            Site::SteamWorkshop.rate_limited()
        } else {
            Site::SteamWorkshop.net(e)
        }
    })?;
    read_browse(&String::from_utf8_lossy(&body))
        .ok_or_else(|| Site::SteamWorkshop.data("browse page", "not as UMM expects"))
}

/// An item's page on Steam's community site.
pub fn item_web_page(item: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={item}")
}

/// The game's Workshop on Steam's community site.
pub fn workshop_web_page(app_id: u32) -> String {
    format!("https://steamcommunity.com/app/{app_id}/workshop/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_users_subscriptions() {
        let users = "\"users\"\n{\n\t\"76561197960265729\"\n\t{\n\t\t\"MostRecent\"\t\t\"0\"\n\t}\n\t\"76561197996525245\"\n\t{\n\t\t\"AccountName\"\t\t\"a\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n";
        assert_eq!(recent_account(users), Some(36_259_517));
        let one = "\"users\"\n{\n\t\"76561197960265730\"\n\t{\n\t}\n}\n";
        assert_eq!(recent_account(one), Some(2));
        // As Steam writes it for Crusader Kings II, which has no other record.
        let subs = "\"subscribedfiles\"\n{\n\t\"appid\"\t\t\"203770\"\n\t\"time_last_updated\"\t\t\"1795767108\"\n\t\"0\"\n\t{\n\t\t\"publishedfileid\"\t\t\"608738995\"\n\t\t\"time_subscribed\"\t\t\"1795767183\"\n\t\t\"disabled_locally\"\t\t\"0\"\n\t}\n}\n";
        let subs = read_subscriptions(subs);
        assert_eq!(subs.as_deref(), Some(&["608738995".to_string()][..]));
        let record = with_subscriptions(Vec::new(), subs);
        assert_eq!(record.len(), 1);
        assert!(record[0].1.subscribed && !record[0].1.downloaded);
        // The list says sooner than the record: one unsubscribed from, one just subscribed to.
        let old = vec![(
            "1".to_string(),
            Recorded {
                subscribed: true,
                downloaded: true,
                ..Default::default()
            },
        )];
        let record = with_subscriptions(old, Some(vec!["2".into()]));
        assert_eq!(record.len(), 2);
        assert!(!record[0].1.subscribed && record[0].1.downloaded);
        assert!(record[1].1.subscribed);
        // No list: the record as it is.
        assert!(with_subscriptions(Vec::new(), None).is_empty());
    }

    #[test]
    fn reads_the_workshops_browse_page() {
        // As Steam's page embeds it: a JSON document, with others nested as strings.
        let queries = serde_json::json!({ "queries": [
            { "queryKey": ["declared_tags_v6", 1], "state": { "data": { "readytouse_tags": [
                { "name": "Mods", "tags": [{ "name": "Character" }, { "name": "Stage" }] },
                { "name": "Style", "tags": [{ "name": "Style" }, { "name": "Character" }] }
            ] } } },
            { "queryKey": ["workshop_browse"], "state": { "data": {
                "current_page": 2, "total_pages": 9, "total_count": 250,
                "results": [
                    { "publishedfileid": "42", "creator": "7656", "title": "Wing \"Gaster\"",
                      "short_description": "It's me", "preview_url": "https://images.steamusercontent.com/ugc/1/",
                      "subscriptions": 119, "favorited": 3, "file_size": "24172",
                      "time_updated": 1782757933, "tags": [{ "tag": "Style" }] },
                    { "publishedfileid": "43", "creator": "1", "title": "No preview",
                      "preview_url": "http://insecure/", "subscriptions": "7" }
                ] } } },
            { "queryKey": ["PlayerLinkDetails", "7656"], "state": { "data": {
                "public_data": { "steamid": "7656", "persona_name": "hous" } } } }
        ] });
        let doc = serde_json::json!({
            "route": "/workshop/browse",
            "renderContext": { "queryData": queries.to_string() },
        });
        let html = format!(
            "<html><script type=\"application/json\" id=\"valve-ssr-data\" nonce=\"x\">{doc}</script></html>"
        );
        let p = read_browse(&html).unwrap();
        assert_eq!((p.page, p.total_pages, p.total), (2, 9, 250));
        assert_eq!(p.tags, ["Character", "Stage", "Style"]);
        let a = &p.items[0];
        assert_eq!(
            (
                a.id.as_str(),
                a.title.as_str(),
                a.author.as_deref(),
                a.subscriptions,
                a.file_size
            ),
            ("42", "Wing \"Gaster\"", Some("hous"), 119, 24172)
        );
        assert_eq!(a.tags, ["Style"]);
        let b = &p.items[1];
        assert_eq!(
            (
                b.author.as_deref(),
                b.preview_url.as_deref(),
                b.subscriptions
            ),
            (None, None, 7)
        );
        assert!(read_browse("<html>no data</html>").is_none());

        let q = WorkshopQuery {
            text: "mod options & more".into(),
            sort: WorkshopSort::TopRated,
            tag: Some("Texture Replacement".into()),
            page: 3,
        };
        let url = browse_url(2212330, &q);
        assert!(url.contains("appid=2212330"), "{url}");
        assert!(
            url.contains("browsesort=textsearch"),
            "searching sorts by relevance: {url}"
        );
        assert!(url.contains("searchtext=mod+options+%26+more"), "{url}");
        assert!(
            url.contains("requiredtags%5B%5D=Texture+Replacement"),
            "{url}"
        );
        assert!(url.contains("&p=3"), "{url}");
        let url = browse_url(
            1,
            &WorkshopQuery {
                sort: WorkshopSort::Recent,
                ..Default::default()
            },
        );
        assert!(
            url.contains("browsesort=mostrecent") && url.contains("&p=1"),
            "{url}"
        );
    }

    #[test]
    fn reads_steams_record_and_details() {
        let acf = r#""AppWorkshop"
{
	"appid"		"294100"
	"WorkshopItemsInstalled"
	{
		"818773962"
		{
			"size"		"1234"
			"timeupdated"		"1700000000"
			"manifest"		"1"
		}
		"2009463077"
		{
			"size"		"55"
			"timeupdated"		"1600000000"
			"manifest"		"2"
		}
	}
	"WorkshopItemDetails"
	{
		"818773962"
		{
			"manifest"		"1"
			"timeupdated"		"1700000000"
			"subscribedby"		"12345678"
		}
		"2009463077"
		{
			"manifest"		"2"
			"timeupdated"		"1600000000"
			"subscribedby"		"0"
		}
	}
}"#;
        let r = read_record(acf);
        assert_eq!(
            r.iter()
                .map(|(id, r)| (id.as_str(), r.subscribed, r.size, r.updated, r.downloaded))
                .collect::<Vec<_>>(),
            [
                ("2009463077", false, 55, 1600000000, true),
                ("818773962", true, 1234, 1700000000, true),
            ]
        );
        let body = br#"{"response":{"result":1,"resultcount":2,"publishedfiledetails":[
            {"publishedfileid":"818773962","result":1,"title":"HugsLib","description":"A library",
             "preview_url":"https://images.steamusercontent.com/ugc/1/x/","file_size":"2048",
             "time_updated":1710000000,"tags":[{"tag":"Mod"},{"tag":"1.5"}]},
            {"publishedfileid":"2009463077","result":9}]}}"#;
        let (d, gone) = read_details(body).unwrap();
        assert_eq!(d.len(), 1, "a removed item has no details");
        assert!(gone.contains("2009463077"));
        let h = &d["818773962"];
        assert_eq!(
            (h.title.as_str(), h.file_size, h.time_updated, h.tags.len()),
            ("HugsLib", 2048, 1710000000, 2)
        );
    }
}
