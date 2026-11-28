//! RimWorld: each mod is a folder in `Mods/`, known by the `packageId` in its
//! `About/About.xml`, and loaded when `<activeMods>` in `ModsConfig.xml` (in `AppData\LocalLow`)
//! lists it, in load order. Steam Workshop items are listed the same way.

use crate::games::plugins::{eol, ModList, PluginEntry};
use std::fmt::Write;
use std::fs;
use std::path::Path;
use umm_deploy::PlannedFile;
use umm_gamedef::RelPath;

pub(crate) struct RimWorld;

const ACTIVE: &str = "<activeMods>";
const ACTIVE_END: &str = "</activeMods>";

/// Where the `<activeMods>` contents are, and their `<li>` entries.
fn active_mods(text: &str) -> Option<(usize, usize, Vec<String>)> {
    let start = text.find(ACTIVE)? + ACTIVE.len();
    let end = start + text[start..].find(ACTIVE_END)?;
    let mut ids = Vec::new();
    let mut rest = &text[start..end];
    while let Some(i) = rest.find("<li>") {
        rest = &rest[i + 4..];
        let Some(j) = rest.find("</li>") else { break };
        ids.push(rest[..j].trim().to_string());
        rest = &rest[j + 5..];
    }
    Some((start, end, ids))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A mod's package id, from its `About/About.xml`, lower-case as the game lists it.
fn package_id(about: &Path) -> Option<String> {
    let text = String::from_utf8_lossy(&fs::read(about).ok()?).into_owned();
    let lower = text.to_ascii_lowercase();
    let start = lower.find("<packageid>")? + "<packageid>".len();
    let end = start + lower[start..].find('<')?;
    let id = lower[start..end].trim().to_string();
    (!id.is_empty()).then_some(id)
}

impl ModList for RimWorld {
    fn kind(&self) -> &'static str {
        "mods"
    }

    fn read(&self, text: &str) -> Vec<PluginEntry> {
        active_mods(text)
            .map(|(_, _, ids)| {
                ids.into_iter()
                    .map(|name| PluginEntry {
                        name,
                        enabled: true,
                        owner: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn render(&self, original: &str, entries: &[PluginEntry]) -> String {
        let nl = eol(original);
        let mut items = String::new();
        for e in entries.iter().filter(|e| e.enabled) {
            let _ = write!(items, "{nl}    <li>{}</li>", xml_escape(&e.name));
        }
        if let Some((start, end, _)) = active_mods(original) {
            return format!("{}{items}{nl}  {}", &original[..start], &original[end..]);
        }
        let block = format!("{ACTIVE}{items}{nl}  {ACTIVE_END}");
        for empty in ["<activeMods />", "<activeMods/>"] {
            if let Some(i) = original.find(empty) {
                return format!("{}{block}{}", &original[..i], &original[i + empty.len()..]);
            }
        }
        if let Some(i) = original.find("</ModsConfigData>") {
            return format!("{}  {block}{nl}{}", &original[..i], &original[i..]);
        }
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>{nl}<ModsConfigData>{nl}  {block}{nl}</ModsConfigData>{nl}"
        )
    }

    /// A folder each in the mods folder, named by its `About/About.xml`.
    fn entries<'a>(&self, files: &'a [PlannedFile], folder: &RelPath) -> Vec<(&'a str, String)> {
        let prefix = match folder.as_str() {
            "" => String::new(),
            f => format!("{}/", f.to_lowercase()),
        };
        files
            .iter()
            .filter(|f| {
                f.target
                    .as_str()
                    .to_lowercase()
                    .strip_prefix(&prefix)
                    .and_then(|r| r.strip_suffix("/about/about.xml"))
                    .is_some_and(|d| !d.is_empty() && !d.contains('/'))
            })
            .filter_map(|f| Some((f.owner.as_str(), package_id(&f.source)?)))
            .collect()
    }

    fn lists_workshop_items(&self) -> bool {
        true
    }

    fn workshop_entry(&self, folder: &Path) -> Option<String> {
        package_id(&folder.join("About").join("About.xml"))
    }
}

/// Its exes, 64-bit and 32-bit.
const EXES: &[&str] = &["RimWorldWin64.exe", "RimWorldWin.exe"];
const MODS: &str = "Mods";

/// RimWorld's list, in its folder under `AppData\\LocalLow` (which has no variable of its own).
const LIST: &str =
    "%USERPROFILE%/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Config/ModsConfig.xml";

impl umm_gamedef::GameKind for RimWorld {
    fn key(&self) -> &'static str {
        "rimworld"
    }

    fn recognise(&self, _root: &Path, exes: &[String]) -> Option<umm_gamedef::Recognised> {
        EXES.iter()
            .any(|e| umm_gamedef::kind::has_top_exe(exes, e))
            .then_some(umm_gamedef::Recognised {
                key: "rimworld",
                label: None,
                exe: None,
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
                "plugins",
                format!(
                    "RimWorld: each mod goes in its own folder in Mods/, and while mods are deployed, UMM activates them in {LIST}, in load order."
                ),
            )],
            install: umm_gamedef::kind::mod_folder_rules(MODS),
            plugins: Some(umm_gamedef::Plugins {
                list: LIST.into(),
                format: umm_gamedef::PluginListFormat::RimworldXml,
                folder: MODS.into(),
                in_game_folder_when: None,
            }),
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == umm_gamedef::PluginListFormat::RimworldXml)
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        umm_gamedef::kind::mod_folder_rules(
            def.plugins.as_ref().map_or(MODS, |p| p.folder.as_str()),
        )
    }
}

impl crate::games::GameIntegration for RimWorld {
    fn mod_root(
        &self,
        rels: &[umm_gamedef::RelPath],
        _def: &umm_gamedef::CompiledDef,
    ) -> Option<Option<umm_store::Placement>> {
        Some(mod_root(rels))
    }

    fn workshop(&self) -> crate::workshop::WorkshopControl {
        crate::workshop::WorkshopControl::Profile
    }
}

/// A RimWorld mod's folder is the one holding `About/About.xml`: whatever wraps it is
/// dropped (the install rules give each mod its own folder in `Mods/`). Several at the same
/// depth (a pack of mods) is a guess.
fn mod_root(rels: &[umm_gamedef::RelPath]) -> Option<umm_store::Placement> {
    let mut roots: Vec<(usize, String)> = rels
        .iter()
        .filter(|r| {
            let c: Vec<&str> = r.components().collect();
            c.len() >= 2
                && c[c.len() - 1].eq_ignore_ascii_case("About.xml")
                && c[c.len() - 2].eq_ignore_ascii_case("About")
        })
        .map(|r| {
            let root = r
                .parent()
                .and_then(|p| p.parent())
                .unwrap_or_else(umm_gamedef::RelPath::root);
            (root.depth(), root.as_str().to_string())
        })
        .collect();
    roots.sort();
    let (depth, root) = roots.first()?.clone();
    let guessed = roots.get(1).is_some_and(|(d, _)| *d == depth);
    let p = umm_store::Placement {
        root,
        guessed,
        ..Default::default()
    };
    (p != umm_store::Placement::default()).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rimworld_lists_its_active_mods() {
        let original = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.5.4104 rev435</version>\n  <activeMods>\n    <li>brrainz.harmony</li>\n    <li>ludeon.rimworld</li>\n  </activeMods>\n  <knownExpansions>\n    <li>ludeon.rimworld.royalty</li>\n  </knownExpansions>\n</ModsConfigData>\n";
        let e = RimWorld.read(original);
        assert_eq!(
            e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["brrainz.harmony", "ludeon.rimworld"]
        );
        let l: Vec<PluginEntry> = ["brrainz.harmony", "ludeon.rimworld", "me.mine"]
            .iter()
            .map(|n| PluginEntry {
                name: n.to_string(),
                enabled: true,
                owner: None,
            })
            .collect();
        assert_eq!(
            RimWorld.render(original, &l),
            original.replace(
                "    <li>ludeon.rimworld</li>\n",
                "    <li>ludeon.rimworld</li>\n    <li>me.mine</li>\n"
            )
        );
        assert_eq!(
            RimWorld.render("<ModsConfigData>\r\n  <activeMods />\r\n</ModsConfigData>", &l),
            "<ModsConfigData>\r\n  <activeMods>\r\n    <li>brrainz.harmony</li>\r\n    <li>ludeon.rimworld</li>\r\n    <li>me.mine</li>\r\n  </activeMods>\r\n</ModsConfigData>"
        );
        assert!(RimWorld
            .render("", &l)
            .contains("<ModsConfigData>\r\n  <activeMods>\r\n    <li>brrainz.harmony</li>"));
    }
}
