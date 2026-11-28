//! Adding games from the local Steam libraries.

use crate::{CoreError, Result, Session};
use umm_gamedef::steam::{draft_from, inspect, installed_games, stand_in, to_toml, Inspection};
pub use umm_gamedef::steam::{Draft, Engine, Note, SteamApp};
use umm_gamedef::{Community, LoaderSetup, BEPINEX_KIND};
use umm_sources::thunderstore::ecosystem::SteamMatch;
use umm_sources::thunderstore::Ecosystem;

/// What Thunderstore says about a game, as far as UMM can use it.
#[derive(Debug, Clone, Default)]
pub(crate) struct TsFit {
    /// The community, when UMM supports its mod loader.
    pub community: Option<String>,
    /// That mod loader's kind (`bepinex`, `shimloader`).
    pub loader: Option<String>,
    /// The community's mod loader when UMM does not support it.
    pub other_loader: Option<String>,
}

impl TsFit {
    pub fn of(m: Option<SteamMatch>) -> Self {
        match m {
            Some(m) if m.is_supported() => TsFit {
                community: Some(m.community().to_string()),
                loader: Some(m.loader().to_string()),
                other_loader: None,
            },
            Some(m) => TsFit {
                other_loader: Some(m.loader().to_string()),
                ..Default::default()
            },
            None => TsFit::default(),
        }
    }

    pub fn as_community(&self) -> Option<Community<'_>> {
        Some(Community {
            label: self.community.as_deref()?,
            loader: self.loader.as_deref()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SteamCandidate {
    pub app: SteamApp,
    pub engine: Engine,
    /// Id of the game definition that already covers this app, if any.
    pub defined_as: Option<String>,
    /// The game's Thunderstore community, when UMM supports its mod loader there.
    pub community: Option<String>,
    /// That community's mod loader kind (`bepinex`, `shimloader`).
    pub ts_loader: Option<String>,
    /// Thunderstore's mod loader for the game when UMM does not support it.
    pub other_loader: Option<String>,
    /// The setup Add from Steam picks unless told otherwise.
    pub default_setup: LoaderSetup,
    /// Every setup that can be picked, default first.
    pub setups: Vec<LoaderSetup>,
    /// The game's own mod support, which names the generic setup (`None`: files copied in).
    pub own_mods: Option<&'static str>,
}

/// Installed Steam games, plus why Thunderstore's list could not be used, if it could not.
#[derive(Debug, Clone)]
pub struct SteamList {
    pub games: Vec<SteamCandidate>,
    pub thunderstore_error: Option<String>,
}

/// The setups a game can be added with, the default first. Games whose Thunderstore loader
/// UMM supports (`ts_loader`) get it by default; everything else starts generic. BepInEx
/// with your own pack suits Unity games and Thunderstore's BepInEx games.
pub(crate) fn setups_for(engine: Engine, ts_loader: Option<&str>) -> Vec<LoaderSetup> {
    let unity = matches!(engine, Engine::UnityMono | Engine::UnityIl2cpp);
    let mut setups = vec![if ts_loader.is_some() {
        LoaderSetup::Thunderstore
    } else {
        LoaderSetup::Generic
    }];
    if ts_loader.map_or(unity, |k| k == BEPINEX_KIND) {
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
    let fit = TsFit::of(eco.and_then(|e| e.by_steam_app(app.app_id)));
    let setups = setups_for(found.engine, fit.loader.as_deref());
    let default_setup = setups[0];
    SteamCandidate {
        engine: found.engine,
        app,
        defined_as,
        community: fit.community,
        ts_loader: fit.loader,
        other_loader: fit.other_loader,
        default_setup,
        setups,
        own_mods: found.own_mods(),
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
            .iter()
            .map(|app| {
                // Terraria is set up as tModLoader when it is installed, and either one's
                // definition covers both.
                let stand = stand_in(app, &apps);
                let ids = [Some(app.app_id), stand.map(|s| s.app_id)];
                let defined_as = defs
                    .iter()
                    .find(|d| {
                        d.detect.steam_app_id.is_some() && ids.contains(&d.detect.steam_app_id)
                    })
                    .map(|d| d.id.clone());
                let found = inspect(stand.unwrap_or(app));
                candidate(app.clone(), &found, eco.as_deref(), defined_as)
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

    /// Writes a drafted game definition to `games/<id>/game.toml`. Never overwrites a file, and
    /// refuses an app that a definition already covers.
    pub fn add_steam_app(&self, app: &SteamApp, setup: Option<LoaderSetup>) -> Result<Draft> {
        let installed = match app.app_id {
            umm_gamedef::steam::TERRARIA_APP => installed_games(),
            _ => Vec::new(),
        };
        self.add_steam_app_among(app, &installed, setup)
    }

    /// [`Self::add_steam_app`], with the other installed apps (`installed`): Terraria is set up
    /// as tModLoader when it is one of them, under Terraria's name.
    pub fn add_steam_app_among(
        &self,
        app: &SteamApp,
        installed: &[SteamApp],
        setup: Option<LoaderSetup>,
    ) -> Result<Draft> {
        let stand = stand_in(app, installed);
        let ids = [Some(app.app_id), stand.map(|s| s.app_id)];
        // Drafted from the stand-in's folder, under the game's name.
        let as_app = stand.map(|s| SteamApp {
            name: app.name.clone(),
            ..s.clone()
        });
        let app = as_app.as_ref().unwrap_or(app);
        let (defs, _) = self.defs();
        if let Some(d) = defs
            .iter()
            .find(|d| d.detect.steam_app_id.is_some() && ids.contains(&d.detect.steam_app_id))
        {
            return Err(CoreError::Invalid(format!(
                "{} is already set up as `{}`",
                app.name, d.id
            )));
        }
        if !cfg!(windows) && umm_gamedef::proton_user(app.app_id, Some(&app.path)).is_none() {
            return Err(crate::not_through_proton(&app.name));
        }
        let eco = self.ecosystem(false).ok();
        let found = inspect(app);
        let c = candidate(app.clone(), &found, eco.as_deref(), None);
        let setup = setup.unwrap_or(c.default_setup);
        if !c.setups.contains(&setup) {
            return Err(CoreError::Invalid(format!(
                "{} cannot be set up as {}",
                app.name,
                setup.label(c.ts_loader.as_deref(), c.own_mods)
            )));
        }
        let fit = TsFit {
            community: c.community.clone(),
            loader: c.ts_loader.clone(),
            other_loader: None,
        };
        let mut draft = draft_from(app, &found, setup, fit.as_community())?;
        if let Some(l) = &c.other_loader {
            draft.notes.push(Note::new(
                "thunderstore-other-loader",
                format!("On Thunderstore this game uses {l}, which UMM does not support yet."),
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
