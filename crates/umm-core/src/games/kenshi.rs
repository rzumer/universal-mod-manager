//! Kenshi: each mod is a folder in `mods/`, named after the `.mod` file in it, and enabled in
//! `data/mods.cfg` (see [`crate::games::modfile`] and [`crate::games::plugins`]).

use crate::games::{modfile, GameIntegration};
use crate::{Result, Session};
use umm_deploy::Plan;
use umm_gamedef::{CompiledDef, GameDef, PluginListFormat};
use umm_store::Profile;

pub(crate) struct Kenshi;

/// A Kenshi mod's file.
pub(crate) const EXTENSION: &str = ".mod";

impl GameIntegration for Kenshi {
    fn mod_root(
        &self,
        rels: &[umm_gamedef::RelPath],
        _def: &umm_gamedef::CompiledDef,
    ) -> Option<Option<umm_store::Placement>> {
        Some(crate::games::modfile::mod_root(rels, EXTENSION))
    }

    fn workshop(&self) -> crate::workshop::WorkshopControl {
        crate::workshop::WorkshopControl::Profile
    }

    fn plan(
        &self,
        s: &Session,
        def: &CompiledDef,
        _profile: &Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        s.plan_mod_files(def, plan, EXTENSION, "Kenshi mod")
    }
}

impl crate::games::plugins::ModList for Kenshi {
    fn kind(&self) -> &'static str {
        "mods"
    }

    /// `mods.cfg`: the enabled mods' files, one per line.
    fn read(&self, text: &str) -> Vec<crate::games::plugins::PluginEntry> {
        crate::games::bethesda::read_lines(text, false)
    }

    fn render(&self, original: &str, entries: &[crate::games::plugins::PluginEntry]) -> String {
        crate::games::bethesda::render_lines(original, entries, false)
    }

    /// A folder each, by the `<Name>.mod` file in it.
    fn entries<'a>(
        &self,
        files: &'a [umm_deploy::PlannedFile],
        folder: &umm_gamedef::RelPath,
    ) -> Vec<(&'a str, String)> {
        let prefix = format!("{}/", folder.as_str());
        files
            .iter()
            .filter_map(|f| {
                let (_, file) = modfile::mod_file(f.target.as_str(), &prefix, EXTENSION)?;
                Some((f.owner.as_str(), file.to_string()))
            })
            .collect()
    }

    fn lists_workshop_items(&self) -> bool {
        true
    }

    fn workshop_entry(&self, folder: &std::path::Path) -> Option<String> {
        modfile::mod_in(folder, EXTENSION)
    }
}

/// Its exes: Steam's and GOG's builds.
const EXES: &[&str] = &["kenshi_x64.exe", "kenshi_GOG_x64.exe"];
const MODS: &str = "mods";

impl umm_gamedef::GameKind for Kenshi {
    fn key(&self) -> &'static str {
        "kenshi"
    }

    fn recognise(
        &self,
        _root: &std::path::Path,
        exes: &[String],
    ) -> Option<umm_gamedef::Recognised> {
        EXES.iter()
            .any(|e| umm_gamedef::kind::has_top_exe(exes, e))
            .then_some(umm_gamedef::Recognised {
                key: "kenshi",
                label: None,
                exe: None,
                own_mods: Some(umm_gamedef::BUILT_IN_MODS),
            })
    }

    fn shape(
        &self,
        _root: &std::path::Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "plugins",
                "Kenshi: each mod goes in its own folder in mods/, named after its .mod file as the game wants, and while mods are deployed, UMM enables them in data/mods.cfg, in load order (your own list comes back afterwards). Steam Workshop items can be turned on or off in a profile too.",
            )],
            install: umm_gamedef::kind::mod_folder_rules(MODS),
            plugins: Some(umm_gamedef::Plugins {
                list: "data/mods.cfg".into(),
                format: PluginListFormat::KenshiCfg,
                folder: MODS.into(),
                in_game_folder_when: None,
            }),
            ..Default::default()
        }
    }

    fn applies(&self, def: &GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == PluginListFormat::KenshiCfg)
    }

    fn own_rules(&self, def: &GameDef) -> Vec<umm_gamedef::InstallRule> {
        umm_gamedef::kind::mod_folder_rules(
            def.plugins.as_ref().map_or(MODS, |p| p.folder.as_str()),
        )
    }
}
