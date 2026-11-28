//! Games whose mods are each a folder in a folder of mods, named after a file of the mod's in
//! it, which the game finds only in a folder of that name, and enabled by that name in a list
//! (see [`crate::games::plugins`]), Steam Workshop items included:
//!
//! - Kenshi: `mods/<Name>/<Name>.mod`, listed in `data/mods.cfg` (`<Name>.mod`, one a line,
//!   in load order).
//! - XCOM 2: `XComGame/Mods/<Name>/<Name>.XComMod` (War of the Chosen's in
//!   `XCom2-WarOfTheChosen/`), listed in the user's `XComModOptions.ini`
//!   (`ActiveMods="<Name>"`).
//!
//! UMM puts each mod in a folder named after its file, and lists it while deployed.

use crate::{CoreError, Result, Session};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use umm_deploy::Plan;
use umm_gamedef::{CompiledDef, PluginListFormat, RelPath};

/// The extension of the file naming a mod, lower-case, for the games that have one.
pub fn extension(format: PluginListFormat) -> Option<&'static str> {
    match format {
        PluginListFormat::KenshiCfg => Some(".mod"),
        PluginListFormat::XcomIni => Some(".xcommod"),
        _ => None,
    }
}

/// A mod's file (with extension `ext`) directly in its folder in the mods folder (`prefix`,
/// ending in `/`): the folder and the file's name, as spelled.
pub fn mod_file<'a>(target: &'a str, prefix: &str, ext: &str) -> Option<(&'a str, &'a str)> {
    let rest = target.get(prefix.len()..).filter(|_| {
        target
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    })?;
    let (dir, file) = rest.split_once('/')?;
    (!dir.is_empty() && !file.contains('/') && is_mod(file, ext)).then_some((dir, file))
}

fn is_mod(name: &str, ext: &str) -> bool {
    name.len() > ext.len() && name.to_ascii_lowercase().ends_with(ext)
}

/// The mod's name, from its file's.
pub fn name_of<'a>(file: &'a str, ext: &str) -> &'a str {
    &file[..file.len() - ext.len()]
}

/// The mod's file at the top of its folder (a Workshop item's), by name; the first by name
/// when there are several.
pub fn mod_in(folder: &Path, ext: &str) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_mod(n, ext))
        .collect();
    names.sort();
    names.into_iter().next()
}

impl Session {
    /// For Kenshi and XCOM 2: each mod's folder in the mods folder named after its file.
    pub(crate) fn plan_mod_files(&self, def: &CompiledDef, plan: &mut Plan) -> Result<()> {
        let Some((cfg, ext)) = def
            .def
            .plugins
            .as_ref()
            .and_then(|p| Some((p, extension(p.format)?)))
        else {
            return Ok(());
        };
        let prefix = format!("{}/", cfg.folder.trim_matches('/'));
        let mut names: HashMap<(String, String), String> = HashMap::new();
        for f in &plan.files {
            if let Some((dir, file)) = mod_file(f.target.as_str(), &prefix, ext) {
                names
                    .entry((f.owner.clone(), dir.to_lowercase()))
                    .or_insert_with(|| name_of(file, ext).to_string());
            }
        }
        let what = match cfg.format {
            PluginListFormat::XcomIni => "XCOM 2 mod",
            _ => "Kenshi mod",
        };
        rename_folders(plan, &prefix, &names, what)
    }
}

/// Renames mods' folders in a folder of mods (`prefix`, ending in `/`): each `(owner,
/// folder)` in `names` (the folder lower-case) to its name there. Refused when two mods would
/// have the same folder (`what` says what they are to the game).
pub(crate) fn rename_folders(
    plan: &mut Plan,
    prefix: &str,
    names: &HashMap<(String, String), String>,
    what: &str,
) -> Result<()> {
    let mut owners: HashMap<String, &str> = HashMap::new();
    for ((owner, _), name) in names {
        if let Some(other) = owners.insert(name.to_lowercase(), owner) {
            if other != owner {
                let (a, b) = if other < owner.as_str() {
                    (other, owner.as_str())
                } else {
                    (owner.as_str(), other)
                };
                return Err(CoreError::Invalid(format!(
                    "mods `{a}` and `{b}` are both the {what} `{name}`: disable one"
                )));
            }
        }
    }
    for f in &mut plan.files {
        let t = f.target.as_str();
        let Some(rest) = t
            .get(..prefix.len())
            .filter(|p| p.eq_ignore_ascii_case(prefix))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_mods_file_in_its_folder() {
        assert_eq!(
            mod_file("mods/dark-ui/Dark UI.mod", "mods/", ".mod"),
            Some(("dark-ui", "Dark UI.mod"))
        );
        assert_eq!(
            mod_file("Mods/x/X.MOD", "mods/", ".mod"),
            Some(("x", "X.MOD"))
        );
        assert_eq!(mod_file("mods/x/sub/X.mod", "mods/", ".mod"), None);
        assert_eq!(mod_file("mods/X.mod", "mods/", ".mod"), None);
        assert_eq!(mod_file("mods/x/readme.txt", "mods/", ".mod"), None);
        assert_eq!(mod_file("data/x/X.mod", "mods/", ".mod"), None);
    }
}
