//! Paradox games (Crusader Kings III, Stellaris, Hearts of Iron IV…): their mods live in the
//! game's user folder (`Documents\Paradox Interactive\<game>\mod`), not the game folder.
//! Each is a folder found through a `.mod` descriptor beside it, whose `path` names the
//! folder, and the game loads those `dlc_load.json` lists under `enabled_mods`.
//!
//! The install rules put each mod in its own folder there (`@<folder>/mod/<mod>/`). While
//! mods are deployed, UMM writes a descriptor for each (from the mod's own `descriptor.mod`,
//! its `path` set), and `dlc_load.json` with the user's own enabled mods followed by UMM's;
//! the user's file comes back afterwards. The game is started directly, not through Paradox's
//! launcher, which would write `dlc_load.json` from its own playset.

use crate::{CoreError, Result, Session};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use umm_deploy::{Plan, PlannedExternal, PlannedFile};
use umm_gamedef::{CompiledDef, RelPath};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::Profile;

/// A mod's own descriptor, at its folder's top.
pub const DESCRIPTOR: &str = "descriptor.mod";
/// The game's list of enabled mods (and disabled DLC), in its user folder.
const DLC_LOAD: &str = "dlc_load.json";
const GENERATED: &str = "paradox";

/// A descriptor's lines for UMM's copy: the mod's own, without where it was (`path`,
/// `archive`), and `path` set to where it is now. A mod without one is named after itself.
pub fn descriptor(own: Option<&str>, name: &str, path: &Path) -> String {
    let mut out = String::new();
    let mut named = false;
    if let Some(text) = own {
        for line in text.lines() {
            let key = line
                .trim_start()
                .split(['=', ' ', '\t'])
                .next()
                .unwrap_or_default();
            if key.eq_ignore_ascii_case("path") || key.eq_ignore_ascii_case("archive") {
                continue;
            }
            named |= key.eq_ignore_ascii_case("name");
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    if !named {
        out.insert_str(0, &format!("name=\"{}\"\n", name.replace('"', "'")));
    }
    // Paradox games read `/` in paths, on Windows too.
    let p = path.display().to_string().replace('\\', "/");
    out.push_str(&format!("path=\"{p}\"\n"));
    out
}

/// `dlc_load.json` with these descriptors (`mod/<name>.mod`) enabled after the user's own,
/// everything else in it kept.
pub fn dlc_load(original: Option<&str>, ours: &[String]) -> Result<String> {
    let mut v: serde_json::Value = match original.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => serde_json::from_str(t)
            .map_err(|e| CoreError::Invalid(format!("reading {DLC_LOAD}: {e}")))?,
        None => serde_json::json!({ "enabled_mods": [], "disabled_dlcs": [] }),
    };
    let obj = v
        .as_object_mut()
        .ok_or_else(|| CoreError::Invalid(format!("{DLC_LOAD} is not an object")))?;
    let mut enabled: Vec<serde_json::Value> = obj
        .get("enabled_mods")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    for m in ours {
        if !enabled.iter().any(|e| e.as_str() == Some(m)) {
            enabled.push(serde_json::Value::String(m.clone()));
        }
    }
    obj.insert("enabled_mods".into(), serde_json::Value::Array(enabled));
    obj.entry("disabled_dlcs")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    serde_json::to_string(&v).map_err(|e| CoreError::Invalid(e.to_string()))
}

impl Session {
    /// The game definition's folders outside the game folder, their variables filled in.
    pub(crate) fn resolve_folders(&self, def: &CompiledDef) -> Result<BTreeMap<String, PathBuf>> {
        def.def
            .folders
            .iter()
            .map(|(name, path)| {
                let p = umm_gamedef::expand_path(path).ok_or_else(|| {
                    CoreError::Invalid(format!(
                        "[folders] {name}: cannot find {path} (an absolute path, with its variables set)"
                    ))
                })?;
                Ok((name.clone(), p))
            })
            .collect()
    }

    /// For a Paradox game: a descriptor for each mod in its user folder, and the game's list
    /// of enabled mods with them.
    pub(crate) fn plan_paradox(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        let Some(px) = &def.def.paradox else {
            return Ok(());
        };
        let Some(user) = plan.folders.get(&px.folder).cloned() else {
            return Ok(());
        };
        let game = self.data.game(&def.def.id);
        let out = game.generated_dir().join(GENERATED);
        if out.exists() {
            fs::remove_dir_all(&out).ctx(|| format!("removing {}", out.display()))?;
        }
        fs::create_dir_all(&out).ctx(|| format!("creating {}", out.display()))?;
        let prefix = format!("@{}/mod/", px.folder);
        let mut ours = Vec::new();
        for id in profile.enabled_ids() {
            let base = format!("{prefix}{id}/");
            if !plan
                .files
                .iter()
                .any(|f| f.target.as_str().starts_with(&base))
            {
                continue;
            }
            let own = plan
                .files
                .iter()
                .find(|f| {
                    f.target
                        .as_str()
                        .eq_ignore_ascii_case(&format!("{base}{DESCRIPTOR}"))
                })
                .and_then(|f| fs::read(&f.source).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            let name = umm_store::load_mod(&game, id)
                .map(|m| m.name)
                .unwrap_or_else(|_| id.to_string());
            let file = format!("umm-{id}.mod");
            let text = descriptor(own.as_deref(), &name, &user.join("mod").join(id));
            let src = out.join(&file);
            write_atomic(&src, text.as_bytes()).ctx(|| format!("writing {}", src.display()))?;
            let target = RelPath::parse(&format!("{prefix}{file}"))?;
            plan.files.retain(|f| f.target.key() != target.key());
            plan.files.push(PlannedFile {
                hash: hash_file(&src).ctx(|| format!("hashing {}", src.display()))?,
                size: text.len() as u64,
                source: src,
                owner: id.to_string(),
                writable: false,
                target,
            });
            ours.push(format!("mod/{file}"));
        }
        plan.files.sort_by_key(|f| f.target.key());
        if ours.is_empty() {
            return Ok(());
        }
        let list = user.join(DLC_LOAD);
        let original = fs::read(&list)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let text = dlc_load(original.as_deref(), &ours)?;
        let src = out.join(DLC_LOAD);
        write_atomic(&src, text.as_bytes()).ctx(|| format!("writing {}", src.display()))?;
        plan.external.push(PlannedExternal {
            hash: hash_file(&src).ctx(|| format!("hashing {}", src.display()))?,
            path: list,
            source: src,
            tolerated: Vec::new(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_descriptors_and_the_enabled_list() {
        let own = "version=\"1.2\"\ntags={\n\t\"Gameplay\"\n}\nname=\"Better Barons\"\nsupported_version=\"1.12.*\"\npath=\"C:/old/place\"\nremote_file_id=\"123\"\n";
        let d = descriptor(
            Some(own),
            "x",
            Path::new(
                "C:\\Users\\a\\Documents\\Paradox Interactive\\Crusader Kings III\\mod\\better",
            ),
        );
        assert!(d.contains("name=\"Better Barons\""));
        assert!(!d.contains("C:/old/place"));
        assert!(
            d.ends_with(
                "path=\"C:/Users/a/Documents/Paradox Interactive/Crusader Kings III/mod/better\"\n"
            ),
            "{d}"
        );
        assert!(descriptor(None, "Plain", Path::new("/m")).starts_with("name=\"Plain\"\n"));

        let mine = ["mod/umm-a.mod".to_string()];
        let j: serde_json::Value = serde_json::from_str(
            &dlc_load(
                Some(r#"{"enabled_mods":["mod/ugc_1.mod"],"disabled_dlcs":["dlc/x.dlc"]}"#),
                &mine,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            j["enabled_mods"],
            serde_json::json!(["mod/ugc_1.mod", "mod/umm-a.mod"])
        );
        assert_eq!(j["disabled_dlcs"], serde_json::json!(["dlc/x.dlc"]));
        let j: serde_json::Value = serde_json::from_str(&dlc_load(None, &mine).unwrap()).unwrap();
        assert_eq!(j["enabled_mods"], serde_json::json!(["mod/umm-a.mod"]));
    }
}
