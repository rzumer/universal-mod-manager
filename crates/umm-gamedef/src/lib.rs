//! Game definitions: one TOML file per game describing how to find it, which loader it needs and
//! where each mod's files go. Adding a game means adding a file, not code.

pub mod appinfo;
pub mod detect;
pub mod edit;
pub mod gamemaker;
pub mod relpath;
pub mod setup;
pub mod steam;
pub mod vdf;

pub use detect::{detect_install, proton_user, Detected};
pub use relpath::{RelPath, RelPathError};
pub use setup::{Community, LoaderSetup, BUILT_IN_MODS};

use globset::{GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// A game's file in its folder under `games/`: its definition and the settings UMM writes
/// there (the install path, the mod loader). UMM edits it in place, keeping comments.
pub const GAME_FILE: &str = "game.toml";

/// Name of the backup folder used inside a game folder when the data directory is on another
/// drive. Mods may never write into it.
pub const IN_GAME_BACKUP_DIR: &str = ".umm-backup";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameDef {
    pub schema: u32,
    /// The game's folder name under `games/`. Not part of the file.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub detect: Detect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<Launch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<Loader>,
    /// The game's Thunderstore community, when its mods come from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunderstore: Option<Thunderstore>,
    /// The game on Nexus Mods, when UMM installs its mods from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus: Option<Nexus>,
    /// The game on GameBanana, when UMM installs its mods from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamebanana: Option<GameBanana>,
    /// The game's list of enabled plugins or mods, which UMM writes while mods are deployed
    /// (Bethesda games' `plugins.txt`, RimWorld's `ModsConfig.xml`): `[mod_list]` in the file.
    #[serde(default, rename = "mod_list", skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Plugins>,
    /// Settings in the game's ini files that UMM sets while mods are deployed, and puts back
    /// afterwards (Fallout 4's `bEnableFileSelection`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ini: Vec<IniSetting>,
    #[serde(default)]
    pub install: Vec<InstallRule>,
    /// Game paths that are never replaced, even if a mod ships them. A mod may still add a
    /// file matching them that the game does not have (a script extender's loader `.exe`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protect: Vec<String>,
    /// Game paths that are always copied rather than hardlinked, because the game writes to them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<String>,
    /// Folders outside the game folder that mods go to, by name: an absolute path with
    /// variables (see [`expand_path`]: `%DOCUMENTS%/Paradox Interactive/Crusader Kings III`).
    /// Install rules target one as `@<name>/…`; deploying there is journaled like the game
    /// folder.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub folders: BTreeMap<String, String>,
    /// A Paradox game (Crusader Kings III, Stellaris…): mods are folders in its user folder's
    /// `mod/`, each found by a `.mod` descriptor beside it, and enabled in `dlc_load.json`
    /// (Crusader Kings II: `settings.txt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paradox: Option<Paradox>,
    /// A Larian game (Baldur's Gate 3, Divinity: Original Sin 2): mods are `.pak` files in its
    /// user folder's `Mods/`, enabled in its player profiles' `modsettings.lsx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub larian: Option<Larian>,
    /// Mount & Blade II: Bannerlord: mods are modules, each a folder named after its id in
    /// the game folder's `Modules/`, loaded when the game is started with them named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bannerlord: Option<Bannerlord>,
}

/// Bannerlord's mod setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bannerlord {
    /// The game folder's folder of modules.
    #[serde(default = "default_modules_folder")]
    pub folder: String,
}

fn default_modules_folder() -> String {
    "Modules".into()
}

/// A Larian game's mod setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Larian {
    /// The [`GameDef::folders`] entry of the game's user folder (holding `Mods/` and
    /// `PlayerProfiles/`).
    pub folder: String,
    /// Which game's way of listing modules (`bg3`, the default, or `dos2`).
    #[serde(default, skip_serializing_if = "LarianFormat::is_bg3")]
    pub format: LarianFormat,
}

/// How a Larian game lists its enabled modules, and where its files are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LarianFormat {
    /// Baldur's Gate 3: `PlayerProfiles/Public/modsettings.lsx`, its `Mods` node.
    #[default]
    Bg3,
    /// Divinity: Original Sin 2 Definitive Edition: each player profile's
    /// `modsettings.lsx`, its `ModOrder` and `Mods` nodes; the game in `DefEd/`.
    Dos2,
}

impl LarianFormat {
    pub fn is_bg3(&self) -> bool {
        *self == LarianFormat::Bg3
    }

    /// The game folder's folder holding the exe, where the Script Extender goes.
    pub fn bin(self) -> &'static str {
        match self {
            LarianFormat::Bg3 => "bin",
            LarianFormat::Dos2 => "DefEd/bin",
        }
    }

    /// The game folder's data folder.
    pub fn data(self) -> &'static str {
        match self {
            LarianFormat::Bg3 => "Data",
            LarianFormat::Dos2 => "DefEd/Data",
        }
    }

    /// The Script Extender's loader, beside the exe.
    pub fn extender(self) -> &'static str {
        match self {
            LarianFormat::Bg3 => "DWrite.dll",
            LarianFormat::Dos2 => "DXGI.dll",
        }
    }
}

/// A Paradox game's mod setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paradox {
    /// The [`GameDef::folders`] entry of the game's user folder (holding `mod/` and
    /// `dlc_load.json`).
    pub folder: String,
    /// Where the game lists its enabled mods.
    #[serde(default, skip_serializing_if = "ParadoxList::is_default")]
    pub list: ParadoxList,
}

/// Where a Paradox game lists its enabled mods, in its user folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParadoxList {
    /// `dlc_load.json`'s `enabled_mods` (the games of Paradox's launcher). Descriptors give
    /// their mod's folder as an absolute path.
    #[default]
    DlcLoad,
    /// `settings.txt`'s `last_mods={…}` (Crusader Kings II). Descriptors give their mod's
    /// folder relative to the user folder (`mod/<mod>`).
    Settings,
}

impl ParadoxList {
    fn is_default(&self) -> bool {
        *self == ParadoxList::DlcLoad
    }
}

/// A path's named folder, when it starts with `@<name>/` and the definition has a folder of
/// that name: the name and the path inside it. Other paths are in the game folder (a game's
/// own `@Mod` folders, as Arma has, stay there).
pub fn folder_of<'a>(
    rel: &RelPath,
    folders: &'a BTreeMap<String, String>,
) -> Option<(&'a str, RelPath)> {
    let first = rel.first()?;
    let name = first.strip_prefix('@')?;
    let (key, _) = folders.get_key_value(name)?;
    Some((key.as_str(), rel.strip_components(1)))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detect {
    /// The install folder, used instead of detection. Set by UMM when you pick a folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub steam_app_id: Option<u32>,
    pub steam_dir: Option<String>,
    /// `HKLM\SOFTWARE\Vendor\Game:ValueName`
    pub registry: Option<String>,
    /// A file that must exist in the folder for detection to succeed.
    pub marker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub exe: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Left out for games not started through Steam.
    #[serde(default, skip_serializing_if = "SteamRelaunch::is_none")]
    pub steam_relaunch: SteamRelaunch,
    #[serde(default)]
    pub process_names: Vec<String>,
    /// Launch the game as usual even when a script extender's loader (`fose_loader.exe`,
    /// `skse64_loader.exe`…) is beside its exe: for an extender set up in Steam's launch
    /// options, or loaded by a patched game exe.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore_extender_loader: bool,
}

/// Script extenders' loaders: run instead of the game's exe, they start it with the extender
/// loaded. Found beside the game's exe.
pub const EXTENDER_LOADERS: &[&str] = &[
    "skse64_loader.exe",
    "sksevr_loader.exe",
    "skse_loader.exe",
    "f4se_loader.exe",
    "f4sevr_loader.exe",
    "sfse_loader.exe",
    "nvse_loader.exe",
    "fose_loader.exe",
    "obse_loader.exe",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamRelaunch {
    AppidFile,
    ViaSteam,
    #[default]
    None,
}

impl SteamRelaunch {
    pub fn is_none(&self) -> bool {
        *self == SteamRelaunch::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thunderstore {
    /// The community's label, as in `thunderstore.io/c/<community>/`.
    pub community: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nexus {
    /// The game's name in Nexus URLs, as in `nexusmods.com/<domain>/mods/`.
    pub domain: String,
}

/// The game on GameBanana.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameBanana {
    /// The game's number on GameBanana, as in `gamebanana.com/games/<game>`.
    pub game: u64,
}

/// A game that only loads the plugins or mods a list file names (Bethesda games, RimWorld,
/// Factorio).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plugins {
    /// The list file: an absolute path with variables (see [`expand_path`]:
    /// `%LOCALAPPDATA%/Skyrim Special Edition/plugins.txt`), or a path relative to the game
    /// folder (`Morrowind.ini`).
    pub list: String,
    #[serde(default)]
    pub format: PluginListFormat,
    /// The folder holding plugins, relative to the game folder. For the formats listing mod
    /// folders ([`PluginListFormat::lists_mod_folders`]), the folder holding one folder per
    /// mod (`@<folder>/…` for one of [`GameDef::folders`]).
    #[serde(default = "default_plugin_folder")]
    pub folder: String,
    /// When this setting of an ini file in the game folder holds, the list is in the game
    /// folder instead (Oblivion with `bUseMyGamesDirectory=0`). Checked at each deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_game_folder_when: Option<GameIniCondition>,
}

/// A setting of an ini file in the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameIniCondition {
    /// Relative to the game folder.
    pub ini: String,
    pub section: String,
    pub key: String,
    pub value: String,
}

fn default_plugin_folder() -> String {
    "Data".into()
}

/// How a plugin list marks plugins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginListFormat {
    /// Every plugin listed in load order, enabled ones starting with `*` (Skyrim Special
    /// Edition, Fallout 4, Starfield).
    #[default]
    Asterisk,
    /// Only enabled plugins are listed, one per line (Oblivion, Oblivion Remastered, Fallout
    /// 3, Fallout: New Vegas, Skyrim Legendary Edition).
    Plain,
    /// Morrowind: enabled plugins as `GameFile0=`, `GameFile1=`... in the `[Game Files]`
    /// section of `Morrowind.ini`. The game loads them in the order of their file dates.
    MorrowindIni,
    /// RimWorld: active mods as `<li>` entries of `<activeMods>` in `ModsConfig.xml`, in load
    /// order, by the `packageId` in each mod's `About/About.xml`.
    RimworldXml,
    /// Factorio: `{"name": …, "enabled": …}` objects in the `mods` array of `mod-list.json`,
    /// by the `name` in each mod's `info.json`. Each mod's folder is named after it.
    FactorioJson,
    /// Oxygen Not Included: the `mods` array of `mods.json`, each mod by its folder in
    /// `mods/Local`, enabled for each game version (`enabledForDlc`).
    OniJson,
    /// Kenshi: enabled mods in `data/mods.cfg`, one `<Name>.mod` a line, in load order, by
    /// the `.mod` file in each mod's folder. Each mod's folder is named after it.
    KenshiCfg,
    /// XCOM 2: `ActiveMods="<Name>"` lines in the `[Engine.XComModOptions]` section of the
    /// user's `XComModOptions.ini`, by the `<Name>.XComMod` file in each mod's folder. Each
    /// mod's folder is named after it.
    XcomIni,
    /// tModLoader: `enabled.json` in its mods folder, a JSON array of the enabled mods'
    /// names, by the name inside each `.tmod` file there. Each file is named after it.
    TmodloaderJson,
}

/// A tModLoader mod's file extension.
pub const TML_EXT: &str = ".tmod";
/// A Terraria resource pack's description, at the top of its folder (or `.zip`).
pub const TML_PACK: &str = "pack.json";

/// The resource packs folder beside tModLoader's mods folder (`mods`), ending in `/`: both are
/// in its save folder.
pub fn tml_packs_prefix(mods: &str) -> String {
    match mods.trim_matches('/').rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/ResourcePacks/"),
        None => "ResourcePacks/".into(),
    }
}

impl PluginListFormat {
    /// The list names mods, each a folder in [`Plugins::folder`], rather than plugin files.
    pub fn lists_mod_folders(self) -> bool {
        matches!(
            self,
            PluginListFormat::RimworldXml
                | PluginListFormat::FactorioJson
                | PluginListFormat::OniJson
                | PluginListFormat::KenshiCfg
                | PluginListFormat::XcomIni
        )
    }
}

/// The file extensions of plugins.
pub const PLUGIN_EXTENSIONS: &[&str] = &["esm", "esl", "esp"];

/// What a Bethesda game's data folder (`Data`, `Data Files`) holds besides plugins: their
/// archives at its top, and the folders of loose files the engine reads from it.
pub const DATA_ARCHIVE_EXTENSIONS: &[&str] = &["bsa", "ba2"];
pub const DATA_DIRS: &[&str] = &[
    "meshes",
    "textures",
    "sound",
    "music",
    "menus",
    "shaders",
    "scripts",
    "interface",
    "fonts",
    "strings",
    "video",
    "lodsettings",
    "grass",
    "trees",
    "distantlod",
    "seq",
    "materials",
];

/// Script extenders, whose plugins go in `Data/<extender>/plugins/`. Only that folder: their
/// own archives also hold a `src/<extender>/` folder of source code, which goes nowhere.
pub const EXTENDER_DIRS: &[&str] = &["fose", "nvse", "obse", "skse", "f4se", "sfse"];

impl Plugins {
    /// The list file's path, with variables filled in (see [`expand_path_with`]), or in the
    /// game folder `root` when relative. `None` when a variable is not set.
    pub fn list_path(&self, root: &Path, vars: &PathVars) -> Option<PathBuf> {
        match self.relative_list() {
            Some(r) => Some(r.to_path(root)),
            None => expand_path_with(&self.list, vars),
        }
    }

    /// The list file's path in the game folder, when it is given as one.
    pub fn relative_list(&self) -> Option<RelPath> {
        if self.list.contains('%') || Path::new(&self.list).is_absolute() {
            return None;
        }
        RelPath::parse(&self.list).ok().filter(|r| !r.is_root())
    }
}

/// One setting in an ini file outside the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IniSetting {
    /// The file: an absolute path with variables, as for [`Plugins::list`].
    pub file: String,
    pub section: String,
    pub key: String,
    pub value: String,
    /// Create the file when it does not exist (a `Custom.ini` of overrides), and delete it
    /// again afterwards if nothing else was added to it. Otherwise a missing file is left for
    /// the game to write.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub create: bool,
}

/// What a game's path variables stand for: the system's own, or, for a Windows game run
/// through Proton, its prefix's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathVars {
    /// The Windows user's folder in the game's prefix (`…/pfx/drive_c/users/steamuser`).
    pub prefix_user: Option<PathBuf>,
}

impl PathVars {
    /// The variables of a game whose prefix has this Windows user folder.
    pub fn in_prefix(user: PathBuf) -> Self {
        PathVars {
            prefix_user: Some(user),
        }
    }

    /// What `name` stands for in the prefix, for the Windows folders a game looks up there;
    /// `None` for any other variable, or without a prefix.
    fn in_prefix_var(&self, name: &str) -> Option<Option<PathBuf>> {
        let user = self.prefix_user.as_ref()?;
        let sub = match name.to_ascii_uppercase().as_str() {
            "DOCUMENTS" => {
                // Older prefixes spell it so.
                let older = user.join("My Documents");
                let docs = user.join("Documents");
                return Some(Some(if !docs.is_dir() && older.is_dir() {
                    older
                } else {
                    docs
                }));
            }
            "USERPROFILE" => return Some(Some(user.clone())),
            "APPDATA" => "AppData/Roaming",
            "LOCALAPPDATA" => "AppData/Local",
            // `drive_c/users/<user>` → `drive_c/ProgramData`.
            "PROGRAMDATA" => {
                return Some(
                    user.parent()
                        .and_then(Path::parent)
                        .map(|c| c.join("ProgramData")),
                )
            }
            _ => return None,
        };
        Some(Some(user.join(sub)))
    }
}

/// A path written with variables: `%NAME%` for an environment variable, and `%DOCUMENTS%`
/// for the user's Documents folder (wherever Windows keeps it). `None` when a variable is not
/// set or the result is not an absolute path.
pub fn expand_path(s: &str) -> Option<PathBuf> {
    expand_path_with(s, &PathVars::default())
}

/// [`expand_path`], with the Windows folders (`%DOCUMENTS%`, `%APPDATA%`, `%LOCALAPPDATA%`,
/// `%USERPROFILE%`, `%PROGRAMDATA%`) in the game's prefix when `vars` has one.
pub fn expand_path_with(s: &str, vars: &PathVars) -> Option<PathBuf> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let j = after.find('%')?;
        let name = &after[..j];
        let value = if let Some(v) = vars.in_prefix_var(name) {
            v?.display().to_string()
        } else if name.eq_ignore_ascii_case("DOCUMENTS") {
            dirs::document_dir()?.display().to_string()
        } else {
            std::env::var(name).ok()?
        };
        out.push_str(&value);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    // Definitions write `/`; shown to the user, the path should read as the system writes it.
    if cfg!(windows) {
        out = out.replace('/', "\\");
    }
    let p = PathBuf::from(out);
    p.is_absolute().then_some(p)
}

/// Whether `s` can be a Nexus game domain name (lower-case letters, digits, `-` and `_`).
pub fn is_nexus_domain(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Loader source meaning "the BepInEx pack installed from Thunderstore", which is stored like
/// any other Thunderstore package.
pub const THUNDERSTORE_SOURCE: &str = "thunderstore";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loader {
    pub kind: String,
    /// `mod:<id>`: the loader is stored and imported like any other mod.
    /// `thunderstore`: it is one of the game's Thunderstore packages.
    pub source: Option<String>,
    #[serde(default)]
    pub inject: Inject,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Inject {
    Doorstop,
    #[default]
    Files,
    Args,
    Env,
}

impl Loader {
    pub fn source_mod(&self) -> Option<&str> {
        self.source.as_deref().and_then(|s| s.strip_prefix("mod:"))
    }

    pub fn from_thunderstore(&self) -> bool {
        self.source.as_deref() == Some(THUNDERSTORE_SOURCE)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRule {
    #[serde(rename = "match")]
    pub pattern: String,
    pub target: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GameDefError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("unsupported schema version {0} (this version of UMM reads up to {SCHEMA_VERSION}; a newer one may read it)")]
    Schema(u32),
    #[error("invalid id `{0}` (use a-z, 0-9, `-` and `_`)")]
    Id(String),
    #[error("invalid glob `{pattern}`: {source}")]
    Glob {
        pattern: String,
        source: globset::Error,
    },
    #[error("invalid install target `{0}`: {1}")]
    Target(String, RelPathError),
    #[error("loader source must look like `mod:<id>`, or be `thunderstore` with a [thunderstore] table; got `{0}`")]
    LoaderSource(String),
    #[error("{0}")]
    Unsupported(String),
}

/// What UMM knows about a loader kind that can be pointed at a folder outside the game
/// (Unity Doorstop). Only the proxy DLL and its config then go into the game folder; the
/// loader's own tree and every mod under it live in a per-profile runtime folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorstopLoader {
    /// Top-level folders that live in the runtime folder instead of the game folder.
    pub runtime_dirs: &'static [&'static str],
    /// The builds the loader comes in; which one a profile has is read from its files.
    pub builds: &'static [DoorstopTarget],
    /// What the profile is missing without it, as the user knows it.
    pub name: &'static str,
}

/// The loaders UMM can start through Doorstop, told apart by the assembly Doorstop loads:
/// BepInEx builds, and Unity Mod Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorstopTarget {
    /// BepInEx 5, for Mono Unity games.
    V5,
    /// BepInEx 6 for Mono Unity games.
    V6Mono,
    /// BepInEx 6 for IL2CPP Unity games; runs on the .NET runtime in the pack's `dotnet/`.
    V6Il2cpp,
    /// Unity Mod Manager (its Thunderstore build), which reads mods from `UMM/Mods/` beside
    /// its own `UMM/Core/`.
    UnityModManager,
}

impl DoorstopTarget {
    /// The assembly Doorstop loads, relative to the runtime folder.
    pub fn target(self) -> &'static str {
        match self {
            DoorstopTarget::V5 => "BepInEx/core/BepInEx.Preloader.dll",
            DoorstopTarget::V6Mono => "BepInEx/core/BepInEx.Unity.Mono.Preloader.dll",
            DoorstopTarget::V6Il2cpp => "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
            DoorstopTarget::UnityModManager => "UMM/Core/UnityModManager.dll",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DoorstopTarget::V5 => "BepInEx 5",
            DoorstopTarget::V6Mono => "BepInEx 6 (Mono)",
            DoorstopTarget::V6Il2cpp => "BepInEx 6 (IL2CPP)",
            DoorstopTarget::UnityModManager => "Unity Mod Manager",
        }
    }
}

/// The IL2CPP build's .NET runtime, relative to the runtime folder.
pub const CORECLR_PATH: &str = "dotnet/coreclr.dll";
pub const CORLIB_DIR: &str = "dotnet";
/// Doorstop 4 flags for the .NET runtime that IL2CPP builds load.
pub const DOORSTOP_CORECLR_FLAG: &str = "--doorstop-clr-runtime-coreclr-path";
pub const DOORSTOP_CORLIB_FLAG: &str = "--doorstop-clr-corlib-dir";

/// Doorstop 4 command-line flags (BepInEx 5.4.22 and later ship Doorstop 4).
pub const DOORSTOP_ENABLED_FLAG: &str = "--doorstop-enabled";
pub const DOORSTOP_TARGET_FLAG: &str = "--doorstop-target-assembly";
/// Doorstop 3 spellings (BepInEx 5 before 5.4.22).
pub const DOORSTOP3_ENABLED_FLAG: &str = "--doorstop-enable";
pub const DOORSTOP3_TARGET_FLAG: &str = "--doorstop-target";

/// Loader kind for BepInEx, any version.
pub const BEPINEX_KIND: &str = "bepinex";

pub fn doorstop_loader(kind: &str) -> Option<DoorstopLoader> {
    match kind {
        BEPINEX_KIND => Some(DoorstopLoader {
            runtime_dirs: &["BepInEx", "dotnet"],
            builds: &[
                DoorstopTarget::V5,
                DoorstopTarget::V6Mono,
                DoorstopTarget::V6Il2cpp,
            ],
            name: "BepInEx pack",
        }),
        UMM_KIND => Some(DoorstopLoader {
            runtime_dirs: &[UMM_DIR],
            builds: &[DoorstopTarget::UnityModManager],
            name: "Unity Mod Manager",
        }),
        _ => None,
    }
}

/// Loader kind for Unity Mod Manager (newman55's), started through Unity Doorstop.
pub const UMM_KIND: &str = "umm";
/// Its folder: `Core/` with the manager, `Mods/` with a folder per mod.
pub const UMM_DIR: &str = "UMM";
/// Unity Doorstop's own files, which go into the game folder with Unity Mod Manager.
pub const DOORSTOP_FILES: &[&str] = &["winhttp.dll", "doorstop_config.ini", ".doorstop_version"];

/// Loader kind for Thunderstore's unreal-shimloader (RE-UE4SS behind a `dwmapi.dll` proxy).
pub const SHIMLOADER_KIND: &str = "shimloader";

/// Shimloader's folder in the runtime folder, and the launch flags pointing it at each of its
/// parts there (the mod, pak, config and overlay folders it maps into the game).
pub const SHIMLOADER_DIR: &str = "shimloader";
/// Shimloader's mod folder, which stands in for UE4SS's `Mods/`.
pub const SHIMLOADER_MOD_DIR: &str = "shimloader/mod";
pub const SHIMLOADER_DIRS: &[(&str, &str)] = &[
    ("--mod-dir", SHIMLOADER_MOD_DIR),
    ("--pak-dir", "shimloader/pak"),
    ("--cfg-dir", "shimloader/cfg"),
    ("--overlay-dir", "shimloader/overlay"),
];

/// The folder of an Unreal game's real exe, where Shimloader's proxy goes, for the game's
/// project folder (`Pal` for Palworld).
pub fn unreal_binaries_dir(project: &str) -> String {
    format!("{}/Binaries/Win64", project.trim_matches('/'))
}

/// Loader kinds for MelonLoader. Thunderstore lists two: `melonloader`, whose mods follow the
/// game's install rules, and `recursive-melonloader` (MelonLoader 0.7 and later), which puts
/// each mod in its own folder under `Mods/`. Both start the same way.
pub const MELONLOADER_KIND: &str = "melonloader";
pub const RECURSIVE_MELONLOADER_KIND: &str = "recursive-melonloader";

/// Whether a loader kind is MelonLoader, either layout.
pub fn is_melonloader(kind: &str) -> bool {
    kind == MELONLOADER_KIND || kind == RECURSIVE_MELONLOADER_KIND
}

/// The oldest version of a loader kind's package that works for it: MelonLoader 0.7 is the
/// first to load mods from their own folders.
pub fn min_loader_version(kind: &str) -> Option<&'static str> {
    (kind == RECURSIVE_MELONLOADER_KIND).then_some("0.7.0")
}

/// MelonLoader's folders, which `--melonloader.basedir` moves to the runtime folder.
pub const MELONLOADER_DIRS: &[&str] = &["MelonLoader", "Mods", "Plugins", "UserData", "UserLibs"];
/// MelonLoader's launch flags: its base folder, and forcing IL2CPP assemblies to be
/// generated again (needed while the base folder has none yet).
pub const MELONLOADER_BASEDIR_FLAG: &str = "--melonloader.basedir";
pub const MELONLOADER_REGENERATE_FLAG: &str = "--melonloader.agfregenerate";
/// Files whose presence in the base folder means the game's assemblies were generated
/// (MelonLoader 0.5, then 0.6 and later).
pub const MELONLOADER_GENERATED: &[&str] = &[
    "MelonLoader/Managed/Assembly-CSharp.dll",
    "MelonLoader/Il2CppAssemblies/Assembly-CSharp.dll",
];

/// Loader kind for Lovely, the LÖVE injector Balatro mods use.
pub const LOVELY_KIND: &str = "lovely";
/// Lovely's mod folder in the runtime folder, and the flag pointing it there.
pub const LOVELY_DIR: &str = "mods";
pub const LOVELY_MOD_DIR_FLAG: &str = "--mod-dir";
/// Lovely's proxy DLLs: `version.dll` up to 0.9, `winmm.dll` from 0.10.
pub const LOVELY_PROXIES: &[&str] = &["version.dll", "winmm.dll"];

/// Loader kind for GDWeave, the Godot loader WEBFISHING mods use.
pub const GDWEAVE_KIND: &str = "gdweave";
/// GDWeave's folder (its core, mods, configs and log) in the runtime folder, and the flag
/// pointing it there (`--gdweave-folder-override=<folder>`, one argument).
pub const GDWEAVE_DIR: &str = "GDWeave";
pub const GDWEAVE_FOLDER_FLAG: &str = "--gdweave-folder-override=";
/// GDWeave's proxy DLL.
pub const GDWEAVE_PROXY: &str = "winmm.dll";

/// Loader kind for the Godot Mod Loader (GodotML), which Brotato and Dome Keeper ship built in.
pub const GODOTML_KIND: &str = "godotml";
/// Where GodotML loads mod zips from, in the runtime folder; the flags pointing it there and
/// turning mods on (games may need `--enable-mods`).
pub const GODOTML_MODS_DIR: &str = "mods";
pub const GODOTML_ENABLE_FLAG: &str = "--enable-mods";
pub const GODOTML_MODS_FLAG: &str = "--mods-path";
/// The loader's own files from its Thunderstore package, kept in the runtime folder: games
/// that have it built in do not use them.
pub const GODOTML_LOADER_DIR: &str = "addons";
/// What a game's project settings say when GodotML is built in (its autoload).
pub const GODOTML_AUTOLOAD: &str = "res://addons/mod_loader/mod_loader.gd";

/// Loader kind for Return of Modding (Risk of Rain Returns, and Hades II as Hell2Modding).
pub const ROM_KIND: &str = "return-of-modding";
/// Its folder (mods, their data and configs) in the runtime folder, which the flag names the
/// parent of.
pub const ROM_DIR: &str = "ReturnOfModding";
pub const ROM_ROOT_FLAG: &str = "--rom_modding_root_folder";
/// Its proxy DLLs, next to the game's exe: `version.dll` for Return of Modding, `d3d12.dll`
/// for Hell2Modding.
pub const ROM_PROXIES: &[&str] = &["version.dll", "d3d12.dll"];

/// Loader kind for Northstar (Titanfall 2).
pub const NORTHSTAR_KIND: &str = "northstar";
/// Its profile folder (`Northstar.dll`, mods, plugins, saves) in the runtime folder, and the
/// flags starting it from there (`-profile=<folder>`, one argument).
pub const NORTHSTAR_DIR: &str = "R2Northstar";
pub const NORTHSTAR_FLAG: &str = "-northstar";
pub const NORTHSTAR_PROFILE_FLAG: &str = "-profile=";
/// The files that must be in the profile for Northstar to start: its proxy, which the game
/// loads, and the DLL the proxy loads from the profile.
pub const NORTHSTAR_PROXY: &str = "bin/x64_retail/wsock32.dll";
pub const NORTHSTAR_DLL: &str = "R2Northstar/Northstar.dll";

/// Loader kind for the MIO Mod Loader (MIO: Memories in Orbit).
pub const MIO_KIND: &str = "mio";
/// Its proxy and its own folder, which it loads from the game's folder; the mods and their
/// settings, in the runtime folder, with the flags pointing it there (each one argument,
/// then the path).
pub const MIO_PROXY: &str = "winhttp.dll";
pub const MIO_LOADER: &str = "mio-mod-loader/MioModLoader.dll";
pub const MIO_MODS_DIR: &str = "mods";
pub const MIO_CONFIG_DIR: &str = "modconfig";
pub const MIO_MODS_FLAG: &str = "--mods-path";
pub const MIO_CONFIG_FLAG: &str = "--mods-config-path";

/// Loader kind for Rivet (Scrap Mechanic).
pub const RIVET_KIND: &str = "rivet";
/// Its proxy, which reads `Rivet.ini` beside it in the game folder; its folder (the loader and
/// the mods) in the runtime folder, and the flags pointing it there (each one argument, then
/// the value).
pub const RIVET_PROXY: &str = "version.dll";
pub const RIVET_DIR: &str = "Rivet";
pub const RIVET_LOADER: &str = "Rivet/Loader.dll";
pub const RIVET_MODS_DIR: &str = "Rivet/Mods";
/// Its log, written in the game folder unless told otherwise.
pub const RIVET_LOG: &str = "Rivet/rivet.log";
pub const RIVET_FLAGS: [&str; 4] = [
    "-rivetEnable",
    "-rivetTarget",
    "-rivetDirectory",
    "-rivetLog",
];

/// Loader kind for BepisLoader (Resonite): BepInEx for the game's .NET process, started by a
/// `hostfxr.dll` hook, and BepInEx for its Unity renderer, started by Doorstop.
pub const BEPISLOADER_KIND: &str = "bepisloader";
/// The hook, in the game folder; BepInEx for each process, in the runtime folder (the
/// renderer's under `Renderer/`, which r2modman never copies into the game either).
pub const BEPISLOADER_HOOK: &str = "hostfxr.dll";
pub const BEPISLOADER_DIRS: &[&str] = &["BepInEx", "Renderer"];
pub const BEPISLOADER_ENABLE_FLAG: &str = "--hookfxr-enable";
pub const BEPISLOADER_TARGET_FLAG: &str = "--bepinex-target";
/// The renderer's BepInEx preloaders Doorstop can load (BepInEx 6, then 5).
pub const BEPISLOADER_RENDERER_PRELOADERS: &[&str] = &[
    "Renderer/BepInEx/core/BepInEx.Unity.Mono.Preloader.dll",
    "Renderer/BepInEx/core/BepInEx.Preloader.dll",
];
/// Loader packages that come with a game's loader rather than replacing it: Resonite's
/// renderer BepInEx, beside BepisLoader. Laid out like a loader package, but never the game's
/// one loader.
pub const COMPANION_LOADER_PACKAGES: &[&str] = &["ResoniteModding-BepInExRenderer"];

/// Whether `Owner-Name` is a companion loader package (see [`COMPANION_LOADER_PACKAGES`]).
pub fn is_companion_loader(full_name: &str) -> bool {
    COMPANION_LOADER_PACKAGES
        .iter()
        .any(|p| p.eq_ignore_ascii_case(full_name))
}

/// A loader that is pointed at a per-profile runtime folder outside the game, so that only
/// its proxy goes into the game folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLoader {
    /// BepInEx, started by Unity Doorstop.
    Doorstop(DoorstopLoader),
    /// Unreal Shimloader, which takes its folders as launch arguments.
    Shimloader,
    /// MelonLoader, given its base folder as a launch argument.
    MelonLoader,
    /// Lovely, given its mod folder as a launch argument.
    Lovely,
    /// GDWeave, given its folder as a launch argument.
    GDWeave,
    /// The Godot Mod Loader built into the game, given its mod folder as a launch argument.
    GodotML,
    /// Return of Modding, given the folder holding its own as a launch argument.
    ReturnOfModding,
    /// Northstar, given its profile folder as a launch argument.
    Northstar,
    /// The MIO Mod Loader, given its mod and settings folders as launch arguments.
    Mio,
    /// Rivet, given its loader, mod folder and log as launch arguments.
    Rivet,
    /// BepisLoader, given each process's BepInEx as launch arguments.
    BepisLoader,
}

impl RuntimeLoader {
    /// Top-level folders that live in the runtime folder instead of the game folder.
    pub fn runtime_dirs(&self) -> &'static [&'static str] {
        match self {
            RuntimeLoader::Doorstop(d) => d.runtime_dirs,
            RuntimeLoader::Shimloader => &[SHIMLOADER_DIR],
            RuntimeLoader::MelonLoader => MELONLOADER_DIRS,
            RuntimeLoader::Lovely => &[LOVELY_DIR],
            RuntimeLoader::GDWeave => &[GDWEAVE_DIR],
            RuntimeLoader::GodotML => &[GODOTML_MODS_DIR, GODOTML_LOADER_DIR],
            RuntimeLoader::ReturnOfModding => &[ROM_DIR],
            RuntimeLoader::Northstar => &[NORTHSTAR_DIR],
            RuntimeLoader::Mio => &[MIO_MODS_DIR, MIO_CONFIG_DIR],
            RuntimeLoader::Rivet => &[RIVET_DIR],
            RuntimeLoader::BepisLoader => BEPISLOADER_DIRS,
        }
    }
}

/// Thunderstore mod loaders UMM can set a game up with, and how each is injected.
pub fn thunderstore_inject(kind: &str) -> Option<Inject> {
    match kind {
        BEPINEX_KIND => Some(Inject::Doorstop),
        SHIMLOADER_KIND => Some(Inject::Args),
        k if is_melonloader(k) => Some(Inject::Args),
        LOVELY_KIND => Some(Inject::Args),
        GDWEAVE_KIND => Some(Inject::Args),
        GODOTML_KIND => Some(Inject::Args),
        UMM_KIND => Some(Inject::Doorstop),
        ROM_KIND => Some(Inject::Args),
        NORTHSTAR_KIND => Some(Inject::Args),
        MIO_KIND => Some(Inject::Args),
        RIVET_KIND => Some(Inject::Args),
        BEPISLOADER_KIND => Some(Inject::Args),
        _ => None,
    }
}

/// A loader kind's name as people know it.
pub fn loader_name(kind: &str) -> &str {
    match kind {
        BEPINEX_KIND => "BepInEx",
        SHIMLOADER_KIND => "Shimloader",
        k if is_melonloader(k) => "MelonLoader",
        LOVELY_KIND => "Lovely",
        GDWEAVE_KIND => "GDWeave",
        GODOTML_KIND => "Godot Mod Loader",
        UMM_KIND => "Unity Mod Manager",
        ROM_KIND => "Return of Modding",
        NORTHSTAR_KIND => "Northstar",
        MIO_KIND => "MIO Mod Loader",
        RIVET_KIND => "Rivet",
        BEPISLOADER_KIND => "BepisLoader",
        other => other,
    }
}

impl GameDef {
    /// The Doorstop loader to inject through launch arguments, if this game uses one.
    pub fn doorstop(&self) -> Option<DoorstopLoader> {
        let l = self.loader.as_ref()?;
        (l.inject == Inject::Doorstop)
            .then(|| doorstop_loader(&l.kind))
            .flatten()
    }

    /// The loader that keeps its files in a runtime folder, if this game uses one.
    pub fn runtime_loader(&self) -> Option<RuntimeLoader> {
        if let Some(d) = self.doorstop() {
            return Some(RuntimeLoader::Doorstop(d));
        }
        let l = self.loader.as_ref()?;
        match l.inject {
            Inject::Args if l.kind == SHIMLOADER_KIND => Some(RuntimeLoader::Shimloader),
            Inject::Args if is_melonloader(&l.kind) => Some(RuntimeLoader::MelonLoader),
            Inject::Args if l.kind == LOVELY_KIND => Some(RuntimeLoader::Lovely),
            Inject::Args if l.kind == GDWEAVE_KIND => Some(RuntimeLoader::GDWeave),
            Inject::Args if l.kind == GODOTML_KIND => Some(RuntimeLoader::GodotML),
            Inject::Args if l.kind == ROM_KIND => Some(RuntimeLoader::ReturnOfModding),
            Inject::Args if l.kind == NORTHSTAR_KIND => Some(RuntimeLoader::Northstar),
            Inject::Args if l.kind == MIO_KIND => Some(RuntimeLoader::Mio),
            Inject::Args if l.kind == RIVET_KIND => Some(RuntimeLoader::Rivet),
            Inject::Args if l.kind == BEPISLOADER_KIND => Some(RuntimeLoader::BepisLoader),
            _ => None,
        }
    }

    /// Executable names that mean the game is running: `process_names`, or the exe's name.
    pub fn process_names(&self) -> Vec<String> {
        let Some(l) = &self.launch else {
            return Vec::new();
        };
        if !l.process_names.is_empty() {
            return l.process_names.clone();
        }
        RelPath::parse(&l.exe)
            .ok()
            .and_then(|r| r.file_name().map(str::to_string))
            .into_iter()
            .collect()
    }
}

/// Ids name folders on disk: lowercase letters (any script), digits, `-` and `_`, at most 64
/// characters, and never a name Windows reserves (`con`, `nul`, ...). Lowercase only, so two
/// ids never collide on a case-insensitive filesystem.
pub fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 64
        && !s.starts_with(['-', '_'])
        && s.chars()
            .all(|c| (c.is_alphanumeric() && !c.is_uppercase()) || c == '-' || c == '_')
        && relpath::valid_component(s)
}

/// An id made from a title: letters and digits kept as written (lowercased), everything else
/// collapsed to `-`. `None` if nothing usable is left.
pub fn slug(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(64).collect();
    let out = out.trim_end_matches('-').to_string();
    is_valid_id(&out).then_some(out)
}

impl GameDef {
    /// Parses a game's `game.toml`; `id` is its folder's name.
    pub fn parse_game(text: &str, origin: &Path, id: &str) -> Result<Self, GameDefError> {
        let mut def: GameDef = toml::from_str(text).map_err(|source| GameDefError::Parse {
            path: origin.to_path_buf(),
            source: Box::new(source),
        })?;
        def.id = id.to_string();
        def.validate()?;
        Ok(def)
    }

    /// Loads `<dir>/game.toml`, where `dir` is the game's folder under `games/`.
    pub fn load_game(dir: &Path) -> Result<Self, GameDefError> {
        let path = dir.join(GAME_FILE);
        let text = fs::read_to_string(&path).map_err(|source| GameDefError::Io {
            path: path.clone(),
            source,
        })?;
        let id = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::parse_game(&text, &path, &id)
    }

    /// Loads every `<games_dir>/<id>/game.toml`. Invalid files are returned separately so one
    /// bad definition does not hide the others; folders without a `game.toml` are skipped.
    pub fn load_games(games_dir: &Path) -> (Vec<GameDef>, Vec<GameDefError>) {
        let mut defs = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = fs::read_dir(games_dir) else {
            return (defs, errors);
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join(GAME_FILE).is_file())
            .collect();
        dirs.sort();
        for d in dirs {
            match Self::load_game(&d) {
                Ok(def) => defs.push(def),
                Err(e) => errors.push(e),
            }
        }
        (defs, errors)
    }

    pub fn validate(&self) -> Result<(), GameDefError> {
        // A file from an older UMM is read as it is (or converted here, once there is an
        // older schema); a newer one is refused.
        if self.schema > SCHEMA_VERSION || self.schema < 1 {
            return Err(GameDefError::Schema(self.schema));
        }
        if !is_valid_id(&self.id) {
            return Err(GameDefError::Id(self.id.clone()));
        }
        for (name, path) in &self.folders {
            let ok = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !ok {
                return Err(GameDefError::Unsupported(format!(
                    "[folders] `{name}`: names are lower-case letters, digits, - and _"
                )));
            }
            if !(path.contains('%') || Path::new(path).is_absolute()) {
                return Err(GameDefError::Unsupported(format!(
                    "[folders] `{name}` must be an absolute path (variables like %DOCUMENTS% allowed)"
                )));
            }
        }
        if let Some(p) = self
            .paradox
            .as_ref()
            .filter(|p| !self.folders.contains_key(&p.folder))
        {
            return Err(GameDefError::Unsupported(format!(
                "[paradox] folder `{}` is not one of [folders]",
                p.folder
            )));
        }
        if let Some(l) = self
            .larian
            .as_ref()
            .filter(|l| !self.folders.contains_key(&l.folder))
        {
            return Err(GameDefError::Unsupported(format!(
                "[larian] folder `{}` is not one of [folders]",
                l.folder
            )));
        }
        if self.paradox.is_some() && self.larian.is_some() {
            return Err(GameDefError::Unsupported(
                "a game has [paradox] or [larian], not both".into(),
            ));
        }
        if let Some(n) = self.nexus.as_ref().filter(|n| !is_nexus_domain(&n.domain)) {
            return Err(GameDefError::Unsupported(format!(
                "[nexus] domain `{}` is not a Nexus Mods game name (lower-case letters, digits, - and _)",
                n.domain
            )));
        }
        if let Some(p) = &self.plugins {
            RelPath::parse(&p.folder)
                .map_err(|e| GameDefError::Unsupported(format!("[mod_list] folder: {e}")))?;
            if p.list.trim().is_empty() {
                return Err(GameDefError::Unsupported("[mod_list] list is empty".into()));
            }
            let absolute = p.list.contains('%') || Path::new(&p.list).is_absolute();
            if !absolute && p.relative_list().is_none() {
                return Err(GameDefError::Unsupported(format!(
                    "[mod_list] list `{}` must be an absolute path or a file in the game folder",
                    p.list
                )));
            }
            if let Some(c) = &p.in_game_folder_when {
                RelPath::parse(&c.ini)
                    .ok()
                    .filter(|r| !r.is_root())
                    .ok_or_else(|| {
                        GameDefError::Unsupported(format!(
                            "[mod_list.in_game_folder_when] ini `{}` must be a file in the game folder",
                            c.ini
                        ))
                    })?;
            }
        }
        for s in &self.ini {
            let bad = |what: &str| GameDefError::Unsupported(format!("[[ini]] {what}"));
            if s.file.trim().is_empty() {
                return Err(bad("file is empty"));
            }
            if s.section.trim().is_empty() || s.section.contains([']', '\n', '\r']) {
                return Err(bad("section must be a plain name"));
            }
            if s.key.trim().is_empty() || s.key.contains(['=', '\n', '\r']) {
                return Err(bad("key must be a plain name"));
            }
            if s.value.contains(['\n', '\r']) {
                return Err(bad("value must be on one line"));
            }
        }
        if let Some(loader) = &self.loader {
            if let Some(src) = &loader.source {
                match loader.source_mod() {
                    Some(id) if is_valid_id(id) => {}
                    None if loader.from_thunderstore() && self.thunderstore.is_some() => {}
                    _ => return Err(GameDefError::LoaderSource(src.clone())),
                }
            }
        }
        if let Some(loader) = &self.loader {
            match loader.inject {
                Inject::Files => {}
                Inject::Doorstop if doorstop_loader(&loader.kind).is_some() => {}
                Inject::Doorstop => {
                    return Err(GameDefError::Unsupported(format!(
                        "loader kind `{}` cannot be injected with doorstop (supported: bepinex); use inject = \"files\"",
                        loader.kind
                    )))
                }
                Inject::Args
                    if loader.kind == SHIMLOADER_KIND
                        || loader.kind == LOVELY_KIND
                        || loader.kind == GDWEAVE_KIND
                        || loader.kind == GODOTML_KIND
                        || loader.kind == ROM_KIND
                        || loader.kind == NORTHSTAR_KIND
                        || loader.kind == MIO_KIND
                        || loader.kind == RIVET_KIND
                        || loader.kind == BEPISLOADER_KIND
                        || is_melonloader(&loader.kind) => {}
                Inject::Args => {
                    return Err(GameDefError::Unsupported(format!(
                        "loader kind `{}` cannot be injected with arguments (supported: shimloader, melonloader, recursive-melonloader, lovely, gdweave, godotml, return-of-modding, northstar, mio, rivet, bepisloader); use inject = \"files\"",
                        loader.kind
                    )))
                }
                Inject::Env => {
                    return Err(GameDefError::Unsupported(
                        "inject = \"env\" is not supported yet; use \"files\", \"doorstop\" or \"args\"".into(),
                    ))
                }
            }
        }
        if let Some(launch) = &self.launch {
            RelPath::parse(&launch.exe)
                .ok()
                .filter(|r| !r.is_root())
                .ok_or_else(|| {
                    GameDefError::Unsupported(format!("invalid launch exe `{}`", launch.exe))
                })?;
            if launch.steam_relaunch != SteamRelaunch::None && self.detect.steam_app_id.is_none() {
                return Err(GameDefError::Unsupported(
                    "steam_relaunch needs [detect] steam_app_id".into(),
                ));
            }
        }
        self.compile().map(|_| ())
    }

    pub fn compile(&self) -> Result<CompiledDef, GameDefError> {
        let mut rules = Vec::new();
        for r in &self.install {
            let matcher = glob(&r.pattern)?.compile_matcher();
            let comps: Vec<&str> = r.pattern.split('/').filter(|c| !c.is_empty()).collect();
            let literal = comps
                .iter()
                .take_while(|c| !c.contains(['*', '?', '[', '{']))
                .count();
            // For a pattern with no wildcards at all, keep the file name.
            let strip = literal.min(comps.len().saturating_sub(1));
            // `{mod}` is substituted later; check the rest of the target now.
            RelPath::parse(&r.target.replace("{mod}", "x"))
                .map_err(|e| GameDefError::Target(r.target.clone(), e))?;
            rules.push(CompiledRule {
                matcher,
                strip,
                target: r.target.clone(),
                catch_all: r.pattern.trim_matches('/') == "**"
                    && r.target.trim_matches('/').is_empty(),
            });
        }
        Ok(CompiledDef {
            def: self.clone(),
            rules,
            protect: globset(&self.protect)?,
            writable: globset(&self.writable)?,
        })
    }
}

fn glob(pattern: &str) -> Result<globset::Glob, GameDefError> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        // Mods are made for Windows, which ignores case.
        .case_insensitive(true)
        .build()
        .map_err(|source| GameDefError::Glob {
            pattern: pattern.to_string(),
            source,
        })
}

fn globset(patterns: &[String]) -> Result<GlobSet, GameDefError> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(glob(p)?);
    }
    b.build().map_err(|source| GameDefError::Glob {
        pattern: patterns.join(", "),
        source,
    })
}

struct CompiledRule {
    matcher: GlobMatcher,
    strip: usize,
    target: String,
    /// `match = "**"`, `target = ""`: keeps every path as it is.
    catch_all: bool,
}

/// A validated game definition with its globs compiled.
pub struct CompiledDef {
    pub def: GameDef,
    rules: Vec<CompiledRule>,
    protect: GlobSet,
    writable: GlobSet,
}

impl CompiledDef {
    /// Maps a file inside a mod to its place in the game folder. The first matching rule wins:
    /// the literal folders before the pattern's first wildcard are replaced by the rule's
    /// target. With no rules, files keep their path. `None` means no rule matched.
    pub fn map_mod_file(
        &self,
        mod_id: &str,
        rel: &RelPath,
    ) -> Result<Option<RelPath>, RelPathError> {
        if self.rules.is_empty() {
            return Ok(Some(rel.clone()));
        }
        for r in &self.rules {
            if r.matcher.is_match(rel.as_str()) {
                let target = RelPath::parse(&r.target.replace("{mod}", mod_id))?;
                return Ok(Some(target.join(&rel.strip_components(r.strip))));
            }
        }
        Ok(None)
    }

    /// Whether a rule other than the keep-the-path catch-all (`**` → game folder) decides
    /// where this file goes: the game definition knows something about it.
    pub fn has_specific_rule(&self, rel: &RelPath) -> bool {
        self.rules
            .iter()
            .find(|r| r.matcher.is_match(rel.as_str()))
            .is_some_and(|r| !r.catch_all)
    }

    pub fn is_protected(&self, target: &RelPath) -> bool {
        self.protect.is_match(target.as_str())
    }

    pub fn is_writable(&self, target: &RelPath) -> bool {
        self.writable.is_match(target.as_str())
    }

    /// Top-level folders of rules that give each mod a folder of its own (`plugins/**` →
    /// `BepInEx/plugins/{mod}/`): where a mod's content can go when nothing else says.
    pub fn per_mod_top_dirs(&self) -> Vec<String> {
        self.def
            .install
            .iter()
            .filter(|r| r.target.contains("{mod}"))
            .filter_map(|r| {
                r.pattern
                    .split('/')
                    .next()
                    .filter(|_| r.pattern.contains('/'))
            })
            .filter(|c| !c.is_empty() && !c.contains(['*', '?', '[', '{']))
            .map(|c| c.to_string())
            .collect()
    }

    /// Top-level folder names the install rules expect inside mods (e.g. `BepInEx`), used to
    /// decide whether an archive's single top-level folder is a wrapper or real content.
    pub fn content_top_dirs(&self) -> Vec<String> {
        self.def
            .install
            .iter()
            .filter_map(|r| r.pattern.split('/').next())
            .filter(|c| !c.is_empty() && !c.contains(['*', '?', '[', '{']))
            .map(|c| c.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_folders_are_in_the_prefix_for_a_game_under_proton() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("pfx/drive_c/users/steamuser");
        let vars = PathVars::in_prefix(user.clone());
        let at = |s: &str| expand_path_with(s, &vars).unwrap();
        assert_eq!(
            at("%LOCALAPPDATA%/Fallout3/plugins.txt"),
            user.join("AppData/Local").join("Fallout3/plugins.txt")
        );
        assert_eq!(at("%APPDATA%/x"), user.join("AppData/Roaming").join("x"));
        assert_eq!(at("%USERPROFILE%/x"), user.join("x"));
        assert_eq!(
            at("%PROGRAMDATA%/x"),
            tmp.path().join("pfx/drive_c/ProgramData").join("x")
        );
        assert_eq!(at("%DOCUMENTS%/x"), user.join("Documents").join("x"));
        // An older prefix's Documents.
        std::fs::create_dir_all(user.join("My Documents")).unwrap();
        assert_eq!(at("%DOCUMENTS%/x"), user.join("My Documents").join("x"));
    }

    // Elsewhere these are not set, unless in a prefix.
    #[test]
    #[cfg(windows)]
    fn expanded_paths_read_as_the_system_writes_them() {
        let p = expand_path("%LOCALAPPDATA%/Fallout3/plugins.txt").unwrap();
        assert!(p.ends_with("Fallout3/plugins.txt"));
        if cfg!(windows) {
            assert!(!p.display().to_string().contains('/'), "{}", p.display());
        }
    }

    const DEF: &str = r#"
schema = 1
name = "Example"
protect = ["*.exe"]
writable = ["BepInEx/config/**"]

[detect]
marker = "Game.exe"

[loader]
kind = "bepinex"
source = "mod:bepinex-pack"
inject = "doorstop"

[[install]]
match = "BepInEx/plugins/**"
target = "BepInEx/plugins/{mod}/"

[[install]]
match = "*.dll"
target = "BepInEx/plugins/{mod}/"

[[install]]
match = "**"
target = ""
"#;

    fn compiled() -> CompiledDef {
        GameDef::parse_game(DEF, Path::new("game.toml"), "example")
            .unwrap()
            .compile()
            .unwrap()
    }

    fn map(c: &CompiledDef, p: &str) -> String {
        c.map_mod_file("my-mod", &RelPath::parse(p).unwrap())
            .unwrap()
            .unwrap()
            .to_string()
    }

    #[test]
    fn install_rules_map_files() {
        let c = compiled();
        assert_eq!(
            map(&c, "BepInEx/plugins/Foo/Foo.dll"),
            "BepInEx/plugins/my-mod/Foo/Foo.dll"
        );
        assert_eq!(map(&c, "Bar.dll"), "BepInEx/plugins/my-mod/Bar.dll");
        // `*.dll` does not cross folders, so this falls through to the identity rule.
        assert_eq!(map(&c, "sub/Baz.dll"), "sub/Baz.dll");
        let rel = |p: &str| RelPath::parse(p).unwrap();
        assert!(c.has_specific_rule(&rel("Bar.dll")));
        assert!(
            !c.has_specific_rule(&rel("sub/Baz.dll")),
            "only the catch-all"
        );
        assert_eq!(
            c.def.loader.as_ref().unwrap().source_mod(),
            Some("bepinex-pack")
        );
    }

    #[test]
    fn protect_and_writable() {
        let c = compiled();
        assert!(c.is_protected(&RelPath::parse("Game.exe").unwrap()));
        assert!(!c.is_protected(&RelPath::parse("sub/Game.exe").unwrap()));
        assert!(c.is_writable(&RelPath::parse("BepInEx/config/a.cfg").unwrap()));
        assert_eq!(c.content_top_dirs(), ["BepInEx"]);
    }

    #[test]
    fn the_example_definition_loads() {
        let text = include_str!("../../../examples/example-unity-bepinex.toml");
        let def = GameDef::parse_game(text, Path::new("game.toml"), "example-unity-game").unwrap();
        assert!(def.loader.as_ref().unwrap().from_thunderstore());
        assert_eq!(def.nexus.unwrap().domain, "examplegame");
    }

    #[test]
    fn rejects_bad_definitions() {
        let bad_target = DEF.replace("target = \"\"", "target = \"../escape\"");
        let p = Path::new("game.toml");
        assert!(matches!(
            GameDef::parse_game(&bad_target, p, "example"),
            Err(GameDefError::Target(..))
        ));
        assert!(matches!(
            GameDef::parse_game(DEF, p, "Bad Id"),
            Err(GameDefError::Id(_))
        ));
        let typo = DEF.replace("protect =", "protekt =");
        assert!(matches!(
            GameDef::parse_game(&typo, p, "example"),
            Err(GameDefError::Parse { .. })
        ));
    }
}
