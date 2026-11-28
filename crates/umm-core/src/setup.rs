//! Choosing a game's mod loader setup. The choice is written into its `game.toml`.

use crate::steam::TsFit;
use crate::{CoreError, Result, Session};
use umm_deploy::GameLock;
use umm_gamedef::steam::Engine;
use umm_gamedef::{loader_name, GameDef, LoaderSetup, BEPINEX_KIND};

#[derive(Debug, Clone)]
pub struct LoaderChoice {
    /// The setup in effect. `None`: a loader none of the setups describes (an ASI loader,
    /// say), set in `game.toml` by hand.
    pub current: Option<LoaderSetup>,
    /// Setups this game can switch to.
    pub available: Vec<LoaderSetup>,
    /// The game's Thunderstore community, if Thunderstore covers it with a loader UMM
    /// supports.
    pub community: Option<String>,
    /// That community's mod loader kind (`bepinex`, `shimloader`).
    pub ts_loader: Option<String>,
    /// Thunderstore's mod loader for the game when UMM does not support it (Northstar, say).
    pub other_loader: Option<String>,
    /// The game's own mod support, which names the generic setup (`None`: files copied in).
    pub own_mods: Option<&'static str>,
}

impl Session {
    /// The game's Thunderstore community and its loader: the loader the definition uses,
    /// else Thunderstore's list of games by the definition's community, Steam app id or exe
    /// name. Empty when offline with nothing cached.
    pub(crate) fn find_community(&self, def: &GameDef) -> TsFit {
        if let (Some(c), Some(l)) = (def.community(), def.ts_loader()) {
            return TsFit {
                community: Some(c.to_string()),
                loader: Some(l.to_string()),
                other_loader: None,
            };
        }
        let Ok(eco) = self.ecosystem(false) else {
            // Offline: a community named in the definition is taken to be a BepInEx one.
            return TsFit {
                community: def.community().map(str::to_string),
                loader: def.community().map(|_| BEPINEX_KIND.to_string()),
                other_loader: None,
            };
        };
        let named = def.community().and_then(|c| eco.community(c));
        let by_app = || def.detect.steam_app_id.and_then(|a| eco.by_steam_app(a));
        let by_exe = || {
            let exe = def.launch.as_ref()?.exe.rsplit('/').next()?.to_string();
            eco.by_exe_name(&exe)
        };
        TsFit::of(named.or_else(by_app).or_else(by_exe))
    }

    /// Whether the game may be a Unity game, what BepInEx needs: from its folder, and when
    /// its folder cannot be found, it may be.
    fn may_be_unity(&self, def: &GameDef) -> bool {
        match self.install_path(def) {
            Ok(Some((root, _))) if root.is_dir() => matches!(
                umm_gamedef::steam::inspect_folder(&root).engine,
                Engine::UnityMono | Engine::UnityIl2cpp
            ),
            _ => true,
        }
    }

    pub fn loader_choice(&self, id: &str) -> Result<LoaderChoice> {
        let def = self.game_def(id)?;
        let fit = self.find_community(&def);
        let mut available = Vec::new();
        if fit.community.is_some() {
            available.push(LoaderSetup::Thunderstore);
        }
        // BepInEx is for Unity games, and makes no sense for one whose Thunderstore loader is
        // another. A game set up for it already keeps the choice.
        let bepinex =
            fit.loader.as_deref().is_none_or(|l| l == BEPINEX_KIND) && self.may_be_unity(&def);
        if bepinex || def.setup() == Some(LoaderSetup::Bepinex) {
            available.push(LoaderSetup::Bepinex);
        }
        available.push(LoaderSetup::Generic);
        Ok(LoaderChoice {
            current: def.setup(),
            available,
            community: fit.community,
            ts_loader: fit.loader,
            other_loader: fit.other_loader,
            own_mods: def.own_mods(),
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
        let fit = self.find_community(&def);
        if setup == LoaderSetup::Bepinex && !self.may_be_unity(&def) {
            return Err(CoreError::Invalid(format!(
                "{} is not a Unity game: BepInEx cannot load mods into it",
                def.name
            )));
        }
        if setup == LoaderSetup::Thunderstore && fit.community.is_none() {
            return Err(CoreError::Invalid(match &fit.other_loader {
                Some(other) => format!(
                    "{} uses {} on Thunderstore, which UMM does not support yet",
                    def.name,
                    loader_name(other)
                ),
                None => format!(
                    "{} is not on Thunderstore (or Thunderstore could not be reached)",
                    def.name
                ),
            }));
        }
        let community = fit
            .as_community()
            .filter(|_| setup == LoaderSetup::Thunderstore);
        let new = def.with_setup(setup, community)?;
        self.edit_game_file(id, |text, origin| {
            umm_gamedef::edit::set_setup(text, origin, &new)
        })?;
        drop(lock);
        self.loader_choice(id)
    }
}
