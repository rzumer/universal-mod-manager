//! Larian games (Baldur's Gate 3, Divinity: Original Sin 2): mods are `.pak` packages in the
//! game's user folder's `Mods/` (`%LOCALAPPDATA%\Larian Studios\Baldur's Gate 3\Mods`), not
//! the game folder. A package with a module of its own (`Mods/<Folder>/meta.lsx` inside it)
//! loads only when the game's `modsettings.lsx` lists that module; one without (an override
//! of the game's files) loads as it is.
//!
//! Baldur's Gate 3 has one list, `PlayerProfiles/Public/modsettings.lsx`'s `Mods`. Divinity:
//! Original Sin 2 has one per player profile (`PlayerProfiles/<profile>/modsettings.lsx`),
//! each with the modules in load order (`ModOrder`) as well as their descriptions (`Mods`).
//!
//! While mods are deployed, UMM adds each enabled mod's modules to those lists, after the
//! game's own and the user's, and takes them out again afterwards: the rest of the file is
//! the game's, which it rewrites as it likes.

use crate::{CoreError, Result, Session};
use roxmltree::Document;
use std::fs;
use umm_deploy::lsx::attribute;
use umm_deploy::{ListFormat, Plan, PlannedList};
use umm_gamedef::{CompiledDef, LarianFormat};
use umm_store::Profile;

/// The game's list of enabled modules, in its user folder.
pub const MODSETTINGS: &str = "PlayerProfiles/Public/modsettings.lsx";
/// Divinity: Original Sin 2's player profiles, each with its `modsettings.lsx`.
const PROFILES: &str = "PlayerProfiles";
/// The list's node in it.
const MODS: &str = "Mods";
/// Divinity: Original Sin 2's load order, the other list there.
const MOD_ORDER: &str = "ModOrder";
/// `Version` for 1.0.0.0 in Divinity: Original Sin 2, for a module that gives none.
const VERSION_1_DOS2: &str = "268435456";
/// `Version64` for 1.0.0.0, for a module that gives none.
const VERSION_1: &str = "36028797018963968";
/// The game's own modules (GustavX, GustavDev, Gustav), which a mod's package may hold a copy
/// of to change, but which are not the mod's to list.
/// Divinity: Original Sin 2's are Divinity Origins and Shared.
const BASE_MODULES: &[&str] = &[
    "cb555efe-2d9e-131f-8195-a89329d218ea",
    "28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8",
    "991c9c7a-fb80-40cb-8f0d-b92d4e80e9b1",
    "1301db3d-1f54-4e98-9be5-5094030916e4",
    "2bd9bdbe-22ae-4aa2-9c93-205880fc6564",
];

/// A module, as its `meta.lsx` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub uuid: String,
    pub folder: String,
    pub name: String,
    pub md5: String,
    pub version64: String,
    /// Its `Version` (Divinity: Original Sin 2's), when it gives one.
    pub version: Option<String>,
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
        version: get("Version").map(str::to_string),
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

/// The module's entries in a Divinity: Original Sin 2 `modsettings.lsx`, as the game writes
/// them: in `ModOrder`, and in `Mods`. Attribute types are numbers there (22: `FixedString`,
/// 23: `LSString`, 30: `LSWString`, 4: `int32`).
pub fn dos2_entries(m: &Module) -> (String, String) {
    let a = |id: &str, ty: u8, v: &str| {
        format!(
            "<attribute id=\"{id}\" value=\"{}\" type=\"{ty}\" />",
            escape(v)
        )
    };
    (
        format!("<node id=\"Module\">{}</node>", a("UUID", 22, &m.uuid)),
        format!(
            "<node id=\"ModuleShortDesc\">{}{}{}{}{}</node>",
            a("Folder", 30, &m.folder),
            a("MD5", 23, &m.md5),
            a("Name", 22, &m.name),
            a("UUID", 22, &m.uuid),
            a("Version", 4, m.version.as_deref().unwrap_or(VERSION_1_DOS2)),
        ),
    )
}

/// The `modsettings.lsx` files UMM adds modules to, in the user folder: Baldur's Gate 3's
/// one, or each Divinity: Original Sin 2 player profile's.
fn modsettings_files(user: &std::path::Path, format: LarianFormat) -> Vec<std::path::PathBuf> {
    match format {
        LarianFormat::Bg3 => vec![user.join(MODSETTINGS)],
        LarianFormat::Dos2 => {
            let mut files: Vec<_> = fs::read_dir(user.join(PROFILES))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join("modsettings.lsx"))
                .filter(|f| f.is_file())
                .collect();
            files.sort();
            files
        }
    }
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
        let mut modules = Vec::new();
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
                let meta = crate::games::pak::read_matching(&f.source, is_meta)
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
                modules.push(m);
            }
        }
        if modules.is_empty() {
            return Ok(());
        }
        let files = modsettings_files(&user, l.format);
        if files.is_empty() {
            return Err(CoreError::Invalid(format!(
                "{} has not written its player profiles' modsettings.lsx in {} yet: start the \
                 game once, then deploy again",
                def.def.name,
                user.join(PROFILES).display()
            )));
        }
        let lists: Vec<(&str, Vec<String>)> = match l.format {
            LarianFormat::Bg3 => vec![(MODS, modules.iter().map(short_desc).collect())],
            LarianFormat::Dos2 => {
                let (order, mods) = modules.iter().map(dos2_entries).unzip();
                vec![(MOD_ORDER, order), (MODS, mods)]
            }
        };
        for path in files {
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
            for (key, entries) in &lists {
                if ListFormat::Lsx.get(&text, key).is_none() {
                    return Err(CoreError::Invalid(format!(
                        "{} has no list of mods (<node id=\"{key}\">) to add to",
                        path.display()
                    )));
                }
                plan.lists.push(PlannedList {
                    path: path.clone(),
                    key: (*key).into(),
                    entries: entries.clone(),
                    format: ListFormat::Lsx,
                });
            }
        }
        Ok(())
    }
}

pub(crate) struct Larian;

impl crate::games::GameIntegration for Larian {
    fn plan(
        &self,
        s: &Session,
        def: &CompiledDef,
        profile: &umm_store::Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        s.plan_larian(def, profile, plan)
    }
}

/// A Larian game: its exe (relative to the game folder), its user folder, with variables, and
/// how it lists its modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LarianGame {
    pub exe: &'static str,
    pub user: &'static str,
    pub format: LarianFormat,
}

/// Larian games whose mods go in their user folder.
pub const LARIAN_GAMES: &[LarianGame] = &[
    LarianGame {
        // Started directly, past Larian's launcher (`Launcher/LariLauncher.exe`).
        exe: "bin/bg3.exe",
        user: "%LOCALAPPDATA%/Larian Studios/Baldur's Gate 3",
        format: LarianFormat::Bg3,
    },
    LarianGame {
        // Definitive Edition, started directly, past its launcher.
        exe: "DefEd/bin/EoCApp.exe",
        user: "%DOCUMENTS%/Larian Studios/Divinity Original Sin 2 Definitive Edition",
        format: LarianFormat::Dos2,
    },
];

/// The Larian game installed in `root`, by its exe.
pub fn larian_game(root: &std::path::Path) -> Option<&'static LarianGame> {
    LARIAN_GAMES
        .iter()
        .find(|g| umm_gamedef::kind::has_file_ci(root, g.exe))
}

/// Install rules for a Larian game: `.pak` files to the user folder's `Mods/` (named
/// `folder` in the definition's folders), where the game finds them only at its top, and the
/// game folder's own folders (`bin/` for the Script Extender) where they are. Other files
/// (`info.json`, readmes) are left out.
pub fn larian_rules(folder: &str, format: LarianFormat) -> Vec<umm_gamedef::InstallRule> {
    use umm_gamedef::kind::rule;
    let (bin, data) = (format.bin(), format.data());
    let mut rules = vec![
        rule("*.pak", &format!("@{folder}/Mods/")),
        rule(&format!("{bin}/**"), &format!("{bin}/")),
        rule(&format!("{data}/**"), &format!("{data}/")),
    ];
    if format == LarianFormat::Dos2 {
        // Laid out from `DefEd/`, as many are.
        rules.push(rule("bin/**", &format!("{bin}/")));
        rules.push(rule("Data/**", &format!("{data}/")));
    }
    // The Script Extender's loader.
    rules.push(rule(format.extender(), &format!("{bin}/")));
    rules
}

impl umm_gamedef::GameKind for Larian {
    fn key(&self) -> &'static str {
        "larian"
    }

    /// By its exe in `bin/` (the top holds its launcher's folder).
    fn recognise(
        &self,
        root: &std::path::Path,
        _exes: &[String],
    ) -> Option<umm_gamedef::Recognised> {
        let g = larian_game(root)?;
        Some(umm_gamedef::Recognised {
            key: "larian",
            label: Some("Divinity Engine (Larian)"),
            exe: Some(g.exe.to_string()),
            own_mods: Some(umm_gamedef::BUILT_IN_MODS),
        })
    }

    fn shape(
        &self,
        root: &std::path::Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let Some(g) = larian_game(root) else {
            return umm_gamedef::Shape::default();
        };
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "larian",
                format!(
                    "Larian game: .pak mods go to the Mods folder in {}, and are enabled in \
                     {} while mods are deployed (your own mods there stay). The game must \
                     have been started once, so that it has written that file. Script \
                     Extender and {}/ or {}/ files go to the game folder. UMM starts the \
                     game directly, not through Larian's launcher.",
                    g.user.replace('/', "\\"),
                    match g.format {
                        LarianFormat::Bg3 => "modsettings.lsx",
                        LarianFormat::Dos2 => "each player profile's modsettings.lsx",
                    },
                    g.format.bin(),
                    g.format.data(),
                ),
            )],
            user_folder: Some(g.user.to_string()),
            install: larian_rules(umm_gamedef::steam::USER_FOLDER, g.format),
            larian: Some(umm_gamedef::Larian {
                folder: umm_gamedef::steam::USER_FOLDER.into(),
                format: g.format,
            }),
            // Larian's launcher stands before the game.
            direct: true,
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.larian.is_some()
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        def.larian.as_ref().map_or_else(
            || vec![umm_gamedef::kind::rule("**", "")],
            |l| larian_rules(&l.folder, l.format),
        )
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
