//! Listing installed Steam games and drafting a game definition for one.
//!
//! The draft is a starting point: it finds the exe, recognises Unity (Mono or IL2CPP) and
//! Unreal layouts, and picks matching loader and install rules. Unusual games still need
//! their rules edited by hand.

use crate::detect::steam_libraries;
use crate::vdf::{self, Value};
use crate::{
    slug, Community, Detect, GameDef, GameDefError, GameIniCondition, IniSetting, InstallRule,
    Launch, LoaderSetup, ParadoxList, PluginListFormat, Plugins, SteamRelaunch, Thunderstore,
    SCHEMA_VERSION, SHIMLOADER_KIND,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamApp {
    pub app_id: u32,
    pub name: String,
    /// Folder name under `steamapps/common`.
    pub install_dir: String,
    /// Full path of the install folder.
    pub path: PathBuf,
}

/// Steam's own runtimes and redistributables, which are not games.
const NOT_GAMES: &[u32] = &[
    228980,  // Steamworks Common Redistributables
    1070560, // Steam Linux Runtime 1.0 (scout)
    1391110, // Steam Linux Runtime 2.0 (soldier)
    1628350, // Steam Linux Runtime 3.0 (sniper)
    250820,  // SteamVR
];

fn is_tool(app_id: u32, name: &str) -> bool {
    NOT_GAMES.contains(&app_id)
        || name.starts_with("Proton ")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks ")
}

/// Every installed game in every Steam library on this machine, sorted by name.
pub fn installed_games() -> Vec<SteamApp> {
    installed_games_in(&steam_libraries())
}

/// Every installed game in the given library folders (each holding a `steamapps` folder).
pub fn installed_games_in(libraries: &[PathBuf]) -> Vec<SteamApp> {
    let mut out: Vec<SteamApp> = Vec::new();
    for lib in libraries {
        let steamapps = lib.join("steamapps");
        let Ok(rd) = fs::read_dir(&steamapps) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Some(app) = read_manifest(&e.path(), &steamapps) else {
                continue;
            };
            if !is_tool(app.app_id, &app.name) && out.iter().all(|a| a.app_id != app.app_id) {
                out.push(app);
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn read_manifest(path: &Path, steamapps: &Path) -> Option<SteamApp> {
    let text = fs::read_to_string(path).ok()?;
    let doc = vdf::parse(&text)?;
    let Value::Obj(state) = vdf::get(&doc, "AppState")? else {
        return None;
    };
    let get = |k: &str| match vdf::get(state, k) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let app_id: u32 = get("appid")?.parse().ok()?;
    let install_dir = get("installdir")?;
    let path = steamapps.join("common").join(&install_dir);
    if !path.is_dir() {
        return None; // listed but not (fully) installed
    }
    Some(SteamApp {
        app_id,
        name: get("name").unwrap_or_else(|| install_dir.clone()),
        install_dir,
        path,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    UnityMono,
    UnityIl2cpp,
    Unreal,
    /// Bethesda's engines (NetImmerse, Gamebryo, Creation Engine): a data folder of plugins
    /// and archives.
    Bethesda,
    /// Paradox's engines: mods in the game's user folder, enabled in `dlc_load.json`.
    Clausewitz,
    /// Factorio's own engine: a folder per mod in its user folder, enabled in `mod-list.json`.
    Factorio,
    /// Larian's Divinity Engine: `.pak` mods in the game's user folder, enabled in
    /// `modsettings.lsx`.
    Divinity,
    /// Oxygen Not Included: a Unity game with Klei's own mod loader, a folder per mod in
    /// Documents, enabled in `mods.json`.
    OxygenNotIncluded,
    /// Mount & Blade II: Bannerlord: modules in `Modules/`, loaded as the game is told.
    Bannerlord,
    /// XCOM 2 (Unreal Engine 3): a folder per mod in `XComGame/Mods/`, enabled in the user's
    /// `XComModOptions.ini`.
    Xcom2,
    /// GameMaker: the game's code and assets in one data file (`data.win`), which mods
    /// replace or patch (`.xdelta`).
    GameMaker,
    Unknown,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::UnityMono => "Unity (Mono)",
            Engine::UnityIl2cpp => "Unity (IL2CPP)",
            Engine::Unreal => "Unreal Engine",
            Engine::Bethesda => "Gamebryo / Creation Engine",
            Engine::Clausewitz => "Clausewitz / Jomini (Paradox)",
            Engine::Factorio => "Factorio (Wube)",
            Engine::Divinity => "Divinity Engine (Larian)",
            Engine::OxygenNotIncluded => "Unity (Mono), Klei's mod loader",
            Engine::Bannerlord => "TaleWorlds Engine (Bannerlord)",
            Engine::Xcom2 => "Unreal Engine 3 (XCOM 2)",
            Engine::GameMaker => "GameMaker",
            Engine::Unknown => "Unknown engine",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub def: GameDef,
    pub engine: Engine,
    pub setup: LoaderSetup,
    /// Things the user should know or check, in plain words.
    pub notes: Vec<Note>,
}

/// A message about a draft. `key` names the kind of message (`"bepinex"`,
/// `"unknown-engine"`, ...) and stays the same when the wording names a specific game, so an
/// interface can let the user hide a kind of message for good.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Note {
    pub key: &'static str,
    pub text: String,
}

impl Note {
    pub fn new(key: &'static str, text: impl Into<String>) -> Self {
        Note {
            key,
            text: text.into(),
        }
    }
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// An id for the app: its Steam name as written, else its folder name, else `steam-<app id>`.
fn app_id_slug(app: &SteamApp) -> String {
    [slug(&app.name), slug(&app.install_dir)]
        .into_iter()
        .flatten()
        .next()
        .unwrap_or_else(|| format!("steam-{}", app.app_id))
}

const NOT_THE_GAME: &[&str] = &[
    "unitycrashhandler",
    "crashreport",
    "crashhandler",
    "unins",
    "setup",
    "vcredist",
    "dxsetup",
    "dotnet",
    "redist",
    "launcherpatcher",
    "easyanticheat",
    "battleye",
    "cefsharp",
    "ue4prereq",
    "uninstall",
    "updater",
    "benchmark",
];

fn exe_files(dir: &Path) -> Vec<(String, u64)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if !lower.ends_with(".exe") || NOT_THE_GAME.iter().any(|n| lower.contains(n)) {
                return None;
            }
            Some((name, e.metadata().ok()?.len()))
        })
        .collect()
}

fn exists_ci(dir: &Path, name: &str) -> bool {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        })
        .unwrap_or(false)
}

/// What the install folder says about the game: its engine, exe and (Unreal) project folder.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub engine: Engine,
    /// Relative path of the likely game exe; empty when none was found.
    pub exe: String,
    /// Every exe that could be the game, the likely one first.
    pub exes: Vec<String>,
    pub unreal_project: Option<String>,
    /// A Bethesda game's data folder (`Data`, `Data Files`), as spelled on disk.
    pub data_folder: Option<String>,
    /// A Paradox game, recognised by its exe.
    pub paradox: Option<&'static ParadoxGame>,
    /// A Larian game, recognised by its exe.
    pub larian: Option<&'static LarianGame>,
}

/// A Paradox game: its exe (relative to the game folder) and its user folder's name under
/// `Documents\Paradox Interactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParadoxGame {
    pub exe: &'static str,
    pub user: &'static str,
    /// Where it lists its enabled mods.
    pub list: ParadoxList,
    /// Its exe's arguments to start the game directly.
    pub args: &'static [&'static str],
}

/// Paradox games whose mods go in their user folder.
pub const PARADOX_GAMES: &[ParadoxGame] = &[
    ParadoxGame {
        exe: "binaries/ck3.exe",
        user: "Crusader Kings III",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "binaries/victoria3.exe",
        user: "Victoria 3",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "binaries/imperator.exe",
        user: "Imperator",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "stellaris.exe",
        user: "Stellaris",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "hoi4.exe",
        user: "Hearts of Iron IV",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "eu4.exe",
        user: "Europa Universalis IV",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    // Its own launcher, built into the exe, is skipped: the game is then given its mods as
    // `-mod=` arguments, from `last_mods` in `settings.txt` (what that launcher shows ticked).
    ParadoxGame {
        exe: "CK2game.exe",
        user: "Crusader Kings II",
        list: ParadoxList::Settings,
        args: &["-skiplauncher"],
    },
];

/// Whether `root` has the file at `rel` (`/`-separated), in any case.
fn has_file_ci(root: &Path, rel: &str) -> bool {
    let mut p = root.to_path_buf();
    for c in rel.split('/') {
        let Some(found) = fs::read_dir(&p).ok().and_then(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case(c))
        }) else {
            return false;
        };
        p = p.join(found);
    }
    p.is_file()
}

/// The Paradox game installed in `root`, by its exe.
pub fn paradox_game(root: &Path) -> Option<&'static ParadoxGame> {
    PARADOX_GAMES.iter().find(|g| has_file_ci(root, g.exe))
}

/// A Larian game: its exe (relative to the game folder), its user folder, with variables, and
/// how it lists its modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LarianGame {
    pub exe: &'static str,
    pub user: &'static str,
    pub format: crate::LarianFormat,
}

/// Larian games whose mods go in their user folder.
pub const LARIAN_GAMES: &[LarianGame] = &[
    LarianGame {
        // Started directly, past Larian's launcher (`Launcher/LariLauncher.exe`).
        exe: "bin/bg3.exe",
        user: "%LOCALAPPDATA%/Larian Studios/Baldur's Gate 3",
        format: crate::LarianFormat::Bg3,
    },
    LarianGame {
        // Definitive Edition, started directly, past its launcher.
        exe: "DefEd/bin/EoCApp.exe",
        user: "%DOCUMENTS%/Larian Studios/Divinity Original Sin 2 Definitive Edition",
        format: crate::LarianFormat::Dos2,
    },
];

/// The Larian game installed in `root`, by its exe.
pub fn larian_game(root: &Path) -> Option<&'static LarianGame> {
    LARIAN_GAMES.iter().find(|g| has_file_ci(root, g.exe))
}

/// The name UMM gives a game's user folder outside the game folder in its definition
/// (Paradox games, Factorio): `@user/…` in its install rules.
pub const USER_FOLDER: &str = "user";

fn by_size(mut exes: Vec<(String, u64)>) -> Vec<String> {
    exes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    exes.into_iter().map(|(n, _)| n).collect()
}

/// Looks at a Steam game's install folder.
pub fn inspect(app: &SteamApp) -> Inspection {
    inspect_folder(&app.path)
}

/// Looks at an install folder to recognise the engine and find the exe.
pub fn inspect_folder(root: &Path) -> Inspection {
    let mut exes = by_size(exe_files(root));
    // Unity: `<Name>.exe` next to `<Name>_Data/` and UnityPlayer.dll.
    let unity_exe = exes.iter().find(|n| {
        let stem = &n[..n.len() - 4];
        root.join(format!("{stem}_Data")).is_dir()
    });
    if let Some(exe) = unity_exe
        .filter(|_| exists_ci(root, "UnityPlayer.dll"))
        .cloned()
    {
        let engine = if exe.eq_ignore_ascii_case(ONI_EXE) {
            Engine::OxygenNotIncluded
        } else if exists_ci(root, "GameAssembly.dll") {
            Engine::UnityIl2cpp
        } else {
            Engine::UnityMono
        };
        exes.retain(|e| e != &exe);
        exes.insert(0, exe.clone());
        return Inspection {
            engine,
            exe,
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    // Paradox: its exe, often in `binaries/` (the top holds its launcher).
    if let Some(g) = paradox_game(root) {
        exes.retain(|e| !e.eq_ignore_ascii_case(g.exe));
        exes.insert(0, g.exe.to_string());
        return Inspection {
            engine: Engine::Clausewitz,
            exe: g.exe.to_string(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: Some(g),
            larian: None,
        };
    }
    // Larian: its exe in `bin/` (the top holds its launcher's folder).
    if let Some(g) = larian_game(root) {
        exes.retain(|e| !e.eq_ignore_ascii_case(g.exe));
        exes.insert(0, g.exe.to_string());
        return Inspection {
            engine: Engine::Divinity,
            exe: g.exe.to_string(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: Some(g),
        };
    }
    // Bannerlord: its exe in `bin/Win64_Shipping_Client/`, its own modules in `Modules/`.
    if has_file_ci(root, BANNERLORD_EXE) && has_file_ci(root, "Modules/Native/SubModule.xml") {
        exes.retain(|e| !e.eq_ignore_ascii_case(BANNERLORD_EXE));
        exes.insert(0, BANNERLORD_EXE.to_string());
        return Inspection {
            engine: Engine::Bannerlord,
            exe: BANNERLORD_EXE.to_string(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    // XCOM 2: War of the Chosen when installed (its own game, in its folder), else the base
    // game; both started directly, past 2K's launcher.
    if has_file_ci(root, XCOM2_EXE) {
        let exe = if has_file_ci(root, XCOM2_WOTC_EXE) {
            XCOM2_WOTC_EXE
        } else {
            XCOM2_EXE
        };
        exes.retain(|e| !e.eq_ignore_ascii_case(exe));
        exes.insert(0, exe.to_string());
        return Inspection {
            engine: Engine::Xcom2,
            exe: exe.to_string(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    // Factorio: its exe in `bin/x64/`, its base mod in `data/`.
    if root.join(FACTORIO_EXE).is_file() && root.join("data/base/info.json").is_file() {
        exes.retain(|e| !e.eq_ignore_ascii_case(FACTORIO_EXE));
        exes.insert(0, FACTORIO_EXE.to_string());
        return Inspection {
            engine: Engine::Factorio,
            exe: FACTORIO_EXE.to_string(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    // Bethesda: a data folder at the top holding a master plugin (`Fallout3.esm`).
    if let Some(data) = bethesda_data_folder(root) {
        return Inspection {
            engine: Engine::Bethesda,
            exe: exes.first().cloned().unwrap_or_default(),
            exes,
            unreal_project: None,
            data_folder: Some(data),
            paradox: None,
            larian: None,
        };
    }
    if exists_ci(root, "Engine") {
        // Unreal: a small launcher exe at the root, the real one in <Project>/Binaries/Win64.
        let project = fs::read_dir(root).ok().and_then(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| {
                    !n.eq_ignore_ascii_case("Engine") && root.join(n).join("Binaries").is_dir()
                })
        });
        if let Some(p) = &project {
            exes.extend(
                by_size(exe_files(&root.join(p).join("Binaries").join("Win64")))
                    .into_iter()
                    .map(|e| format!("{p}/Binaries/Win64/{e}")),
            );
        }
        return Inspection {
            engine: Engine::Unreal,
            exe: exes.first().cloned().unwrap_or_default(),
            exes,
            unreal_project: project,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    // GameMaker: its data file next to the exe, a real one (`FORM` … `GEN8`).
    let gamemaker_data = fs::read_dir(root).ok().and_then(|rd| {
        rd.flatten()
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(GAMEMAKER_DATA)
            })
            .map(|e| e.path())
    });
    if !exes.is_empty() && gamemaker_data.is_some_and(|p| crate::gamemaker::is_data_file(&p)) {
        return Inspection {
            engine: Engine::GameMaker,
            exe: exes.first().cloned().unwrap_or_default(),
            exes,
            unreal_project: None,
            data_folder: None,
            paradox: None,
            larian: None,
        };
    }
    Inspection {
        engine: Engine::Unknown,
        exe: exes.first().cloned().unwrap_or_default(),
        exes,
        unreal_project: None,
        data_folder: None,
        paradox: None,
        larian: None,
    }
}

/// A GameMaker game's data file (Windows builds').
pub const GAMEMAKER_DATA: &str = "data.win";

/// Factorio's exe, relative to its folder.
pub const FACTORIO_EXE: &str = "bin/x64/factorio.exe";

/// Factorio's user folder when it uses the system's (`config-path.cfg` says so: Steam's and
/// the installer's copies). A zip copy keeps everything in its own folder instead.
const FACTORIO_USER: &str = "%APPDATA%/Factorio";

/// Whether the Factorio copy in `root` keeps its mods in its own folder (`config-path.cfg`
/// with `use-system-read-write-data-directories=false`, or none).
fn factorio_portable(root: &Path) -> bool {
    let Ok(text) = fs::read_to_string(root.join("config-path.cfg")) else {
        return true;
    };
    !text.lines().any(|l| {
        l.split_once('=').is_some_and(|(k, v)| {
            k.trim() == "use-system-read-write-data-directories" && v.trim() == "true"
        })
    })
}

/// Bannerlord's exe, started directly past its launcher
/// (`TaleWorlds.MountAndBlade.Launcher.exe` beside it).
pub const BANNERLORD_EXE: &str = "bin/Win64_Shipping_Client/Bannerlord.exe";

/// XCOM 2's exe, and War of the Chosen's, in its folder within the game's.
pub const XCOM2_EXE: &str = "Binaries/Win64/XCom2.exe";
pub const XCOM2_WOTC_EXE: &str = "XCom2-WarOfTheChosen/Binaries/Win64/XCom2.exe";

/// Oxygen Not Included's exe, at the top of its folder.
pub const ONI_EXE: &str = "OxygenNotIncluded.exe";

/// Oxygen Not Included's user folder (its mods in `mods/Local`, their list in `mods/mods.json`).
const ONI_USER: &str = "%DOCUMENTS%/Klei/OxygenNotIncluded";

/// The folder at the top of a game folder named `Data` or `Data Files` that holds an `.esm`
/// master, as spelled on disk.
fn bethesda_data_folder(root: &Path) -> Option<String> {
    let rd = fs::read_dir(root).ok()?;
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.eq_ignore_ascii_case("Data") || n.eq_ignore_ascii_case("Data Files"))
        .find(|n| {
            fs::read_dir(root.join(n)).is_ok_and(|rd| {
                rd.flatten().any(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".esm") && e.path().is_file()
                })
            })
        })
}

/// Where a draft's game comes from: everything besides the files that goes in its definition.
struct Origin {
    id: String,
    name: String,
    detect: Detect,
    relaunch: SteamRelaunch,
    /// The game folder.
    root: PathBuf,
}

/// A game whose plugins (or mods) must be listed to load, recognised by its own exe in the
/// game folder: every edition has it, regional ones (Fallout: New Vegas PCR) included,
/// whichever store it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListGame {
    pub exe: &'static str,
    pub list: ListAt,
    /// The folder holding plugins (RimWorld: mods), relative to the game folder.
    pub folder: &'static str,
    /// Editions keeping their `plugins.txt` in another folder under local app data: a file
    /// only that edition has in the game folder, and the folder. As LOOT's libloadorder tells
    /// them apart.
    pub editions: &'static [(&'static str, &'static str)],
    pub format: PluginListFormat,
    /// Where its `Custom.ini` (and `Prefs.ini`) live, under Documents\My Games, when it needs
    /// ini settings for mods to load.
    pub ini: Option<&'static str>,
}

/// Where a game keeps its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAt {
    /// `plugins.txt` in this folder under the user's local app data.
    LocalAppData(&'static str),
    /// This file in the game folder.
    Game(&'static str),
    /// This absolute path, with variables (see [`crate::expand_path`]).
    Path(&'static str),
}

const fn game(
    exe: &'static str,
    dir: &'static str,
    editions: &'static [(&'static str, &'static str)],
    format: PluginListFormat,
    ini: Option<&'static str>,
) -> ListGame {
    ListGame {
        exe,
        list: ListAt::LocalAppData(dir),
        folder: "Data",
        editions,
        format,
        ini,
    }
}

const fn other(
    exe: &'static str,
    list: ListAt,
    folder: &'static str,
    format: PluginListFormat,
) -> ListGame {
    ListGame {
        exe,
        list,
        folder,
        editions: &[],
        format,
        ini: None,
    }
}

use PluginListFormat::{
    Asterisk, FactorioJson, KenshiCfg, MorrowindIni, OniJson, Plain, RimworldXml, XcomIni,
};

/// RimWorld's list, in its folder under `AppData\LocalLow` (which has no variable of its own).
const RIMWORLD_LIST: &str =
    "%USERPROFILE%/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Config/ModsConfig.xml";

/// Games keeping a list, VR editions before the flat ones they share files with.
pub const LIST_GAMES: &[ListGame] = &[
    game("SkyrimVR.exe", "Skyrim VR", &[], Asterisk, None),
    game(
        "SkyrimSE.exe",
        "Skyrim Special Edition",
        &[
            // Only GOG's installer installs Galaxy64.dll.
            ("Galaxy64.dll", "Skyrim Special Edition GOG"),
            ("EOSSDK-Win64-Shipping.dll", "Skyrim Special Edition EPIC"),
            ("appxmanifest.xml", "Skyrim Special Edition MS"),
        ],
        Asterisk,
        None,
    ),
    game("Fallout4VR.exe", "Fallout4VR", &[], Asterisk, None),
    game(
        "Fallout4.exe",
        "Fallout4",
        &[
            ("appxmanifest.xml", "Fallout4 MS"),
            ("EOSSDK-Win64-Shipping.dll", "Fallout4 EPIC"),
        ],
        Asterisk,
        Some("Fallout4"),
    ),
    game(
        "Starfield.exe",
        "Starfield",
        &[],
        Asterisk,
        Some("Starfield"),
    ),
    game("TESV.exe", "Skyrim", &[], Plain, None),
    game(
        "FalloutNV.exe",
        "FalloutNV",
        &[("EOSSDK-Win32-Shipping.dll", "FalloutNV_Epic")],
        Plain,
        None,
    ),
    game("Fallout3.exe", "Fallout3", &[], Plain, None),
    game("Oblivion.exe", "Oblivion", &[], Plain, None),
    // Its plugins and their list are in the Unreal project's content folder.
    other(
        "OblivionRemastered.exe",
        ListAt::Game("OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt"),
        "OblivionRemastered/Content/Dev/ObvData/Data",
        Plain,
    ),
    other(
        "Morrowind.exe",
        ListAt::Game("Morrowind.ini"),
        "Data Files",
        MorrowindIni,
    ),
    other(
        "RimWorldWin64.exe",
        ListAt::Path(RIMWORLD_LIST),
        "Mods",
        RimworldXml,
    ),
    other(
        "RimWorldWin.exe",
        ListAt::Path(RIMWORLD_LIST),
        "Mods",
        RimworldXml,
    ),
    // Steam's and GOG's builds.
    other(
        "kenshi_x64.exe",
        ListAt::Game("data/mods.cfg"),
        "mods",
        KenshiCfg,
    ),
    other(
        "kenshi_GOG_x64.exe",
        ListAt::Game("data/mods.cfg"),
        "mods",
        KenshiCfg,
    ),
];

/// The game keeping a list whose exe is at the top of a game folder (`exes`, relative paths,
/// as [`Inspection`] lists them).
pub fn list_game(exes: &[String]) -> Option<&'static ListGame> {
    LIST_GAMES.iter().find(|g| {
        exes.iter()
            .any(|x| !x.contains('/') && x.eq_ignore_ascii_case(g.exe))
    })
}

/// Gives a draft of a game keeping a list its plugin list (for the edition in `folder`), the
/// ini settings it needs, and (RimWorld) a folder per mod, with notes. Other games are left
/// as they are.
fn add_list_game(draft: &mut Draft, folder: &Path, found: &Inspection) {
    let Some(g) = list_game(&found.exes) else {
        return;
    };
    let plugins = g.plugins(folder);
    let note = match g.format {
        Asterisk | Plain => format!(
            "Bethesda game: while mods are deployed, UMM enables their plugins in {}.",
            plugins.list
        ),
        MorrowindIni => "Morrowind: while mods are deployed, UMM enables their plugins in the [Game Files] section of Morrowind.ini. The game loads plugins in the order of their file dates.".into(),
        KenshiCfg => "Kenshi: each mod goes in its own folder in mods/, named after its .mod file as the game wants, and while mods are deployed, UMM enables them in data/mods.cfg, in load order (your own list comes back afterwards). Steam Workshop items can be turned on or off in a profile too.".into(),
        // Factorio, Oxygen Not Included and XCOM 2 are set up in `build`, not as list games.
        RimworldXml | FactorioJson | OniJson | XcomIni => format!(
            "RimWorld: each mod goes in its own folder in Mods/, and while mods are deployed, UMM activates them in {}, in load order.",
            plugins.list
        ),
    };
    draft.notes.push(Note::new("plugins", note));
    // The list says what the game is, whatever its engine.
    draft.notes.retain(|n| n.key != "unknown-engine");
    if matches!(g.format, RimworldXml | KenshiCfg) && draft.setup == LoaderSetup::Generic {
        for r in &mut draft.def.install {
            if r.pattern == "**" && r.target.is_empty() {
                r.target = format!("{}/{{mod}}/", g.folder);
            }
        }
    }
    draft.def.plugins = Some(plugins);
    draft.def.ini = g.ini();
    if !draft.def.ini.is_empty() {
        draft.notes.push(Note::new(
            "ini",
            "Bethesda game: UMM sets the ini settings it needs to load mods (listed in this file) while they are deployed, and puts them back afterwards.",
        ));
    }
}

impl ListGame {
    /// Its plugin list, for the edition installed in `folder`.
    pub fn plugins(&self, folder: &Path) -> Plugins {
        let list = match self.list {
            ListAt::LocalAppData(dir) => {
                let dir = self
                    .editions
                    .iter()
                    .find(|(marker, _)| folder.join(marker).exists())
                    .map_or(dir, |(_, dir)| *dir);
                format!("%LOCALAPPDATA%/{dir}/plugins.txt")
            }
            ListAt::Game(p) | ListAt::Path(p) => p.into(),
        };
        Plugins {
            list,
            format: self.format,
            folder: self.folder.into(),
            // Oblivion set not to use My Games keeps its list in its own folder.
            in_game_folder_when: (self.exe == "Oblivion.exe").then(|| GameIniCondition {
                ini: "Oblivion.ini".into(),
                section: "General".into(),
                key: "bUseMyGamesDirectory".into(),
                value: "0".into(),
            }),
        }
    }

    /// The ini settings it needs for mods to load: Fallout 4 reads its plugin list only with
    /// `bEnableFileSelection`, and Fallout 4 and Starfield load loose files (outside `.ba2`
    /// archives) only with archive invalidation in their `Custom.ini`, a file of overrides UMM
    /// may create.
    pub fn ini(&self) -> Vec<IniSetting> {
        let Some(dir) = self.ini else {
            return Vec::new();
        };
        let set = |file: String, section: &str, key: &str, value: &str, create: bool| IniSetting {
            file,
            section: section.into(),
            key: key.into(),
            value: value.into(),
            create,
        };
        let base = format!("%DOCUMENTS%/My Games/{dir}");
        let mut v = Vec::new();
        if dir == "Fallout4" {
            v.push(set(
                format!("{base}/Fallout4Prefs.ini"),
                "Launcher",
                "bEnableFileSelection",
                "1",
                false,
            ));
        }
        let custom = format!("{base}/{dir}Custom.ini");
        v.push(set(
            custom.clone(),
            "Archive",
            "bInvalidateOlderFiles",
            "1",
            true,
        ));
        v.push(set(custom, "Archive", "sResourceDataDirsFinal", "", true));
        v
    }
}

/// Drafts a game definition with the given setup. `community` is the game's Thunderstore
/// community, needed for [`LoaderSetup::Thunderstore`].
pub fn draft_definition(
    app: &SteamApp,
    setup: LoaderSetup,
    community: Option<Community>,
) -> Result<Draft, GameDefError> {
    draft_from(app, &inspect(app), setup, community)
}

pub fn draft_from(
    app: &SteamApp,
    found: &Inspection,
    setup: LoaderSetup,
    community: Option<Community>,
) -> Result<Draft, GameDefError> {
    let origin = Origin {
        id: app_id_slug(app),
        name: app.name.clone(),
        detect: Detect {
            steam_app_id: Some(app.app_id),
            steam_dir: Some(app.install_dir.clone()),
            ..Default::default()
        },
        // Started by Steam, as players expect: their Steam launch options, Steam Cloud and
        // DRM all apply. UMM finds the game by its process names.
        relaunch: SteamRelaunch::ViaSteam,
        root: app.path.clone(),
    };
    let mut draft = build(origin, found, &found.exe, setup, community)?;
    add_list_game(&mut draft, &app.path, found);
    Ok(draft)
}

/// Games whose exe starts the game under other process names, which UMM watches too: Resonite
/// runs as a .NET host and a Unity renderer.
const HANDED_OFF: &[(&str, &[&str])] = &[(
    "Resonite.exe",
    &["Renderite.Host.exe", "Renderite.Renderer.exe"],
)];

/// Drafts a definition for a game added by hand: its folder, the name to show, and the exe
/// to launch (relative to the folder; empty for none).
pub fn draft_manual(
    folder: &Path,
    name: &str,
    exe: &str,
    found: &Inspection,
    setup: LoaderSetup,
    community: Option<Community>,
) -> Result<Draft, GameDefError> {
    let folder_name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let origin = Origin {
        id: slug(name)
            .or_else(|| slug(&folder_name))
            .unwrap_or_else(|| "game".into()),
        name: name.trim().to_string(),
        detect: Detect {
            path: Some(folder.to_path_buf()),
            ..Default::default()
        },
        relaunch: SteamRelaunch::None,
        root: folder.to_path_buf(),
    };
    let mut draft = build(origin, found, exe, setup, community)?;
    add_list_game(&mut draft, folder, found);
    Ok(draft)
}

fn build(
    origin: Origin,
    found: &Inspection,
    exe: &str,
    setup: LoaderSetup,
    community: Option<Community>,
) -> Result<Draft, GameDefError> {
    let mut notes = Vec::new();
    let mut install = Vec::new();
    let mut protect = vec!["*.exe".to_string()];
    let writable = vec![
        "**/*.cfg".to_string(),
        "**/*.ini".to_string(),
        "**/*.json".to_string(),
    ];
    let rule = |m: &str, t: &str| InstallRule {
        pattern: m.into(),
        target: t.into(),
    };

    match found.engine {
        Engine::UnityMono | Engine::UnityIl2cpp => protect.push("UnityPlayer.dll".into()),
        Engine::OxygenNotIncluded => {
            protect.push("UnityPlayer.dll".into());
            notes.push(Note::new(
                "oni",
                "Oxygen Not Included: each mod goes in its own folder in \
                 Documents\\Klei\\OxygenNotIncluded\\mods\\Local, and is enabled in mods.json \
                 there while mods are deployed (your own list comes back afterwards). The game \
                 loads mods with its own loader; no BepInEx is needed.",
            ));
        }
        Engine::Unreal => {
            // Under a mod loader its own rules apply instead.
            if let Some(p) = found
                .unreal_project
                .as_ref()
                .filter(|_| setup == LoaderSetup::Generic)
            {
                for ext in ["pak", "utoc", "ucas"] {
                    install.push(rule(
                        &format!("*.{ext}"),
                        &format!("{p}/Content/Paks/~mods/"),
                    ));
                }
                notes.push(Note::new(
                    "unreal",
                    format!(
                        "Unreal Engine game: loose .pak/.utoc/.ucas files go to {p}/Content/Paks/~mods/. \
                         Everything else keeps its path."
                    ),
                ));
            }
        }
        Engine::Bethesda => {
            if let Some(d) = found
                .data_folder
                .as_ref()
                .filter(|_| setup == LoaderSetup::Generic)
            {
                install.extend(
                    crate::setup::bethesda_rules(d)
                        .into_iter()
                        .filter(|r| r.pattern != "**"),
                );
                notes.push(Note::new(
                    "bethesda",
                    format!(
                        "Bethesda game: plugins, archives and asset folders (meshes, textures, \
                         sound, script extender plugins…) go to {d}/, and FOMOD installers \
                         install there. Everything else keeps its path."
                    ),
                ));
            }
        }
        Engine::Clausewitz => {
            if let Some(g) = found.paradox {
                notes.push(Note::new(
                    "paradox",
                    format!(
                        "Paradox game: each mod goes in its own folder in Documents\\Paradox Interactive\\{}\\mod, \
                         with a descriptor UMM writes, and is enabled in {} while mods are deployed \
                         (your own list comes back afterwards). UMM starts the game directly, not through \
                         {}.",
                        g.user,
                        match g.list {
                            ParadoxList::DlcLoad => "dlc_load.json",
                            ParadoxList::Settings => "settings.txt",
                        },
                        match g.list {
                            ParadoxList::DlcLoad =>
                                "Paradox's launcher, which would replace the list with its playset",
                            ParadoxList::Settings => "its launcher",
                        }
                    ),
                ));
            }
        }
        Engine::Xcom2 => notes.push(Note::new(
            "xcom2",
            format!(
                "XCOM 2{}: each mod goes in its own folder in {}XComGame/Mods/, named after its \
                 .XComMod file as the game wants, and while mods are deployed, UMM enables them \
                 in XComModOptions.ini in Documents\\My Games (your own list comes back \
                 afterwards). Steam Workshop items can be turned on or off in a profile too. UMM \
                 starts the game directly, not through 2K's launcher.",
                if exe.eq_ignore_ascii_case(XCOM2_WOTC_EXE) {
                    ": War of the Chosen"
                } else {
                    ""
                },
                if exe.eq_ignore_ascii_case(XCOM2_WOTC_EXE) {
                    "XCom2-WarOfTheChosen/"
                } else {
                    ""
                },
            ),
        )),
        Engine::Bannerlord => notes.push(Note::new(
            "bannerlord",
            "Bannerlord: each mod goes in its own folder in Modules/, named after the module's \
             id as the game wants. UMM starts the game past its launcher with the modules to \
             load: the game's own, your profile's (Steam Workshop items included), and those \
             you selected in the launcher, each after the ones it depends on.",
        )),
        Engine::Factorio => notes.push(Note::new(
            "factorio",
            "Factorio: each mod goes in its own folder in the game's mods folder, named after \
             the name in its info.json, and is enabled in mod-list.json while mods are deployed \
             (your own list comes back afterwards). Import mods as the .zip files from the mod \
             portal.",
        )),
        Engine::Divinity => {
            if let Some(g) = found.larian {
                notes.push(Note::new(
                    "larian",
                    format!(
                        "Larian game: .pak mods go to the Mods folder in {}, and are enabled in \
                         {} while mods are deployed (your own mods there stay). The game must \
                         have been started once, so that it has written that file. Script \
                         Extender and {}/ or {}/ files go to the game folder. UMM starts the \
                         game directly, not through Larian's launcher.",
                        g.user.replace('/', "\\"),
                        match g.format {
                            crate::LarianFormat::Bg3 => "modsettings.lsx",
                            crate::LarianFormat::Dos2 => "each player profile's modsettings.lsx",
                        },
                        g.format.bin(),
                        g.format.data(),
                    ),
                ));
            }
        }
        Engine::GameMaker => notes.push(Note::new(
            "gamemaker",
            "GameMaker: mods replace files in the game folder, data files included (one going \
             where the game's data file of the same name is, as a DELTARUNE chapter's), or \
             patch them with .xdelta files, which UMM applies while mods are deployed and takes \
             back afterwards. UMM finds the file each patch is for by trying it, and follows a \
             DELTAMOD package's modding.xml. Two mods patching the same file, each made for the \
             game's own, clash: UMM says so and launches nothing until one is disabled.",
        )),
        Engine::Unknown if setup == LoaderSetup::Generic => notes.push(Note::new(
            "unknown-engine",
            "Engine not recognised: mod files keep their paths relative to the game folder.",
        )),
        Engine::Unknown => {}
    }
    // Games listing a folder per mod: their user folder (if not the game's), its mods folder
    // and their list. Factorio's is in its user folder, or its own when it keeps everything
    // there.
    let mod_list = match found.engine {
        Engine::Factorio => Some(match factorio_portable(&origin.root) {
            true => (None, "mods".to_string(), "mods/mod-list.json".to_string()),
            false => (
                Some(FACTORIO_USER.to_string()),
                format!("@{USER_FOLDER}/mods"),
                format!("{FACTORIO_USER}/mods/mod-list.json"),
            ),
        })
        .map(|(u, m, l)| (u, m, l, FactorioJson)),
        Engine::Xcom2 => {
            let wotc = exe.eq_ignore_ascii_case(XCOM2_WOTC_EXE);
            Some((
                None,
                format!(
                    "{}XComGame/Mods",
                    if wotc { "XCom2-WarOfTheChosen/" } else { "" }
                ),
                format!(
                    "%DOCUMENTS%/My Games/{}/XComGame/Config/XComModOptions.ini",
                    if wotc {
                        "XCOM2 War of the Chosen"
                    } else {
                        "XCOM2"
                    }
                ),
                XcomIni,
            ))
        }
        Engine::OxygenNotIncluded => Some((
            Some(ONI_USER.to_string()),
            format!("@{USER_FOLDER}/mods/Local"),
            format!("{ONI_USER}/mods/mods.json"),
            OniJson,
        )),
        _ => None,
    };
    match (found.paradox, found.larian, &mod_list) {
        // A folder per module.
        _ if found.engine == Engine::Bannerlord => {
            install.extend(crate::setup::mod_folder_rules("Modules"))
        }
        // Mods go in the user folder's `mod/`, not the game folder.
        (Some(_), _, _) => install.extend(crate::setup::paradox_rules(USER_FOLDER)),
        // Paks in the user folder's `Mods/`.
        (None, Some(g), _) => install.extend(crate::setup::larian_rules(USER_FOLDER, g.format)),
        (None, None, Some((_, mods, _, _))) => install.extend(crate::setup::mod_folder_rules(mods)),
        (None, None, None) => install.push(rule("**", "")),
    }

    match (setup, found.engine) {
        (LoaderSetup::Thunderstore, _)
            if community.is_some_and(|c| c.loader == SHIMLOADER_KIND) =>
        {
            notes.push(Note::new(
                "thunderstore",
                "This game is set up for Shimloader (UE4SS). You can download mods directly \
                 from the Thunderstore tab.",
            ))
        }
        (LoaderSetup::Thunderstore, _) => notes.push(Note::new(
            "thunderstore",
            format!(
                "This game is set up for {}. You can download mods directly from the \
                 Thunderstore tab.",
                crate::loader_name(community.map_or(crate::BEPINEX_KIND, |c| c.loader))
            ),
        )),
        (LoaderSetup::Bepinex, Engine::UnityIl2cpp) => notes.push(Note::new(
            "bepinex",
            "Set up for BepInEx through Doorstop. Import a BepInEx 6 IL2CPP pack with the mod \
             id `bepinex`.",
        )),
        (LoaderSetup::Bepinex, _) => notes.push(Note::new(
            "bepinex",
            "Set up for BepInEx through Doorstop. Import a BepInEx pack (BepInEx 5.4 or 6) with \
             the mod id `bepinex`.",
        )),
        (LoaderSetup::Generic, Engine::UnityMono) => notes.push(Note::new(
            "generic-unity",
            "Unity game set up as generic: mod files are copied as they are. To use BepInEx \
             mods, change the mod loader in the Actions tab.",
        )),
        (LoaderSetup::Generic, Engine::UnityIl2cpp) => notes.push(Note::new(
            "unity-il2cpp",
            "IL2CPP Unity game set up as generic: mod files are copied as they are. To use \
             BepInEx 6 mods, change the mod loader in the Actions tab.",
        )),
        (LoaderSetup::Generic, _) => {}
    }

    let launch = if exe.is_empty() {
        notes.push(Note::new(
            "no-exe",
            "No game exe found; add a [launch] section by hand to launch from UMM.",
        ));
        None
    } else {
        let file = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
        let mut process_names = vec![file(exe)];
        // An Unreal game's launcher hands off to the real exe in <Project>/Binaries/Win64 and
        // exits: started through Steam, only the real one tells UMM the game is still running.
        if let Some(p) = &found.unreal_project {
            let bin = format!("{}/", crate::unreal_binaries_dir(p));
            for e in found.exes.iter().filter(|e| e.starts_with(&bin)) {
                let name = file(e);
                if !process_names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    process_names.push(name);
                }
            }
        }
        // The game's own launcher beside its exe (Fallout3Launcher.exe), which Steam may start
        // first: the game counts as running while either is, and a launcher closed without
        // playing ends the session.
        let dir = |p: &str| p.rsplit_once('/').map_or("", |(d, _)| d).to_lowercase();
        for e in found.exes.iter().filter(|e| dir(e) == dir(exe)) {
            let name = file(e);
            if name.to_lowercase().contains("launcher")
                && !process_names.iter().any(|n| n.eq_ignore_ascii_case(&name))
            {
                process_names.push(name);
            }
        }
        // Games whose exe hands off to processes of other names.
        for (launcher, others) in HANDED_OFF {
            if file(exe).eq_ignore_ascii_case(launcher) {
                process_names.extend(others.iter().map(|o| o.to_string()));
            }
        }
        Some(Launch {
            exe: exe.to_string(),
            args: found
                .paradox
                .filter(|g| exe.eq_ignore_ascii_case(g.exe))
                .map(|g| g.args.iter().map(|a| a.to_string()).collect())
                .unwrap_or_default(),
            // A Paradox game started through Steam runs Paradox's launcher, which writes the
            // list of enabled mods from its playset: started directly instead, Steam running.
            // Larian's and Bannerlord's launchers likewise stand before the game.
            steam_relaunch: if (found.paradox.is_some()
                || found.larian.is_some()
                || matches!(found.engine, Engine::Bannerlord | Engine::Xcom2))
                && origin.relaunch == SteamRelaunch::ViaSteam
            {
                SteamRelaunch::AppidFile
            } else {
                origin.relaunch
            },
            process_names,
            ignore_extender_loader: false,
        })
    };

    let mut detect = origin.detect;
    detect.marker = launch.as_ref().map(|l| l.exe.clone());
    let generic = GameDef {
        schema: SCHEMA_VERSION,
        id: origin.id,
        name: origin.name,
        detect,
        launch,
        loader: None,
        nexus: None,
        gamebanana: None,
        plugins: mod_list.as_ref().map(|(_, mods, list, format)| Plugins {
            list: list.clone(),
            format: *format,
            folder: mods.clone(),
            in_game_folder_when: None,
        }),
        ini: Vec::new(),
        thunderstore: community.map(|c| Thunderstore {
            community: c.label.to_string(),
        }),
        install,
        protect,
        writable,
        folders: found
            .paradox
            .map(|g| format!("%DOCUMENTS%/Paradox Interactive/{}", g.user))
            .or_else(|| found.larian.map(|g| g.user.to_string()))
            .or_else(|| mod_list.and_then(|(user, ..)| user))
            .map(|user| [(USER_FOLDER.to_string(), user)].into_iter().collect())
            .unwrap_or_default(),
        paradox: found.paradox.map(|g| crate::Paradox {
            folder: USER_FOLDER.into(),
            list: g.list,
        }),
        bannerlord: (found.engine == Engine::Bannerlord).then(|| crate::Bannerlord {
            folder: "Modules".into(),
        }),
        larian: found.larian.map(|g| crate::Larian {
            folder: USER_FOLDER.into(),
            format: g.format,
        }),
    };
    let def = generic.with_setup(setup, community)?;
    Ok(Draft {
        def,
        engine: found.engine,
        setup,
        notes,
    })
}

/// The draft as a `game.toml`, with a header saying where it came from.
pub fn to_toml(draft: &Draft) -> String {
    let source = match draft.def.detect.steam_app_id {
        Some(app) => format!(
            "Generated from Steam by UMM ({}, {}; app {app}).",
            draft.engine.label(),
            draft.def.setup_label()
        ),
        None => format!(
            "Added by UMM ({}, {}).",
            draft.engine.label(),
            draft.def.setup_label()
        ),
    };
    let mut header = vec![
        source,
        "Review the install rules before adding mods. Edit freely: UMM only changes the install path and the mod loader sections.".to_string(),
    ];
    header.extend(draft.notes.iter().map(|n| format!("Note: {n}")));
    crate::edit::new_file(&draft.def, &header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
    }

    fn manifest(lib: &Path, id: u32, name: &str, dir: &str) {
        let text = format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{dir}\"\n}}\n"
        );
        touch(
            &lib.join("steamapps").join(format!("appmanifest_{id}.acf")),
            0,
        );
        fs::write(
            lib.join("steamapps").join(format!("appmanifest_{id}.acf")),
            text,
        )
        .unwrap();
    }

    fn library() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("SteamLibrary");
        let common = lib.join("steamapps/common");

        manifest(&lib, 1001, "Mono Quest: Deluxe", "Mono Quest");
        touch(&common.join("Mono Quest/MonoQuest.exe"), 600);
        touch(&common.join("Mono Quest/UnityCrashHandler64.exe"), 900);
        touch(&common.join("Mono Quest/UnityPlayer.dll"), 10);
        touch(
            &common.join("Mono Quest/MonoQuest_Data/globalgamemanagers"),
            10,
        );

        manifest(&lib, 1002, "Cpp Odyssey", "CppOdyssey");
        touch(&common.join("CppOdyssey/Odyssey.exe"), 600);
        touch(&common.join("CppOdyssey/UnityPlayer.dll"), 10);
        touch(&common.join("CppOdyssey/GameAssembly.dll"), 10);
        touch(&common.join("CppOdyssey/Odyssey_Data/x"), 10);

        manifest(&lib, 1003, "Unreal Trip", "UnrealTrip");
        touch(&common.join("UnrealTrip/Trip.exe"), 300);
        touch(&common.join("UnrealTrip/Engine/Binaries/x"), 10);
        touch(
            &common.join("UnrealTrip/Trip/Binaries/Win64/Trip-Win64-Shipping.exe"),
            9000,
        );

        manifest(&lib, 1004, "Plain Game", "Plain");
        touch(&common.join("Plain/plain.exe"), 900);
        touch(&common.join("Plain/unins000.exe"), 5000);

        manifest(
            &lib,
            228980,
            "Steamworks Common Redistributables",
            "Steamworks Shared",
        );
        touch(&common.join("Steamworks Shared/x"), 1);
        manifest(&lib, 1005, "Not Installed", "Missing"); // no folder
        (tmp, lib)
    }

    fn draft_for(apps: &[SteamApp], id: u32) -> Draft {
        draft_definition(
            apps.iter().find(|a| a.app_id == id).unwrap(),
            LoaderSetup::Generic,
            None,
        )
        .unwrap()
    }

    #[test]
    fn lists_installed_games_only() {
        let (_tmp, lib) = library();
        let apps = installed_games_in(&[lib.clone(), lib]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Cpp Odyssey",
                "Mono Quest: Deluxe",
                "Plain Game",
                "Unreal Trip"
            ]
        );
    }

    #[test]
    fn paradox_games_are_set_up_for_their_user_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("binaries/ck3.exe"), 100);
        touch(&root.join("launcher/dowser.exe"), 500);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Clausewitz, "binaries/ck3.exe")
        );
        let d = draft_manual(root, "CK3", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%DOCUMENTS%/Paradox Interactive/Crusader Kings III")
        );
        assert_eq!(d.def.paradox.as_ref().unwrap().folder, USER_FOLDER);
        assert_eq!(d.def.install.len(), 1);
        assert_eq!(d.def.install[0].target, "@user/mod/{mod}/");
        assert!(d.notes.iter().any(|n| n.key == "paradox"));
        // Reloads as written, and back to generic keeps the rule.
        let text = to_toml(&d);
        let back = GameDef::parse_game(&text, Path::new("game.toml"), "ck3").unwrap();
        assert_eq!(back.paradox, d.def.paradox);
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(again.install[0].target, "@user/mod/{mod}/");
    }

    #[test]
    fn factorio_mods_go_to_its_user_folder_unless_it_keeps_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("bin/x64/factorio.exe"), 100);
        touch(&root.join("data/base/info.json"), 10);
        fs::write(
            root.join("config-path.cfg"),
            "config-path=__PATH__system-write-data__/config\n\
             # A comment\nuse-system-read-write-data-directories=true\n",
        )
        .unwrap();
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Factorio, FACTORIO_EXE)
        );
        let d = draft_manual(
            root,
            "Factorio",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%APPDATA%/Factorio")
        );
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.format, p.folder.as_str()),
            (
                "%APPDATA%/Factorio/mods/mod-list.json",
                PluginListFormat::FactorioJson,
                "@user/mods"
            )
        );
        assert!(d.notes.iter().any(|n| n.key == "factorio"));
        let c = d.def.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("k2", &crate::RelPath::parse(p).unwrap())
                .unwrap()
                .map(|t| t.to_string())
        };
        assert_eq!(map("info.json").as_deref(), Some("@user/mods/k2/info.json"));
        let text = to_toml(&d);
        let back = GameDef::parse_game(&text, Path::new("game.toml"), "factorio").unwrap();
        assert_eq!(back.plugins, d.def.plugins);
        // Back to generic from another setup: the same rules.
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(again.install[0].target, "@user/mods/{mod}/");

        // A zip copy keeps its mods (and their list) in its own folder.
        fs::write(
            root.join("config-path.cfg"),
            "use-system-read-write-data-directories=false\n",
        )
        .unwrap();
        let d = draft_manual(
            root,
            "Factorio",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert!(d.def.folders.is_empty());
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.folder.as_str()),
            ("mods/mod-list.json", "mods")
        );
        assert_eq!(d.def.install[0].target, "mods/{mod}/");
    }

    #[test]
    fn divinity_original_sin_2_mods_go_to_its_user_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("DefEd/bin/EoCApp.exe"), 100);
        touch(&root.join("DefEd/Data/Arena.pak"), 10);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Divinity, "DefEd/bin/EoCApp.exe")
        );
        let d = draft_manual(root, "DOS2", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%DOCUMENTS%/Larian Studios/Divinity Original Sin 2 Definitive Edition")
        );
        let l = d.def.larian.as_ref().unwrap();
        assert_eq!(
            (l.folder.as_str(), l.format),
            (USER_FOLDER, crate::LarianFormat::Dos2)
        );
        assert!(d
            .notes
            .iter()
            .any(|n| n.key == "larian" && n.text.contains("each player profile")));
        let c = d.def.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("m", &crate::RelPath::parse(p).unwrap())
                .unwrap()
                .map(|t| t.to_string())
        };
        assert_eq!(map("Armory.pak").as_deref(), Some("@user/Mods/Armory.pak"));
        assert_eq!(map("DXGI.dll").as_deref(), Some("DefEd/bin/DXGI.dll"));
        assert_eq!(
            map("bin/OsiExtenderEoCApp.dll").as_deref(),
            Some("DefEd/bin/OsiExtenderEoCApp.dll")
        );
        assert_eq!(map("DefEd/Data/x.pak").as_deref(), Some("DefEd/Data/x.pak"));
        let text = to_toml(&d);
        assert!(text.contains("format = \"dos2\""), "{text}");
        let back = GameDef::parse_game(&text, Path::new("game.toml"), "dos2").unwrap();
        assert_eq!(back.larian, d.def.larian);
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        let targets = |rules: &[crate::InstallRule]| {
            rules.iter().map(|r| r.target.clone()).collect::<Vec<_>>()
        };
        assert_eq!(targets(&again.install), targets(&d.def.install));
    }

    #[test]
    fn baldurs_gate_3_mods_go_to_its_user_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("bin/bg3.exe"), 100);
        touch(&root.join("bin/bg3_dx11.exe"), 100);
        touch(&root.join("Launcher/LariLauncher.exe"), 50);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Divinity, "bin/bg3.exe")
        );
        let d = draft_manual(root, "BG3", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%LOCALAPPDATA%/Larian Studios/Baldur's Gate 3")
        );
        assert_eq!(d.def.larian.as_ref().unwrap().folder, USER_FOLDER);
        assert!(d.notes.iter().any(|n| n.key == "larian"));
        let c = d.def.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("m", &crate::RelPath::parse(p).unwrap())
                .unwrap()
                .map(|t| t.to_string())
        };
        assert_eq!(map("Bags.pak").as_deref(), Some("@user/Mods/Bags.pak"));
        assert_eq!(map("DWrite.dll").as_deref(), Some("bin/DWrite.dll"));
        assert_eq!(
            map("bin/NativeMods/x.dll").as_deref(),
            Some("bin/NativeMods/x.dll")
        );
        assert_eq!(map("info.json"), None, "left out");
        let text = to_toml(&d);
        let back = GameDef::parse_game(&text, Path::new("game.toml"), "bg3").unwrap();
        assert_eq!(back.larian, d.def.larian);
        // Back to generic from another setup: the same rules.
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(again.install[0].target, "@user/Mods/");
    }

    #[test]
    fn oxygen_not_included_mods_go_to_its_local_mods_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join(ONI_EXE), 100);
        touch(&root.join("UnityPlayer.dll"), 10);
        touch(&root.join("OxygenNotIncluded_Data/globalgamemanagers"), 10);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::OxygenNotIncluded, ONI_EXE)
        );
        let d = draft_manual(
            root,
            "Oxygen Not Included",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%DOCUMENTS%/Klei/OxygenNotIncluded")
        );
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.format, p.folder.as_str()),
            (
                "%DOCUMENTS%/Klei/OxygenNotIncluded/mods/mods.json",
                PluginListFormat::OniJson,
                "@user/mods/Local"
            )
        );
        assert!(d.notes.iter().any(|n| n.key == "oni"));
        assert!(!d.notes.iter().any(|n| n.key == "generic-unity"));
        assert!(d.def.protect.contains(&"UnityPlayer.dll".to_string()));
        let c = d.def.compile().unwrap();
        let mapped = c
            .map_mod_file("tweaks", &crate::RelPath::parse("Tweaks.dll").unwrap())
            .unwrap()
            .map(|t| t.to_string());
        assert_eq!(
            mapped.as_deref(),
            Some("@user/mods/Local/tweaks/Tweaks.dll")
        );
        let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "oni").unwrap();
        assert_eq!(back.plugins, d.def.plugins);
    }

    #[test]
    fn kenshi_mods_get_a_folder_each_and_are_listed_in_its_mods_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("kenshi_x64.exe"), 100);
        touch(&root.join("data/gamedata.base"), 10);
        let found = inspect_folder(root);
        let d = draft_manual(
            root,
            "Kenshi",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.format, p.folder.as_str()),
            ("data/mods.cfg", PluginListFormat::KenshiCfg, "mods")
        );
        assert!(d
            .notes
            .iter()
            .any(|n| n.key == "plugins" && n.text.starts_with("Kenshi")));
        assert!(!d.notes.iter().any(|n| n.key == "unknown-engine"));
        assert_eq!(d.def.install[0].target, "mods/{mod}/");
        let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "kenshi").unwrap();
        assert_eq!(back.plugins, d.def.plugins);
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(again.install[0].target, "mods/{mod}/");
    }

    #[test]
    fn gamemaker_games_are_recognised_by_their_data_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("UNDERTALE.exe"), 100);
        // Not a real data file: not GameMaker.
        touch(&root.join("data.win"), 1000);
        assert_eq!(inspect_folder(root).engine, Engine::Unknown);
        let mut data = b"FORM".to_vec();
        data.extend(8u32.to_le_bytes());
        data.extend(b"GEN8\0\0\0\0");
        std::fs::write(root.join("data.win"), data).unwrap();
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::GameMaker, "UNDERTALE.exe")
        );
        let d = draft_manual(
            root,
            "Undertale",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert!(d.notes.iter().any(|n| n.key == "gamemaker"));
        assert!(!d.notes.iter().any(|n| n.key == "unknown-engine"));
        assert_eq!(d.def.install[0].target, "");
    }

    #[test]
    fn bannerlord_modules_get_a_folder_each() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join(BANNERLORD_EXE), 100);
        touch(
            &root.join("bin/Win64_Shipping_Client/TaleWorlds.MountAndBlade.Launcher.exe"),
            50,
        );
        touch(&root.join("Modules/Native/SubModule.xml"), 10);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Bannerlord, BANNERLORD_EXE)
        );
        let d = draft_manual(
            root,
            "Bannerlord",
            &found.exe,
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert_eq!(d.def.bannerlord.as_ref().unwrap().folder, "Modules");
        assert_eq!(d.def.install[0].target, "Modules/{mod}/");
        assert!(d.notes.iter().any(|n| n.key == "bannerlord"));
        let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "mb2").unwrap();
        assert_eq!(back.bannerlord, d.def.bannerlord);
        let again = back
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(again.install[0].target, "Modules/{mod}/");
    }

    #[test]
    fn xcom_2_mods_go_to_war_of_the_chosen_when_it_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join(XCOM2_EXE), 100);
        touch(&root.join("Binaries/Win64/Launcher/ModLauncherWPF.exe"), 50);
        let base = inspect_folder(root);
        assert_eq!((base.engine, base.exe.as_str()), (Engine::Xcom2, XCOM2_EXE));
        let d = draft_manual(root, "XCOM 2", &base.exe, &base, LoaderSetup::Generic, None).unwrap();
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.format, p.folder.as_str()),
            (
                "%DOCUMENTS%/My Games/XCOM2/XComGame/Config/XComModOptions.ini",
                PluginListFormat::XcomIni,
                "XComGame/Mods"
            )
        );
        assert_eq!(d.def.install[0].target, "XComGame/Mods/{mod}/");

        touch(&root.join(XCOM2_WOTC_EXE), 100);
        let wotc = inspect_folder(root);
        assert_eq!(wotc.exe, XCOM2_WOTC_EXE);
        let d = draft_manual(root, "XCOM 2", &wotc.exe, &wotc, LoaderSetup::Generic, None).unwrap();
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.list.as_str(), p.folder.as_str()),
            (
                "%DOCUMENTS%/My Games/XCOM2 War of the Chosen/XComGame/Config/XComModOptions.ini",
                "XCom2-WarOfTheChosen/XComGame/Mods"
            )
        );
        assert_eq!(
            d.def.install[0].target,
            "XCom2-WarOfTheChosen/XComGame/Mods/{mod}/"
        );
        assert!(d
            .notes
            .iter()
            .any(|n| n.key == "xcom2" && n.text.contains("War of the Chosen")));
        let back = GameDef::parse_game(&to_toml(&d), Path::new("game.toml"), "xcom2").unwrap();
        assert_eq!(back.plugins, d.def.plugins);
    }

    #[test]
    fn crusader_kings_ii_lists_its_mods_in_its_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("CK2game.exe"), 100);
        let found = inspect_folder(root);
        assert_eq!(
            (found.engine, found.exe.as_str()),
            (Engine::Clausewitz, "CK2game.exe")
        );
        let d = draft_manual(root, "CK2", &found.exe, &found, LoaderSetup::Generic, None).unwrap();
        assert_eq!(
            d.def.folders.get(USER_FOLDER).map(String::as_str),
            Some("%DOCUMENTS%/Paradox Interactive/Crusader Kings II")
        );
        assert_eq!(d.def.paradox.as_ref().unwrap().list, ParadoxList::Settings);
        assert_eq!(d.def.launch.as_ref().unwrap().args, ["-skiplauncher"]);
        let note = d.notes.iter().find(|n| n.key == "paradox").unwrap();
        assert!(note.text.contains("settings.txt"), "{}", note.text);
        let text = to_toml(&d);
        assert!(text.contains("list = \"settings\""), "{text}");
        let back = GameDef::parse_game(&text, Path::new("game.toml"), "ck2").unwrap();
        assert_eq!(back.paradox, d.def.paradox);
    }

    #[test]
    fn drafts_match_the_engine_and_reload_cleanly() {
        let (_tmp, lib) = library();
        let apps = installed_games_in(&[lib]);

        let mono = draft_for(&apps, 1001);
        assert_eq!(mono.engine, Engine::UnityMono);
        assert_eq!(mono.def.id, "mono-quest-deluxe");
        assert_eq!(mono.def.launch.as_ref().unwrap().exe, "MonoQuest.exe");
        assert!(mono.def.loader.is_none(), "generic unless asked otherwise");
        assert_eq!(mono.def.detect.marker.as_deref(), Some("MonoQuest.exe"));

        let mono_app = apps.iter().find(|a| a.app_id == 1001).unwrap();
        let own = draft_definition(mono_app, LoaderSetup::Bepinex, None).unwrap();
        assert_eq!(
            own.def.doorstop().unwrap().runtime_dirs,
            ["BepInEx", "dotnet"]
        );
        let quest = Community {
            label: "mono-quest",
            loader: "bepinex",
        };
        let ts = draft_definition(mono_app, LoaderSetup::Thunderstore, Some(quest)).unwrap();
        assert_eq!(ts.def.community(), Some("mono-quest"));
        assert_eq!(ts.def.setup(), Some(LoaderSetup::Thunderstore));
        assert!(draft_definition(mono_app, LoaderSetup::Thunderstore, None).is_err());

        let il2cpp = draft_for(&apps, 1002);
        assert_eq!(il2cpp.engine, Engine::UnityIl2cpp);
        assert!(il2cpp.def.loader.is_none());

        let unreal = draft_for(&apps, 1003);
        assert_eq!(unreal.engine, Engine::Unreal);
        assert_eq!(unreal.def.launch.as_ref().unwrap().exe, "Trip.exe");
        assert_eq!(unreal.def.install[0].target, "Trip/Content/Paks/~mods/");
        let launch = unreal.def.launch.as_ref().unwrap();
        assert_eq!(
            launch.steam_relaunch,
            SteamRelaunch::ViaSteam,
            "started by Steam"
        );
        assert_eq!(
            launch.process_names,
            ["Trip.exe", "Trip-Win64-Shipping.exe"],
            "the launcher and the real exe it hands off to"
        );

        // Bethesda games get their plugin list, recognised by their exe.
        let exe = |e: &str| list_game(&[e.to_string()]).unwrap();
        let steam = tempfile::tempdir().unwrap();
        let sse = exe("SkyrimSE.exe").plugins(steam.path());
        assert_eq!(
            (sse.list.as_str(), sse.format, sse.folder.as_str()),
            (
                "%LOCALAPPDATA%/Skyrim Special Edition/plugins.txt",
                PluginListFormat::Asterisk,
                "Data"
            )
        );
        // Any edition, regional ones included, has the exe; case does not matter.
        let exes = vec![
            "FalloutNVLauncher.exe".to_string(),
            "falloutnv.exe".to_string(),
        ];
        let nv = list_game(&exes).unwrap();
        assert_eq!(
            (nv.list, nv.format),
            (ListAt::LocalAppData("FalloutNV"), PluginListFormat::Plain)
        );
        let vr = vec!["Fallout4VR.exe".to_string(), "Fallout4.exe".to_string()];
        assert_eq!(
            list_game(&vr).unwrap().list,
            ListAt::LocalAppData("Fallout4VR")
        );
        assert!(list_game(&["Game.exe".to_string(), "bin/SkyrimSE.exe".to_string()]).is_none());
        let fo4 = exe("Fallout4.exe").ini();
        assert_eq!(fo4.len(), 3);
        assert!(
            !fo4[0].create && fo4[1].create,
            "Prefs is the game's own; Custom is overrides"
        );
        assert_eq!(
            fo4[1].file,
            "%DOCUMENTS%/My Games/Fallout4/Fallout4Custom.ini"
        );
        let sf = exe("Starfield.exe").ini();
        assert_eq!(
            (sf[0].file.as_str(), sf[1].value.as_str()),
            ("%DOCUMENTS%/My Games/Starfield/StarfieldCustom.ini", "")
        );
        assert!(exe("SkyrimSE.exe").ini().is_empty());

        // Other stores' editions keep their list elsewhere, told apart by a file of theirs.
        let gog = tempfile::tempdir().unwrap();
        touch(&gog.path().join("Galaxy64.dll"), 1);
        assert_eq!(
            exe("SkyrimSE.exe").plugins(gog.path()).list,
            "%LOCALAPPDATA%/Skyrim Special Edition GOG/plugins.txt"
        );
        let epic = tempfile::tempdir().unwrap();
        touch(&epic.path().join("EOSSDK-Win32-Shipping.dll"), 1);
        assert_eq!(
            exe("FalloutNV.exe").plugins(epic.path()).list,
            "%LOCALAPPDATA%/FalloutNV_Epic/plugins.txt"
        );
        // Oblivion's list moves to its own folder when its ini says not to use My Games.
        let c = exe("Oblivion.exe")
            .plugins(gog.path())
            .in_game_folder_when
            .unwrap();
        assert_eq!(
            (c.ini.as_str(), c.key.as_str(), c.value.as_str()),
            ("Oblivion.ini", "bUseMyGamesDirectory", "0")
        );
        assert!(exe("SkyrimSE.exe")
            .plugins(gog.path())
            .in_game_folder_when
            .is_none());
        // GOG's Fallout 4 uses the same folder as Steam's.
        assert_eq!(
            exe("Fallout4.exe").plugins(gog.path()).list,
            "%LOCALAPPDATA%/Fallout4/plugins.txt"
        );

        // A game added by hand (a GOG install, say) is recognised the same way.
        let manual = tempfile::tempdir().unwrap();
        touch(&manual.path().join("FalloutNV.exe"), 100);
        touch(&manual.path().join("FalloutNVLauncher.exe"), 10);
        touch(&manual.path().join("Data/FalloutNV.esm"), 1);
        let found = inspect_folder(manual.path());
        let d = draft_manual(
            manual.path(),
            "New Vegas",
            "FalloutNV.exe",
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        assert_eq!(
            d.def.plugins.as_ref().unwrap().list,
            "%LOCALAPPDATA%/FalloutNV/plugins.txt"
        );
        // Its launcher, which Steam starts first, is followed too.
        assert_eq!(
            d.def.launch.as_ref().unwrap().process_names,
            ["FalloutNV.exe", "FalloutNVLauncher.exe"]
        );
        // Recognised by its data folder, which plugins, archives and assets go to.
        assert_eq!(found.engine, Engine::Bethesda);
        assert_eq!(found.data_folder.as_deref(), Some("Data"));
        let c = d.def.compile().unwrap();
        let map = |p: &str| {
            c.map_mod_file("m", &crate::RelPath::parse(p).unwrap())
                .unwrap()
                .unwrap()
                .to_string()
        };
        assert_eq!(map("Mod.esp"), "Data/Mod.esp");
        assert_eq!(map("Mod - Main.bsa"), "Data/Mod - Main.bsa");
        assert_eq!(map("meshes/a/b.nif"), "Data/meshes/a/b.nif");
        assert_eq!(map("nvse/plugins/x.dll"), "Data/nvse/plugins/x.dll");
        assert_eq!(map("src/nvse/x.cpp"), "src/nvse/x.cpp");
        assert_eq!(map("nvse/x.cpp"), "nvse/x.cpp");
        assert_eq!(map("Data/textures/t.dds"), "Data/textures/t.dds");
        assert_eq!(map("nvse_loader.exe"), "nvse_loader.exe");
        assert_eq!(map("sub/Mod.esp"), "sub/Mod.esp");
        assert!(d.notes.iter().any(|n| n.key == "bethesda"));
        // Set to generic again from another setup, the rules come back.
        let other = d
            .def
            .with_setup(LoaderSetup::Bepinex, None)
            .unwrap()
            .with_setup(LoaderSetup::Generic, None)
            .unwrap();
        assert_eq!(other.install.len(), d.def.install.len());
        // Morrowind's is `Data Files`.
        let mw_dir = tempfile::tempdir().unwrap();
        touch(&mw_dir.path().join("Morrowind.exe"), 100);
        touch(&mw_dir.path().join("Data Files/Morrowind.esm"), 1);
        let mw_found = inspect_folder(mw_dir.path());
        assert_eq!(mw_found.data_folder.as_deref(), Some("Data Files"));

        // Other games keeping a list: in the game folder, or with a folder per mod.
        let or = exe("OblivionRemastered.exe").plugins(manual.path());
        assert_eq!(
            (or.list.as_str(), or.folder.as_str(), or.format),
            (
                "OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt",
                "OblivionRemastered/Content/Dev/ObvData/Data",
                PluginListFormat::Plain
            )
        );
        assert_eq!(
            or.list_path(Path::new("C:/g")),
            Some(PathBuf::from(
                "C:/g/OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt"
            ))
        );
        let mw = exe("Morrowind.exe").plugins(manual.path());
        assert_eq!(
            (mw.list.as_str(), mw.folder.as_str(), mw.format),
            (
                "Morrowind.ini",
                "Data Files",
                PluginListFormat::MorrowindIni
            )
        );
        let rim = tempfile::tempdir().unwrap();
        touch(&rim.path().join("RimWorldWin64.exe"), 100);
        touch(&rim.path().join("RimWorldWin64_Data/x"), 1);
        touch(&rim.path().join("UnityPlayer.dll"), 1);
        let found = inspect_folder(rim.path());
        let d = draft_manual(
            rim.path(),
            "RimWorld",
            "RimWorldWin64.exe",
            &found,
            LoaderSetup::Generic,
            None,
        )
        .unwrap();
        let p = d.def.plugins.as_ref().unwrap();
        assert_eq!(
            (p.format, p.folder.as_str()),
            (PluginListFormat::RimworldXml, "Mods")
        );
        assert!(p.list.ends_with("/Config/ModsConfig.xml"));
        assert_eq!(
            d.def.install.last().unwrap().target,
            "Mods/{mod}/",
            "a folder per mod"
        );
        let text = to_toml(&d);
        GameDef::parse_game(&text, Path::new("game.toml"), &d.def.id).unwrap();

        let plain = draft_for(&apps, 1004);
        assert_eq!(plain.engine, Engine::Unknown);
        assert_eq!(
            plain.def.launch.as_ref().unwrap().exe,
            "plain.exe",
            "uninstaller ignored"
        );

        for d in [mono, own, ts, il2cpp, unreal, plain] {
            let text = to_toml(&d);
            let back = GameDef::parse_game(&text, Path::new("game.toml"), &d.def.id)
                .unwrap_or_else(|e| panic!("{} does not reload: {e}\n{text}", d.def.id));
            assert_eq!(back.id, d.def.id);
        }
    }

    #[test]
    fn ids_keep_the_title_as_written() {
        let app = |name: &str, dir: &str| SteamApp {
            app_id: 77,
            name: name.into(),
            install_dir: dir.into(),
            path: PathBuf::new(),
        };
        assert_eq!(
            app_id_slug(&app("Mono Quest: Deluxe", "MQ")),
            "mono-quest-deluxe"
        );
        assert_eq!(
            app_id_slug(&app("おわかれのほし", "Owakare")),
            "おわかれのほし"
        );
        assert_eq!(
            app_id_slug(&app("みんなで空気読み。2", "x")),
            "みんなで空気読み-2"
        );
        assert_eq!(app_id_slug(&app("Ünïcode Game 2", "x")), "ünïcode-game-2");
        assert_eq!(app_id_slug(&app("!!!", "Folder Name")), "folder-name");
        assert_eq!(app_id_slug(&app("!!!", "???")), "steam-77");
        assert_eq!(app_id_slug(&app("CON", "Con Game")), "con-game");
    }
}
