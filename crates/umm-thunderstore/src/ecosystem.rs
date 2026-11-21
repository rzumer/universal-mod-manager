//! Thunderstore's ecosystem schema
//! (<https://thunderstore.io/api/experimental/schema/dev/latest/>): every game Thunderstore
//! covers, the platforms it ships on, its mod loader, how its packages are laid out, and which
//! packages are mod loaders. Only the fields UMM uses are read; the rest is ignored.

use crate::{cache, Result, TsError, BASE_URL};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub const ECOSYSTEM_URL: &str = "https://thunderstore.io/api/experimental/schema/dev/latest/";
/// The schema changes when Thunderstore adds games; a week-old copy is fine.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ecosystem {
    #[serde(default)]
    pub games: BTreeMap<String, EcosystemGame>,
    #[serde(default)]
    pub modloader_packages: Vec<ModloaderPackage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EcosystemGame {
    /// The community label, as in `thunderstore.io/c/<label>/`.
    pub label: String,
    #[serde(default)]
    pub meta: GameMeta,
    #[serde(default)]
    pub distributions: Vec<Distribution>,
    #[serde(default)]
    pub r2modman: Option<Vec<R2modmanGame>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMeta {
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Distribution {
    pub platform: String,
    pub identifier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct R2modmanGame {
    #[serde(default)]
    pub package_loader: String,
    #[serde(default)]
    pub install_rules: Vec<InstallRule>,
    /// The game's data folder: for Unreal games, the project folder (`Pal` for Palworld).
    #[serde(default)]
    pub data_folder_name: String,
    #[serde(default)]
    pub exe_names: Vec<String>,
    #[serde(default)]
    pub package_index: Option<String>,
    #[serde(default)]
    pub relative_file_exclusions: Option<Vec<String>>,
    #[serde(default)]
    pub game_instance_type: Option<String>,
    #[serde(default)]
    pub distributions: Vec<Distribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRule {
    pub route: String,
    #[serde(default)]
    pub default_file_extensions: Vec<String>,
    pub tracking_method: TrackingMethod,
    #[serde(default)]
    pub sub_routes: Vec<InstallRule>,
    #[serde(default)]
    pub is_default_location: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackingMethod {
    None,
    Subdir,
    SubdirNoFlatten,
    State,
    PackageZip,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModloaderPackage {
    /// `Owner-Name`.
    pub package_id: String,
    /// The folder inside the package's zip whose contents go to the game folder.
    #[serde(default)]
    pub root_folder: String,
    pub loader: String,
}

/// What Thunderstore says about one Steam game.
#[derive(Debug, Clone, Copy)]
pub struct SteamMatch<'a> {
    pub game: &'a EcosystemGame,
    pub r2: &'a R2modmanGame,
}

impl SteamMatch<'_> {
    pub fn community(&self) -> &str {
        &self.game.label
    }
    pub fn loader(&self) -> &str {
        &self.r2.package_loader
    }
    pub fn is_bepinex(&self) -> bool {
        self.r2.package_loader == "bepinex"
    }
    /// UMM can set the game up with its Thunderstore mod loader.
    pub fn is_supported(&self) -> bool {
        umm_gamedef::thunderstore_inject(self.loader()).is_some()
    }
    pub fn as_community(&self) -> umm_gamedef::Community<'_> {
        umm_gamedef::Community {
            label: self.community(),
            loader: self.loader(),
        }
    }
}

impl Ecosystem {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| TsError::Data {
            what: "ecosystem schema".into(),
            message: e.to_string(),
        })
    }

    /// The game whose Steam distribution has this app id. Prefers the game instance over a
    /// dedicated server entry.
    pub fn by_steam_app(&self, app_id: u32) -> Option<SteamMatch<'_>> {
        let id = app_id.to_string();
        let on_steam = |d: &[Distribution]| {
            d.iter().any(|d| {
                (d.platform == "steam" || d.platform == "steam-direct")
                    && d.identifier.as_deref() == Some(id.as_str())
            })
        };
        let mut server = None;
        for game in self.games.values() {
            let Some(r2s) = &game.r2modman else { continue };
            for r2 in r2s {
                if !(on_steam(&r2.distributions) || on_steam(&game.distributions)) {
                    continue;
                }
                let m = SteamMatch { game, r2 };
                if r2.game_instance_type.as_deref() != Some("server") {
                    return Some(m);
                }
                server.get_or_insert(m);
            }
        }
        server
    }

    /// The game whose r2modman entry lists this exe name (case-insensitive), for games not
    /// added through Steam. `None` when no game, or more than one, lists it.
    pub fn by_exe_name(&self, exe: &str) -> Option<SteamMatch<'_>> {
        let mut found: Option<SteamMatch> = None;
        for game in self.games.values() {
            for r2 in game.r2modman.iter().flatten() {
                if r2.game_instance_type.as_deref() == Some("server") {
                    continue;
                }
                if r2.exe_names.iter().any(|e| e.eq_ignore_ascii_case(exe)) {
                    if found.is_some_and(|f| f.game.label != game.label) {
                        return None;
                    }
                    found.get_or_insert(SteamMatch { game, r2 });
                }
            }
        }
        found
    }

    pub fn community(&self, label: &str) -> Option<SteamMatch<'_>> {
        let game = self.games.values().find(|g| g.label == label)?;
        let r2s = game.r2modman.as_ref()?;
        let r2 = r2s
            .iter()
            .find(|r| r.game_instance_type.as_deref() != Some("server"))
            .or(r2s.first())?;
        Some(SteamMatch { game, r2 })
    }

    /// The mod-loader package entry for `Owner-Name`, if it is one.
    pub fn modloader(&self, full_name: &str) -> Option<&ModloaderPackage> {
        self.modloader_packages
            .iter()
            .find(|m| m.package_id.eq_ignore_ascii_case(full_name))
    }

    /// The mod-loader package entry for `Owner-Name` as a game with `loader` uses it: a
    /// package listed for several loaders (MelonLoader is) has one entry for each.
    pub fn modloader_for(&self, full_name: &str, loader: &str) -> Option<&ModloaderPackage> {
        let mut entries = self
            .modloader_packages
            .iter()
            .filter(|m| m.package_id.eq_ignore_ascii_case(full_name));
        let first = entries.clone().next();
        entries.find(|m| m.loader == loader).or(first)
    }
}

/// The community's package index URL.
pub fn package_index_url(m: &SteamMatch) -> String {
    m.r2.package_index.clone().unwrap_or_else(|| {
        format!(
            "{BASE_URL}/c/{}/api/v1/package-listing-index/",
            m.community()
        )
    })
}

/// Loads the ecosystem from `cache_file`, downloading it when missing or a week old.
pub fn load(
    http: &dyn crate::Http,
    cache_file: &Path,
    force: bool,
) -> Result<(Ecosystem, cache::Freshness)> {
    let (bytes, fresh) = cache::cached(cache_file, MAX_AGE, force, || http.get(ECOSYSTEM_URL))?;
    Ok((Ecosystem::parse(&bytes)?, fresh))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A trimmed copy of the real schema's shape.
    pub const SAMPLE: &str = r#"{
      "schemaVersion": "0.3.0",
      "games": {
        "lethal-company": {
          "uuid": "x", "label": "lethal-company",
          "meta": {"displayName": "Lethal Company", "iconUrl": null},
          "distributions": [{"platform": "steam", "identifier": "1966720"}],
          "r2modman": [{
            "internalFolderName": "LethalCompany",
            "packageIndex": "https://thunderstore.io/c/lethal-company/api/v1/package-listing-index/",
            "exeNames": ["Lethal Company.exe"],
            "gameInstanceType": "game",
            "packageLoader": "bepinex",
            "installRules": [
              {"route": "BepInEx/plugins", "defaultFileExtensions": [".dll"], "trackingMethod": "subdir", "subRoutes": [], "isDefaultLocation": true},
              {"route": "BepInEx/core", "defaultFileExtensions": [], "trackingMethod": "subdir", "subRoutes": [], "isDefaultLocation": false},
              {"route": "BepInEx/patchers", "defaultFileExtensions": [], "trackingMethod": "subdir", "subRoutes": [], "isDefaultLocation": false},
              {"route": "BepInEx/monomod", "defaultFileExtensions": [".mm.dll"], "trackingMethod": "subdir", "subRoutes": [], "isDefaultLocation": false},
              {"route": "BepInEx/config", "defaultFileExtensions": [], "trackingMethod": "none", "subRoutes": [], "isDefaultLocation": false}
            ],
            "relativeFileExclusions": null,
            "distributions": []
          }]
        },
        "bonelab": {
          "label": "bonelab",
          "meta": {"displayName": "BONELAB"},
          "distributions": [{"platform": "steam", "identifier": "1592190"}],
          "r2modman": [{"packageLoader": "melonloader", "installRules": [], "exeNames": ["BONELAB_Steam_Windows64.exe"]}]
        },
        "wiki-only": {"label": "wiki-only", "distributions": [{"platform": "steam", "identifier": "5"}]}
      },
      "communities": {},
      "packageInstallers": {},
      "modloaderPackages": [
        {"packageId": "BepInEx-BepInExPack", "rootFolder": "BepInExPack", "loader": "bepinex"},
        {"packageId": "denikson-BepInExPack_Valheim", "rootFolder": "BepInExPack_Valheim", "loader": "bepinex"}
      ]
    }"#;

    #[test]
    fn finds_games_by_steam_app_id() {
        let eco = Ecosystem::parse(SAMPLE.as_bytes()).unwrap();
        let lc = eco.by_steam_app(1966720).unwrap();
        assert_eq!(lc.community(), "lethal-company");
        assert!(lc.is_bepinex());
        assert_eq!(lc.r2.install_rules.len(), 5);
        assert_eq!(lc.r2.install_rules[4].tracking_method, TrackingMethod::None);
        assert_eq!(
            package_index_url(&lc),
            "https://thunderstore.io/c/lethal-company/api/v1/package-listing-index/"
        );

        let bl = eco.by_steam_app(1592190).unwrap();
        assert_eq!(bl.loader(), "melonloader");
        assert!(
            eco.by_steam_app(5).is_none(),
            "no r2modman entry: not moddable here"
        );
        assert!(eco.by_steam_app(42).is_none());

        assert_eq!(
            eco.modloader("bepinex-bepinexpack").unwrap().root_folder,
            "BepInExPack"
        );
        assert!(eco.community("lethal-company").is_some());
        assert_eq!(
            eco.by_exe_name("lethal company.EXE").unwrap().community(),
            "lethal-company"
        );
        assert!(eco.by_exe_name("Game.exe").is_none());
    }
}
