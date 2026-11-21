//! Game definitions: one TOML file per game describing how to find it, which loader it needs and
//! where each mod's files go. Adding a game means adding a file, not code.

pub mod detect;
pub mod relpath;
pub mod steam;
pub mod vdf;

pub use detect::{detect_install, Detected};
pub use relpath::{RelPath, RelPathError};

use globset::{GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// Name of the backup folder used inside a game folder when the data directory is on another
/// drive. Mods may never write into it.
pub const IN_GAME_BACKUP_DIR: &str = ".umm-backup";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameDef {
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub detect: Detect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<Launch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<Loader>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loader {
    pub kind: String,
    /// `mod:<id>`: the loader is stored and imported like any other mod.
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
    #[error("loader source must look like `mod:<id>`, got `{0}`")]
    LoaderSource(String),
    #[error("game id `{0}` is defined more than once")]
    Duplicate(String),
    #[error("{0}")]
    Unsupported(String),
}

/// What UMM knows about a loader kind that can be pointed at a folder outside the game
/// (Unity Doorstop). Only the proxy DLL and its config then go into the game folder; the
/// loader's own tree and every mod under it live in a per-profile runtime folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorstopLoader {
    /// Top-level folder that lives in the runtime folder instead of the game folder.
    pub runtime_dir: &'static str,
    /// The assembly Doorstop should load, relative to the runtime folder.
    pub target_assembly: &'static str,
}

/// Doorstop 4 command-line flags (BepInEx 5.4.22 and later ship Doorstop 4).
pub const DOORSTOP_ENABLED_FLAG: &str = "--doorstop-enabled";
pub const DOORSTOP_TARGET_FLAG: &str = "--doorstop-target-assembly";

pub fn doorstop_loader(kind: &str) -> Option<DoorstopLoader> {
    match kind {
        "bepinex5" => Some(DoorstopLoader {
            runtime_dir: "BepInEx",
            target_assembly: "BepInEx/core/BepInEx.Preloader.dll",
        }),
        _ => None,
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

    pub fn load_file(path: &Path) -> Result<Self, GameDefError> {
        let text = fs::read_to_string(path).map_err(|source| GameDefError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// Loads every `*.toml` in `dir`. Invalid files are returned separately so one bad
    /// definition does not hide the others.
    pub fn load_dir(dir: &Path) -> (Vec<GameDef>, Vec<GameDefError>) {
        let mut defs: Vec<GameDef> = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return (defs, errors);
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
            })
            .collect();
        paths.sort();
        for p in paths {
            match Self::load_file(&p) {
                Ok(d) if defs.iter().any(|x| x.id == d.id) => {
                    errors.push(GameDefError::Duplicate(d.id))
                }
                Ok(d) => defs.push(d),
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
                        "loader kind `{}` cannot be injected with doorstop (supported: bepinex5); use inject = \"files\"",
                        loader.kind
                    )))
                }
                Inject::Args | Inject::Env => {
                    return Err(GameDefError::Unsupported(format!(
                        "inject = \"{:?}\" is not supported yet; use \"files\" or \"doorstop\"",
                        loader.inject
                    )
                    .to_lowercase()))
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
kind = "bepinex5"
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
