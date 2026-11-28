//! Factorio: each mod is a folder in the game's mods folder, named after the `name` in its
//! `info.json` (the game refuses a folder named otherwise), and enabled in `mod-list.json`
//! there. UMM puts each mod in a folder of that name, and lists it enabled while deployed
//! (see [`crate::games::plugins`]).

use crate::games::plugins::PluginEntry;
use crate::{CoreError, Result, Session};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use umm_deploy::Plan;
use umm_gamedef::{CompiledDef, PluginListFormat, RelPath};

/// The file in a mod's folder naming it.
pub const INFO: &str = "info.json";

/// A mod's name, from its `info.json`; `None` when it has none fit for a folder name.
pub fn mod_name(info: &Path) -> Option<String> {
    let v: Value = serde_json::from_slice(&fs::read(info).ok()?).ok()?;
    let name = v.get("name")?.as_str()?.trim();
    let fit = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);
    fit.then(|| name.to_string())
}

/// The folder in the mods folder (`prefix`, lower-cased, ending in `/`) whose `info.json`
/// `target` is.
pub fn info_folder<'a>(target: &'a RelPath, prefix: &str) -> Option<&'a str> {
    let t = target.as_str();
    let rest = t.get(prefix.len()..).filter(|_| {
        t.get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    })?;
    let dir = rest
        .len()
        .checked_sub(INFO.len() + 1)
        .filter(|&i| rest[i..].eq_ignore_ascii_case(&format!("/{INFO}")))
        .map(|i| &rest[..i])?;
    (!dir.is_empty() && !dir.contains('/')).then_some(dir)
}

/// The list's objects.
fn objects(text: &str) -> Vec<Map<String, Value>> {
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    match root.remove("mods") {
        Some(Value::Array(mods)) => mods
            .into_iter()
            .filter_map(|m| match m {
                Value::Object(o) if o.get("name").is_some_and(Value::is_string) => Some(o),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The list's entries.
pub fn read(text: &str) -> Vec<PluginEntry> {
    objects(text)
        .into_iter()
        .map(|o| PluginEntry {
            name: o["name"].as_str().unwrap_or_default().to_string(),
            enabled: o.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            owner: None,
        })
        .collect()
}

/// The list with these entries, as the game writes it. What else the user's entries have
/// (a `version` the game is held to) is kept.
pub fn render(original: &str, entries: &[PluginEntry]) -> String {
    let theirs: HashMap<String, Map<String, Value>> = objects(original)
        .into_iter()
        .map(|o| (o["name"].as_str().unwrap_or_default().to_string(), o))
        .collect();
    let mut out = String::from("\n{\n  \"mods\": \n  [\n");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        let mut fields = vec![
            ("name".to_string(), Value::String(e.name.clone())),
            ("enabled".to_string(), Value::Bool(e.enabled)),
        ];
        if let Some(o) = theirs.get(&e.name).filter(|_| e.owner.is_none()) {
            fields.extend(
                o.iter()
                    .filter(|(k, _)| *k != "name" && *k != "enabled")
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        out.push_str("    \n    {\n");
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("      {}: {v}", Value::String(k.clone())))
            .collect();
        out.push_str(&body.join(",\n"));
        out.push_str("\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}

impl Session {
    /// For Factorio: each mod's folder named after its `info.json`, rather than the mod's id
    /// in UMM. Two mods of the same name cannot both be deployed.
    pub(crate) fn plan_factorio(&self, def: &CompiledDef, plan: &mut Plan) -> Result<()> {
        let Some(cfg) = def
            .def
            .plugins
            .as_ref()
            .filter(|p| p.format == PluginListFormat::FactorioJson)
        else {
            return Ok(());
        };
        let prefix = format!("{}/", cfg.folder.trim_matches('/'));
        // (owner, folder) → the mod's name; name → owner.
        let mut names: HashMap<(String, String), String> = HashMap::new();
        let mut owners: HashMap<String, String> = HashMap::new();
        for f in &plan.files {
            let Some(dir) = info_folder(&f.target, &prefix) else {
                continue;
            };
            let Some(name) = mod_name(&f.source) else {
                continue;
            };
            if let Some(other) = owners.insert(name.to_lowercase(), f.owner.clone()) {
                if other != f.owner {
                    return Err(CoreError::Invalid(format!(
                        "mods `{other}` and `{}` are both the Factorio mod `{name}`: disable one",
                        f.owner
                    )));
                }
            }
            names.insert((f.owner.clone(), dir.to_lowercase()), name);
        }
        for f in &mut plan.files {
            let t = f.target.as_str();
            let Some(rest) = t
                .get(..prefix.len())
                .filter(|p| p.eq_ignore_ascii_case(&prefix))
                .map(|_| &t[prefix.len()..])
            else {
                continue;
            };
            let (dir, inner) = rest.split_once('/').unwrap_or((rest, ""));
            let Some(name) = names.get(&(f.owner.clone(), dir.to_lowercase())) else {
                continue;
            };
            if dir != name {
                let renamed = match inner {
                    "" => format!("{}{name}", &t[..prefix.len()]),
                    _ => format!("{}{name}/{inner}", &t[..prefix.len()]),
                };
                f.target = RelPath::parse(&renamed)?;
            }
        }
        plan.files.sort_by_key(|f| f.target.key());
        Ok(())
    }
}

pub(crate) struct Factorio;

impl crate::games::GameIntegration for Factorio {
    fn mod_root(
        &self,
        rels: &[RelPath],
        _def: &CompiledDef,
    ) -> Option<Option<umm_store::Placement>> {
        Some(mod_root(rels))
    }

    /// Factorio shows a mod's changelog in its mod manager.
    fn kept_docs(&self) -> &'static [&'static str] {
        &["changelog.txt"]
    }

    fn plan(
        &self,
        s: &Session,
        def: &CompiledDef,
        _profile: &umm_store::Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        s.plan_factorio(def, plan)
    }
}

impl crate::games::plugins::ModList for Factorio {
    fn kind(&self) -> &'static str {
        "mods"
    }

    fn read(&self, text: &str) -> Vec<PluginEntry> {
        read(text)
    }

    fn render(&self, original: &str, entries: &[PluginEntry]) -> String {
        render(original, entries)
    }

    /// A folder each, named by its `info.json`.
    fn entries<'a>(
        &self,
        files: &'a [umm_deploy::PlannedFile],
        folder: &RelPath,
    ) -> Vec<(&'a str, String)> {
        let prefix = format!("{}/", folder.as_str());
        files
            .iter()
            .filter(|f| info_folder(&f.target, &prefix).is_some())
            .filter_map(|f| Some((f.owner.as_str(), mod_name(&f.source)?)))
            .collect()
    }
}

/// Factorio's exe, relative to its folder.
pub const FACTORIO_EXE: &str = "bin/x64/factorio.exe";

/// Factorio's user folder when it uses the system's (`config-path.cfg` says so: Steam's and
/// the installer's copies). A zip copy keeps everything in its own folder instead.
const FACTORIO_USER: &str = "%APPDATA%/Factorio";

/// Whether the Factorio copy in `root` keeps its mods in its own folder (`config-path.cfg`
/// with `use-system-read-write-data-directories=false`, or none).
fn factorio_portable(root: &Path) -> bool {
    let Ok(text) = fs::read_to_string(root.join("config-path.cfg")) else {
        return true;
    };
    !text.lines().any(|l| {
        l.split_once('=').is_some_and(|(k, v)| {
            k.trim() == "use-system-read-write-data-directories" && v.trim() == "true"
        })
    })
}

impl umm_gamedef::GameKind for Factorio {
    fn key(&self) -> &'static str {
        "factorio"
    }

    /// By its exe in `bin/x64/` and its base mod in `data/`.
    fn recognise(&self, root: &Path, _exes: &[String]) -> Option<umm_gamedef::Recognised> {
        (root.join(FACTORIO_EXE).is_file() && root.join("data/base/info.json").is_file()).then(
            || umm_gamedef::Recognised {
                key: "factorio",
                label: Some("Factorio (Wube)"),
                exe: Some(FACTORIO_EXE.to_string()),
                own_mods: Some(umm_gamedef::BUILT_IN_MODS),
            },
        )
    }

    /// Its mods folder is in its user folder, or its own when it keeps everything there.
    fn shape(
        &self,
        root: &Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let (user, mods, list) = if factorio_portable(root) {
            (None, "mods".to_string(), "mods/mod-list.json".to_string())
        } else {
            (
                Some(FACTORIO_USER.to_string()),
                format!("@{}/mods", umm_gamedef::steam::USER_FOLDER),
                format!("{FACTORIO_USER}/mods/mod-list.json"),
            )
        };
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "factorio",
                "Factorio: each mod goes in its own folder in the game's mods folder, named after \
                 the name in its info.json, and is enabled in mod-list.json while mods are deployed \
                 (your own list comes back afterwards). Import mods as the .zip files from the mod \
                 portal.",
            )],
            user_folder: user,
            install: umm_gamedef::kind::mod_folder_rules(&mods),
            plugins: Some(umm_gamedef::Plugins {
                list,
                format: PluginListFormat::FactorioJson,
                folder: mods,
                in_game_folder_when: None,
            }),
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == PluginListFormat::FactorioJson)
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        umm_gamedef::kind::mod_folder_rules(def.plugins.as_ref().map_or("", |p| p.folder.as_str()))
    }
}

/// A Factorio mod's folder is the one holding its `info.json` (the shallowest).
fn mod_root(rels: &[RelPath]) -> Option<umm_store::Placement> {
    crate::placement::marked_root(rels, |n| n.eq_ignore_ascii_case(INFO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_the_list_as_the_game_does() {
        let original = "\n{\n  \"mods\": \n  [\n    \n    {\n      \"name\": \"base\",\n      \"enabled\": true\n    },\n    \n    {\n      \"name\": \"held\",\n      \"enabled\": false,\n      \"version\": \"1.0.2\"\n    }\n  ]\n}\n";
        let e = read(original);
        assert_eq!(
            e.iter()
                .map(|e| (e.name.as_str(), e.enabled))
                .collect::<Vec<_>>(),
            [("base", true), ("held", false)]
        );
        assert_eq!(render(original, &e), original, "written back as it was");
        let mut more = e.clone();
        more.push(PluginEntry {
            name: "Krastorio".into(),
            enabled: true,
            owner: Some("k".into()),
        });
        assert!(render(original, &more).ends_with(
            "\"version\": \"1.0.2\"\n    },\n    \n    {\n      \"name\": \"Krastorio\",\n      \"enabled\": true\n    }\n  ]\n}\n"
        ));
        assert!(read("not json").is_empty());
        let target = RelPath::parse("@user/mods/k/info.json").unwrap();
        assert_eq!(info_folder(&target, "@user/mods/"), Some("k"));
        let deeper = RelPath::parse("@user/mods/k/x/info.json").unwrap();
        assert_eq!(info_folder(&deeper, "@user/mods/"), None);
    }
}
