//! Adding games from the local Steam libraries.

use crate::{CoreError, Result, Session};
use std::fs::OpenOptions;
use std::io::Write;
use umm_gamedef::steam::{draft_definition, installed_games, to_toml};
pub use umm_gamedef::steam::{Draft, Engine, Note, SteamApp};
use umm_store::fsutil::IoCtx;

#[derive(Debug, Clone)]
pub struct SteamCandidate {
    pub app: SteamApp,
    pub engine: Engine,
    /// Id of the game definition that already covers this app, if any.
    pub defined_as: Option<String>,
}

impl Session {
    /// Installed Steam games, each with its guessed engine and whether UMM already has it.
    pub fn steam_games(&self) -> Vec<SteamCandidate> {
        self.steam_candidates(installed_games())
    }

    pub fn steam_candidates(&self, apps: Vec<SteamApp>) -> Vec<SteamCandidate> {
        let (defs, _) = self.defs();
        apps.into_iter()
            .map(|app| {
                let defined_as = defs
                    .iter()
                    .find(|d| d.detect.steam_app_id == Some(app.app_id))
                    .map(|d| d.id.clone());
                let engine = draft_definition(&app).engine;
                SteamCandidate {
                    app,
                    engine,
                    defined_as,
                }
            })
            .collect()
    }

    /// Adds an installed Steam game by app id.
    pub fn add_steam_game(&self, app_id: u32) -> Result<Draft> {
        let app = installed_games()
            .into_iter()
            .find(|a| a.app_id == app_id)
            .ok_or_else(|| CoreError::Invalid(format!("Steam app {app_id} is not installed")))?;
        self.add_steam_app(&app)
    }

    /// Writes a drafted game definition to `gamedefs/<id>.toml`. Never overwrites a file, and
    /// refuses an app that a definition already covers.
    pub fn add_steam_app(&self, app: &SteamApp) -> Result<Draft> {
        let (defs, _) = self.defs();
        if let Some(d) = defs
            .iter()
            .find(|d| d.detect.steam_app_id == Some(app.app_id))
        {
            return Err(CoreError::Invalid(format!(
                "{} is already set up as `{}`",
                app.name, d.id
            )));
        }
        let mut draft = draft_definition(app);
        let dir = self.data().gamedefs_dir();
        let taken =
            |id: &str| defs.iter().any(|d| d.id == id) || dir.join(format!("{id}.toml")).exists();
        if taken(&draft.def.id) {
            draft.def.id = format!("{}-{}", draft.def.id, app.app_id);
        }
        if taken(&draft.def.id) {
            return Err(CoreError::Invalid(format!(
                "a game definition named `{}` already exists",
                draft.def.id
            )));
        }
        let path = dir.join(format!("{}.toml", draft.def.id));
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ctx(|| format!("creating {}", path.display()))?;
        f.write_all(to_toml(&draft).as_bytes())
            .ctx(|| format!("writing {}", path.display()))?;
        // Steam already knows where it is, so no manual path is needed.
        Ok(draft)
    }
}
