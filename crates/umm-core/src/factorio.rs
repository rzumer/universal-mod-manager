//! Factorio: each mod is a folder in the game's mods folder, named after the `name` in its
//! `info.json` (the game refuses a folder named otherwise), and enabled in `mod-list.json`
//! there. UMM puts each mod in a folder of that name, and lists it enabled while deployed
//! (see [`crate::plugins`]).

use crate::plugins::PluginEntry;
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
