//! GameBanana (`gamebanana.com`): its public API (`/apiv11`) for finding a game, browsing and
//! searching its mods, and a mod's files. None of it needs an account, and files download
//! straight from their links, each with an MD5 checksum to check it by.
//!
//! The API's names are GameBanana's own: `_idRow` for an id, `_s…` strings, `_n…` numbers,
//! `_ts…` Unix times, `_a…` objects and lists, `_b…` flags.

use serde_json::Value;
use umm_net::Http;

/// The website, for links shown to the user.
pub const SITE_URL: &str = "https://gamebanana.com";
const API_URL: &str = "https://gamebanana.com/apiv11";

/// The most mods the API gives in one page.
pub const MAX_PER_PAGE: u32 = 50;

#[derive(Debug, thiserror::Error)]
pub enum GameBananaError {
    #[error("GameBanana has no {0}")]
    NotFound(String),
    #[error("GameBanana says UMM made too many requests; try again in a few minutes")]
    RateLimited,
    #[error("could not reach GameBanana ({url}): {message}")]
    Network { url: String, message: String },
    #[error("unexpected data from GameBanana ({what}): {message}")]
    Data { what: String, message: String },
}

pub type Result<T, E = GameBananaError> = std::result::Result<T, E>;

/// A game's page on the website.
pub fn game_url(game_id: u64) -> String {
    format!("{SITE_URL}/games/{game_id}")
}

/// A mod's page on the website.
pub fn mod_url(mod_id: u64) -> String {
    format!("{SITE_URL}/mods/{mod_id}")
}

/// Reads a mod page link (`https://gamebanana.com/mods/<id>`, with or without `www.`, a
/// query, a fragment or a trailing name): the mod's id.
pub fn parse_mod_url(url: &str) -> Option<u64> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("gamebanana.com/")?;
    let path = rest.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        ["mods", id, ..] => id.parse().ok().filter(|&n| n > 0),
        _ => None,
    }
}

/// A game on GameBanana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub icon_url: Option<String>,
    /// How many mods it has, when the API says (a game's own page does; search does not).
    pub mod_count: Option<u64>,
}

/// One mod in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSummary {
    pub mod_id: u64,
    pub name: String,
    pub author: Option<String>,
    pub version: String,
    pub category: String,
    pub thumbnail_url: Option<String>,
    pub likes: u64,
    pub views: u64,
    /// Unix time of its last change.
    pub updated: u64,
    /// It carries content ratings (mature themes, violence, …).
    pub rated: bool,
    /// Marked obsolete by its author.
    pub obsolete: bool,
    pub has_files: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPage {
    pub total: u64,
    /// No further pages.
    pub complete: bool,
    pub mods: Vec<ModSummary>,
}

/// One file of a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModFile {
    pub file_id: u64,
    /// The archive's name (`mymod_v2.zip`).
    pub name: String,
    /// What the author says it is ("v1.4.0 (Mac)"); often empty.
    pub description: String,
    pub version: Option<String>,
    pub size: u64,
    /// Unix time it was added.
    pub added: u64,
    pub downloads: u64,
    pub download_url: String,
    /// Lower-case hex.
    pub md5: String,
    /// Its antivirus scan came back clean (`None` while pending or unknown).
    pub clean: Option<bool>,
    /// Programs in it, as GameBanana's analysis lists them.
    pub contains_exe: Vec<String>,
    /// The platform it is for, when its name or description says one other than Windows
    /// ("v1.2.0 (Mac)", `mymod_linux.zip`).
    pub platform: Option<&'static str>,
}

/// The platform a file is for, from words in its name or description: `Mac`, `Linux`,
/// `Android` or `Switch`; `None` for Windows or when nothing says.
pub fn platform_of(name: &str, description: &str) -> Option<&'static str> {
    let text = format!("{name} {description}").to_lowercase();
    if text.ends_with(".apk") || name.to_lowercase().ends_with(".apk") {
        return Some("Android");
    }
    let words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |ws: &[&str]| words.iter().any(|w| ws.contains(w));
    if has(&["windows", "win", "win64", "pc"]) {
        return None;
    }
    if has(&["mac", "macos", "osx", "macosx"]) {
        Some("Mac")
    } else if has(&["linux", "steamdeck", "deck"]) {
        Some("Linux")
    } else if has(&["android", "apk"]) {
        Some("Android")
    } else if has(&["switch", "nsp"]) {
        Some("Switch")
    } else {
        None
    }
}

/// A mod with what its page shows: its description and files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModDetails {
    pub summary: ModSummary,
    pub game_id: u64,
    pub game_name: String,
    /// The description, as plain text.
    pub text: String,
    pub downloads: u64,
    pub files: Vec<ModFile>,
}

/// How to order a game's mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Downloads,
    Likes,
    Updated,
    Newest,
    Name,
}

impl Sort {
    /// The API's name for it, when listing a game's mods.
    fn alias(self) -> &'static str {
        match self {
            Sort::Downloads => "Generic_MostDownloaded",
            Sort::Likes => "Generic_MostLiked",
            Sort::Updated => "Generic_LatestUpdated",
            Sort::Newest => "Generic_Newest",
            Sort::Name => "Generic_Alphabetically",
        }
    }

    /// The order of text search results, which has only two: by popularity, or by how well
    /// they match.
    fn search_order(self) -> &'static str {
        match self {
            Sort::Downloads | Sort::Likes => "popularity",
            _ => "best_match",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    pub text: String,
    pub sort: Sort,
    /// From 1.
    pub page: u32,
    pub per_page: u32,
}

/// Percent-encodes a query value.
fn query_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Turns a description's HTML into plain text: line breaks and paragraphs kept, tags dropped,
/// the common entities decoded.
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            rest = &rest[i..];
            break;
        };
        let tag = rest[i + 1..i + j]
            .trim_start_matches('/')
            .to_ascii_lowercase();
        let name = tag.split([' ', '/']).next().unwrap_or("");
        if matches!(
            name,
            "br" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "tr"
        ) {
            out.push('\n');
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut lines: Vec<&str> = decoded.lines().map(str::trim_end).collect();
    // No runs of blank lines.
    lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
    lines.join("\n").trim().to_string()
}

fn u(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A record's first preview image, small.
fn thumbnail(v: &Value) -> Option<String> {
    let img = v
        .get("_aPreviewMedia")?
        .get("_aImages")?
        .as_array()?
        .first()?;
    let base = img.get("_sBaseUrl")?.as_str()?;
    let file = ["_sFile220", "_sFile100", "_sFile"]
        .iter()
        .find_map(|k| img.get(*k).and_then(Value::as_str))?;
    Some(format!("{base}/{file}"))
}

fn summary(v: &Value) -> ModSummary {
    let category = v
        .get("_aRootCategory")
        .or_else(|| v.get("_aCategory"))
        .map(|c| s(c, "_sName"))
        .unwrap_or_default();
    ModSummary {
        mod_id: u(v, "_idRow"),
        name: s(v, "_sName"),
        author: v
            .get("_aSubmitter")
            .map(|a| s(a, "_sName"))
            .filter(|n| !n.is_empty()),
        version: s(v, "_sVersion"),
        category,
        thumbnail_url: thumbnail(v),
        likes: u(v, "_nLikeCount"),
        views: u(v, "_nViewCount"),
        updated: u(v, "_tsDateModified").max(u(v, "_tsDateUpdated")),
        rated: b(v, "_bHasContentRatings")
            || v.get("_aContentRatings").is_some_and(|r| r.is_object()),
        obsolete: b(v, "_bIsObsolete"),
        has_files: v.get("_bHasFiles").and_then(Value::as_bool).unwrap_or(true),
    }
}

fn file(v: &Value) -> ModFile {
    let (name, description) = (s(v, "_sFile"), s(v, "_sDescription"));
    ModFile {
        platform: platform_of(&name, &description),
        file_id: u(v, "_idRow"),
        name,
        description,
        version: Some(s(v, "_sVersion")).filter(|x| !x.is_empty()),
        size: u(v, "_nFilesize"),
        added: u(v, "_tsDateAdded"),
        downloads: u(v, "_nDownloadCount"),
        download_url: s(v, "_sDownloadUrl"),
        md5: s(v, "_sMd5Checksum").to_ascii_lowercase(),
        clean: match v.get("_sAvResult").and_then(Value::as_str) {
            Some("clean") => Some(true),
            Some("") | None => None,
            Some(_) => Some(false),
        },
        contains_exe: v
            .get("_aAnalysisWarnings")
            .and_then(|w| w.get("contains_exe"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The API, through UMM's HTTP client.
pub struct Api<'a> {
    http: &'a dyn Http,
}

impl<'a> Api<'a> {
    pub fn new(http: &'a dyn Http) -> Self {
        Api { http }
    }

    fn get(&self, path: &str, what: &str) -> Result<Value> {
        let url = format!("{API_URL}/{path}");
        let body = self.http.get(&url).map_err(|e| match e.status {
            Some(429) => GameBananaError::RateLimited,
            Some(404) => GameBananaError::NotFound(what.to_string()),
            _ => GameBananaError::Network {
                url: e.url,
                message: e.message,
            },
        })?;
        let v: Value = serde_json::from_slice(&body).map_err(|e| GameBananaError::Data {
            what: what.to_string(),
            message: e.to_string(),
        })?;
        if let Some(code) = v.get("_sErrorCode").and_then(Value::as_str) {
            return Err(match code {
                "NOT_FOUND" | "ROW_NOT_FOUND" => GameBananaError::NotFound(what.to_string()),
                _ => GameBananaError::Data {
                    what: what.to_string(),
                    message: format!("{code}: {}", v.get("_aErrorData").unwrap_or(&Value::Null)),
                },
            });
        }
        Ok(v)
    }

    fn page(v: &Value) -> (u64, bool, Vec<Value>) {
        let meta = v.get("_aMetadata");
        let total = meta.map_or(0, |m| u(m, "_nRecordCount"));
        let complete = meta
            .and_then(|m| m.get("_bIsComplete"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let records = v
            .get("_aRecords")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        (total, complete, records)
    }

    /// Games whose name matches `name`, best matches first.
    pub fn find_games(&self, name: &str) -> Result<Vec<Game>> {
        let v = self.get(
            &format!(
                "Util/Search/Results?_sModelName=Game&_sOrder=best_match&_sSearchString={}&_nPage=1",
                query_escape(name.trim())
            ),
            "game search",
        )?;
        let (_, _, records) = Self::page(&v);
        Ok(records
            .iter()
            .map(|r| Game {
                id: u(r, "_idRow"),
                name: s(r, "_sName"),
                icon_url: r
                    .get("_aPreviewMedia")
                    .and_then(|m| m.get("_aImages"))
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|i| s(i, "_sType") == "icon"))
                    .map(|i| s(i, "_sUrl")),
                mod_count: None,
            })
            .filter(|g| g.id != 0)
            .collect())
    }

    /// The game with this id, with how many mods it has.
    pub fn game(&self, game_id: u64) -> Result<Game> {
        let v = self.get(
            &format!("Game/{game_id}/ProfilePage"),
            &format!("game {game_id}"),
        )?;
        let mods = v
            .get("_aModRootCategories")
            .and_then(Value::as_array)
            .map(|cats| cats.iter().map(|c| u(c, "_nItemCount")).sum());
        let icon = v
            .get("_aPreviewMedia")
            .and_then(|m| m.get("_aImages"))
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|i| s(i, "_sType") == "icon"))
            .map(|i| s(i, "_sUrl"));
        Ok(Game {
            id: u(&v, "_idRow"),
            name: s(&v, "_sName"),
            icon_url: icon,
            mod_count: mods,
        })
    }

    /// A page of the game's mods: all of them in the order asked, or those matching the
    /// search text (by popularity or match only: the API's search has no other orders).
    pub fn mods(&self, game_id: u64, q: &Search) -> Result<ModPage> {
        let page = q.page.max(1);
        let per = q.per_page.clamp(1, MAX_PER_PAGE);
        let text = q.text.trim();
        let path = if text.is_empty() {
            format!(
                "Mod/Index?_nPerpage={per}&_aFilters%5BGeneric_Game%5D={game_id}&_sSort={}&_nPage={page}",
                q.sort.alias()
            )
        } else {
            format!(
                "Util/Search/Results?_sModelName=Mod&_idGameRow={game_id}&_sOrder={}&_sSearchString={}&_nPerpage={per}&_nPage={page}",
                q.sort.search_order(),
                query_escape(text)
            )
        };
        let v = self.get(&path, "mod list")?;
        let (total, complete, records) = Self::page(&v);
        Ok(ModPage {
            total,
            complete,
            mods: records
                .iter()
                .filter(|r| {
                    r.get("_sModelName")
                        .and_then(Value::as_str)
                        .unwrap_or("Mod")
                        == "Mod"
                })
                .map(summary)
                .filter(|m| m.mod_id != 0)
                .collect(),
        })
    }

    /// A mod's page: its description and files, newest first.
    pub fn mod_details(&self, mod_id: u64) -> Result<ModDetails> {
        let v = self.get(
            &format!("Mod/{mod_id}/ProfilePage"),
            &format!("mod {mod_id}"),
        )?;
        if b(&v, "_bIsTrashed") || b(&v, "_bIsPrivate") {
            return Err(GameBananaError::NotFound(format!("mod {mod_id}")));
        }
        let mut files: Vec<ModFile> = v
            .get("_aFiles")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(file).filter(|f| f.file_id != 0).collect())
            .unwrap_or_default();
        files.sort_by(|a, b| b.added.cmp(&a.added).then(b.file_id.cmp(&a.file_id)));
        let game = v.get("_aGame");
        Ok(ModDetails {
            summary: summary(&v),
            game_id: game.map_or(0, |g| u(g, "_idRow")),
            game_name: game.map(|g| s(g, "_sName")).unwrap_or_default(),
            text: plain_text(&s(&v, "_sText")),
            downloads: u(&v, "_nDownloadCount"),
            files,
        })
    }
}

/// A file's name without its version numbers and the random suffixes GameBanana gives
/// uploads (`_f454e`), which successive uploads of the same file share (`my_mod_v140.zip`
/// and `my_mod_v141_9930e.zip`; not `my_mod_v140_mac.zip`).
pub fn file_line(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let (mut stem, ext) = match lower.rsplit_once('.') {
        Some((s, e)) if !e.is_empty() && e.len() <= 4 => (s.to_string(), format!(".{e}")),
        _ => (lower.clone(), String::new()),
    };
    while let Some((rest, tail)) = stem.rsplit_once('_') {
        if tail.len() == 5 && tail.bytes().all(|b| b.is_ascii_hexdigit()) && !rest.is_empty() {
            stem = rest.to_string();
        } else {
            break;
        }
    }
    let mut out = String::new();
    for c in format!("{stem}{ext}").chars() {
        if c.is_ascii_digit() {
            continue;
        }
        // `v1.4.0` → `v`; the dots between numbers go too.
        if c == '.' && out.ends_with(['v', '.', '_', '-']) {
            continue;
        }
        out.push(c);
    }
    out
}

/// The newest file of the same line as `installed` (see [`file_line`]) added after it, for
/// an update.
pub fn newer_in_line<'f>(files: &'f [ModFile], installed: &ModFile) -> Option<&'f ModFile> {
    let line = file_line(&installed.name);
    files
        .iter()
        .filter(|f| f.file_id != installed.file_id && f.added > installed.added)
        .filter(|f| file_line(&f.name) == line)
        .max_by_key(|f| (f.added, f.file_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_mod_links() {
        assert_eq!(
            parse_mod_url("https://gamebanana.com/mods/428457"),
            Some(428457)
        );
        assert_eq!(
            parse_mod_url("https://www.gamebanana.com/mods/428457?tab=files#x"),
            Some(428457)
        );
        assert_eq!(parse_mod_url("https://gamebanana.com/mods/abc"), None);
        assert_eq!(parse_mod_url("https://gamebanana.com/games/5506"), None);
        assert_eq!(parse_mod_url("https://evil.example/mods/1"), None);
        assert_eq!(parse_mod_url("gamebanana.com/mods/1"), None);
    }

    #[test]
    fn descriptions_become_plain_text() {
        assert_eq!(
            plain_text("Use <b>Gaster</b> Blasters<br><br>patch 1.1:<br>- sound &amp; more"),
            "Use Gaster Blasters\n\npatch 1.1:\n- sound & more"
        );
    }

    #[test]
    fn files_of_one_line_are_told_apart_by_their_numbers_only() {
        assert_eq!(
            file_line("undertale_-_just_have_fun_v140.zip"),
            file_line("undertale_-_just_have_fun_v130.zip")
        );
        assert_ne!(
            file_line("undertale_-_just_have_fun_v140.zip"),
            file_line("undertale_-_just_have_fun_v120_mac.zip")
        );
        // GameBanana's random suffixes.
        assert_eq!(
            file_line("ut60fps_98fa8.zip"),
            file_line("ut60fps_1c2d3.zip")
        );
        assert_eq!(
            file_line("undertale_gaster_blaster_67b0c_fde95.zip"),
            file_line("undertale_gaster_blaster.zip")
        );
        assert_ne!(file_line("spells_9930e.zip"), file_line("magic_9930e.zip"));
        let f = |id, name: &str, added| ModFile {
            file_id: id,
            name: name.into(),
            description: String::new(),
            version: None,
            size: 1,
            added,
            downloads: 0,
            download_url: String::new(),
            md5: String::new(),
            clean: None,
            contains_exe: vec![],
            platform: None,
        };
        let files = [
            f(3, "fun_v140.zip", 30),
            f(4, "fun_v140_mac.zip", 40),
            f(2, "fun_v130.zip", 20),
        ];
        assert_eq!(newer_in_line(&files, &files[2]).map(|x| x.file_id), Some(3));
        assert_eq!(newer_in_line(&files, &files[0]), None);
    }

    #[test]
    fn files_for_other_platforms_are_told() {
        assert_eq!(platform_of("fun_v120_mac.zip", "v1.2.0 (Mac)"), Some("Mac"));
        assert_eq!(platform_of("fun_linux.zip", ""), Some("Linux"));
        assert_eq!(platform_of("fun.apk", ""), Some("Android"));
        assert_eq!(platform_of("fun_v140.zip", "v1.4.0"), None);
        assert_eq!(platform_of("fun.zip", "Windows and Mac"), None);
        assert_eq!(platform_of("machine.zip", "Macro pack"), None);
    }

    #[test]
    fn query_values_are_escaped() {
        assert_eq!(query_escape("sans & papyrus"), "sans+%26+papyrus");
    }
}
