//! Larian games (Baldur's Gate 3): mods are `.pak` packages in the game's user folder's
//! `Mods/` (`%LOCALAPPDATA%\Larian Studios\Baldur's Gate 3\Mods`), not the game folder. A
//! package with a module of its own (`Mods/<Folder>/meta.lsx` inside it) loads only when
//! `PlayerProfiles/Public/modsettings.lsx` lists that module; one without (an override of the
//! game's files) loads as it is.
//!
//! While mods are deployed, UMM adds each enabled mod's modules to that list, after the
//! game's own and the user's, and takes them out again afterwards: the rest of the file is
//! the game's, which it rewrites as it likes.

use crate::{CoreError, Result, Session};
use roxmltree::Document;
use std::fs;
use umm_deploy::lsx::attribute;
use umm_deploy::{ListFormat, Plan, PlannedList};
use umm_gamedef::CompiledDef;
use umm_store::Profile;

/// The game's list of enabled modules, in its user folder.
pub const MODSETTINGS: &str = "PlayerProfiles/Public/modsettings.lsx";
/// The list's node in it.
const MODS: &str = "Mods";
/// `Version64` for 1.0.0.0, for a module that gives none.
const VERSION_1: &str = "36028797018963968";
/// The game's own modules (GustavX, GustavDev, Gustav), which a mod's package may hold a copy
/// of to change, but which are not the mod's to list.
const BASE_MODULES: &[&str] = &[
    "cb555efe-2d9e-131f-8195-a89329d218ea",
    "28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8",
    "991c9c7a-fb80-40cb-8f0d-b92d4e80e9b1",
];

/// A module, as its `meta.lsx` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub uuid: String,
    pub folder: String,
    pub name: String,
    pub md5: String,
    pub version64: String,
    pub publish_handle: String,
}

/// Whether a package's file is a module's description: `Mods/<Folder>/meta.lsx`.
fn is_meta(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() == 3
        && parts[0].eq_ignore_ascii_case("Mods")
        && parts[2].eq_ignore_ascii_case("meta.lsx")
}

/// The module a `meta.lsx` describes (its `ModuleInfo` node), if it names one.
pub fn module_info(xml: &str) -> Option<Module> {
    let doc = Document::parse(xml.trim_start_matches('\u{feff}')).ok()?;
    let info = doc
        .descendants()
        .find(|n| n.has_tag_name("node") && n.attribute("id") == Some("ModuleInfo"))?;
    let get = |id: &str| attribute(info, id).map(str::trim).filter(|v| !v.is_empty());
    let uuid = get("UUID")?.to_string();
    let folder = get("Folder")?.to_string();
    Some(Module {
        name: get("Name").unwrap_or(&folder).to_string(),
        md5: get("MD5").unwrap_or_default().to_string(),
        version64: get("Version64")
            .or_else(|| get("Version"))
            .unwrap_or(VERSION_1)
            .to_string(),
        publish_handle: get("PublishHandle").unwrap_or("0").to_string(),
        uuid,
        folder,
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The module's entry in `modsettings.lsx`, as the game writes them.
pub fn short_desc(m: &Module) -> String {
    let a = |id: &str, ty: &str, v: &str| {
        format!(
            "<attribute id=\"{id}\" type=\"{ty}\" value=\"{}\"/>",
            escape(v)
        )
    };
    format!(
        "<node id=\"ModuleShortDesc\">{}{}{}{}{}{}</node>",
        a("Folder", "LSString", &m.folder),
        a("MD5", "LSString", &m.md5),
        a("Name", "LSString", &m.name),
        a("PublishHandle", "uint64", &m.publish_handle),
        a("UUID", "guid", &m.uuid),
        a("Version64", "int64", &m.version64),
    )
}

impl Session {
    /// For a Larian game: the enabled mods' modules, read from their packages in the user
    /// folder's `Mods/`, added to the game's list of enabled modules.
    pub(crate) fn plan_larian(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        let Some(l) = &def.def.larian else {
            return Ok(());
        };
        let Some(user) = plan.folders.get(&l.folder).cloned() else {
            return Ok(());
        };
        let prefix = format!("@{}/mods/", l.folder).to_ascii_lowercase();
        let mut entries = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for id in profile.enabled_ids() {
            for f in plan.files.iter().filter(|f| f.owner == id) {
                let t = f.target.as_str().to_ascii_lowercase();
                // The game reads packages at the top of `Mods/` only.
                let Some(name) = t.strip_prefix(&prefix) else {
                    continue;
                };
                if name.contains('/') || !name.ends_with(".pak") {
                    continue;
                }
                let meta = crate::pak::read_matching(&f.source, is_meta)
                    .map_err(|e| CoreError::Invalid(format!("reading {}: {e}", f.target)))?;
                // No module: it changes the game's files, and loads without being listed.
                let Some(m) = meta.and_then(|(_, b)| module_info(&String::from_utf8_lossy(&b)))
                else {
                    continue;
                };
                let uuid = m.uuid.to_ascii_lowercase();
                if BASE_MODULES.contains(&uuid.as_str()) || seen.contains(&uuid) {
                    continue;
                }
                seen.push(uuid);
                entries.push(short_desc(&m));
            }
        }
        if entries.is_empty() {
            return Ok(());
        }
        let path = user.join(MODSETTINGS);
        let text = match fs::read(&path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::Invalid(format!(
                    "{} has not written {} yet: start the game once, then deploy again",
                    def.def.name,
                    path.display()
                )))
            }
            Err(e) => {
                return Err(CoreError::Invalid(format!(
                    "reading {}: {e}",
                    path.display()
                )))
            }
        };
        if ListFormat::Lsx.get(&text, MODS).is_none() {
            return Err(CoreError::Invalid(format!(
                "{} has no list of mods (<node id=\"Mods\">) to add to",
                path.display()
            )));
        }
        plan.lists.push(PlannedList {
            path,
            key: MODS.into(),
            entries,
            format: ListFormat::Lsx,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_module_and_writes_its_entry() {
        let meta = r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <version major="4" minor="0" revision="9" build="331"/>
    <region id="Config">
        <node id="root">
            <children>
                <node id="Dependencies"/>
                <node id="ModuleInfo">
                    <attribute id="Author" type="LSString" value="Me"/>
                    <attribute id="Folder" type="LSString" value="BetterBags"/>
                    <attribute id="Name" type="LSString" value="Better &amp; Bags"/>
                    <attribute id="UUID" type="FixedString" value="1a2b3c4d-0000-0000-0000-000000000001"/>
                    <attribute id="Version64" type="int64" value="36028797018963969"/>
                    <children>
                        <node id="PublishVersion">
                            <attribute id="Version64" type="int64" value="1"/>
                        </node>
                    </children>
                </node>
            </children>
        </node>
    </region>
</save>"#;
        let m = module_info(meta).unwrap();
        assert_eq!(
            (m.folder.as_str(), m.name.as_str(), m.version64.as_str()),
            ("BetterBags", "Better & Bags", "36028797018963969")
        );
        assert_eq!((m.md5.as_str(), m.publish_handle.as_str()), ("", "0"));
        let e = short_desc(&m);
        assert!(e.contains(r#"<attribute id="Name" type="LSString" value="Better &amp; Bags"/>"#));
        assert_eq!(
            umm_deploy::lsx::id(&e),
            "1a2b3c4d-0000-0000-0000-000000000001"
        );
        assert!(is_meta("Mods/BetterBags/meta.lsx"));
        assert!(!is_meta("Mods/BetterBags/Story/meta.lsx"));
        assert_eq!(module_info("<save/>"), None);
    }
}
