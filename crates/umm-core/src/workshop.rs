//! Steam Workshop items of a Steam game: the ones subscribed to on this machine and the ones
//! downloaded, as Steam records them, with their titles and pictures from Steam's public
//! `GetPublishedFileDetails` (no key needed).
//!
//! Steam downloads each item into `steamapps/workshop/content/<app id>/<item id>/` in the
//! game's library, and lists them in `steamapps/workshop/appworkshop_<app id>.acf`. UMM only
//! reads these: subscribing and browsing happen in the Steam client, which UMM opens on the
//! item's or the game's Workshop page.

use crate::{CompiledDef, CoreError, Result, Session};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::{vdf, GameDef, ParadoxList, PluginListFormat};

/// Steam's public endpoint for Workshop item details.
pub const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// Items asked about per request.
const PER_REQUEST: usize = 100;

/// Details older than this are asked for again.
const MAX_AGE_SECS: u64 = 6 * 3600;

/// A Workshop item for the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopItem {
    /// Its id on the Workshop.
    pub id: String,
    /// Subscribed to by an account on this machine.
    pub subscribed: bool,
    /// Its downloaded files, if Steam has them.
    pub folder: Option<PathBuf>,
    /// Size on disk, as Steam records it.
    pub size: u64,
    /// When the downloaded version was published (Unix time); 0 if not downloaded.
    pub updated_unix: u64,
    /// When the newest version Steam knows of was published, as it records it.
    pub latest_unix: u64,
    /// Its details from Steam, when known.
    pub details: Option<WorkshopDetails>,
}

impl WorkshopItem {
    /// Steam has a newer version than the one downloaded (it downloads it on its own).
    pub fn update_pending(&self) -> bool {
        let newest = self
            .details
            .as_ref()
            .map_or(self.latest_unix, |d| d.time_updated.max(self.latest_unix));
        self.folder.is_some() && self.updated_unix > 0 && newest > self.updated_unix
    }
}

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

/// The game's Workshop items.
#[derive(Debug, Clone)]
pub struct WorkshopList {
    pub app_id: u32,
    /// Subscribed or downloaded items, by title (else id).
    pub items: Vec<WorkshopItem>,
    /// Steam's folder of downloaded items, if the game has one.
    pub content_dir: Option<PathBuf>,
    /// Steam could not be reached: details are an older copy, or missing.
    pub stale: bool,
    /// The game downloads its items itself when it starts, into its own folder, not Steam
    /// (Steam's older Workshop: Crusader Kings II).
    pub game_downloads: bool,
}

/// Details kept between runs, by item id.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    fetched_unix: u64,
    items: BTreeMap<String, WorkshopDetails>,
    /// Items Steam does not show (removed, private): not asked about again until refreshed.
    #[serde(default)]
    gone: std::collections::BTreeSet<String>,
}

/// The `steamapps` folder holding the game's Workshop record: the one above its install
/// folder (`steamapps/common/<game>`), else any Steam library's that has one.
fn workshop_steamapps(app_id: u32, root: Option<&Path>) -> Option<PathBuf> {
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
fn subscriptions_file(app_id: u32) -> Option<PathBuf> {
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
fn read_subscriptions(text: &str) -> Option<Vec<String>> {
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
fn with_subscriptions(
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

/// A game's app id, its folder of downloaded items, and Steam's record of them.
type GameRecord = (u32, Option<PathBuf>, Vec<(String, Recorded)>);

/// An item as `appworkshop_<app>.acf` records it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Recorded {
    subscribed: bool,
    size: u64,
    updated: u64,
    latest: u64,
    downloaded: bool,
}

/// The items in `appworkshop_<app>.acf`, by id: from `WorkshopItemsInstalled` (downloaded)
/// and `WorkshopItemDetails` (`subscribedby`, `latest_timeupdated`).
fn read_record(text: &str) -> Vec<(String, Recorded)> {
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
fn read_details(
    body: &[u8],
) -> Option<(
    BTreeMap<String, WorkshopDetails>,
    std::collections::BTreeSet<String>,
)> {
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

impl Session {
    fn workshop_cache(&self, app_id: u32) -> PathBuf {
        self.data
            .root()
            .join("steam")
            .join("workshop")
            .join(format!("{app_id}.json"))
    }

    /// The game's subscribed and downloaded Workshop items, with their details (asked of
    /// Steam when missing, older than a few hours, or with `refresh`). Refused for a game
    /// not on Steam.
    /// Steam's record of the game's items, and its folder of downloaded ones: read from disk
    /// only.
    fn workshop_record(&self, def: &GameDef) -> Result<GameRecord> {
        let app_id = def.detect.steam_app_id.ok_or_else(|| {
            CoreError::Invalid(format!(
                "{} is not a Steam game (no steam_app_id)",
                def.name
            ))
        })?;
        let root = self.install_path(def)?.map(|(r, _)| r);
        let steamapps = workshop_steamapps(app_id, root.as_deref());
        let record = steamapps
            .as_ref()
            .and_then(|s| {
                fs::read_to_string(s.join("workshop").join(format!("appworkshop_{app_id}.acf")))
                    .ok()
            })
            .map(|t| read_record(&t))
            .unwrap_or_default();
        let subs = subscriptions_file(app_id)
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|t| read_subscriptions(&t));
        let record = with_subscriptions(record, subs);
        let content_dir = steamapps
            .as_ref()
            .map(|s| s.join("workshop").join("content").join(app_id.to_string()));
        Ok((app_id, content_dir, record))
    }

    /// The folder of each downloaded Workshop item of the game, by item id: from disk only,
    /// for deploys. Empty for a game not on Steam.
    pub(crate) fn workshop_folders(&self, def: &GameDef) -> BTreeMap<String, PathBuf> {
        let Ok((_, Some(content), record)) = self.workshop_record(def) else {
            return BTreeMap::new();
        };
        record
            .into_iter()
            .filter(|(_, r)| r.downloaded)
            .map(|(id, _)| (content.join(&id), id))
            .filter(|(f, _)| f.is_dir())
            .map(|(f, id)| (id, f))
            .collect()
    }

    /// Steam's files that change when one of the game's items is subscribed to, downloaded
    /// or removed: its Workshop record (`appworkshop_<app>.acf`, once Steam has one) and its
    /// user's list of subscriptions (which may not be there yet).
    pub fn workshop_record_files(&self, id: &str) -> Result<Vec<PathBuf>> {
        let def = self.def(id)?.def;
        let app_id = def
            .detect
            .steam_app_id
            .ok_or_else(|| CoreError::Invalid(format!("{} is not a Steam game", def.name)))?;
        let root = self.install_path(&def)?.map(|(r, _)| r);
        Ok(workshop_steamapps(app_id, root.as_deref())
            .map(|s| s.join("workshop").join(format!("appworkshop_{app_id}.acf")))
            .into_iter()
            .chain(subscriptions_file(app_id))
            .collect())
    }

    /// The Workshop page in Steam's community site: an item's, or the game's Workshop.
    pub fn workshop_web_page(&self, id: &str, item: Option<&str>) -> Result<String> {
        let def = self.def(id)?.def;
        let app_id = def
            .detect
            .steam_app_id
            .ok_or_else(|| CoreError::Invalid(format!("{} is not a Steam game", def.name)))?;
        match item {
            Some(i) if !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()) => Ok(format!(
                "https://steamcommunity.com/sharedfiles/filedetails/?id={i}"
            )),
            Some(i) => Err(CoreError::Invalid(format!(
                "`{i}` is not a Workshop item id"
            ))),
            None => Ok(format!("https://steamcommunity.com/app/{app_id}/workshop/")),
        }
    }

    /// The ids of the game's items Steam records as subscribed to: from disk only. `None` when
    /// Steam records none of its items (not a Steam game, or nothing known of its Workshop).
    pub(crate) fn workshop_subscribed_ids(
        &self,
        def: &GameDef,
    ) -> Option<std::collections::BTreeSet<String>> {
        let (_, _, record) = self.workshop_record(def).ok()?;
        if record.is_empty() {
            return None;
        }
        Some(
            record
                .into_iter()
                .filter(|(_, r)| r.subscribed)
                .map(|(id, _)| id)
                .collect(),
        )
    }

    /// How many of the game's Workshop items are subscribed to: from disk only. 0 for a game
    /// not on Steam.
    pub fn workshop_subscribed(&self, def: &GameDef) -> usize {
        self.workshop_record(def)
            .map(|(_, _, record)| record.iter().filter(|(_, r)| r.subscribed).count())
            .unwrap_or(0)
    }

    pub fn workshop_items(&self, id: &str, refresh: bool) -> Result<WorkshopList> {
        let compiled = self.def(id)?;
        let def = &compiled.def;
        let (app_id, content_dir, record) = self.workshop_record(def)?;
        let legacy = self.legacy_mod_folder(&compiled);

        // Details: the cached copy, completed from Steam when needed.
        let cache_file = self.workshop_cache(app_id);
        let mut cache = read_cache(&cache_file);
        let now = umm_store::fsutil::unix_now();
        let ids: Vec<&str> = record.iter().map(|r| r.0.as_str()).collect();
        // On the older Workshop, the file each item is finds what the game downloaded.
        let missing = ids.iter().any(|i| match cache.items.get(*i) {
            Some(d) => legacy.is_some() && d.filename.is_empty(),
            None => !cache.gone.contains(*i),
        });
        let old = now.saturating_sub(cache.fetched_unix) > MAX_AGE_SECS;
        let mut stale = false;
        if !ids.is_empty() && (refresh || missing || old) {
            let mut fresh = BTreeMap::new();
            let mut gone = std::collections::BTreeSet::new();
            let mut failed = false;
            for chunk in ids.chunks(PER_REQUEST) {
                let mut body = format!("itemcount={}", chunk.len());
                for (i, item) in chunk.iter().enumerate() {
                    body.push_str(&format!("&publishedfileids%5B{i}%5D={item}"));
                }
                match self
                    .http()
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
                    None => failed = true,
                }
            }
            if failed {
                stale = true;
                // What did come back is still news.
                cache.items.extend(fresh);
                cache.gone.extend(gone);
            } else {
                cache.items = fresh;
                cache.gone = gone;
                cache.fetched_unix = now;
            }
            if let Some(dir) = cache_file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = umm_store::fsutil::write_atomic(
                &cache_file,
                &serde_json::to_vec(&cache).unwrap_or_default(),
            );
        }

        let descriptors = legacy
            .as_deref()
            .map(|m| legacy_descriptors(m, &cache.items))
            .unwrap_or_default();
        let mut items: Vec<WorkshopItem> = record
            .into_iter()
            .map(|(item, r)| {
                let folder = content_dir
                    .as_ref()
                    .map(|c| c.join(&item))
                    .filter(|f| r.downloaded && f.is_dir())
                    .or_else(|| legacy.clone().filter(|_| descriptors.contains_key(&item)));
                WorkshopItem {
                    details: cache.items.get(&item).cloned(),
                    id: item,
                    subscribed: r.subscribed,
                    folder,
                    size: r.size,
                    updated_unix: r.updated,
                    latest_unix: r.latest,
                }
            })
            .collect();
        items.sort_by_key(|i| {
            (
                i.details
                    .as_ref()
                    .map_or(String::new(), |d| d.title.to_lowercase()),
                i.id.clone(),
            )
        });
        // Untitled ones last.
        items.sort_by_key(|i| i.details.is_none());
        Ok(WorkshopList {
            app_id,
            items,
            content_dir: content_dir.filter(|c| c.is_dir()),
            stale,
            game_downloads: legacy.is_some(),
        })
    }

    /// The `mod` folder a game on Steam's older Workshop downloads its items into (Crusader
    /// Kings II, in its user folder); `None` for any other game.
    fn legacy_mod_folder(&self, def: &CompiledDef) -> Option<PathBuf> {
        let px = def
            .def
            .paradox
            .as_ref()
            .filter(|p| p.list == ParadoxList::Settings)?;
        Some(self.resolve_folders(def).ok()?.get(&px.folder)?.join("mod"))
    }

    /// For a game on Steam's older Workshop: the descriptor it wrote for each item it
    /// downloaded, by item id, as its mod list names it (`mod/<name>.mod`). From disk only,
    /// with Steam's details as UMM last had them. Empty for any other game.
    pub(crate) fn workshop_descriptors(&self, def: &CompiledDef) -> BTreeMap<String, String> {
        let (Some(mods), Some(app_id)) = (self.legacy_mod_folder(def), def.def.detect.steam_app_id)
        else {
            return BTreeMap::new();
        };
        legacy_descriptors(&mods, &read_cache(&self.workshop_cache(app_id)).items)
    }
}

fn read_cache(file: &Path) -> Cache {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The descriptors in `mods` (a game's `mod` folder) of the items Steam names the files of,
/// by item id, as `mod/<name>.mod`: the one whose `archive` (or `path`) is the item's file.
/// The game names them after the file, not the item.
fn legacy_descriptors(
    mods: &Path,
    items: &BTreeMap<String, WorkshopDetails>,
) -> BTreeMap<String, String> {
    let norm = |s: &str| s.trim().trim_matches('"').replace('\\', "/").to_lowercase();
    let mut by_file = BTreeMap::new();
    for e in fs::read_dir(mods).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".mod") {
            continue;
        }
        let Ok(bytes) = fs::read(e.path()) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if matches!(k.trim().to_ascii_lowercase().as_str(), "archive" | "path") {
                by_file
                    .entry(norm(v))
                    .or_insert_with(|| format!("mod/{name}"));
            }
        }
    }
    items
        .iter()
        .filter(|(_, d)| !d.filename.is_empty())
        .filter_map(|(id, d)| Some((id.clone(), by_file.get(&norm(&d.filename))?.clone())))
        .collect()
}

/// How far UMM controls whether the game loads a Workshop item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkshopControl {
    /// The game loads the items its mod list names, and UMM writes that list: each profile
    /// turns items on or off (RimWorld, Paradox games). Items a profile does not have are left
    /// as the game has them.
    Profile,
    /// The game loads every subscribed item through Steam (or decides by itself).
    Steam,
}

/// How far UMM controls the game's Workshop items.
pub fn workshop_control(def: &GameDef) -> WorkshopControl {
    // Bannerlord loads the modules it is started with.
    if def.bannerlord.is_some() {
        return WorkshopControl::Profile;
    }
    match (&def.paradox, def.plugins.as_ref().map(|p| p.format)) {
        (Some(_), _) => WorkshopControl::Profile,
        (
            None,
            Some(
                PluginListFormat::RimworldXml
                | PluginListFormat::OniJson
                | PluginListFormat::KenshiCfg
                | PluginListFormat::XcomIni
                | PluginListFormat::TmodloaderJson,
            ),
        ) => WorkshopControl::Profile,
        _ => WorkshopControl::Steam,
    }
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
fn read_browse(html: &str) -> Option<WorkshopPage> {
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

impl Session {
    /// A page of the game's Workshop, as Steam's browse page lists it (no sign-in needed),
    /// with which items are subscribed to on this machine.
    pub fn workshop_browse(&self, id: &str, q: &WorkshopQuery) -> Result<WorkshopPage> {
        let def = self.def(id)?.def;
        let (app_id, _, record) = self.workshop_record(&def)?;
        let url = browse_url(app_id, q);
        let body = self.http().get(&url).map_err(|e| {
            CoreError::Invalid(if e.status == Some(429) {
                "Steam is limiting requests from this computer for now: try again in a minute"
                    .into()
            } else {
                format!("the Steam Workshop could not be reached: {e}")
            })
        })?;
        let mut page = read_browse(&String::from_utf8_lossy(&body)).ok_or_else(|| {
            CoreError::Invalid("the Steam Workshop's page was not as UMM expects".into())
        })?;
        for item in &mut page.items {
            item.subscribed = record.iter().any(|(i, r)| i == &item.id && r.subscribed);
        }
        Ok(page)
    }

    /// Whether the game has a Steam Workshop, as far as this machine says, without asking
    /// Steam's servers: `None` when it does not say (ask with [`Self::workshop_available`]).
    pub fn workshop_known(&self, def: &GameDef) -> Option<bool> {
        let app_id = def.detect.steam_app_id?;
        if workshop_control(def) != WorkshopControl::Steam {
            return Some(true);
        }
        if self.workshop_record(def).is_ok_and(|r| !r.2.is_empty()) {
            return Some(true);
        }
        if let Some(has) = umm_gamedef::appinfo::workshop_visible(app_id) {
            return Some(has);
        }
        let now = umm_store::fsutil::unix_now();
        read_available(&self.workshop_available_file())
            .get(&app_id.to_string())
            .filter(|(has, at)| *has || now.saturating_sub(*at) < 24 * 60 * 60)
            .map(|(has, _)| *has)
    }

    fn workshop_available_file(&self) -> PathBuf {
        self.data
            .root()
            .join("steam")
            .join("workshop")
            .join("available.json")
    }

    /// Whether the game has a Steam Workshop, `None` when that is not known (Steam could not
    /// be reached). Known when Steam has items for it or UMM manages its items, or from
    /// Steam's own record of the app on this machine; otherwise (Steam has no record of it)
    /// asked of Steam, whose Workshop page for a game without one goes to Steam's Workshop
    /// home instead, and remembered (an answer of no for a day, in case one opens).
    pub fn workshop_available(&self, id: &str) -> Result<Option<bool>> {
        let def = self.def(id)?.def;
        if let Some(has) = self.workshop_known(&def) {
            return Ok(Some(has));
        }
        let Some(app_id) = def.detect.steam_app_id else {
            return Ok(Some(false));
        };
        let file = self.workshop_available_file();
        let mut known = read_available(&file);
        let now = umm_store::fsutil::unix_now();
        let Ok(body) = self
            .http()
            .get(&browse_url(app_id, &WorkshopQuery::default()))
        else {
            return Ok(None);
        };
        // A Workshop with nothing in it is none to browse.
        let has = read_browse(&String::from_utf8_lossy(&body)).is_some_and(|p| p.total > 0);
        known.insert(app_id.to_string(), (has, now));
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ =
            umm_store::fsutil::write_atomic(&file, &serde_json::to_vec(&known).unwrap_or_default());
        Ok(Some(has))
    }
}

/// What Steam's servers said of games' Workshops: by app id, whether it has one and when.
fn read_available(file: &Path) -> BTreeMap<String, (bool, u64)> {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Opens an item's page in the Steam client, where it can be subscribed to.
pub fn item_page(item: &str) -> String {
    format!("steam://url/CommunityFilePage/{item}")
}

/// Opens the game's Workshop in the Steam client.
pub fn workshop_page(app_id: u32) -> String {
    format!("steam://url/SteamWorkshopPage/{app_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_the_game_downloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path();
        // As Crusader Kings II writes them, named after the file, not the item.
        fs::write(
            mods.join("sketchycheatmenu.mod.mod"),
            "name=\"Sketchy Cheat Menu\"\narchive=\"mod/sketchycheatmenu.zip\"\n",
        )
        .unwrap();
        fs::write(mods.join("umm_x.mod"), "name=\"x\"\npath=\"mod/x\"\n").unwrap();
        let item = |filename: &str| WorkshopDetails {
            title: String::new(),
            description: String::new(),
            preview_url: None,
            file_size: 0,
            time_updated: 0,
            time_created: 0,
            subscriptions: 0,
            favorited: 0,
            filename: filename.into(),
            tags: Vec::new(),
        };
        let items = BTreeMap::from([
            ("608738995".to_string(), item("mod/SketchyCheatMenu.zip")),
            ("2".to_string(), item("mod/elsewhere.zip")),
            ("3".to_string(), item("")),
        ]);
        assert_eq!(
            legacy_descriptors(mods, &items),
            BTreeMap::from([(
                "608738995".to_string(),
                "mod/sketchycheatmenu.mod.mod".to_string()
            )])
        );
    }

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
