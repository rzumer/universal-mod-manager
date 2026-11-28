//! Listing installed Steam games and drafting a game definition for one.
//!
//! The draft is a starting point: it finds the exe, recognises Unity (Mono or IL2CPP) and
//! Unreal layouts, and picks matching loader and install rules. Unusual games still need
//! their rules edited by hand.

use crate::detect::steam_libraries;
use crate::kind::{kind, kinds, rule, Recognised};
use crate::vdf::{self, Value};
use crate::{
    slug, Community, Detect, GameDef, GameDefError, Launch, LoaderSetup, SteamRelaunch,
    Thunderstore, SCHEMA_VERSION, SHIMLOADER_KIND,
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

/// Terraria's and tModLoader's Steam apps.
pub const TERRARIA_APP: u32 = 105600;
pub const TMODLOADER_APP: u32 = 1281930;

/// The installed app UMM sets `app` up as instead, when the game's mods need another app of
/// its own on Steam: tModLoader for Terraria. The game keeps its name, so its mod sites know
/// it.
pub fn stand_in<'a>(app: &SteamApp, installed: &'a [SteamApp]) -> Option<&'a SteamApp> {
    if app.app_id != TERRARIA_APP {
        return None;
    }
    installed.iter().find(|a| a.app_id == TMODLOADER_APP)
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
    /// GameMaker: the game's code and assets in one data file (`data.win`), which mods
    /// replace or patch (`.xdelta`).
    GameMaker,
    /// A game UMM knows that names its engine itself (see [`crate::kind`]).
    Game(&'static str),
    Unknown,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::UnityMono => "Unity (Mono)",
            Engine::UnityIl2cpp => "Unity (IL2CPP)",
            Engine::Unreal => "Unreal Engine",
            Engine::GameMaker => "GameMaker",
            Engine::Game(label) => label,
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

/// What the install folder says about the game: its engine, exe and (Unreal) project folder,
/// and the game it is when UMM knows it.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub engine: Engine,
    /// Relative path of the likely game exe; empty when none was found.
    pub exe: String,
    /// Every exe that could be the game, the likely one first.
    pub exes: Vec<String>,
    pub unreal_project: Option<String>,
    /// The game it is, when UMM knows it.
    pub game: Option<Recognised>,
}

impl Inspection {
    /// The game's own mod support, as [`GameDef::own_mods`] will say for its draft.
    pub fn own_mods(&self) -> Option<&'static str> {
        self.game.as_ref().and_then(|g| g.own_mods)
    }
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

/// Looks at an install folder to recognise the game or its engine, and find the exe.
pub fn inspect_folder(root: &Path) -> Inspection {
    let mut exes = by_size(exe_files(root));
    let game = kinds().iter().find_map(|k| k.recognise(root, &exes));
    let mut unreal_project = None;
    let engine = match game.as_ref().and_then(|g| g.label) {
        Some(label) => Engine::Game(label),
        None => {
            let (engine, project) = engine_of(root, &mut exes);
            unreal_project = project;
            engine
        }
    };
    if let Some(exe) = game.as_ref().and_then(|g| g.exe.clone()) {
        exes.retain(|e| !e.eq_ignore_ascii_case(&exe));
        exes.insert(0, exe);
    }
    Inspection {
        engine,
        exe: exes.first().cloned().unwrap_or_default(),
        exes,
        unreal_project,
        game,
    }
}

/// The engine the files in `root` say, with its likely exe moved first in `exes` (Unity's) or
/// the real ones added (Unreal's, in `<Project>/Binaries/Win64`), and the Unreal project.
fn engine_of(root: &Path, exes: &mut Vec<String>) -> (Engine, Option<String>) {
    // Unity: `<Name>.exe` next to `<Name>_Data/` and UnityPlayer.dll.
    let unity_exe = exes.iter().find(|n| {
        let stem = &n[..n.len() - 4];
        root.join(format!("{stem}_Data")).is_dir()
    });
    if let Some(exe) = unity_exe
        .filter(|_| exists_ci(root, "UnityPlayer.dll"))
        .cloned()
    {
        exes.retain(|e| e != &exe);
        exes.insert(0, exe);
        let engine = if exists_ci(root, "GameAssembly.dll") {
            Engine::UnityIl2cpp
        } else {
            Engine::UnityMono
        };
        return (engine, None);
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
        return (Engine::Unreal, project);
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
        return (Engine::GameMaker, None);
    }
    (Engine::Unknown, None)
}

/// A GameMaker game's data file (Windows builds').
pub const GAMEMAKER_DATA: &str = "data.win";

/// Where a draft's game comes from: everything besides the files that goes in its definition.
struct Origin {
    id: String,
    name: String,
    detect: Detect,
    relaunch: SteamRelaunch,
    /// The game folder.
    root: PathBuf,
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
    let draft = build(origin, found, &found.exe, setup, community)?;
    // The setup was offered by this name before the draft was made.
    debug_assert_eq!(found.own_mods(), draft.def.own_mods(), "{}", draft.def.name);
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
    let draft = build(origin, found, exe, setup, community)?;
    // The setup was offered by this name before the draft was made.
    debug_assert_eq!(found.own_mods(), draft.def.own_mods(), "{}", draft.def.name);
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
    // What the game UMM knows it to be adds.
    let shape = found
        .game
        .as_ref()
        .and_then(|g| kind(g.key))
        .map(|k| k.shape(&origin.root, found, exe, setup))
        .unwrap_or_default();

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
        Engine::GameMaker => notes.push(Note::new(
            "gamemaker",
            "GameMaker: mods replace files in the game folder, data files included (one going \
             where the game's data file of the same name is, as a DELTARUNE chapter's), or \
             patch them with .xdelta files, G3M patches (.g3mpatch) or UndertaleModTool \
             scripts (.csx), which UMM applies while mods are deployed and takes back \
             afterwards. UMM finds the file each patch is for by trying it, and follows a \
             DELTAMOD package's modding.xml. Two mods' xdelta patches for the same data file, \
             each made for the game's own, are merged, and G3M patches and scripts apply on top \
             of other mods' changes, with G3MTool (downloaded the first time), the later mod \
             winning where both change the same thing; if that fails, UMM says why and launches \
             nothing until one is disabled.",
        )),
        Engine::Unknown if found.game.is_none() && setup == LoaderSetup::Generic => {
            notes.push(Note::new(
                "unknown-engine",
                "Engine not recognised: mod files keep their paths relative to the game folder.",
            ))
        }
        Engine::Unknown | Engine::Game(_) => {}
    }
    notes.extend(shape.notes);
    protect.extend(shape.protect);
    if shape.install.is_empty() {
        install.push(rule("**", ""));
    } else {
        install.extend(shape.install);
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
            args: shape.args,
            // A game whose own launcher stands before it (and would undo what UMM set up, as
            // Paradox's rewrites the list of enabled mods) is started directly, Steam running.
            steam_relaunch: if shape.direct && origin.relaunch == SteamRelaunch::ViaSteam {
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
        plugins: shape.plugins,
        ini: shape.ini,
        thunderstore: community.map(|c| Thunderstore {
            community: c.label.to_string(),
        }),
        install,
        protect,
        writable,
        folders: shape
            .user_folder
            .map(|user| [(USER_FOLDER.to_string(), user)].into_iter().collect())
            .unwrap_or_default(),
        paradox: shape.paradox,
        bannerlord: shape.bannerlord,
        larian: shape.larian,
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
        assert_eq!(found.game, None);
        assert!(!d.notes.iter().any(|n| n.key == "unknown-engine"));
        assert_eq!(d.def.install[0].target, "");
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
