//! FOMOD installers: the `fomod/ModuleConfig.xml` many Nexus mods ship, which asks the user to
//! pick options and decides from those choices which of the archive's files go where.
//!
//! [`Installer::parse`] reads the file (UTF-8 or UTF-16, as mod authors save it).
//! [`Installer::evaluate`] takes the user's choices and says which steps show, what each
//! option is (required, recommended, not usable, ...) and which files install where.
//! [`resolve`] turns those file entries into the archive's actual files.
//!
//! Choices are kept by name (step, group, option), so they can be replayed on an update of
//! the mod as long as its installer still offers them.

mod eval;
mod parse;

pub use eval::{resolve, Evaluated, FileState, GroupState, Placed, PluginState, StepState};

use serde::{Deserialize, Serialize};

/// The file, relative to the folder holding `fomod/`.
pub const CONFIG_FILE: &str = "fomod/ModuleConfig.xml";

#[derive(Debug, thiserror::Error)]
pub enum FomodError {
    #[error("the FOMOD installer is not valid XML: {0}")]
    Xml(String),
    #[error("the FOMOD installer has no <config> element")]
    NoConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub name: String,
    pub image: Option<String>,
    /// Installed whatever the choices.
    pub required: Vec<FileItem>,
    pub steps: Vec<Step>,
    /// Files installed when their condition holds, after all steps.
    pub conditional: Vec<ConditionalFiles>,
    /// What the mod needs before it can be installed at all.
    pub needs: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    /// Shown only when this holds (flags set by earlier steps, files in the game).
    pub visible: Option<Condition>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// Exactly one option.
    ExactlyOne,
    /// One option or none.
    AtMostOne,
    /// One or more options.
    AtLeastOne,
    /// Every option, always.
    All,
    /// Any options, or none.
    Any,
}

/// One option of a group (a "plugin" in FOMOD's terms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub files: Vec<FileItem>,
    /// Flags set when it is chosen: `(name, value)`.
    pub flags: Vec<(String, String)>,
    pub kind: PluginKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginKind {
    Fixed(PluginType),
    /// Depends on flags or files: the first pattern that holds, else the default.
    Dependent {
        default: PluginType,
        patterns: Vec<(Condition, PluginType)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    /// Always chosen; cannot be unticked.
    Required,
    /// Chosen to begin with.
    Recommended,
    Optional,
    /// Cannot be chosen.
    NotUsable,
    /// May work: offered like an optional one.
    CouldBeUsable,
}

/// A file or folder to install: `source` inside the archive (relative to the folder holding
/// `fomod/`), `destination` relative to the game's mod folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    pub source: String,
    /// `None`: a file keeps its source path; a folder goes to the top.
    pub destination: Option<String>,
    pub folder: bool,
    /// When two entries install to the same place, the higher priority wins.
    pub priority: i64,
    /// Installed even when its option is not chosen.
    pub always: bool,
    /// Installed whenever its option is usable, chosen or not.
    pub if_usable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalFiles {
    pub when: Condition,
    pub files: Vec<FileItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Flag {
        name: String,
        value: String,
    },
    File {
        path: String,
        state: FileState,
    },
    /// A version of the game or the mod manager: UMM does not check these.
    Version,
}

/// One group's chosen options, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub step: String,
    pub group: String,
    pub plugins: Vec<String>,
}

impl Installer {
    /// Reads a `ModuleConfig.xml`.
    pub fn parse(bytes: &[u8]) -> Result<Self, FomodError> {
        parse::parse(bytes)
    }
}
