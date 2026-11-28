//! XCOM 2: each mod is a folder in `XComGame/Mods/`, named after the `.XComMod` file in it,
//! and enabled by `ActiveMods=` in the user's `XComModOptions.ini` (see
//! [`crate::games::modfile`] and [`crate::games::plugins`]).

use crate::games::plugins::{eol, header, ini_section, ModList, PluginEntry};
use crate::games::{modfile, GameIntegration};
use crate::{Result, Session};
use std::path::Path;
use umm_deploy::Plan;
use umm_deploy::PlannedFile;
use umm_gamedef::RelPath;
use umm_gamedef::{CompiledDef, GameDef, PluginListFormat};
use umm_store::Profile;

pub(crate) struct Xcom2;

/// An XCOM 2 mod's file.
pub(crate) const EXTENSION: &str = ".xcommod";

impl GameIntegration for Xcom2 {
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
        s.plan_mod_files(def, plan, EXTENSION, "XCOM 2 mod")
    }
}

/// The user's section of enabled mods, and its key.
const SECTION: &str = "Engine.XComModOptions";
const ACTIVE_MODS: &str = "ActiveMods";

/// An `ActiveMods=` line's value, without its quotes.
fn active_mod(line: &str) -> Option<&str> {
    let (k, v) = line.trim().split_once('=')?;
    k.trim()
        .eq_ignore_ascii_case(ACTIVE_MODS)
        .then(|| v.trim().trim_matches('"').trim())
}

impl ModList for Xcom2 {
    fn kind(&self) -> &'static str {
        "mods"
    }

    fn read(&self, text: &str) -> Vec<PluginEntry> {
        let mut section = "";
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(h) = header(line) {
                section = h;
            } else if section.eq_ignore_ascii_case(SECTION) {
                if let Some(v) = active_mod(line).filter(|v| !v.is_empty()) {
                    out.push(PluginEntry {
                        name: v.to_string(),
                        enabled: true,
                        owner: None,
                    });
                }
            }
        }
        out
    }

    fn render(&self, original: &str, entries: &[PluginEntry]) -> String {
        let nl = eol(original);
        let lines: String = entries
            .iter()
            .filter(|e| e.enabled)
            .flat_map(|e| [ACTIVE_MODS, "=\"", e.name.as_str(), "\"", nl])
            .collect();
        ini_section(original, SECTION, |l| active_mod(l).is_some(), &lines, nl)
    }

    /// A folder each, by the `<Name>.XComMod` file in it: listed as `<Name>`.
    fn entries<'a>(&self, files: &'a [PlannedFile], folder: &RelPath) -> Vec<(&'a str, String)> {
        let prefix = format!("{}/", folder.as_str());
        files
            .iter()
            .filter_map(|f| {
                let (_, file) = modfile::mod_file(f.target.as_str(), &prefix, EXTENSION)?;
                Some((
                    f.owner.as_str(),
                    modfile::name_of(file, EXTENSION).to_string(),
                ))
            })
            .collect()
    }

    fn lists_workshop_items(&self) -> bool {
        true
    }

    fn workshop_entry(&self, folder: &Path) -> Option<String> {
        modfile::mod_in(folder, EXTENSION).map(|f| modfile::name_of(&f, EXTENSION).to_string())
    }
}

/// XCOM 2's exe, and War of the Chosen's, in its folder within the game's.
pub const XCOM2_EXE: &str = "Binaries/Win64/XCom2.exe";
pub const XCOM2_WOTC_EXE: &str = "XCom2-WarOfTheChosen/Binaries/Win64/XCom2.exe";

impl umm_gamedef::GameKind for Xcom2 {
    fn key(&self) -> &'static str {
        "xcom2"
    }

    /// War of the Chosen when installed (its own game, in its folder), else the base game;
    /// both started directly, past 2K's launcher.
    fn recognise(&self, root: &Path, _exes: &[String]) -> Option<umm_gamedef::Recognised> {
        use umm_gamedef::kind::has_file_ci;
        if !has_file_ci(root, XCOM2_EXE) {
            return None;
        }
        let exe = if has_file_ci(root, XCOM2_WOTC_EXE) {
            XCOM2_WOTC_EXE
        } else {
            XCOM2_EXE
        };
        Some(umm_gamedef::Recognised {
            key: "xcom2",
            label: Some("Unreal Engine 3 (XCOM 2)"),
            exe: Some(exe.to_string()),
            own_mods: Some(umm_gamedef::BUILT_IN_MODS),
        })
    }

    fn shape(
        &self,
        _root: &Path,
        _found: &umm_gamedef::steam::Inspection,
        exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let wotc = exe.eq_ignore_ascii_case(XCOM2_WOTC_EXE);
        let mods = format!(
            "{}XComGame/Mods",
            if wotc { "XCom2-WarOfTheChosen/" } else { "" }
        );
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "xcom2",
                format!(
                    "XCOM 2{}: each mod goes in its own folder in {}XComGame/Mods/, named after its \
                     .XComMod file as the game wants, and while mods are deployed, UMM enables them \
                     in XComModOptions.ini in Documents\\My Games (your own list comes back \
                     afterwards). Steam Workshop items can be turned on or off in a profile too. UMM \
                     starts the game directly, not through 2K's launcher.",
                    if wotc { ": War of the Chosen" } else { "" },
                    if wotc { "XCom2-WarOfTheChosen/" } else { "" },
                ),
            )],
            install: umm_gamedef::kind::mod_folder_rules(&mods),
            plugins: Some(umm_gamedef::Plugins {
                list: format!(
                    "%DOCUMENTS%/My Games/{}/XComGame/Config/XComModOptions.ini",
                    if wotc { "XCOM2 War of the Chosen" } else { "XCOM2" }
                ),
                format: PluginListFormat::XcomIni,
                folder: mods,
                in_game_folder_when: None,
            }),
            direct: true,
            ..Default::default()
        }
    }

    fn applies(&self, def: &GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == PluginListFormat::XcomIni)
    }

    fn own_rules(&self, def: &GameDef) -> Vec<umm_gamedef::InstallRule> {
        umm_gamedef::kind::mod_folder_rules(def.plugins.as_ref().map_or("", |p| p.folder.as_str()))
    }
}
