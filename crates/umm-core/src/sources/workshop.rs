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
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::GameDef;
pub use umm_sources::workshop::{
    browse_url, item_page, workshop_page, WorkshopDetails, WorkshopFound, WorkshopPage,
    WorkshopQuery, WorkshopSort, DETAILS_URL,
};
use umm_sources::workshop::{
    content_dir, fetch_details, read_browse, read_record, read_subscriptions, record_file,
    steamapps_of, subscriptions_file, with_subscriptions, Recorded,
};

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

/// A game's app id, its folder of downloaded items, and Steam's record of them.
type GameRecord = (u32, Option<PathBuf>, Vec<(String, Recorded)>);

impl Session {
    fn workshop_cache(&self, app_id: u32) -> PathBuf {
        self.data
            .root()
            .join("steam")
            .join("workshop")
            .join(format!("{app_id}.json"))
    }

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
        let steamapps = steamapps_of(app_id, root.as_deref());
        let record = steamapps
            .as_ref()
            .and_then(|s| fs::read_to_string(record_file(s, app_id)).ok())
            .map(|t| read_record(&t))
            .unwrap_or_default();
        let subs = subscriptions_file(app_id)
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|t| read_subscriptions(&t));
        let record = with_subscriptions(record, subs);
        let content_dir = steamapps.as_ref().map(|s| content_dir(s, app_id));
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
        Ok(steamapps_of(app_id, root.as_deref())
            .map(|s| record_file(&s, app_id))
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
            Some(i) if !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()) => {
                Ok(umm_sources::workshop::item_web_page(i))
            }
            Some(i) => Err(CoreError::Invalid(format!(
                "`{i}` is not a Workshop item id"
            ))),
            None => Ok(umm_sources::workshop::workshop_web_page(app_id)),
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

    /// The game's subscribed and downloaded Workshop items, with their details (asked of
    /// Steam when missing, older than a few hours, or with `refresh`). Refused for a game
    /// not on Steam.
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
            let (fresh, gone, complete) = fetch_details(self.http(), &ids);
            if !complete {
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
            .map(|m| legacy_descriptors(def, m, &cache.items))
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

    /// The folder a game on Steam's older Workshop downloads its items into; `None` for any
    /// other game.
    fn legacy_mod_folder(&self, def: &CompiledDef) -> Option<PathBuf> {
        crate::games::integration(&def.def)?.workshop_downloads(self, def)
    }

    /// For a game on Steam's older Workshop: the descriptor it wrote for each item it
    /// downloaded, by item id, as its mod list names it (`mod/<name>.mod`). From disk only,
    /// with Steam's details as UMM last had them. Empty for any other game.
    pub(crate) fn workshop_descriptors(&self, def: &CompiledDef) -> BTreeMap<String, String> {
        let (Some(mods), Some(app_id)) = (self.legacy_mod_folder(def), def.def.detect.steam_app_id)
        else {
            return BTreeMap::new();
        };
        legacy_descriptors(
            &def.def,
            &mods,
            &read_cache(&self.workshop_cache(app_id)).items,
        )
    }
}

fn read_cache(file: &Path) -> Cache {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// What a game on Steam's older Workshop has downloaded into `folder`, by item id.
fn legacy_descriptors(
    def: &GameDef,
    folder: &Path,
    items: &BTreeMap<String, WorkshopDetails>,
) -> BTreeMap<String, String> {
    crate::games::integration(def)
        .map(|g| g.workshop_downloaded(folder, items))
        .unwrap_or_default()
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
    crate::games::integration(def).map_or(WorkshopControl::Steam, |g| g.workshop())
}

impl Session {
    /// A page of the game's Workshop, as Steam's browse page lists it (no sign-in needed),
    /// with which items are subscribed to on this machine.
    pub fn workshop_browse(&self, id: &str, q: &WorkshopQuery) -> Result<WorkshopPage> {
        let def = self.def(id)?.def;
        let (app_id, _, record) = self.workshop_record(&def)?;
        let mut page = umm_sources::workshop::browse(self.http(), app_id, q)?;
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
