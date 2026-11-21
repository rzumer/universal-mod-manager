//! Game definitions: one TOML file per game describing how to find it, which loader it needs and
//! where each mod's files go. Adding a game means adding a file, not code.

pub mod detect;
pub mod edit;
pub mod relpath;
pub mod setup;
pub mod steam;
pub mod vdf;

pub use detect::{detect_install, Detected};
pub use relpath::{RelPath, RelPathError};
pub use setup::{Community, LoaderSetup};

use globset::{GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
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
    /// The game's folder name under `games/`. Files there may leave it out; a file that
    /// states it must match the folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
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
    #[serde(default)]
    pub install: Vec<InstallRule>,
    /// Game paths that are never replaced, even if a mod ships them.
    #[serde(default)]
    pub protect: PathSet,
    /// Game paths that are always copied rather than hardlinked, because the game writes to them.
    #[serde(default)]
    pub writable: PathSet,
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
    #[serde(default)]
    pub steam_relaunch: SteamRelaunch,
    #[serde(default)]
    pub process_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamRelaunch {
    AppidFile,
    ViaSteam,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thunderstore {
    /// The community's label, as in `thunderstore.io/c/<community>/`.
    pub community: String,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSet {
    #[serde(default)]
    pub paths: Vec<String>,
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
    #[error("unsupported schema version {0} (this build supports {SCHEMA_VERSION})")]
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
    #[error("{path} says `id = \"{id}\"` but is in the folder `{folder}`; the folder name is the id")]
    IdMismatch {
        path: PathBuf,
        id: String,
        folder: String,
    },
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
    pub builds: &'static [BepInExBuild],
}

/// The BepInEx builds UMM can start, told apart by the assembly Doorstop loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BepInExBuild {
    /// BepInEx 5, for Mono Unity games.
    V5,
    /// BepInEx 6 for Mono Unity games.
    V6Mono,
    /// BepInEx 6 for IL2CPP Unity games; runs on the .NET runtime in the pack's `dotnet/`.
    V6Il2cpp,
}

impl BepInExBuild {
    /// The assembly Doorstop loads, relative to the runtime folder.
    pub fn target(self) -> &'static str {
        match self {
            BepInExBuild::V5 => "BepInEx/core/BepInEx.Preloader.dll",
            BepInExBuild::V6Mono => "BepInEx/core/BepInEx.Unity.Mono.Preloader.dll",
            BepInExBuild::V6Il2cpp => "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BepInExBuild::V5 => "BepInEx 5",
            BepInExBuild::V6Mono => "BepInEx 6 (Mono)",
            BepInExBuild::V6Il2cpp => "BepInEx 6 (IL2CPP)",
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
    (kind == BEPINEX_KIND).then_some(DoorstopLoader {
        runtime_dirs: &["BepInEx", "dotnet"],
        builds: &[BepInExBuild::V5, BepInExBuild::V6Mono, BepInExBuild::V6Il2cpp],
    })
}

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
}

impl RuntimeLoader {
    /// Top-level folders that live in the runtime folder instead of the game folder.
    pub fn runtime_dirs(&self) -> &'static [&'static str] {
        match self {
            RuntimeLoader::Doorstop(d) => d.runtime_dirs,
            RuntimeLoader::Shimloader => &[SHIMLOADER_DIR],
            RuntimeLoader::MelonLoader => MELONLOADER_DIRS,
            RuntimeLoader::Lovely => &[LOVELY_DIR],
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
    pub fn parse(text: &str, origin: &Path) -> Result<Self, GameDefError> {
        let def: GameDef = toml::from_str(text).map_err(|source| GameDefError::Parse {
            path: origin.to_path_buf(),
            source: Box::new(source),
        })?;
        def.validate()?;
        Ok(def)
    }

    /// Parses a game's `game.toml`; `id` is its folder's name.
    pub fn parse_game(text: &str, origin: &Path, id: &str) -> Result<Self, GameDefError> {
        let mut def: GameDef = toml::from_str(text).map_err(|source| GameDefError::Parse {
            path: origin.to_path_buf(),
            source: Box::new(source),
        })?;
        if !def.id.is_empty() && def.id != id {
            return Err(GameDefError::IdMismatch {
                path: origin.to_path_buf(),
                id: def.id,
                folder: id.to_string(),
            });
        }
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
        if self.schema != SCHEMA_VERSION {
            return Err(GameDefError::Schema(self.schema));
        }
        if !is_valid_id(&self.id) {
            return Err(GameDefError::Id(self.id.clone()));
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
                        || is_melonloader(&loader.kind) => {}
                Inject::Args => {
                    return Err(GameDefError::Unsupported(format!(
                        "loader kind `{}` cannot be injected with arguments (supported: shimloader, melonloader, recursive-melonloader, lovely); use inject = \"files\"",
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
            });
        }
        Ok(CompiledDef {
            def: self.clone(),
            rules,
            protect: globset(&self.protect.paths)?,
            writable: globset(&self.writable.paths)?,
        })
    }
}

fn glob(pattern: &str) -> Result<globset::Glob, GameDefError> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .case_insensitive(cfg!(windows))
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

    pub fn is_protected(&self, target: &RelPath) -> bool {
        self.protect.is_match(target.as_str())
    }

    pub fn is_writable(&self, target: &RelPath) -> bool {
        self.writable.is_match(target.as_str())
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

    const DEF: &str = r#"
schema = 1
id = "example"
name = "Example"

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

[protect]
paths = ["*.exe"]

[writable]
paths = ["BepInEx/config/**"]
"#;

    fn compiled() -> CompiledDef {
        GameDef::parse(DEF, Path::new("test.toml"))
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
    fn rejects_bad_definitions() {
        let bad_target = DEF.replace("target = \"\"", "target = \"../escape\"");
        assert!(matches!(
            GameDef::parse(&bad_target, Path::new("t")),
            Err(GameDefError::Target(..))
        ));
        let bad_id = DEF.replace("id = \"example\"", "id = \"Bad Id\"");
        assert!(matches!(
            GameDef::parse(&bad_id, Path::new("t")),
            Err(GameDefError::Id(_))
        ));
        let typo = DEF.replace("[protect]", "[protekt]");
        assert!(matches!(
            GameDef::parse(&typo, Path::new("t")),
            Err(GameDefError::Parse { .. })
        ));
    }
}
