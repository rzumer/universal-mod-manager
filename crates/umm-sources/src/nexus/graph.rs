//! Nexus Mods' GraphQL API (`https://api.nexusmods.com/v2/graphql`): searching a game's
//! mods, a mod's files, and finding a file by its MD5 hash. None of this needs an account,
//! so free and signed-out users can browse. Values always go as GraphQL variables, never
//! spliced into the query text.

use crate::net::Http;
use crate::nexus::APP_NAME;
use crate::nexus::{Result, SourceError};
use crate::Site;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const GRAPHQL_URL: &str = "https://api.nexusmods.com/v2/graphql";

/// A game on Nexus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub domain_name: String,
    #[serde(default)]
    pub mod_count: Option<u64>,
}

/// A mod, as search lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSummary {
    pub mod_id: u64,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub endorsements: u64,
    #[serde(default)]
    pub downloads: u64,
    /// ISO 8601.
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub adult_content: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModPage {
    pub total: u64,
    pub mods: Vec<ModSummary>,
}

/// One file of a mod. Versions of the same file share a `group_id`, even when renamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    pub file_id: u64,
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// `MAIN`, `UPDATE`, `OPTIONAL`, `MISCELLANEOUS`, `OLD_VERSION`, `ARCHIVED`, `REMOVED`.
    pub category: String,
    /// Upload time, Unix seconds.
    #[serde(default)]
    pub date: u64,
    #[serde(default, deserialize_with = "big_int")]
    pub size_in_bytes: Option<u64>,
    /// The archive's name for older uploads (`Glow-2400-1-2-1700000000.zip`); newer ones
    /// have a storage path instead.
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub group_id: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ModFile {
    /// Replaced, archived or removed by its author.
    pub fn is_old(&self) -> bool {
        matches!(
            self.category.as_str(),
            "OLD_VERSION" | "ARCHIVED" | "REMOVED"
        )
    }
}

/// `BigInt` values come as strings.
fn big_int<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(s)) => s.parse().ok(),
        Some(Value::Number(n)) => n.as_u64(),
        _ => None,
    })
}

/// Another mod a mod needs, as its author listed it. Requirements name a mod, never a file
/// or version; some are hosted elsewhere (`external`, with only a link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// The required mod on Nexus; `None` for one hosted elsewhere.
    pub mod_id: Option<u64>,
    pub game_id: Option<u64>,
    pub name: String,
    pub notes: Option<String>,
    /// For a requirement hosted elsewhere, its page.
    pub url: Option<String>,
    pub external: bool,
}

/// A file found by its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashMatch {
    pub domain: String,
    pub game_id: u64,
    pub mod_id: u64,
    pub mod_name: String,
    pub file: ModFile,
}

/// How to order search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Endorsements,
    Downloads,
    Updated,
    Name,
}

/// A collection, as a game's list of them shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSummary {
    /// Its id in URLs (`nexusmods.com/games/<domain>/collections/<slug>`).
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub author: Option<String>,
    pub endorsements: u64,
    pub downloads: u64,
    pub tile: Option<String>,
    pub adult_content: bool,
    /// Its latest published revision, and that revision's mod count and size.
    pub revision: Option<u64>,
    pub mod_count: u64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectionPage {
    pub total: u64,
    pub collections: Vec<CollectionSummary>,
}

/// A revision of a collection: the mod files it lists, in its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub slug: String,
    pub name: String,
    pub revision: u64,
    /// The game's Nexus domain.
    pub domain: String,
    pub files: Vec<CollectionFile>,
}

/// One mod file a collection lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionFile {
    pub mod_id: u64,
    pub file_id: u64,
    pub optional: bool,
    /// The mod's name, and the file's.
    pub mod_name: String,
    pub file_name: String,
    pub version: String,
    pub size: Option<u64>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Search {
    pub text: String,
    pub sort: Sort,
    pub offset: u64,
    pub count: u64,
    pub include_adult: bool,
}

pub struct Graph<'a> {
    http: &'a dyn Http,
    version: &'a str,
}

impl<'a> Graph<'a> {
    /// `version`: UMM's version, sent as `Application-Version`.
    pub fn new(http: &'a dyn Http, version: &'a str) -> Self {
        Graph { http, version }
    }

    fn query<T: DeserializeOwned>(&self, query: &str, variables: Value, what: &str) -> Result<T> {
        let body = json!({ "query": query, "variables": variables }).to_string();
        let resp = self
            .http
            .post_with(
                GRAPHQL_URL,
                "application/json",
                body.as_bytes(),
                &[
                    ("Application-Name", APP_NAME),
                    ("Application-Version", self.version),
                    ("Accept", "application/json"),
                ],
            )
            .map_err(|e| match e.status {
                Some(429) => Site::Nexus.rate_limited(),
                _ => SourceError::Network {
                    site: Site::Nexus,
                    url: e.url,
                    message: e.message,
                },
            })?;
        let bad = |m: String| SourceError::Data {
            site: Site::Nexus,
            what: what.to_string(),
            message: m,
        };
        let v: Value = serde_json::from_slice(&resp).map_err(|e| bad(e.to_string()))?;
        if let Some(errs) = v
            .get("errors")
            .and_then(Value::as_array)
            .filter(|e| !e.is_empty())
        {
            let msg = errs[0]
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error");
            return Err(bad(msg.to_string()));
        }
        serde_json::from_value(v.get("data").cloned().unwrap_or(Value::Null))
            .map_err(|e| bad(e.to_string()))
    }

    /// The game with this domain name.
    pub fn game(&self, domain: &str) -> Result<Game> {
        #[derive(Deserialize)]
        struct D {
            game: Option<Game>,
        }
        let d: D = self.query(
            "query($d: String) { game(domainName: $d) { id name domainName modCount } }",
            json!({ "d": domain }),
            "game",
        )?;
        d.game
            .ok_or_else(|| Site::Nexus.not_found(format!("game `{domain}`")))
    }

    /// Games whose name contains `name`.
    pub fn find_games(&self, name: &str) -> Result<Vec<Game>> {
        #[derive(Deserialize)]
        struct Nodes {
            nodes: Vec<Game>,
        }
        #[derive(Deserialize)]
        struct D {
            games: Nodes,
        }
        let d: D = self.query(
            "query($f: GamesSearchFilter) { games(filter: $f, count: 20) { nodes { id name domainName modCount } } }",
            json!({ "f": { "name": [{ "value": name, "op": "WILDCARD" }] } }),
            "games",
        )?;
        Ok(d.games.nodes)
    }

    /// A page of a game's mods.
    pub fn search(&self, domain: &str, s: &Search) -> Result<ModPage> {
        let mut filter = json!({ "gameDomainName": [{ "value": domain, "op": "EQUALS" }] });
        let text = s.text.trim();
        if !text.is_empty() {
            filter["name"] = json!([{ "value": text, "op": "WILDCARD" }]);
        }
        if !s.include_adult {
            filter["adultContent"] = json!([{ "value": false }]);
        }
        let sort = match s.sort {
            Sort::Endorsements => json!({ "endorsements": { "direction": "DESC" } }),
            Sort::Downloads => json!({ "downloads": { "direction": "DESC" } }),
            Sort::Updated => json!({ "updatedAt": { "direction": "DESC" } }),
            Sort::Name => json!({ "name": { "direction": "ASC" } }),
        };
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            total_count: u64,
            nodes: Vec<ModSummary>,
        }
        #[derive(Deserialize)]
        struct D {
            mods: Page,
        }
        let d: D = self.query(
            "query($f: ModsFilter, $s: [ModsSort!], $o: Int, $c: Int) { mods(filter: $f, sort: $s, offset: $o, count: $c) { totalCount nodes { modId name summary author version endorsements downloads updatedAt category thumbnailUrl adultContent } } }",
            json!({ "f": filter, "s": [sort], "o": s.offset, "c": s.count.clamp(1, 80) }),
            "mod search",
        )?;
        Ok(ModPage {
            total: d.mods.total_count,
            mods: d.mods.nodes,
        })
    }

    /// A page of a game's collections. Collections cannot be sorted by name: that sorts by
    /// endorsements.
    pub fn collections(&self, domain: &str, s: &Search) -> Result<CollectionPage> {
        let mut filter = json!({ "gameDomain": [{ "value": domain, "op": "EQUALS" }] });
        let text = s.text.trim();
        if !text.is_empty() {
            filter["name"] = json!([{ "value": text, "op": "WILDCARD" }]);
        }
        if !s.include_adult {
            filter["adultContent"] = json!([{ "value": false }]);
        }
        let sort = match s.sort {
            Sort::Downloads => json!({ "downloads": { "direction": "DESC" } }),
            Sort::Updated => json!({ "updatedAt": { "direction": "DESC" } }),
            Sort::Endorsements | Sort::Name => json!({ "endorsements": { "direction": "DESC" } }),
        };
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Tile {
            thumbnail_url: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Rev {
            revision_number: u64,
            #[serde(default)]
            mod_count: u64,
            #[serde(default, deserialize_with = "big_int")]
            total_size: Option<u64>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Node {
            slug: String,
            name: String,
            #[serde(default)]
            summary: Option<String>,
            #[serde(default)]
            endorsements: u64,
            #[serde(default)]
            total_downloads: u64,
            #[serde(default)]
            adult_content: bool,
            user: Option<Named>,
            tile_image: Option<Tile>,
            latest_published_revision: Option<Rev>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            total_count: u64,
            nodes: Vec<Node>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct D {
            collections_v2: Page,
        }
        let d: D = self.query(
            "query($f: CollectionsSearchFilter, $s: [CollectionsSearchSort!], $o: Int, $c: Int) { collectionsV2(filter: $f, sort: $s, offset: $o, count: $c) { totalCount nodes { slug name summary endorsements totalDownloads adultContent user { name } tileImage { thumbnailUrl(size: small) } latestPublishedRevision { revisionNumber modCount totalSize } } } }",
            json!({ "f": filter, "s": [sort], "o": s.offset, "c": s.count.clamp(1, 80) }),
            "collection search",
        )?;
        Ok(CollectionPage {
            total: d.collections_v2.total_count,
            collections: d
                .collections_v2
                .nodes
                .into_iter()
                .map(|n| CollectionSummary {
                    slug: n.slug,
                    name: n.name,
                    summary: n.summary.unwrap_or_default(),
                    author: n.user.map(|u| u.name),
                    endorsements: n.endorsements,
                    downloads: n.total_downloads,
                    tile: n.tile_image.and_then(|t| t.thumbnail_url),
                    adult_content: n.adult_content,
                    revision: n
                        .latest_published_revision
                        .as_ref()
                        .map(|r| r.revision_number),
                    mod_count: n
                        .latest_published_revision
                        .as_ref()
                        .map_or(0, |r| r.mod_count),
                    size: n.latest_published_revision.and_then(|r| r.total_size),
                })
                .collect(),
        })
    }

    /// A revision of a collection (`None`: the latest published one), with its files.
    pub fn collection(&self, slug: &str, revision: Option<u64>) -> Result<Collection> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Domain {
            domain_name: String,
        }
        #[derive(Deserialize)]
        struct Coll {
            name: String,
            slug: String,
            game: Domain,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ModOf {
            name: String,
            thumbnail_url: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct File {
            mod_id: u64,
            name: String,
            #[serde(default)]
            version: Option<String>,
            #[serde(default, deserialize_with = "big_int")]
            size_in_bytes: Option<u64>,
            #[serde(rename = "mod")]
            of: Option<ModOf>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            file_id: u64,
            #[serde(default)]
            optional: bool,
            file: Option<File>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Rev {
            revision_number: u64,
            collection: Coll,
            #[serde(default)]
            mod_files: Vec<Entry>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct D {
            collection_revision: Option<Rev>,
        }
        let d: D = self.query(
            "query($s: String, $r: Int) { collectionRevision(slug: $s, revision: $r, viewAdultContent: true) { revisionNumber collection { name slug game { domainName } } modFiles { fileId optional file { modId name version sizeInBytes mod { name thumbnailUrl } } } } }",
            json!({ "s": slug, "r": revision }),
            "collection",
        )?;
        let r = d
            .collection_revision
            .ok_or_else(|| Site::Nexus.not_found(format!("collection {slug}")))?;
        Ok(Collection {
            slug: r.collection.slug,
            name: r.collection.name,
            revision: r.revision_number,
            domain: r.collection.game.domain_name,
            // A file Nexus no longer has (deleted by its author) has no details: left out.
            files: r
                .mod_files
                .into_iter()
                .filter_map(|e| {
                    let f = e.file?;
                    let (mod_name, thumbnail) = match f.of {
                        Some(m) => (m.name, m.thumbnail_url),
                        None => (f.name.clone(), None),
                    };
                    Some(CollectionFile {
                        mod_id: f.mod_id,
                        file_id: e.file_id,
                        optional: e.optional,
                        mod_name,
                        file_name: f.name,
                        version: f.version.unwrap_or_default(),
                        size: f.size_in_bytes,
                        thumbnail,
                    })
                })
                .collect(),
        })
    }

    /// One mod.
    pub fn mod_info(&self, game_id: u64, mod_id: u64) -> Result<ModSummary> {
        #[derive(Deserialize)]
        struct D {
            #[serde(rename = "mod")]
            m: Option<ModSummary>,
        }
        let d: D = self.query(
            "query($m: ID!, $g: ID!) { mod(modId: $m, gameId: $g) { modId name summary author version endorsements downloads updatedAt category thumbnailUrl adultContent } }",
            json!({ "m": mod_id.to_string(), "g": game_id.to_string() }),
            "mod",
        )?;
        d.m.ok_or_else(|| Site::Nexus.not_found(format!("mod {mod_id}")))
    }

    /// The other mods a mod needs.
    pub fn requirements(&self, game_id: u64, mod_id: u64) -> Result<Vec<Requirement>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct R {
            mod_id: Value,
            game_id: Value,
            mod_name: String,
            #[serde(default)]
            notes: Option<String>,
            #[serde(default)]
            url: Option<String>,
            #[serde(default)]
            external_requirement: bool,
        }
        #[derive(Deserialize)]
        struct Page {
            nodes: Vec<R>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Reqs {
            nexus_requirements: Page,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct M {
            mod_requirements: Reqs,
        }
        #[derive(Deserialize)]
        struct D {
            #[serde(rename = "mod")]
            m: Option<M>,
        }
        let d: D = self.query(
            "query($m: ID!, $g: ID!) { mod(modId: $m, gameId: $g) { modRequirements { nexusRequirements { nodes { modId gameId modName notes url externalRequirement } } } } }",
            json!({ "m": mod_id.to_string(), "g": game_id.to_string() }),
            "mod requirements",
        )?;
        // Ids are GraphQL IDs: strings, or numbers.
        let id = |v: &Value| {
            match v {
                Value::String(s) => s.parse::<u64>().ok(),
                Value::Number(n) => n.as_u64(),
                _ => None,
            }
            .filter(|&i| i > 0)
        };
        let some = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Ok(d.m
            .map(|m| m.mod_requirements.nexus_requirements.nodes)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let external = r.external_requirement || id(&r.mod_id).is_none();
                Requirement {
                    mod_id: id(&r.mod_id).filter(|_| !external),
                    game_id: id(&r.game_id),
                    name: r.mod_name,
                    notes: some(r.notes),
                    url: some(r.url),
                    external,
                }
            })
            .collect())
    }

    /// A mod's files, old ones included.
    pub fn files(&self, game_id: u64, mod_id: u64) -> Result<Vec<ModFile>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct D {
            mod_files: Vec<ModFile>,
        }
        let d: D = self.query(
            "query($m: ID!, $g: ID!) { modFiles(modId: $m, gameId: $g) { fileId name version category date sizeInBytes uri groupId description } }",
            json!({ "m": mod_id.to_string(), "g": game_id.to_string() }),
            "mod files",
        )?;
        Ok(d.mod_files)
    }

    /// Files whose archive has this MD5 hash (lower-case hex).
    pub fn by_hash(&self, md5: &str) -> Result<Vec<HashMatch>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GameRef {
            domain_name: String,
        }
        #[derive(Deserialize)]
        struct ModRef {
            name: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct F {
            mod_id: u64,
            #[serde(flatten)]
            file: ModFile,
            #[serde(rename = "mod")]
            m: ModRef,
            game: GameRef,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct H {
            game_id: u64,
            mod_file: Option<F>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct D {
            file_hash: Vec<H>,
        }
        let d: D = self.query(
            "query($h: String!) { fileHash(md5: $h) { gameId modFile { modId fileId name version category date sizeInBytes uri groupId description mod { name } game { domainName } } } }",
            json!({ "h": md5 }),
            "file hash",
        )?;
        Ok(d.file_hash
            .into_iter()
            .filter_map(|h| {
                let f = h.mod_file?;
                Some(HashMatch {
                    domain: f.game.domain_name,
                    game_id: h.game_id,
                    mod_id: f.mod_id,
                    mod_name: f.m.name,
                    file: f.file,
                })
            })
            .collect())
    }
}

/// The newest current file in the same group as `file_id`, when it is newer than that file.
/// Groups hold every version of a file, renamed or not; old, archived and removed files are
/// never offered.
pub fn newer_in_group(files: &[ModFile], file_id: u64) -> Option<&ModFile> {
    let have = files.iter().find(|f| f.file_id == file_id)?;
    let group = have.group_id?;
    files
        .iter()
        .filter(|f| f.group_id == Some(group) && !f.is_old() && f.file_id != file_id)
        .filter(|f| f.date > have.date)
        .max_by_key(|f| f.date)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::FakeHttp;

    pub const FILES: &str = r#"{"data": {"modFiles": [
      {"fileId": 365, "name": "Ultimate Edition", "version": "1.0", "category": "OLD_VERSION", "date": 100, "sizeInBytes": "15380226", "uri": "Ultimate Edition-14-1-0-100.zip", "groupId": 7},
      {"fileId": 491, "name": "DIDMM 3.0", "version": "3.0", "category": "OLD_VERSION", "date": 200, "sizeInBytes": "1", "uri": "DIDMM 3.0-14-3-0-200.zip", "groupId": 9},
      {"fileId": 507, "name": "DIDMM version 3.1", "version": "3.1", "category": "MAIN", "date": 300, "sizeInBytes": "2", "uri": "DIDMM version 3.1-14-3-1-300.zip", "groupId": 9},
      {"fileId": 733, "name": "Audio Fix", "version": "1", "category": "OPTIONAL", "date": 250, "sizeInBytes": "3", "uri": "e5/de/59/e5de594b", "groupId": 11}
    ]}}"#;

    #[test]
    fn reads_collections_and_a_revisions_files() {
        let http = FakeHttp::default()
            .with_post(
                GRAPHQL_URL,
                "collectionsV2(",
                r#"{"data": {"collectionsV2": {"totalCount": 24, "nodes": [
                  {"slug": "g14kxi", "name": "Witchcore", "summary": "Magic.", "endorsements": 703,
                   "totalDownloads": 134725, "adultContent": false, "user": {"name": "Jenna"},
                   "tileImage": {"thumbnailUrl": "https://media.nexusmods.com/t.webp"},
                   "latestPublishedRevision": {"revisionNumber": 70, "modCount": 86, "totalSize": "367177716"}}]}}}"#,
            )
            .with_post(
                GRAPHQL_URL,
                "collectionRevision(",
                r#"{"data": {"collectionRevision": {"revisionNumber": 70,
                  "collection": {"name": "Witchcore", "slug": "g14kxi", "game": {"domainName": "stardewvalley"}},
                  "modFiles": [
                    {"fileId": 99177, "optional": false, "file": {"modId": 22565, "name": "Bathhouse Hotspring", "version": "1.3.1", "sizeInBytes": "4166633",
                     "mod": {"name": "Bathhouse Hot Spring - Continued", "thumbnailUrl": "https://staticdelivery.nexusmods.com/x.png"}}},
                    {"fileId": 172381, "optional": true, "file": {"modId": 30063, "name": "Earthy Maps", "version": "1.2.0", "sizeInBytes": null, "mod": null}},
                    {"fileId": 5, "optional": false, "file": null}
                  ]}}}"#,
            );
        let g = Graph::new(&http, "0.4.0");
        let page = g
            .collections(
                "stardewvalley",
                &Search {
                    text: "witch".into(),
                    sort: Sort::Name,
                    count: 20,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 24);
        let c = &page.collections[0];
        assert_eq!(
            (
                c.slug.as_str(),
                c.author.as_deref(),
                c.revision,
                c.mod_count,
                c.size
            ),
            ("g14kxi", Some("Jenna"), Some(70), 86, Some(367177716))
        );
        let (_, body) = http.posts.lock().unwrap()[0].clone();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["variables"]["f"]["name"][0]["value"], "witch");
        assert!(
            v["variables"]["s"][0]["endorsements"].is_object(),
            "no name sort: endorsements"
        );

        let col = g.collection("g14kxi", None).unwrap();
        assert_eq!((col.revision, col.domain.as_str()), (70, "stardewvalley"));
        assert_eq!(col.files.len(), 2, "a file Nexus no longer has is left out");
        assert_eq!(col.files[0].mod_name, "Bathhouse Hot Spring - Continued");
        assert_eq!(
            (
                col.files[1].mod_name.as_str(),
                col.files[1].optional,
                col.files[1].size
            ),
            ("Earthy Maps", true, None)
        );
        let (_, body) = http.posts.lock().unwrap()[1].clone();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["variables"]["r"], Value::Null, "the latest revision");
    }

    #[test]
    fn searches_with_variables_and_reads_pages() {
        let http = FakeHttp::default().with_post(
            GRAPHQL_URL,
            "mods(filter",
            r#"{"data": {"mods": {"totalCount": 96, "nodes": [
              {"modId": 14, "name": "Mod \"menu\"", "summary": "s", "author": "A", "version": "3.1",
               "endorsements": 914, "downloads": 23762, "updatedAt": "2023-01-03T22:17:02Z",
               "category": "Miscellaneous", "thumbnailUrl": null, "adultContent": false}]}}}"#,
        );
        let g = Graph::new(&http, "0.2.0");
        let page = g
            .search(
                "deadisland",
                &Search {
                    text: "menu\" } evil {".into(),
                    count: 40,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 96);
        assert_eq!(page.mods[0].name, "Mod \"menu\"");
        let (_, body) = http.posts.lock().unwrap()[0].clone();
        let v: Value = serde_json::from_str(&body).unwrap();
        // The text stays a variable, whatever it holds.
        assert_eq!(v["variables"]["f"]["name"][0]["value"], "menu\" } evil {");
        assert_eq!(v["variables"]["f"]["adultContent"][0]["value"], false);
        assert!(!v["query"].as_str().unwrap().contains("evil"));
        let headers = http.headers.lock().unwrap()[0].1.clone();
        assert!(headers.contains(&("Application-Name".into(), APP_NAME.into())));
        assert!(!headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("apikey")));
    }

    #[test]
    fn errors_and_missing_games() {
        let http = FakeHttp::default()
            .with_post(
                GRAPHQL_URL,
                "game(domainName",
                r#"{"data": {"game": null}}"#,
            )
            .with_post(
                GRAPHQL_URL,
                "modFiles",
                r#"{"errors": [{"message": "Mod not found"}], "data": null}"#,
            );
        let g = Graph::new(&http, "0.2.0");
        assert!(matches!(g.game("nope"), Err(SourceError::NotFound { .. })));
        let err = g.files(1, 2).unwrap_err();
        assert!(err.to_string().contains("Mod not found"), "{err}");
    }

    #[test]
    fn newer_files_come_from_the_same_group() {
        let d: Value = serde_json::from_str(FILES).unwrap();
        let files: Vec<ModFile> = serde_json::from_value(d["data"]["modFiles"].clone()).unwrap();
        assert_eq!(files[0].size_in_bytes, Some(15380226));
        assert_eq!(
            newer_in_group(&files, 491).unwrap().file_id,
            507,
            "renamed, same group"
        );
        assert!(newer_in_group(&files, 507).is_none());
        assert!(
            newer_in_group(&files, 365).is_none(),
            "its group has nothing current"
        );
        assert!(newer_in_group(&files, 733).is_none());
        assert!(newer_in_group(&files, 1).is_none());
    }

    #[test]
    fn reads_requirements() {
        let http = FakeHttp::default().with_post(
            GRAPHQL_URL,
            "modRequirements",
            r#"{"data": {"mod": {"modRequirements": {"nexusRequirements": {"nodes": [
              {"modId": "2400", "gameId": "1303", "modName": "SMAPI", "notes": "", "url": "", "externalRequirement": false},
              {"modId": "0", "gameId": "1303", "modName": "Script Extender", "notes": "Get the newest", "url": "https://example.org/se", "externalRequirement": true}]}}}}}"#,
        );
        let r = Graph::new(&http, "0.2.0").requirements(1303, 1915).unwrap();
        assert_eq!(
            (
                r[0].mod_id,
                r[0].game_id,
                r[0].external,
                r[0].notes.as_deref()
            ),
            (Some(2400), Some(1303), false, None)
        );
        assert_eq!(
            (r[1].mod_id, r[1].external, r[1].url.as_deref()),
            (None, true, Some("https://example.org/se"))
        );
    }

    #[test]
    fn finds_files_by_hash() {
        let http = FakeHttp::default().with_post(
            GRAPHQL_URL,
            "fileHash",
            r#"{"data": {"fileHash": [{"gameId": 1539, "modFile": {"modId": 14, "fileId": 507,
               "name": "DIDMM version 3.1", "version": "3.1", "category": "MAIN", "date": 300,
               "sizeInBytes": "2", "uri": "x.zip", "groupId": 9,
               "mod": {"name": "Mod menu"}, "game": {"domainName": "deadisland"}}}]}}"#,
        );
        let m = Graph::new(&http, "0.2.0").by_hash("abc").unwrap();
        assert_eq!(
            (m[0].domain.as_str(), m[0].mod_id, m[0].file.file_id),
            ("deadisland", 14, 507)
        );
        assert_eq!(m[0].mod_name, "Mod menu");
    }
}
