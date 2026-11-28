//! Games UMM knows by more than their engine: how each is recognised in its folder, what its
//! drafted definition gets, and which definitions are its. Each is a [`GameKind`], written in
//! `umm-core`'s `games` module beside what UMM does for it at deploy and launch; `umm-core`
//! registers them here ([`register`]) when a session opens, so the drafting and the setups here
//! need not know any game.

use crate::steam::{Inspection, Note};
use crate::{
    Bannerlord, GameDef, IniSetting, InstallRule, Larian, LoaderSetup, Paradox, Plugins,
    BUILT_IN_MODS,
};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

/// A game recognised in its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognised {
    /// The [`GameKind::key`] of the game it is.
    pub key: &'static str,
    /// What to show as its engine, in place of the one its files say (`None`: that one).
    pub label: Option<&'static str>,
    /// The exe to start it with (relative to its folder), in place of the largest.
    pub exe: Option<String>,
    /// Its own mod support, as its draft's [`GameDef::own_mods`] will say.
    pub own_mods: Option<&'static str>,
}

/// What a game's draft gets beyond what its engine gives it.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub notes: Vec<Note>,
    /// Files no mod may replace, besides the exes.
    pub protect: Vec<String>,
    /// Its user folder outside the game folder (with variables): `@user/…` in its rules.
    pub user_folder: Option<String>,
    /// Its install rules without a mod loader; none: every file keeps its path.
    pub install: Vec<InstallRule>,
    pub plugins: Option<Plugins>,
    pub ini: Vec<IniSetting>,
    pub paradox: Option<Paradox>,
    pub larian: Option<Larian>,
    pub bannerlord: Option<Bannerlord>,
    /// Its exe's arguments.
    pub args: Vec<String>,
    /// Started directly, past its own launcher, rather than by Steam (Steam running).
    pub direct: bool,
}

/// A game UMM knows. Every method but [`key`], [`recognise`], [`shape`] and [`applies`] has
/// a default.
///
/// [`key`]: GameKind::key
/// [`recognise`]: GameKind::recognise
/// [`shape`]: GameKind::shape
/// [`applies`]: GameKind::applies
pub trait GameKind: Sync {
    /// A short name for it (`paradox`, `kenshi`).
    fn key(&self) -> &'static str;

    /// Whether the game in `root` is this one, by its files. `exes`: the exes at its top, the
    /// largest first.
    fn recognise(&self, root: &Path, exes: &[String]) -> Option<Recognised>;

    /// What its draft gets: `found` is what the folder says, `exe` the one it starts with,
    /// `setup` the mod loader setup it gets.
    fn shape(&self, root: &Path, found: &Inspection, exe: &str, setup: LoaderSetup) -> Shape;

    /// Whether a game definition is this game's, by the sections it has (`[paradox]`, its
    /// `[mod_list]`'s format): what was drafted for it is what tells it apart.
    fn applies(&self, def: &GameDef) -> bool;

    /// Its own mod support, for a definition it applies to: the name its generic setup gets.
    fn own_mods(&self) -> Option<&'static str> {
        Some(BUILT_IN_MODS)
    }

    /// Its install rules without a mod loader, for a definition it applies to.
    fn own_rules(&self, _def: &GameDef) -> Vec<InstallRule> {
        vec![rule("**", "")]
    }
}

static KINDS: OnceLock<&'static [&'static dyn GameKind]> = OnceLock::new();

/// Makes these the games UMM knows, in the order they are tried. Only the first call counts.
pub fn register(kinds: &'static [&'static dyn GameKind]) {
    let _ = KINDS.set(kinds);
}

/// The games UMM knows; none until [`register`]ed.
pub fn kinds() -> &'static [&'static dyn GameKind] {
    KINDS.get().copied().unwrap_or(&[])
}

/// The game a definition is, if UMM knows it.
pub fn kind_of(def: &GameDef) -> Option<&'static dyn GameKind> {
    kinds().iter().copied().find(|k| k.applies(def))
}

/// The game with this key.
pub fn kind(key: &str) -> Option<&'static dyn GameKind> {
    kinds().iter().copied().find(|k| k.key() == key)
}

pub fn rule(m: &str, t: &str) -> InstallRule {
    InstallRule {
        pattern: m.into(),
        target: t.into(),
    }
}

/// Install rules for a game with a folder per mod in `folder` (RimWorld, Factorio, Oxygen Not
/// Included).
pub fn mod_folder_rules(folder: &str) -> Vec<InstallRule> {
    vec![rule(
        "**",
        &format!("{}/{{mod}}/", folder.trim_matches('/')),
    )]
}

/// Whether `root` has the file at `rel` (`/`-separated), in any case.
pub fn has_file_ci(root: &Path, rel: &str) -> bool {
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

/// Whether one of `exes` (relative, as an [`Inspection`] lists them) is `name` at the top of
/// the game folder, in any case.
pub fn has_top_exe(exes: &[String], name: &str) -> bool {
    exes.iter()
        .any(|x| !x.contains('/') && x.eq_ignore_ascii_case(name))
}
