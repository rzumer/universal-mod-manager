//! Mount & Blade II: Bannerlord: each mod is a module, a folder in the game folder's
//! `Modules/` named after the module's id and holding its `SubModule.xml`. The game loads the
//! modules it is started with (`/singleplayer _MODULES_*Native*…*_MODULES_`), in that order;
//! its launcher otherwise picks them, from its own list in `LauncherData.xml`.
//!
//! UMM starts the game past its launcher with the modules named: the game's own (single
//! player) ones, the profile's mods and the Steam Workshop items it has on, and those the
//! user selected in the launcher (their own modules, other Workshop items). Each module comes
//! after the ones it depends on, the game's own first.

use crate::games::modfile::rename_folders;
use crate::{Result, Session};
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use umm_deploy::{Journal, Op, Plan};
use umm_gamedef::CompiledDef;
use umm_store::Profile;

/// A module's description, in its folder.
pub const SUBMODULE: &str = "SubModule.xml";
/// The launcher's list of modules, with which ones the user selected.
pub const LAUNCHER_DATA: &str =
    "%DOCUMENTS%/Mount and Blade II Bannerlord/Configs/LauncherData.xml";
/// The game's own modules in the order its launcher has them; others of the game's follow.
const OFFICIAL_ORDER: &[&str] = &[
    "Native",
    "SandBoxCore",
    "BirthAndDeath",
    "CustomBattle",
    "Sandbox",
    "StoryMode",
];

/// A module, as its `SubModule.xml` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub id: String,
    /// One of the game's own.
    pub official: bool,
    /// For multiplayer only.
    pub multiplayer: bool,
    /// Modules it loads after.
    pub after: Vec<String>,
    /// Modules it loads before.
    pub before: Vec<String>,
}

/// The `value` of an element's child of this name.
fn value<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
        .and_then(|c| c.attribute("value"))
        .map(str::trim)
}

fn children<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
        .flat_map(|c| c.children().filter(|g| g.is_element()))
}

/// An attribute, whatever its case (`Id`, `id`).
fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name().eq_ignore_ascii_case(name))
        .map(|a| a.value().trim())
}

/// The module a `SubModule.xml` describes, if it names one.
pub fn module_info(xml: &str) -> Option<Module> {
    let doc = Document::parse(xml.trim_start_matches('\u{feff}')).ok()?;
    let m = doc.root_element();
    let id = value(m, "Id").filter(|i| !i.is_empty())?.to_string();
    let official = value(m, "Official").is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || value(m, "ModuleType").is_some_and(|v| v.starts_with("Official"));
    let multiplayer = value(m, "ModuleCategory").is_some_and(|v| v.starts_with("Multiplayer"))
        || (value(m, "MultiplayerModule") == Some("true")
            && value(m, "SingleplayerModule") != Some("true"));
    let mut after: Vec<String> = children(m, "DependedModules")
        .filter_map(|d| attr(d, "Id"))
        .map(str::to_string)
        .collect();
    let mut before = Vec::new();
    for d in children(m, "DependedModuleMetadatas") {
        let Some(other) = attr(d, "id") else { continue };
        match attr(d, "order") {
            Some("LoadAfterThis") => before.push(other.to_string()),
            _ => after.push(other.to_string()),
        }
    }
    Some(Module {
        id,
        official,
        multiplayer,
        after,
        before,
    })
}

fn read_module(file: &Path) -> Option<Module> {
    module_info(&String::from_utf8_lossy(&fs::read(file).ok()?))
}

/// The modules the launcher lists, in its order, and whether the user selected each.
pub fn launcher_selection(xml: &str) -> Vec<(String, bool)> {
    let Ok(doc) = Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let text = |n: Node, name: &str| {
        n.children()
            .find(|c| c.has_tag_name(name))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    let Some(single) = doc
        .descendants()
        .find(|n| n.has_tag_name("SingleplayerData"))
    else {
        return Vec::new();
    };
    single
        .descendants()
        .filter(|n| n.has_tag_name("UserModData"))
        .filter_map(|n| {
            let id = text(n, "Id").filter(|i| !i.is_empty())?;
            let on = text(n, "IsSelected").is_some_and(|s| s.eq_ignore_ascii_case("true"));
            Some((id, on))
        })
        .collect()
}

/// The modules in this order, each after those it depends on (and before those that ask to
/// come after it), otherwise as they were.
pub fn load_order(modules: Vec<Module>) -> Vec<Module> {
    let key = |s: &str| s.to_ascii_lowercase();
    let ids: Vec<String> = modules.iter().map(|m| key(&m.id)).collect();
    // Each module's predecessors among these.
    let mut first: HashMap<String, Vec<String>> = HashMap::new();
    for m in &modules {
        for a in &m.after {
            if ids.contains(&key(a)) {
                first.entry(key(&m.id)).or_default().push(key(a));
            }
        }
        for b in &m.before {
            if ids.contains(&key(b)) {
                first.entry(key(b)).or_default().push(key(&m.id));
            }
        }
    }
    let mut left = modules;
    let mut out: Vec<Module> = Vec::new();
    while !left.is_empty() {
        let placed: Vec<String> = out.iter().map(|m| key(&m.id)).collect();
        let ready = left
            .iter()
            .position(|m| {
                first
                    .get(&key(&m.id))
                    .is_none_or(|p| p.iter().all(|p| placed.contains(p)))
            })
            // A cycle: as they were.
            .unwrap_or(0);
        out.push(left.remove(ready));
    }
    out
}

impl Session {
    /// For Bannerlord: each module's folder in `Modules/` named after its id.
    pub(crate) fn plan_bannerlord(&self, def: &CompiledDef, plan: &mut Plan) -> Result<()> {
        let Some(b) = &def.def.bannerlord else {
            return Ok(());
        };
        let prefix = format!("{}/", b.folder.trim_matches('/'));
        let mut names: HashMap<(String, String), String> = HashMap::new();
        for f in &plan.files {
            let t = f.target.as_str();
            let Some(rest) = t
                .get(..prefix.len())
                .filter(|p| p.eq_ignore_ascii_case(&prefix))
                .map(|_| &t[prefix.len()..])
            else {
                continue;
            };
            let Some((dir, file)) = rest.split_once('/') else {
                continue;
            };
            if !file.eq_ignore_ascii_case(SUBMODULE) {
                continue;
            }
            if let Some(m) = read_module(&f.source) {
                names.insert((f.owner.clone(), dir.to_lowercase()), m.id);
            }
        }
        rename_folders(plan, &prefix, &names, "Bannerlord module")
    }

    /// The game's arguments for the modules to load, for Bannerlord (see the module's
    /// description); none for another game.
    pub fn bannerlord_args(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        journal: &Journal,
    ) -> Vec<String> {
        if def.def.bannerlord.is_none() {
            return Vec::new();
        }
        let selection = umm_gamedef::expand_path_with(LAUNCHER_DATA, &self.path_vars(&def.def))
            .and_then(|p| fs::read(p).ok())
            .map(|b| launcher_selection(&String::from_utf8_lossy(&b)))
            .unwrap_or_default();
        self.bannerlord_args_with(def, profile, journal, &selection)
    }

    /// [`Self::bannerlord_args`], with the launcher's selection given (see
    /// [`launcher_selection`]).
    pub fn bannerlord_args_with(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        journal: &Journal,
        selection: &[(String, bool)],
    ) -> Vec<String> {
        let Some(b) = &def.def.bannerlord else {
            return Vec::new();
        };
        let folder = b.folder.trim_matches('/');
        let modules_dir = journal.game_root.join(folder);
        // The profile's mods: the modules this deploy placed.
        let prefix = format!("{folder}/").to_ascii_lowercase();
        let mut ours: Vec<Module> = Vec::new();
        for op in &journal.ops {
            let (Op::Add { path, src, .. } | Op::Replace { path, src, .. }) = op else {
                continue;
            };
            let p = path.as_str().to_ascii_lowercase();
            let is_desc = p
                .strip_prefix(&prefix)
                .and_then(|r| r.split_once('/'))
                .is_some_and(|(_, f)| f == SUBMODULE.to_ascii_lowercase());
            if is_desc {
                ours.extend(read_module(src));
            }
        }
        // Workshop items: as the profile says, or else as the launcher does.
        let folders = self.workshop_folders(&def.def);
        let in_profile: HashMap<&str, bool> = profile.workshop().collect();
        let mut workshop_on = Vec::new();
        let mut workshop_off = Vec::new();
        let mut others: Vec<Module> = Vec::new();
        for (item, dir) in &folders {
            let Some(m) = read_module(&dir.join(SUBMODULE)) else {
                continue;
            };
            match in_profile.get(item.as_str()) {
                Some(true) => workshop_on.push((item.clone(), m)),
                Some(false) => workshop_off.push(m.id.to_ascii_lowercase()),
                None => others.push(m),
            }
        }
        workshop_on.sort_by_key(|(item, _)| {
            profile
                .position(&format!("{}{item}", umm_store::WORKSHOP_PREFIX))
                .unwrap_or(usize::MAX)
        });
        let selected = |id: &str| {
            selection
                .iter()
                .find(|(i, _)| i.eq_ignore_ascii_case(id))
                .map(|(_, on)| *on)
        };
        // The game's own modules and the user's, from the modules folder.
        let mut official = Vec::new();
        let mut theirs = Vec::new();
        let mut dirs: Vec<_> = fs::read_dir(&modules_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for dir in dirs {
            let Some(m) = read_module(&dir.join(SUBMODULE)) else {
                continue;
            };
            if ours.iter().any(|o| o.id.eq_ignore_ascii_case(&m.id)) {
                continue;
            }
            if m.official {
                if !m.multiplayer && selected(&m.id) != Some(false) {
                    official.push(m);
                }
            } else if selected(&m.id) == Some(true) {
                theirs.push(m);
            }
        }
        official.sort_by_key(|m| {
            OFFICIAL_ORDER
                .iter()
                .position(|o| o.eq_ignore_ascii_case(&m.id))
                .unwrap_or(OFFICIAL_ORDER.len())
        });
        theirs.extend(others.into_iter().filter(|m| {
            !workshop_off.contains(&m.id.to_ascii_lowercase()) && selected(&m.id) == Some(true)
        }));
        // The user's in the launcher's order.
        theirs.sort_by_key(|m| {
            selection
                .iter()
                .position(|(i, _)| i.eq_ignore_ascii_case(&m.id))
                .unwrap_or(usize::MAX)
        });
        let mut all: Vec<Module> = official;
        all.extend(theirs);
        all.extend(ours);
        all.extend(workshop_on.into_iter().map(|(_, m)| m));
        let mut seen: Vec<String> = Vec::new();
        all.retain(|m| {
            let k = m.id.to_ascii_lowercase();
            !seen.contains(&k) && {
                seen.push(k);
                true
            }
        });
        let ids: Vec<String> = load_order(all).into_iter().map(|m| m.id).collect();
        vec![
            "/singleplayer".to_string(),
            format!("_MODULES_*{}*_MODULES_", ids.join("*")),
        ]
    }
}

pub(crate) struct Bannerlord;

impl crate::games::GameIntegration for Bannerlord {
    /// A module's folder holds its `SubModule.xml`.
    fn mod_root(
        &self,
        rels: &[umm_gamedef::RelPath],
        _def: &CompiledDef,
    ) -> Option<Option<umm_store::Placement>> {
        Some(crate::placement::marked_root(rels, |n| {
            n.eq_ignore_ascii_case(SUBMODULE)
        }))
    }

    fn workshop(&self) -> crate::workshop::WorkshopControl {
        crate::workshop::WorkshopControl::Profile
    }

    fn plan(
        &self,
        s: &Session,
        def: &CompiledDef,
        _profile: &umm_store::Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        s.plan_bannerlord(def, plan)
    }

    fn launch_args(
        &self,
        s: &Session,
        def: &CompiledDef,
        profile: &umm_store::Profile,
        journal: &umm_deploy::Journal,
    ) -> Vec<String> {
        s.bannerlord_args(def, profile, journal)
    }
}

/// Bannerlord's exe, started directly past its launcher
/// (`TaleWorlds.MountAndBlade.Launcher.exe` beside it).
pub const BANNERLORD_EXE: &str = "bin/Win64_Shipping_Client/Bannerlord.exe";

/// The game folder's folder of modules.
const MODULES: &str = "Modules";

impl umm_gamedef::GameKind for Bannerlord {
    fn key(&self) -> &'static str {
        "bannerlord"
    }

    /// By its exe in `bin/Win64_Shipping_Client/` and its own modules in `Modules/`.
    fn recognise(&self, root: &Path, _exes: &[String]) -> Option<umm_gamedef::Recognised> {
        use umm_gamedef::kind::has_file_ci;
        (has_file_ci(root, BANNERLORD_EXE) && has_file_ci(root, "Modules/Native/SubModule.xml"))
            .then(|| umm_gamedef::Recognised {
                key: "bannerlord",
                label: Some("TaleWorlds Engine (Bannerlord)"),
                exe: Some(BANNERLORD_EXE.to_string()),
                own_mods: Some(umm_gamedef::BUILT_IN_MODS),
            })
    }

    fn shape(
        &self,
        _root: &Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "bannerlord",
                "Bannerlord: each mod goes in its own folder in Modules/, named after the module's \
                 id as the game wants. UMM starts the game past its launcher with the modules to \
                 load: the game's own, your profile's (Steam Workshop items included), and those \
                 you selected in the launcher, each after the ones it depends on.",
            )],
            install: umm_gamedef::kind::mod_folder_rules(MODULES),
            bannerlord: Some(umm_gamedef::Bannerlord {
                folder: MODULES.into(),
            }),
            // Its launcher stands before the game.
            direct: true,
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.bannerlord.is_some()
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        umm_gamedef::kind::mod_folder_rules(
            def.bannerlord
                .as_ref()
                .map_or(MODULES, |b| b.folder.as_str()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_module_and_orders_by_dependencies() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Module>
  <Name value="Better Smithing"/>
  <Id value="BetterSmithing"/>
  <Version value="v1.0.0"/>
  <ModuleCategory value="Singleplayer"/>
  <DependedModules>
    <DependedModule Id="Native" DependentVersion="v1.2.9"/>
    <DependedModule Id="Sandbox"/>
  </DependedModules>
  <DependedModuleMetadatas>
    <DependedModuleMetadata id="Bannerlord.Harmony" order="LoadBeforeThis"/>
    <DependedModuleMetadata id="Late" order="LoadAfterThis" optional="true"/>
  </DependedModuleMetadatas>
</Module>"#;
        let m = module_info(xml).unwrap();
        assert_eq!(m.id, "BetterSmithing");
        assert!(!m.official && !m.multiplayer);
        assert_eq!(m.after, ["Native", "Sandbox", "Bannerlord.Harmony"]);
        assert_eq!(m.before, ["Late"]);
        let official = module_info(
            r#"<Module><Id value="Native"/><ModuleType value="Official"/><ModuleCategory value="Singleplayer"/></Module>"#,
        )
        .unwrap();
        assert!(official.official);
        let mp = module_info(
            r#"<Module><Id value="Multiplayer"/><Official value="true"/><ModuleCategory value="Multiplayer"/></Module>"#,
        )
        .unwrap();
        assert!(mp.official && mp.multiplayer);
        assert_eq!(module_info("<Module/>"), None);

        let simple = |id: &str| Module {
            id: id.into(),
            official: false,
            multiplayer: false,
            after: Vec::new(),
            before: Vec::new(),
        };
        let order = load_order(vec![
            simple("Native"),
            simple("Late"),
            m,
            simple("Sandbox"),
            simple("Bannerlord.Harmony"),
        ]);
        let ids: Vec<&str> = order.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "Native",
                "Sandbox",
                "Bannerlord.Harmony",
                "BetterSmithing",
                "Late"
            ]
        );
    }

    #[test]
    fn reads_the_launchers_selection() {
        let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<UserData xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\r\n  <GameType>Singleplayer</GameType>\r\n  <SingleplayerData>\r\n    <ModDatas>\r\n      <UserModData>\r\n        <Id>Native</Id>\r\n        <LastKnownVersion>v1.2.9.0</LastKnownVersion>\r\n        <IsSelected>true</IsSelected>\r\n      </UserModData>\r\n      <UserModData>\r\n        <Id>Mine</Id>\r\n        <IsSelected>false</IsSelected>\r\n      </UserModData>\r\n    </ModDatas>\r\n  </SingleplayerData>\r\n  <MultiplayerData>\r\n    <ModDatas>\r\n      <UserModData>\r\n        <Id>Multiplayer</Id>\r\n        <IsSelected>true</IsSelected>\r\n      </UserModData>\r\n    </ModDatas>\r\n  </MultiplayerData>\r\n</UserData>";
        assert_eq!(
            launcher_selection(xml),
            [("Native".to_string(), true), ("Mine".to_string(), false)]
        );
        assert!(launcher_selection("not xml").is_empty());
    }
}
