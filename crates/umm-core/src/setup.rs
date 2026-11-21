//! Choosing a game's mod loader setup. The choice is written into its `game.toml`.

use crate::{CoreError, Result, Session};
use umm_deploy::GameLock;
use umm_gamedef::{GameDef, LoaderSetup};

#[derive(Debug, Clone)]
pub struct LoaderChoice {
    /// The setup in effect. `None`: a loader none of the setups describes (an ASI loader,
    /// say), set in `game.toml` by hand.
    pub current: Option<LoaderSetup>,
    /// Setups this game can switch to.
    pub available: Vec<LoaderSetup>,
    /// The game's Thunderstore community, if Thunderstore covers it.
    pub community: Option<String>,
    /// Thunderstore's mod loader for the game when it is not BepInEx (MelonLoader, say).
    pub other_loader: Option<String>,
}

impl Session {
    /// The game's Thunderstore community: from its definition, else Thunderstore's list of
    /// games by Steam app id or exe name (only BepInEx games). `None` when offline with
    /// nothing cached.
    pub(crate) fn find_community(&self, def: &GameDef) -> (Option<String>, Option<String>) {
        if let Some(c) = def.community() {
            return (Some(c.to_string()), None);
        }
        let Ok(eco) = self.ecosystem(false) else {
            return (None, None);
        };
        let by_app = def.detect.steam_app_id.and_then(|a| eco.by_steam_app(a));
        let by_exe = || {
            let exe = def.launch.as_ref()?.exe.rsplit('/').next()?.to_string();
            eco.by_exe_name(&exe)
        };
        match by_app.or_else(by_exe) {
            Some(m) if m.is_bepinex() => (Some(m.community().to_string()), None),
            Some(m) => (None, Some(m.loader().to_string())),
            None => (None, None),
        }
    }

    pub fn loader_choice(&self, id: &str) -> Result<LoaderChoice> {
        let def = self.game_def(id)?;
        let (community, other_loader) = self.find_community(&def);
        let mut available = Vec::new();
        if community.is_some() {
            available.push(LoaderSetup::Thunderstore);
        }
        available.extend([LoaderSetup::Bepinex, LoaderSetup::Generic]);
        Ok(LoaderChoice {
            current: def.setup(),
            available,
            community,
            other_loader,
        })
    }

    /// Switches the game's mod loader setup by rewriting the loader and install-rule
    /// sections of its `game.toml` (the rest of the file is kept). Refused while mods are
    /// deployed.
    pub fn set_loader(&self, id: &str, setup: LoaderSetup) -> Result<LoaderChoice> {
        let def = self.game_def(id)?;
        if def.setup() == Some(setup) {
            return self.loader_choice(id);
        }
        let game = self.data.game(id);
        let lock = GameLock::acquire(&game)?;
        if game.journal_path().exists() {
            return Err(CoreError::Deployed(id.into()));
        }
        let community = match setup {
            LoaderSetup::Thunderstore => match self.find_community(&def) {
                (Some(c), _) => Some(c),
                (None, Some(other)) => {
                    return Err(CoreError::Invalid(format!(
                        "{} uses {other} on Thunderstore, which UMM does not support yet",
                        def.name
                    )))
                }
                (None, None) => {
                    return Err(CoreError::Invalid(format!(
                        "{} is not a BepInEx game on Thunderstore (or Thunderstore could not be reached)",
                        def.name
                    )))
                }
            },
            _ => None,
        };
        let new = def.with_setup(setup, community.as_deref())?;
        self.edit_game_file(id, |text, origin| {
            umm_gamedef::edit::set_setup(text, origin, &new)
        })?;
        drop(lock);
        self.loader_choice(id)
    }
}
