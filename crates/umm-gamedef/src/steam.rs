//! Listing installed Steam games and drafting a game definition for one.
//!
//! The draft is a starting point: it finds the exe, recognises Unity (Mono or IL2CPP) and
//! Unreal layouts, and picks matching loader and install rules. Unusual games still need
//! their rules edited by hand.

use crate::detect::steam_libraries;
use crate::vdf::{self, Value};
use crate::{
    slug, Community, Detect, GameDef, GameDefError, GameIniCondition, IniSetting, InstallRule,
    Launch, LoaderSetup, PathSet, PluginListFormat, Plugins, SteamRelaunch, Thunderstore,
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
}

/// A Paradox game: its exe (relative to the game folder) and its user folder's name under
/// `Documents\Paradox Interactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParadoxGame {
    pub exe: &'static str,
    pub user: &'static str,
}

/// Paradox games whose mods go in their user folder and are enabled in `dlc_load.json`.
/// (Crusader Kings II lists its mods in `settings.txt` instead: not here.)
pub const PARADOX_GAMES: &[ParadoxGame] = &[
    ParadoxGame {
        exe: "binaries/ck3.exe",
        user: "Crusader Kings III",
    },
    ParadoxGame {
        exe: "binaries/victoria3.exe",
        user: "Victoria 3",
    },
    ParadoxGame {
        exe: "binaries/imperator.exe",
        user: "Imperator",
    },
    ParadoxGame {
        exe: "stellaris.exe",
        user: "Stellaris",
    },
    ParadoxGame {
        exe: "hoi4.exe",
        user: "Hearts of Iron IV",
    },
    ParadoxGame {
        exe: "eu4.exe",
        user: "Europa Universalis IV",
    },
];

/// The Paradox game installed in `root`, by its exe.
pub fn paradox_game(root: &Path) -> Option<&'static ParadoxGame> {
    PARADOX_GAMES.iter().find(|g| {
        let mut p = root.to_path_buf();
        for c in g.exe.split('/') {
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
    })
}

/// The folder name UMM gives a Paradox game's user folder in its definition.
pub const PARADOX_FOLDER: &str = "user";

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
        let engine = if exists_ci(root, "GameAssembly.dll") {
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
        };
    }
    Inspection {
        engine: Engine::Unknown,
        exe: exes.first().cloned().unwrap_or_default(),
        exes,
        unreal_project: None,
        data_folder: None,
        paradox: None,
    }
}

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

use PluginListFormat::{Asterisk, MorrowindIni, Plain, RimworldXml};

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
        RimworldXml => format!(
            "RimWorld: each mod goes in its own folder in Mods/, and while mods are deployed, UMM activates them in {}, in load order.",
            plugins.list
        ),
    };
    draft.notes.push(Note::new("plugins", note));
    if g.format == RimworldXml && draft.setup == LoaderSetup::Generic {
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
                         with a descriptor UMM writes, and is enabled in dlc_load.json while mods are deployed \
                         (your own list comes back afterwards). UMM starts the game directly, not through \
                         Paradox's launcher, which would replace the list with its playset.",
                        g.user
                    ),
                ));
            }
        }
        Engine::Unknown if setup == LoaderSetup::Generic => notes.push(Note::new(
            "unknown-engine",
            "Engine not recognised: mod files keep their paths relative to the game folder.",
        )),
        Engine::Unknown => {}
    }
    match found.paradox {
        // Mods go in the user folder's `mod/`, not the game folder.
        Some(_) => install.extend(crate::setup::paradox_rules(PARADOX_FOLDER)),
        None => install.push(rule("**", "")),
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
            args: Vec::new(),
            // A Paradox game started through Steam runs Paradox's launcher, which writes the
            // list of enabled mods from its playset: started directly instead, Steam running.
            steam_relaunch: if found.paradox.is_some() && origin.relaunch == SteamRelaunch::ViaSteam
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
        plugins: None,
        ini: Vec::new(),
        thunderstore: community.map(|c| Thunderstore {
            community: c.label.to_string(),
        }),
        install,
        protect: PathSet { paths: protect },
        writable: PathSet { paths: writable },
        folders: found
            .paradox
            .map(|g| {
                [(
                    PARADOX_FOLDER.to_string(),
                    format!("%DOCUMENTS%/Paradox Interactive/{}", g.user),
                )]
                .into_iter()
                .collect()
            })
            .unwrap_or_default(),
        paradox: found.paradox.map(|_| crate::Paradox {
            folder: PARADOX_FOLDER.into(),
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
            d.def.folders.get(PARADOX_FOLDER).map(String::as_str),
            Some("%DOCUMENTS%/Paradox Interactive/Crusader Kings III")
        );
        assert_eq!(d.def.paradox.as_ref().unwrap().folder, PARADOX_FOLDER);
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
