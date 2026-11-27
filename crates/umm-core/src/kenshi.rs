//! Kenshi: each mod is a folder in the game folder's `mods/` holding its `<Name>.mod`, and
//! the game finds it only in a folder of that name. It loads the mods `data/mods.cfg` names
//! (`<Name>.mod`, one a line, in load order), Steam Workshop items included. UMM puts each
//! mod in a folder named after its `.mod`, and lists it while deployed (see
//! [`crate::plugins`]).

use crate::{CoreError, Result, Session};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use umm_deploy::Plan;
use umm_gamedef::{CompiledDef, PluginListFormat, RelPath};

/// A mod's `.mod` file directly in its folder in the mods folder (`prefix`, ending in `/`):
/// the folder and the file's name, as spelled.
pub fn mod_file<'a>(target: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let rest = target.get(prefix.len()..).filter(|_| {
        target
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    })?;
    let (dir, file) = rest.split_once('/')?;
    (!dir.is_empty() && !file.contains('/') && is_mod(file)).then_some((dir, file))
}

fn is_mod(name: &str) -> bool {
    name.len() > 4 && name.to_ascii_lowercase().ends_with(".mod")
}

/// The `.mod` file at the top of a mod's folder (a Workshop item's), by name; the first by
/// name when there are several.
pub fn mod_in(folder: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_mod(n))
        .collect();
    names.sort();
    names.into_iter().next()
}

impl Session {
    /// For Kenshi: each mod's folder in the mods folder named after the `.mod` file in it.
    pub(crate) fn plan_kenshi(&self, def: &CompiledDef, plan: &mut Plan) -> Result<()> {
        let Some(cfg) = def
            .def
            .plugins
            .as_ref()
            .filter(|p| p.format == PluginListFormat::KenshiCfg)
        else {
            return Ok(());
        };
        let prefix = format!("{}/", cfg.folder.trim_matches('/'));
        // (owner, folder) → the mod's name; name → owner.
        let mut names: HashMap<(String, String), String> = HashMap::new();
        let mut owners: HashMap<String, String> = HashMap::new();
        for f in &plan.files {
            let Some((dir, file)) = mod_file(f.target.as_str(), &prefix) else {
                continue;
            };
            let key = (f.owner.clone(), dir.to_lowercase());
            if names.contains_key(&key) {
                continue;
            }
            let name = file[..file.len() - 4].to_string();
            if let Some(other) = owners.insert(name.to_lowercase(), f.owner.clone()) {
                if other != f.owner {
                    return Err(CoreError::Invalid(format!(
                        "mods `{other}` and `{}` are both the Kenshi mod `{name}`: disable one",
                        f.owner
                    )));
                }
            }
            names.insert(key, name);
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
    fn finds_a_mods_file_in_its_folder() {
        assert_eq!(
            mod_file("mods/dark-ui/Dark UI.mod", "mods/"),
            Some(("dark-ui", "Dark UI.mod"))
        );
        assert_eq!(mod_file("Mods/x/X.MOD", "mods/"), Some(("x", "X.MOD")));
        assert_eq!(mod_file("mods/x/sub/X.mod", "mods/"), None);
        assert_eq!(mod_file("mods/X.mod", "mods/"), None);
        assert_eq!(mod_file("mods/x/readme.txt", "mods/"), None);
        assert_eq!(mod_file("data/x/X.mod", "mods/"), None);
    }
}
