//! Mods from Nexus Mods.
//!
//! Browsing needs no account: search, a mod's files, update checks and recognising an
//! archive by its hash all go through Nexus's public GraphQL API. A game is matched to its
//! Nexus game by `[nexus] domain`, or by its name.
//!
//! Downloading depends on the account. Free accounts download on the website: UMM opens the
//! file's page and picks the archive up from the Downloads folder (see `downloads.rs`),
//! checking it by hash. Premium accounts sign in with their personal API key (kept in the
//! system's credential store) and download straight from the API.
//!
//! Archives have no install rules of their own: they go through the game's, like an imported
//! archive. A newer version of an installed file (same file group on Nexus) replaces it.

use crate::{CoreError, Result, Session};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use umm_deploy::GameLock;
use umm_gamedef::GameDef;
use umm_nexus::graph::{self, Graph, ModFile, ModPage, ModSummary};
use umm_nexus::{Client, NexusError, NxmLink};
use umm_store::fsutil::{self, IoCtx};
use umm_store::{ImportOptions, ModMeta, NexusMeta};

pub use umm_nexus::graph::{Search as NexusSearch, Sort as NexusSort};

/// Name of the API key in the secret store.
const KEY_SECRET: &str = "nexus-api-key";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The signed-in account, as Nexus last described it. Kept in `nexus/account.json` (no key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NexusAccount {
    pub user_id: u64,
    pub name: String,
    pub is_premium: bool,
    pub is_supporter: bool,
}

/// The game on Nexus a UMM game matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NexusGame {
    pub domain: String,
    /// Nexus's name and id for it, when Nexus could be reached.
    pub name: Option<String>,
    pub id: Option<u64>,
    pub mod_count: Option<u64>,
    /// Found by name; not yet written into the game's `game.toml`.
    pub guessed: bool,
}

/// A file to download straight from the API (Premium, or with an `nxm://` link's key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NexusFile {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// The key and expiry from an `nxm://` link; `None` downloads as a Premium account.
    pub credentials: Option<(String, u64)>,
}

impl From<&NxmLink> for NexusFile {
    fn from(l: &NxmLink) -> Self {
        NexusFile {
            domain: l.domain.clone(),
            mod_id: l.mod_id,
            file_id: l.file_id,
            credentials: l.credentials().map(|(k, e)| (k.to_string(), e)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NexusInstallReport {
    pub meta: ModMeta,
    /// The install replaced an earlier version of this file (an update).
    pub replaced: bool,
    /// The game's `[nexus] domain` was set by this install.
    pub domain_set: bool,
}

/// One mod in search results, with what is installed of it.
#[derive(Debug, Clone)]
pub struct NexusItem {
    pub summary: ModSummary,
    /// UMM ids and versions of its files that are installed.
    pub installed: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone)]
pub struct NexusPage {
    pub game: NexusGame,
    pub total: u64,
    pub items: Vec<NexusItem>,
}

/// A mod's files, newest current ones first.
#[derive(Debug, Clone)]
pub struct NexusModFiles {
    pub domain: String,
    pub game_id: u64,
    pub summary: ModSummary,
    pub files: Vec<ModFile>,
    /// File ids installed for the game.
    pub installed: Vec<u64>,
    /// Other mods this one needs, as its author listed them.
    pub requirements: Vec<NexusRequirement>,
}

/// A mod another mod needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NexusRequirement {
    pub name: String,
    /// The required mod, when it is on Nexus for the same game.
    pub mod_id: Option<u64>,
    pub notes: Option<String>,
    /// Its page: on Nexus, or elsewhere.
    pub url: Option<String>,
    /// Hosted outside Nexus, or for another game: UMM cannot install it.
    pub elsewhere: bool,
    /// A file of it is installed for the game.
    pub installed: bool,
}

/// Whether an installed Nexus file has a newer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NexusUpdate {
    /// The mod's id in UMM.
    pub id: String,
    pub name: String,
    pub nexus: NexusMeta,
    pub state: NexusUpdateState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NexusUpdateState {
    Current,
    Newer {
        file_id: u64,
        name: String,
        version: String,
    },
    /// Its author marked it old without a newer version in the same file group.
    Outdated,
    /// No longer listed on Nexus.
    Gone,
}

/// The file a downloaded archive was meant to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedFile {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
}

/// A game name as compared with Nexus's: lower-case letters and digits only.
fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn md5_file(path: &Path) -> Result<String> {
    use md5::Digest;
    let mut f = fs::File::open(path).ctx(|| format!("opening {}", path.display()))?;
    let mut h = md5::Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .ctx(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A safe local name for an archive downloaded by UMM: the file's own name if it is a plain
/// name with an archive extension, else one made from its id.
fn download_name(file: &ModFile) -> String {
    let n = file.uri.trim();
    let plain = !n.is_empty()
        && !n.contains(['/', '\\', ':'])
        && umm_gamedef::RelPath::parse(n).is_ok_and(|r| r.depth() == 1)
        && umm_store::archive_kind(Path::new(n)).is_some();
    if plain {
        n.to_string()
    } else {
        format!("nexus-{}.zip", file.file_id)
    }
}

impl Session {
    fn nexus_dir(&self) -> PathBuf {
        self.data.root().join("nexus")
    }

    fn graph(&self) -> Graph<'_> {
        Graph::new(self.http(), VERSION)
    }

    fn nexus_key(&self) -> Result<String> {
        self.secrets
            .get(KEY_SECRET)
            .map_err(|e| CoreError::Invalid(format!("reading the Nexus Mods API key: {e}")))?
            .ok_or_else(|| {
                CoreError::Invalid(
                    "sign in to Nexus Mods with your personal API key first: Nexus needs it to download into an app".into(),
                )
            })
    }

    /// The signed-in account, without asking Nexus. `None` when signed out.
    pub fn nexus_account(&self) -> Option<NexusAccount> {
        let text = fs::read(self.nexus_dir().join("account.json")).ok()?;
        let account = serde_json::from_slice(&text).ok()?;
        // The account file outlives a key removed from the credential store by hand.
        matches!(self.secrets.get(KEY_SECRET), Ok(Some(_))).then_some(account)
    }

    /// Checks `key` with Nexus and keeps it.
    pub fn nexus_sign_in(&self, key: &str) -> Result<NexusAccount> {
        let key = key.trim();
        if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_whitespace) {
            return Err(CoreError::Invalid(
                "that does not look like a Nexus Mods API key".into(),
            ));
        }
        let user = Client::new(self.http(), key, VERSION).validate()?;
        let account = NexusAccount {
            user_id: user.user_id,
            name: user.name,
            is_premium: user.is_premium,
            is_supporter: user.is_supporter,
        };
        self.secrets
            .set(KEY_SECRET, key)
            .map_err(|e| CoreError::Invalid(format!("saving the Nexus Mods API key: {e}")))?;
        let dir = self.nexus_dir();
        fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
        let body = serde_json::to_vec_pretty(&account).expect("accounts serialize");
        fsutil::write_atomic(&dir.join("account.json"), &body)
            .ctx(|| "saving the Nexus Mods account".to_string())?;
        Ok(account)
    }

    /// Forgets the key and the account.
    pub fn nexus_sign_out(&self) -> Result<()> {
        self.secrets
            .delete(KEY_SECRET)
            .map_err(|e| CoreError::Invalid(format!("removing the Nexus Mods API key: {e}")))?;
        let _ = fs::remove_file(self.nexus_dir().join("account.json"));
        Ok(())
    }

    /// Nexus games whose name contains `name`.
    pub fn nexus_find_games(&self, name: &str) -> Result<Vec<graph::Game>> {
        Ok(self.graph().find_games(name.trim())?)
    }

    /// The Nexus game with this domain name; looked up once per session.
    fn nexus_game_info(&self, domain: &str) -> Result<graph::Game> {
        let mut cache = self.nexus_games.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(g) = cache.get(domain) {
            return Ok(g.clone());
        }
        let g = self.graph().game(domain)?;
        cache.insert(domain.to_string(), g.clone());
        Ok(g)
    }

    /// The game's Nexus game: its `[nexus] domain`, else the Nexus game with the same name.
    /// `None` when neither is known (or Nexus cannot be reached to look the name up).
    pub fn nexus_game(&self, id: &str) -> Result<Option<NexusGame>> {
        let def = self.game_def(id)?;
        if let Some(n) = &def.nexus {
            let info = self.nexus_game_info(&n.domain).ok();
            return Ok(Some(NexusGame {
                domain: n.domain.clone(),
                name: info.as_ref().map(|g| g.name.clone()),
                id: info.as_ref().map(|g| g.id),
                mod_count: info.and_then(|g| g.mod_count),
                guessed: false,
            }));
        }
        let key = name_key(&def.name);
        let found = self
            .graph()
            .find_games(&def.name)
            .unwrap_or_default()
            .into_iter()
            .find(|g| name_key(&g.name) == key);
        Ok(found.map(|g| {
            self.nexus_games
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(g.domain_name.clone(), g.clone());
            NexusGame {
                domain: g.domain_name,
                name: Some(g.name),
                id: Some(g.id),
                mod_count: g.mod_count,
                guessed: true,
            }
        }))
    }

    fn require_nexus_game(&self, id: &str) -> Result<(String, u64)> {
        let g = self
            .nexus_game(id)?
            .ok_or_else(|| CoreError::Invalid("pick this game's Nexus Mods game first".into()))?;
        let gid = match g.id {
            Some(i) => i,
            None => self.nexus_game_info(&g.domain)?.id,
        };
        Ok((g.domain, gid))
    }

    /// Sets the game's `[nexus] domain`, or removes it with `None`.
    pub fn set_nexus_domain(&self, id: &str, domain: Option<&str>) -> Result<GameDef> {
        self.edit_game_file(id, |text, origin| {
            umm_gamedef::edit::set_nexus_domain(text, origin, domain)
        })
    }

    fn nexus_mods(&self, id: &str) -> Result<Vec<ModMeta>> {
        Ok(umm_store::list_mods(&self.data.game(id))?
            .into_iter()
            .filter(|m| m.nexus.is_some())
            .collect())
    }

    /// A page of the game's mods on Nexus, marked with what is installed.
    /// With `installed_only`, only mods with a file installed for the game, looked up one by
    /// one, filtered by name and sorted here.
    pub fn nexus_search(
        &self,
        id: &str,
        q: &NexusSearch,
        installed_only: bool,
    ) -> Result<NexusPage> {
        let game = self
            .nexus_game(id)?
            .ok_or_else(|| CoreError::Invalid("pick this game's Nexus Mods game first".into()))?;
        let installed = self.nexus_mods(id)?;
        let ModPage { total, mods } = if installed_only {
            self.installed_summaries(&game, &installed, q)?
        } else {
            self.graph().search(&game.domain, q)?
        };
        let items = mods
            .into_iter()
            .map(|s| NexusItem {
                installed: installed
                    .iter()
                    .filter(|m| {
                        m.nexus
                            .as_ref()
                            .is_some_and(|n| n.domain == game.domain && n.mod_id == s.mod_id)
                    })
                    .map(|m| (m.id.clone(), m.version.clone()))
                    .collect(),
                summary: s,
            })
            .collect();
        Ok(NexusPage { game, total, items })
    }

    /// The installed Nexus mods of this game, as a search page.
    fn installed_summaries(
        &self,
        game: &NexusGame,
        installed: &[ModMeta],
        q: &NexusSearch,
    ) -> Result<ModPage> {
        let mut ids: Vec<u64> = installed
            .iter()
            .filter_map(|m| m.nexus.as_ref())
            .filter(|n| n.domain == game.domain)
            .map(|n| n.mod_id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let gid = match game.id {
            Some(g) => g,
            None => self.nexus_game_info(&game.domain)?.id,
        };
        let g = self.graph();
        let text = q.text.trim().to_lowercase();
        let mut mods: Vec<ModSummary> = ids
            .into_iter()
            .map(|mod_id| {
                // A mod hidden or removed on Nexus still shows, from what UMM stored.
                g.mod_info(gid, mod_id).unwrap_or_else(|_| {
                    let m = installed
                        .iter()
                        .find(|m| m.nexus.as_ref().is_some_and(|n| n.mod_id == mod_id))
                        .expect("listed above");
                    ModSummary {
                        mod_id,
                        name: m.name.clone(),
                        summary: String::new(),
                        author: None,
                        version: m.version.clone().unwrap_or_default(),
                        endorsements: 0,
                        downloads: 0,
                        updated_at: String::new(),
                        category: String::new(),
                        thumbnail_url: None,
                        adult_content: false,
                    }
                })
            })
            .filter(|m| text.is_empty() || m.name.to_lowercase().contains(&text))
            .filter(|m| q.include_adult || !m.adult_content)
            .collect();
        match q.sort {
            NexusSort::Endorsements => mods.sort_by_key(|m| std::cmp::Reverse(m.endorsements)),
            NexusSort::Downloads => mods.sort_by_key(|m| std::cmp::Reverse(m.downloads)),
            NexusSort::Updated => mods.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            NexusSort::Name => mods.sort_by_key(|m| m.name.to_lowercase()),
        }
        let total = mods.len() as u64;
        let mods = mods
            .into_iter()
            .skip(q.offset as usize)
            .take(q.count.max(1) as usize)
            .collect();
        Ok(ModPage { total, mods })
    }

    /// A mod's files (current ones first, newest first, then old ones) and the mods it needs.
    pub fn nexus_files(&self, id: &str, mod_id: u64) -> Result<NexusModFiles> {
        let (domain, game_id) = self.require_nexus_game(id)?;
        let mut f = self.nexus_files_in(id, &domain, game_id, mod_id)?;
        let installed: Vec<u64> = self
            .nexus_mods(id)?
            .into_iter()
            .filter_map(|m| m.nexus)
            .filter(|n| n.domain == domain)
            .map(|n| n.mod_id)
            .collect();
        // Requirements are a help, not a condition: without them, the files still show.
        f.requirements = self
            .graph()
            .requirements(game_id, mod_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.mod_id != Some(mod_id))
            .map(|r| {
                let here = r.mod_id.filter(|_| r.game_id.is_none_or(|g| g == game_id));
                NexusRequirement {
                    url: match here {
                        Some(m) => Some(umm_nexus::mod_url(&domain, m)),
                        None => r
                            .url
                            .filter(|u| u.starts_with("https://") || u.starts_with("http://")),
                    },
                    installed: here.is_some_and(|m| installed.contains(&m)),
                    elsewhere: here.is_none(),
                    mod_id: here,
                    name: r.name,
                    notes: r.notes,
                }
            })
            .collect();
        Ok(f)
    }

    fn nexus_files_in(
        &self,
        id: &str,
        domain: &str,
        game_id: u64,
        mod_id: u64,
    ) -> Result<NexusModFiles> {
        let g = self.graph();
        let summary = g.mod_info(game_id, mod_id)?;
        let mut files = g.files(game_id, mod_id)?;
        files.sort_by_key(|f| (f.is_old(), std::cmp::Reverse(f.date)));
        let installed = self
            .nexus_mods(id)?
            .into_iter()
            .filter_map(|m| m.nexus)
            .filter(|n| n.domain == domain && n.mod_id == mod_id)
            .map(|n| n.file_id)
            .collect();
        Ok(NexusModFiles {
            domain: domain.to_string(),
            game_id,
            summary,
            files,
            installed,
            requirements: Vec::new(),
        })
    }

    /// Where a free account downloads a file: its page on the website, with the download
    /// opened.
    pub fn nexus_download_page(domain: &str, mod_id: u64, file_id: u64) -> String {
        format!(
            "{}?tab=files&file_id={file_id}",
            umm_nexus::mod_url(domain, mod_id)
        )
    }

    /// The UMM mod an install of `file` goes to, and whether it replaces it: the mod that
    /// holds an earlier version of this file (same file group), else a new id from the
    /// mod's name.
    fn nexus_target(
        &self,
        id: &str,
        domain: &str,
        mod_name: &str,
        mod_id: u64,
        file: &ModFile,
        files: &[ModFile],
    ) -> Result<(String, String, bool)> {
        let mods = umm_store::list_mods(&self.data.game(id))?;
        let group_of = |fid: u64| {
            files
                .iter()
                .find(|f| f.file_id == fid)
                .and_then(|f| f.group_id)
        };
        for m in &mods {
            let Some(n) = &m.nexus else { continue };
            if n.domain != domain || n.mod_id != mod_id {
                continue;
            }
            let same_group = file.group_id.is_some() && group_of(n.file_id) == file.group_id;
            if n.file_id == file.file_id || same_group {
                return Ok((m.id.clone(), m.name.clone(), true));
            }
        }
        // The mod's main file, in any of its versions (an old one is no longer MAIN itself).
        let main_group = file.group_id.is_some()
            && files
                .iter()
                .any(|f| f.group_id == file.group_id && f.category == "MAIN");
        let main =
            file.category == "MAIN" || main_group || file.name.eq_ignore_ascii_case(mod_name);
        let name = if main {
            mod_name.to_string()
        } else {
            format!("{mod_name} - {}", file.name)
        };
        let base = umm_store::slugify(&name);
        let taken = |c: &str| mods.iter().any(|m| m.id == c);
        let id = [
            base.clone(),
            format!("{base}-{mod_id}"),
            format!("{base}-{mod_id}-{}", file.file_id),
        ]
        .into_iter()
        .find(|c| !taken(c))
        .ok_or_else(|| CoreError::Invalid(format!("a mod with the id `{base}` already exists")))?;
        Ok((id, name, false))
    }

    fn check_installable(&self, id: &str, domain: &str) -> Result<bool> {
        let def = self.game_def(id)?;
        let domain_set = match &def.nexus {
            Some(n) if n.domain != domain => {
                return Err(CoreError::Invalid(format!(
                    "{} is set up for the Nexus Mods game `{}`, not `{domain}`",
                    def.name, n.domain
                )))
            }
            Some(_) => false,
            None => true,
        };
        let game = self.data.game(id);
        game.ensure()?;
        let _lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        Ok(domain_set)
    }

    /// Stores an archive as a file of a Nexus mod and adds it to `profile`.
    #[allow(clippy::too_many_arguments)]
    fn import_nexus_archive(
        &self,
        id: &str,
        archive: &Path,
        domain: &str,
        game_id: u64,
        mod_name: &str,
        mod_id: u64,
        file: &ModFile,
        files: &[ModFile],
        profile: &str,
        domain_set: bool,
    ) -> Result<NexusInstallReport> {
        if umm_store::archive_kind(archive).is_none() {
            return Err(CoreError::Invalid(format!(
                "{} is not an archive UMM can open (.zip, .7z or .rar)",
                archive.display()
            )));
        }
        let (umm_id, name, replace) =
            self.nexus_target(id, domain, mod_name, mod_id, file, files)?;
        let meta = self.import(
            id,
            archive,
            ImportOptions {
                id: Some(umm_id),
                name: Some(name),
                version: Some(file.version.clone()).filter(|v| !v.is_empty()),
                source: Some(umm_nexus::mod_url(domain, mod_id)),
                unwrap: true,
                nexus: Some(NexusMeta {
                    domain: domain.to_string(),
                    game_id: Some(game_id),
                    mod_id,
                    file_id: file.file_id,
                    file_name: file.name.clone(),
                    version: Some(file.version.clone()).filter(|v| !v.is_empty()),
                    category: Some(file.category.clone()),
                }),
                replace,
                ..Default::default()
            },
            profile,
        )?;
        if domain_set {
            self.set_nexus_domain(id, Some(domain))?;
        }
        Ok(NexusInstallReport {
            meta,
            replaced: replace,
            domain_set,
        })
    }

    /// Installs an archive the user downloaded from Nexus. The archive is recognised by its
    /// hash; `expected` is the file they set out to download, used when Nexus does not know
    /// the hash only if `trust` is set (the user confirmed it). The archive itself is left
    /// where it is.
    pub fn nexus_install_download(
        &self,
        id: &str,
        archive: &Path,
        expected: Option<&ExpectedFile>,
        trust: bool,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<NexusInstallReport> {
        progress("Checking the file with Nexus Mods");
        let md5 = md5_file(archive)?;
        let hits = self.graph().by_hash(&md5).unwrap_or_default();
        let def = self.game_def(id)?;
        let wanted_domain = expected
            .map(|e| e.domain.clone())
            .or_else(|| def.nexus.as_ref().map(|n| n.domain.clone()));
        let hit = hits
            .iter()
            .find(|h| expected.is_some_and(|e| h.mod_id == e.mod_id && h.file.file_id == e.file_id))
            .or_else(|| {
                hits.iter()
                    .find(|h| expected.is_some_and(|e| h.mod_id == e.mod_id))
            })
            .or_else(|| {
                hits.iter()
                    .find(|h| Some(&h.domain) == wanted_domain.as_ref())
            })
            .or_else(|| hits.first());
        let (domain, game_id, mod_id, file_id) = match (hit, expected) {
            (Some(h), _) => (h.domain.clone(), h.game_id, h.mod_id, h.file.file_id),
            (None, Some(e)) if trust => {
                let gid = self.nexus_game_info(&e.domain)?.id;
                (e.domain.clone(), gid, e.mod_id, e.file_id)
            }
            (None, Some(_)) => return Err(CoreError::NexusUnrecognised),
            (None, None) => {
                return Err(CoreError::Invalid(format!(
                    "Nexus Mods does not know {}; import it as an ordinary archive instead",
                    archive.display()
                )))
            }
        };
        let domain_set = self.check_installable(id, &domain)?;
        let mf = self.nexus_files_in(id, &domain, game_id, mod_id)?;
        let file = mf
            .files
            .iter()
            .find(|f| f.file_id == file_id)
            .cloned()
            .ok_or_else(|| NexusError::NotFound(format!("file {file_id} of mod {mod_id}")))?;
        progress("Unpacking");
        self.import_nexus_archive(
            id,
            archive,
            &domain,
            game_id,
            &mf.summary.name,
            mod_id,
            &file,
            &mf.files,
            profile,
            domain_set,
        )
    }

    /// Downloads one file of a Nexus mod straight from the API and installs it. Needs a
    /// Premium account, or the key from an `nxm://` link (`file.credentials`).
    pub fn nexus_install(
        &self,
        id: &str,
        file: &NexusFile,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<NexusInstallReport> {
        if let Some((_, expires)) = &file.credentials {
            if *expires <= fsutil::unix_now() {
                return Err(NexusError::LinkExpired.into());
            }
        } else if !self.nexus_account().is_some_and(|a| a.is_premium) {
            return Err(NexusError::PremiumOnly.into());
        }
        let key = self.nexus_key()?;
        let domain_set = self.check_installable(id, &file.domain)?;
        progress("Looking up the file on Nexus Mods");
        let game_id = self.nexus_game_info(&file.domain)?.id;
        let mf = self.nexus_files_in(id, &file.domain, game_id, file.mod_id)?;
        let f = mf
            .files
            .iter()
            .find(|f| f.file_id == file.file_id)
            .cloned()
            .ok_or_else(|| {
                NexusError::NotFound(format!("file {} of mod {}", file.file_id, file.mod_id))
            })?;
        let dir = self
            .nexus_dir()
            .join("downloads")
            .join(fsutil::unique_suffix());
        fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
        let result = (|| -> Result<NexusInstallReport> {
            let client = Client::new(self.http(), &key, VERSION);
            let creds = file.credentials.as_ref().map(|(k, e)| (k.as_str(), *e));
            let links = client.download_links(&file.domain, file.mod_id, file.file_id, creds)?;
            let label = format!("Downloading {}", f.name);
            progress(&label);
            let mut last = 0u64;
            let mut dest = dir.join(download_name(&f));
            client.download(&links, &dest, &mut |done, total| {
                let step = total.map_or(5 << 20, |t| (t / 20).max(1));
                if done == 0 || done - last >= step || Some(done) == total {
                    last = done;
                    let mb = |b: u64| b as f64 / 1_048_576.0;
                    progress(&match total {
                        Some(t) => format!("{label} ({:.1} of {:.1} MB)", mb(done), mb(t)),
                        None => format!("{label} ({:.1} MB)", mb(done)),
                    });
                }
            })?;
            // Newer uploads have no file name on Nexus: name the archive by its contents.
            if umm_store::archive_kind(&dest).is_some()
                && dest.ends_with(format!("nexus-{}.zip", f.file_id))
            {
                if let Some(ext) = sniff_archive(&dest) {
                    let named = dest.with_extension(ext);
                    fs::rename(&dest, &named).ctx(|| "naming the download".to_string())?;
                    dest = named;
                }
            }
            progress("Unpacking");
            self.import_nexus_archive(
                id,
                &dest,
                &file.domain,
                game_id,
                &mf.summary.name,
                file.mod_id,
                &f,
                &mf.files,
                profile,
                domain_set,
            )
        })();
        let _ = fs::remove_dir_all(&dir);
        result
    }

    /// Games a Nexus game's files can go into: `(id, name, matches)`, where `matches` means
    /// the game is that Nexus game (set, or by name). Games set to another Nexus game are
    /// left out; the matching ones come first.
    pub fn nexus_link_games(&self, domain: &str) -> Result<Vec<(String, String, bool)>> {
        let mut out = Vec::new();
        for st in self.status()? {
            match self.nexus_game(&st.def.id).ok().flatten() {
                Some(g) if g.domain == domain => out.push((st.def.id, st.def.name, true)),
                Some(g) if !g.guessed => {}
                _ => out.push((st.def.id, st.def.name, false)),
            }
        }
        out.sort_by_key(|(_, _, m)| !*m);
        Ok(out)
    }

    /// The mod's name and the file an `nxm://` link names, for showing before installing.
    pub fn nexus_link_info(&self, link: &NxmLink) -> Result<(String, Option<ModFile>)> {
        let gid = self.nexus_game_info(&link.domain)?.id;
        let g = self.graph();
        let name = g.mod_info(gid, link.mod_id)?.name;
        let file = g
            .files(gid, link.mod_id)?
            .into_iter()
            .find(|f| f.file_id == link.file_id);
        Ok((name, file))
    }

    /// Installs from an `nxm://` link.
    pub fn nexus_install_link(
        &self,
        id: &str,
        link: &str,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<NexusInstallReport> {
        let l = NxmLink::parse(link)
            .ok_or_else(|| CoreError::Invalid(format!("not a Nexus Mods download link: {link}")))?;
        self.nexus_install(id, &NexusFile::from(&l), profile, progress)
    }

    /// Whether each Nexus file installed for the game has a newer version in its file group.
    /// One request per Nexus mod.
    pub fn nexus_updates(&self, id: &str) -> Result<Vec<NexusUpdate>> {
        let mut lists: std::collections::HashMap<(String, u64), Vec<ModFile>> = Default::default();
        let mut out = Vec::new();
        for m in self.nexus_mods(id)? {
            let n = m.nexus.clone().expect("filtered");
            let key = (n.domain.clone(), n.mod_id);
            if !lists.contains_key(&key) {
                let game_id = match n.game_id {
                    Some(g) => g,
                    None => self.nexus_game_info(&n.domain)?.id,
                };
                let files = match self.graph().files(game_id, n.mod_id) {
                    Ok(f) => f,
                    // A removed or hidden mod has no files to compare.
                    Err(NexusError::NotFound(_) | NexusError::Data { .. }) => Vec::new(),
                    Err(e) => return Err(e.into()),
                };
                lists.insert(key.clone(), files);
            }
            let files = &lists[&key];
            let state = match graph::newer_in_group(files, n.file_id) {
                Some(f) => NexusUpdateState::Newer {
                    file_id: f.file_id,
                    name: f.name.clone(),
                    version: f.version.clone(),
                },
                None => match files.iter().find(|f| f.file_id == n.file_id) {
                    None => NexusUpdateState::Gone,
                    Some(f) if f.is_old() => NexusUpdateState::Outdated,
                    Some(_) => NexusUpdateState::Current,
                },
            };
            out.push(NexusUpdate {
                id: m.id,
                name: m.name,
                nexus: n,
                state,
            });
        }
        Ok(out)
    }

    /// Installs the newest version of an installed Nexus file, replacing it (Premium).
    pub fn nexus_update(
        &self,
        id: &str,
        mod_id: &str,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<Option<NexusInstallReport>> {
        let update = self
            .nexus_updates(id)?
            .into_iter()
            .find(|u| u.id == mod_id)
            .ok_or_else(|| {
                CoreError::Invalid(format!("`{mod_id}` is not a mod from Nexus Mods"))
            })?;
        let NexusUpdateState::Newer { file_id, .. } = update.state else {
            return Ok(None);
        };
        let file = NexusFile {
            domain: update.nexus.domain.clone(),
            mod_id: update.nexus.mod_id,
            file_id,
            credentials: None,
        };
        self.nexus_install(id, &file, profile, progress).map(Some)
    }
}

/// The archive format of a file by its first bytes, as an extension.
fn sniff_archive(p: &Path) -> Option<&'static str> {
    let mut head = [0u8; 8];
    let n = fs::File::open(p).ok()?.read(&mut head).ok()?;
    let head = &head[..n];
    if head.starts_with(b"PK") {
        Some("zip")
    } else if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Some("7z")
    } else if head.starts_with(b"Rar!") {
        Some("rar")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_names_compare_loosely() {
        assert_eq!(
            name_key("Dead Island Definitive Edition"),
            name_key("dead-island definitive edition")
        );
        assert_eq!(name_key("Baldur's Gate 3"), "baldursgate3");
        assert_ne!(name_key("Skyrim"), name_key("Skyrim Special Edition"));
    }

    #[test]
    fn downloads_get_plain_names() {
        let mut f: ModFile =
            serde_json::from_str(r#"{"fileId": 7, "name": "Main", "category": "MAIN"}"#).unwrap();
        f.uri = "Glow-1-2.7z".into();
        assert_eq!(download_name(&f), "Glow-1-2.7z");
        f.uri = "../evil.rar".into();
        assert_eq!(download_name(&f), "nexus-7.zip");
        f.uri = "e5/de/59/e5de594b".into();
        assert_eq!(download_name(&f), "nexus-7.zip");
    }

    #[test]
    fn hashes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.zip");
        fs::write(&p, "abc").unwrap();
        assert_eq!(md5_file(&p).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        fs::write(&p, "7z\u{bc}").unwrap();
        assert_eq!(sniff_archive(&p), None);
        fs::write(&p, [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 4]).unwrap();
        assert_eq!(sniff_archive(&p), Some("7z"));
    }
}
