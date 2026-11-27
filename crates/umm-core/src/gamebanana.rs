//! Mods from GameBanana.
//!
//! Everything goes through GameBanana's public API, with no account: a game's mods, a mod's
//! files, and the files themselves, which download straight from their links and are checked
//! against the MD5 checksum GameBanana gives. A game is matched to its GameBanana game by
//! `[gamebanana] game`, or by its name.
//!
//! Archives have no install rules of their own: they go through the game's, like an imported
//! archive. A newer upload of an installed file (the same file name but for its version
//! numbers, added later) replaces it.

use crate::{CoreError, Result, Session};
use std::fs;
use std::path::Path;
use umm_deploy::GameLock;
use umm_gamebanana::{Api, GameBananaError, ModDetails, ModFile, ModPage, ModSummary};
use umm_gamedef::GameDef;
use umm_store::fsutil::IoCtx;
use umm_store::{GameBananaMeta, ImportOptions, ModMeta};

pub use umm_gamebanana::{Game as GbGameInfo, Search as GbSearch, Sort as GbSort};

/// The game on GameBanana a UMM game matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GbGame {
    pub game_id: u64,
    /// GameBanana's name for it, and its mod count, when GameBanana could be reached.
    pub name: Option<String>,
    pub mod_count: Option<u64>,
    /// Found by name; not yet written into the game's `game.toml`.
    pub guessed: bool,
}

/// One mod in a list, with what is installed of it.
#[derive(Debug, Clone)]
pub struct GbItem {
    pub summary: ModSummary,
    /// UMM ids and versions of its files that are installed.
    pub installed: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone)]
pub struct GbPage {
    pub game: GbGame,
    pub total: u64,
    /// No further pages.
    pub complete: bool,
    pub items: Vec<GbItem>,
}

/// A mod's page, with which of its files are installed: `(file id, UMM id)`.
#[derive(Debug, Clone)]
pub struct GbModFiles {
    pub details: ModDetails,
    pub installed: Vec<(u64, String)>,
}

#[derive(Debug, Clone)]
pub struct GbInstallReport {
    pub meta: ModMeta,
    /// The install replaced an earlier version of this file (an update).
    pub replaced: bool,
    /// The game's `[gamebanana] game` was set by this install.
    pub game_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbUpdateState {
    Current,
    /// A newer upload of the installed file.
    Newer {
        file_id: u64,
        file_name: String,
        version: String,
    },
    /// The mod or the installed file is no longer on GameBanana.
    Gone,
}

#[derive(Debug, Clone)]
pub struct GbUpdate {
    /// The UMM mod.
    pub id: String,
    pub name: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub version: Option<String>,
    pub state: GbUpdateState,
}

fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A file's label among a mod's files: what its author says it is, else its name.
fn file_label(f: &ModFile) -> String {
    if f.description.trim().is_empty() {
        f.name.clone()
    } else {
        f.description.trim().to_string()
    }
}

/// The version an install of `file` has: the file's own, else the mod's when it is the
/// mod's newest file, else what its description says when that reads like a version
/// ("v1.2.0 (Mac)").
fn file_version(d: &ModDetails, file: &ModFile) -> Option<String> {
    let newest = d.files.iter().map(|f| f.added).max() == Some(file.added);
    let described = file.description.trim();
    file.version
        .clone()
        .or_else(|| newest.then(|| d.summary.version.clone()))
        .filter(|v| !v.is_empty())
        .or_else(|| {
            (described.len() <= 24 && described.chars().any(|c| c.is_ascii_digit()))
                .then(|| described.to_string())
        })
}

impl Session {
    pub(crate) fn gamebanana(&self) -> Api<'_> {
        Api::new(self.http())
    }

    /// GameBanana games whose name matches `name`.
    pub fn gamebanana_find_games(&self, name: &str) -> Result<Vec<GbGameInfo>> {
        Ok(self.gamebanana().find_games(name)?)
    }

    /// The game's GameBanana game: its `[gamebanana] game`, else the GameBanana game with
    /// the same name. `None` when neither is known (or GameBanana cannot be reached to look
    /// the name up).
    pub fn gamebanana_game(&self, id: &str) -> Result<Option<GbGame>> {
        let def = self.game_def(id)?;
        if let Some(g) = &def.gamebanana {
            let info = self.gamebanana().game(g.game).ok();
            return Ok(Some(GbGame {
                game_id: g.game,
                name: info.as_ref().map(|i| i.name.clone()),
                mod_count: info.and_then(|i| i.mod_count),
                guessed: false,
            }));
        }
        let key = name_key(&def.name);
        let found = self
            .gamebanana()
            .find_games(&def.name)
            .unwrap_or_default()
            .into_iter()
            .find(|g| name_key(&g.name) == key);
        Ok(found.map(|g| GbGame {
            game_id: g.id,
            name: Some(g.name),
            mod_count: self.gamebanana().game(g.id).ok().and_then(|i| i.mod_count),
            guessed: true,
        }))
    }

    fn require_gamebanana_game(&self, id: &str) -> Result<GbGame> {
        self.gamebanana_game(id)?
            .ok_or_else(|| CoreError::Invalid("pick this game's GameBanana game first".into()))
    }

    /// Sets the game's `[gamebanana] game`, or removes it with `None`.
    pub fn set_gamebanana_game(&self, id: &str, game: Option<u64>) -> Result<GameDef> {
        self.edit_game_file(id, |text, origin| {
            umm_gamedef::edit::set_gamebanana_game(text, origin, game)
        })
    }

    fn gamebanana_mods(&self, id: &str) -> Result<Vec<ModMeta>> {
        Ok(umm_store::list_mods(&self.data.game(id))?
            .into_iter()
            .filter(|m| m.gamebanana.is_some())
            .collect())
    }

    /// A page of the game's mods on GameBanana, marked with what is installed. With
    /// `installed_only`, only mods with a file installed for the game, looked up one by one,
    /// filtered by name and sorted here.
    pub fn gamebanana_search(
        &self,
        id: &str,
        q: &GbSearch,
        installed_only: bool,
    ) -> Result<GbPage> {
        let game = self.require_gamebanana_game(id)?;
        let installed = self.gamebanana_mods(id)?;
        let ModPage {
            total,
            complete,
            mods,
        } = if installed_only {
            self.installed_gb_summaries(&game, &installed, q)?
        } else {
            self.gamebanana().mods(game.game_id, q)?
        };
        let items = mods
            .into_iter()
            .map(|s| GbItem {
                installed: installed
                    .iter()
                    .filter(|m| m.gamebanana.as_ref().is_some_and(|g| g.mod_id == s.mod_id))
                    .map(|m| (m.id.clone(), m.version.clone()))
                    .collect(),
                summary: s,
            })
            .collect();
        Ok(GbPage {
            game,
            total,
            complete,
            items,
        })
    }

    /// The installed GameBanana mods of this game, as a page.
    fn installed_gb_summaries(
        &self,
        game: &GbGame,
        installed: &[ModMeta],
        q: &GbSearch,
    ) -> Result<ModPage> {
        let mut ids: Vec<u64> = installed
            .iter()
            .filter_map(|m| m.gamebanana.as_ref())
            .filter(|g| g.game_id == game.game_id)
            .map(|g| g.mod_id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let api = self.gamebanana();
        let text = q.text.trim().to_lowercase();
        let mut mods: Vec<ModSummary> = ids
            .into_iter()
            .map(|mod_id| {
                // A mod removed from GameBanana still shows, from what UMM stored.
                api.mod_details(mod_id)
                    .map(|d| d.summary)
                    .unwrap_or_else(|_| {
                        let m = installed
                            .iter()
                            .find(|m| m.gamebanana.as_ref().is_some_and(|g| g.mod_id == mod_id))
                            .expect("listed above");
                        ModSummary {
                            mod_id,
                            name: m.name.clone(),
                            author: None,
                            version: m.version.clone().unwrap_or_default(),
                            category: String::new(),
                            thumbnail_url: None,
                            likes: 0,
                            views: 0,
                            updated: 0,
                            rated: false,
                            obsolete: false,
                            has_files: false,
                        }
                    })
            })
            .filter(|m| text.is_empty() || m.name.to_lowercase().contains(&text))
            .collect();
        match q.sort {
            GbSort::Downloads | GbSort::Likes => {
                mods.sort_by_key(|m| std::cmp::Reverse((m.likes, m.views)))
            }
            GbSort::Updated | GbSort::Newest => mods.sort_by_key(|m| std::cmp::Reverse(m.updated)),
            GbSort::Name => mods.sort_by_key(|m| m.name.to_lowercase()),
        }
        Ok(ModPage {
            total: mods.len() as u64,
            complete: true,
            mods,
        })
    }

    /// A mod's page and which of its files are installed.
    pub fn gamebanana_files(&self, id: &str, mod_id: u64) -> Result<GbModFiles> {
        let details = self.gamebanana().mod_details(mod_id)?;
        let installed = self
            .gamebanana_mods(id)?
            .into_iter()
            .filter_map(|m| {
                let g = m.gamebanana?;
                (g.mod_id == mod_id).then_some((g.file_id, m.id))
            })
            .collect();
        Ok(GbModFiles { details, installed })
    }

    /// The UMM mod an install of `file` goes to, and whether it replaces it: the mod that
    /// holds an earlier upload of this file (or this very file), else a new id from the
    /// mod's name (and the file's label, when another file of the mod is installed).
    fn gamebanana_target(
        &self,
        id: &str,
        d: &ModDetails,
        file: &ModFile,
    ) -> Result<(String, String, bool)> {
        let mods = umm_store::list_mods(&self.data.game(id))?;
        let line = umm_gamebanana::file_line(&file.name);
        let mut other_file = false;
        for m in &mods {
            let Some(g) = &m.gamebanana else { continue };
            if g.mod_id != d.summary.mod_id {
                continue;
            }
            if g.file_id == file.file_id || umm_gamebanana::file_line(&g.file_name) == line {
                return Ok((m.id.clone(), m.name.clone(), true));
            }
            other_file = true;
        }
        let name = if other_file {
            format!("{} - {}", d.summary.name, file_label(file))
        } else {
            d.summary.name.clone()
        };
        let base = umm_store::slugify(&name);
        let taken = |c: &str| mods.iter().any(|m| m.id == c);
        let mod_id = d.summary.mod_id;
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

    /// Whether files of the GameBanana game `game_id` can go into the game now: not while
    /// its mods are deployed, nor when the game is set up for another GameBanana game.
    /// Returns whether its `[gamebanana] game` is still unset (and is to be set).
    fn gamebanana_installable(&self, id: &str, game_id: u64) -> Result<bool> {
        let def = self.game_def(id)?;
        let unset = match &def.gamebanana {
            Some(g) if g.game != game_id => {
                return Err(CoreError::Invalid(format!(
                    "{} is set up for GameBanana game {}, not {game_id}",
                    def.name, g.game
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
        Ok(unset)
    }

    /// Downloads one file of a mod, checks it against its checksum, and installs it into
    /// `profile`, replacing an earlier upload of it.
    pub fn gamebanana_install(
        &self,
        id: &str,
        mod_id: u64,
        file_id: u64,
        profile: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<GbInstallReport> {
        progress("Looking the mod up on GameBanana");
        let d = self.gamebanana().mod_details(mod_id)?;
        let file = d
            .files
            .iter()
            .find(|f| f.file_id == file_id)
            .cloned()
            .ok_or_else(|| GameBananaError::NotFound(format!("file {file_id} of mod {mod_id}")))?;
        if file.download_url.is_empty() {
            return Err(GameBananaError::NotFound(format!("download for {}", file.name)).into());
        }
        let game_set = self.gamebanana_installable(id, d.game_id)?;
        let dir = self
            .data
            .game(id)
            .generated_dir()
            .join("gamebanana-download");
        if dir.exists() {
            fs::remove_dir_all(&dir).ctx(|| format!("removing {}", dir.display()))?;
        }
        fs::create_dir_all(&dir).ctx(|| format!("creating {}", dir.display()))?;
        // Keep the archive's name: its extension tells how to unpack it.
        let safe: String = file
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let archive = dir.join(if safe.is_empty() {
            "download.zip".into()
        } else {
            safe
        });
        let result =
            self.gamebanana_install_from(id, &d, &file, &archive, profile, game_set, progress);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn gamebanana_install_from(
        &self,
        id: &str,
        d: &ModDetails,
        file: &ModFile,
        archive: &Path,
        profile: &str,
        game_set: bool,
        progress: &mut dyn FnMut(&str),
    ) -> Result<GbInstallReport> {
        let label = file_label(file);
        let mut last = 0u64;
        self.http()
            .download(&file.download_url, archive, &mut |got, total| {
                // A line every megabyte, and the last, is plenty.
                if got > last && (got >= last + (1 << 20) || Some(got) == total) {
                    last = got;
                    let mb = |n: u64| format!("{:.1}", n as f64 / f64::from(1 << 20));
                    let total = total.or(Some(file.size).filter(|s| *s > 0));
                    progress(&match total {
                        Some(t) => format!("Downloading {label}: {} of {} MB", mb(got), mb(t)),
                        None => format!("Downloading {label}: {} MB", mb(got)),
                    });
                }
            })
            .map_err(|e| GameBananaError::Network {
                url: e.url,
                message: e.message,
            })?;
        if !file.md5.is_empty() {
            progress("Checking the download");
            let got = crate::nexus::md5_file(archive)?;
            if got != file.md5 {
                return Err(CoreError::Invalid(format!(
                    "the download of {} does not match GameBanana's checksum (got {got}, expected {}); try again",
                    file.name, file.md5
                )));
            }
        }
        if umm_store::archive_kind(archive).is_none() {
            return Err(CoreError::Invalid(format!(
                "{} is not an archive UMM can open (.zip, .7z or .rar)",
                file.name
            )));
        }
        progress("Unpacking");
        let (umm_id, name, replace) = self.gamebanana_target(id, d, file)?;
        let version = file_version(d, file);
        let meta = self.import(
            id,
            archive,
            ImportOptions {
                id: Some(umm_id),
                name: Some(name),
                version: version.clone(),
                source: Some(umm_gamebanana::mod_url(d.summary.mod_id)),
                unwrap: true,
                gamebanana: Some(GameBananaMeta {
                    game_id: d.game_id,
                    mod_id: d.summary.mod_id,
                    file_id: file.file_id,
                    file_name: file.name.clone(),
                    file_added: file.added,
                    version,
                    md5: file.md5.clone(),
                }),
                replace,
                ..Default::default()
            },
            profile,
        )?;
        if game_set {
            self.set_gamebanana_game(id, Some(d.game_id))?;
        }
        Ok(GbInstallReport {
            meta,
            replaced: replace,
            game_set,
        })
    }

    /// Each installed GameBanana file, and whether a newer upload of it is out.
    pub fn gamebanana_updates(&self, id: &str) -> Result<Vec<GbUpdate>> {
        let api = self.gamebanana();
        let mut details: std::collections::HashMap<u64, Option<ModDetails>> = Default::default();
        let mut out = Vec::new();
        for m in self.gamebanana_mods(id)? {
            let g = m.gamebanana.clone().expect("filtered");
            let d = details
                .entry(g.mod_id)
                .or_insert_with(|| api.mod_details(g.mod_id).ok());
            let state = match d {
                None => GbUpdateState::Gone,
                Some(d) => {
                    let installed = d
                        .files
                        .iter()
                        .find(|f| f.file_id == g.file_id)
                        .cloned()
                        .unwrap_or(ModFile {
                            file_id: g.file_id,
                            name: g.file_name.clone(),
                            description: String::new(),
                            version: None,
                            size: 0,
                            added: g.file_added,
                            downloads: 0,
                            download_url: String::new(),
                            md5: g.md5.clone(),
                            clean: None,
                            contains_exe: vec![],
                        });
                    match umm_gamebanana::newer_in_line(&d.files, &installed) {
                        Some(f) => GbUpdateState::Newer {
                            file_id: f.file_id,
                            file_name: f.name.clone(),
                            version: f
                                .version
                                .clone()
                                .or_else(|| Some(file_label(f)).filter(|l| *l != f.name))
                                .unwrap_or_else(|| d.summary.version.clone()),
                        },
                        None if d.files.iter().any(|f| f.file_id == g.file_id) => {
                            GbUpdateState::Current
                        }
                        // The installed file was taken down: an upload after it is its
                        // replacement (authors often keep only their newest file).
                        None => match d
                            .files
                            .iter()
                            .filter(|f| f.added > installed.added)
                            .max_by_key(|f| (f.added, f.file_id))
                        {
                            Some(f) => GbUpdateState::Newer {
                                file_id: f.file_id,
                                file_name: f.name.clone(),
                                version: f
                                    .version
                                    .clone()
                                    .or_else(|| Some(file_label(f)).filter(|l| *l != f.name))
                                    .unwrap_or_else(|| d.summary.version.clone()),
                            },
                            None => GbUpdateState::Gone,
                        },
                    }
                }
            };
            out.push(GbUpdate {
                id: m.id,
                name: m.name,
                mod_id: g.mod_id,
                file_id: g.file_id,
                version: m.version,
                state,
            });
        }
        Ok(out)
    }

    /// Games a GameBanana game's files can go into: `(id, name, matches)`, where `matches`
    /// means the game is that GameBanana game (set, or by name). Games set to another
    /// GameBanana game are left out; the matching ones come first.
    pub fn gamebanana_link_games(&self, game_id: u64) -> Result<Vec<(String, String, bool)>> {
        let mut out = Vec::new();
        for st in self.status()? {
            match self.gamebanana_game(&st.def.id).ok().flatten() {
                Some(g) if g.game_id == game_id => out.push((st.def.id, st.def.name, true)),
                Some(g) if !g.guessed => {}
                _ => out.push((st.def.id, st.def.name, false)),
            }
        }
        out.sort_by_key(|(_, _, m)| !*m);
        Ok(out)
    }
}
