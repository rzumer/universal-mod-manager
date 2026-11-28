//! One module per game or game family whose mods need more than files placed: their mod
//! lists, user folders and launch arguments. Each is a [`GameIntegration`], listed in
//! [`GAMES`]; adding a game means adding a module and a line there.

pub mod bannerlord;
pub mod bethesda;
mod drafts;
pub mod factorio;
pub mod kenshi;
pub mod larian;
pub mod modfile;
pub mod morrowind;
pub mod oni;
pub mod pak;
pub mod paradox;
pub mod plugins;
pub mod rimworld;
pub mod terraria;
pub mod tmodloader;
pub mod xcom2;

use crate::workshop::{WorkshopControl, WorkshopDetails};
use crate::{Result, Session};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use umm_deploy::{Journal, Plan};
use umm_gamedef::{CompiledDef, GameDef, GameKind, RelPath};
use umm_store::Placement;
use umm_store::Profile;

/// What UMM does for a game at deploy and launch, beside what [`GameKind`] says of it (how it
/// is recognised, what its definition gets). Every method has a default that does nothing, so
/// a game fills in only what it needs.
pub(crate) trait GameIntegration: GameKind {
    /// Shapes the deploy's plan once the profile's files are in it: folders renamed as the
    /// game wants, its own files (descriptors, lists) added.
    fn plan(
        &self,
        _s: &Session,
        _def: &CompiledDef,
        _profile: &Profile,
        _plan: &mut Plan,
    ) -> Result<()> {
        Ok(())
    }

    /// Arguments the game is started with, after the deploy (`journal`).
    fn launch_args(
        &self,
        _s: &Session,
        _def: &CompiledDef,
        _profile: &Profile,
        _journal: &Journal,
    ) -> Vec<String> {
        Vec::new()
    }

    /// Whether a profile turns the game's Steam Workshop items on or off (the game loads the
    /// ones its mod list names, which UMM writes), or Steam loads every subscribed one.
    fn workshop(&self) -> WorkshopControl {
        WorkshopControl::Steam
    }

    /// For a game with a folder per mod: where a mod's folder starts among its files (`rels`,
    /// paths in the mod), found by what marks it. `None`: the general suggestion applies.
    fn mod_root(&self, _rels: &[RelPath], _def: &CompiledDef) -> Option<Option<Placement>> {
        None
    }

    /// Files a mod's folder keeps that look like documents but the game reads (Factorio shows
    /// a mod's changelog), by name.
    fn kept_docs(&self) -> &'static [&'static str] {
        &[]
    }

    /// For a game on Steam's older Workshop, which downloads its items itself (Crusader Kings
    /// II): the folder it puts them in.
    fn workshop_downloads(&self, _s: &Session, _def: &CompiledDef) -> Option<PathBuf> {
        None
    }

    /// The items such a game has downloaded into `folder`, by item id: the entry its mod list
    /// knows each by. `items`: Steam's details of the game's items.
    fn workshop_downloaded(
        &self,
        _folder: &Path,
        _items: &BTreeMap<String, WorkshopDetails>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Whether a mod's file (a path in the mod) is a descriptor of the kind UMM writes the
    /// game its own of (a Paradox mod's launcher `.mod`), so it is left out.
    fn is_mod_descriptor(&self, _file: &str) -> bool {
        false
    }
}

/// Every game, in the order they are tried when recognising one: the most specific first.
/// Both views of the list come from the one written here.
macro_rules! games {
    ($($g:path),* $(,)?) => {
        pub(crate) static GAMES: &[&dyn GameIntegration] = &[$(&$g),*];
        static KINDS: &[&dyn GameKind] = &[$(&$g),*];
    };
}

games![
    oni::Oni,
    paradox::Paradox,
    larian::Larian,
    bannerlord::Bannerlord,
    xcom2::Xcom2,
    factorio::Factorio,
    tmodloader::TModLoader,
    terraria::Terraria,
    morrowind::Morrowind,
    bethesda::Bethesda,
    rimworld::RimWorld,
    kenshi::Kenshi,
];

/// Makes these the games `umm-gamedef` knows when it recognises and drafts games.
pub(crate) fn register() {
    umm_gamedef::kind::register(KINDS);
}

/// The integration for a game, if it has one.
pub(crate) fn integration(def: &GameDef) -> Option<&'static dyn GameIntegration> {
    let found: Vec<_> = GAMES.iter().copied().filter(|g| g.applies(def)).collect();
    debug_assert!(
        found.len() <= 1,
        "several game integrations apply to `{}`: {:?}",
        def.id,
        found.iter().map(|g| g.key()).collect::<Vec<_>>()
    );
    found.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrations_have_distinct_keys() {
        let mut keys: Vec<_> = GAMES.iter().map(|g| g.key()).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }
}
