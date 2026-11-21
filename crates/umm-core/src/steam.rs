//! Adding games from the local Steam libraries.

use crate::{CoreError, Result, Session};
use umm_gamedef::steam::{draft_from, inspect, installed_games, to_toml, Inspection};
pub use umm_gamedef::steam::{Draft, Engine, Note, SteamApp};
use umm_gamedef::LoaderSetup;
use umm_thunderstore::Ecosystem;

#[derive(Debug, Clone)]
pub struct SteamCandidate {
    pub app: SteamApp,
    pub engine: Engine,
    /// Id of the game definition that already covers this app, if any.
    pub defined_as: Option<String>,
    /// The game's Thunderstore community, when Thunderstore lists it as a BepInEx game.
    pub community: Option<String>,
    /// Thunderstore's mod loader for the game when it is not BepInEx.
    pub other_loader: Option<String>,
    /// The setup Add from Steam picks unless told otherwise.
    pub default_setup: LoaderSetup,
    /// Every setup that can be picked, default first.
    pub setups: Vec<LoaderSetup>,
}

/// Installed Steam games, plus why Thunderstore's list could not be used, if it could not.
#[derive(Debug, Clone)]
pub struct SteamList {
    pub games: Vec<SteamCandidate>,
    pub thunderstore_error: Option<String>,
}

/// The setups a game can be added with, the default first. Thunderstore BepInEx games get it
/// by default, except IL2CPP builds (BepInEx 6 is not supported yet); everything else starts
/// generic.
pub(crate) fn setups_for(engine: Engine, on_thunderstore: bool) -> Vec<LoaderSetup> {
    let ts_ok = on_thunderstore && engine != Engine::UnityIl2cpp;
    let mut setups = vec![if ts_ok {
        LoaderSetup::Thunderstore
    } else {
        LoaderSetup::Generic
    }];
    if on_thunderstore && !setups.contains(&LoaderSetup::Thunderstore) {
        setups.push(LoaderSetup::Thunderstore);
    }
    if engine == Engine::UnityMono || ts_ok {
        setups.push(LoaderSetup::Bepinex);
    }
    if !setups.contains(&LoaderSetup::Generic) {
        setups.push(LoaderSetup::Generic);
    }
    setups
}

fn candidate(
    app: SteamApp,
    found: &Inspection,
    eco: Option<&Ecosystem>,
    defined_as: Option<String>,
) -> SteamCandidate {
    let ts = eco.and_then(|e| e.by_steam_app(app.app_id));
    let community = ts
        .filter(|m| m.is_bepinex())
        .map(|m| m.community().to_string());
    let other_loader = ts
        .filter(|m| !m.is_bepinex())
        .map(|m| m.loader().to_string());
    let setups = setups_for(found.engine, community.is_some());
    let default_setup = setups[0];
    SteamCandidate {
        engine: found.engine,
        app,
        defined_as,
        community,
        other_loader,
        default_setup,
        setups,
    }
}

impl Session {
    /// Installed Steam games, each with its engine, Thunderstore status and whether UMM
    /// already has it.
    pub fn steam_games(&self) -> SteamList {
        self.steam_list(installed_games())
    }

    pub fn steam_list(&self, apps: Vec<SteamApp>) -> SteamList {
        let (eco, thunderstore_error) = match self.ecosystem(false) {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let (defs, _) = self.defs();
        let games = apps
            .into_iter()
            .map(|app| {
                let defined_as = defs
                    .iter()
                    .find(|d| d.detect.steam_app_id == Some(app.app_id))
                    .map(|d| d.id.clone());
                let found = inspect(&app);
                candidate(app, &found, eco.as_deref(), defined_as)
            })
            .collect();
        SteamList {
            games,
            thunderstore_error,
        }
    }

    /// Adds an installed Steam game by app id, with `setup` or the default one.
    pub fn add_steam_game(&self, app_id: u32, setup: Option<LoaderSetup>) -> Result<Draft> {
        let app = installed_games()
            .into_iter()
            .find(|a| a.app_id == app_id)
            .ok_or_else(|| CoreError::Invalid(format!("Steam app {app_id} is not installed")))?;
        self.add_steam_app(&app, setup)
    }

    /// Writes a drafted game definition to `gamedefs/<id>.toml`. Never overwrites a file, and
    /// refuses an app that a definition already covers.
    pub fn add_steam_app(&self, app: &SteamApp, setup: Option<LoaderSetup>) -> Result<Draft> {
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
        let eco = self.ecosystem(false).ok();
        let found = inspect(app);
        let c = candidate(app.clone(), &found, eco.as_deref(), None);
        let setup = setup.unwrap_or(c.default_setup);
        if !c.setups.contains(&setup) {
            return Err(CoreError::Invalid(format!(
                "{} cannot be set up as {}",
                app.name,
                setup.label()
            )));
        }
        let mut draft = draft_from(app, &found, setup, c.community.as_deref())?;
        if let Some(l) = &c.other_loader {
            draft.notes.push(Note::new(
                "thunderstore-other-loader",
                format!("On Thunderstore this game uses {l}, which UMM does not support yet."),
            ));
        }
        if c.community.is_some() && found.engine == Engine::UnityIl2cpp {
            draft.notes.push(Note::new(
                "bepinex6",
                "Thunderstore's mods for this game need BepInEx 6, which UMM does not support yet.",
            ));
        }
        draft.def.id = self.free_game_id(&draft.def.id, &app.app_id.to_string())?;
        // Steam already knows where it is, so no install path is written.
        self.create_game_file(&draft.def.id, &to_toml(&draft))?;
        Ok(draft)
    }

    /// `base`, or `base-<alt>` when a game already uses `base`; an error if both are taken.
    pub(crate) fn free_game_id(&self, base: &str, alt: &str) -> Result<String> {
        let taken = |id: &str| self.data.game(id).def_path().exists();
        if !taken(base) {
            return Ok(base.to_string());
        }
        let other = format!("{base}-{alt}");
        if !taken(&other) {
            return Ok(other);
        }
        Err(CoreError::Invalid(format!(
            "a game named `{other}` already exists"
        )))
    }
}
