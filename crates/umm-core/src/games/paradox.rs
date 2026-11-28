//! Paradox games (Crusader Kings III, Stellaris, Hearts of Iron IV…): their mods live in the
//! game's user folder (`Documents\Paradox Interactive\<game>\mod`), not the game folder.
//! Each is a folder found through a `.mod` descriptor beside it, whose `path` names the
//! folder, and the game loads those `dlc_load.json` lists under `enabled_mods` (Crusader
//! Kings II: those `settings.txt` lists under `last_mods`, whose entries UMM adds and takes
//! out again, the rest of that file being the game's settings).
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
use umm_deploy::{Plan, PlannedExternal, PlannedFile, PlannedList};
use umm_gamedef::{CompiledDef, ParadoxList, RelPath};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::{GameDir, Profile};

/// A mod's own descriptor, at its folder's top.
pub const DESCRIPTOR: &str = "descriptor.mod";
/// The game's list of enabled mods (and disabled DLC), in its user folder.
const DLC_LOAD: &str = "dlc_load.json";
/// Crusader Kings II's settings, whose `last_mods` lists its enabled mods.
const SETTINGS: &str = "settings.txt";
const LAST_MODS: &str = "last_mods";
const GENERATED: &str = "paradox";

/// A descriptor's lines for UMM's copy: the mod's own, without where it was (`path`,
/// `archive`) or braces closing nothing (which end the game's reading of it), and `path` set
/// to where it is now. A mod without one is named after itself.
pub fn descriptor(own: Option<&str>, name: &str, path: &str) -> String {
    let mut out = String::new();
    let mut named = false;
    let mut depth = 0usize;
    if let Some(text) = own {
        for line in text.lines() {
            let key = line
                .trim_start()
                .split(['=', ' ', '\t'])
                .next()
                .unwrap_or_default();
            if depth == 0
                && (key.eq_ignore_ascii_case("path") || key.eq_ignore_ascii_case("archive"))
            {
                continue;
            }
            let mut kept = String::new();
            let mut quoted = false;
            let t = line.trim_end();
            for (i, c) in t.char_indices() {
                match c {
                    '"' => quoted = !quoted,
                    '#' if !quoted => {
                        kept.push_str(&t[i..]);
                        break;
                    }
                    '{' if !quoted => depth += 1,
                    '}' if !quoted && depth == 0 => continue,
                    '}' if !quoted => depth -= 1,
                    _ => {}
                }
                kept.push(c);
            }
            if kept.trim().is_empty() && !line.trim().is_empty() {
                continue;
            }
            named |= depth == 0 && key.eq_ignore_ascii_case("name");
            out.push_str(&kept);
            out.push('\n');
        }
    }
    if !named {
        out.insert_str(0, &format!("name=\"{}\"\n", name.replace('"', "'")));
    }
    out.push_str(&format!("path=\"{path}\"\n"));
    out
}

/// The launcher's `<name>.mod` a mod's archive has beside its folder, the shallowest: the
/// descriptor of a mod without a `descriptor.mod` (Crusader Kings II's), read from the store.
fn launcher_descriptor(game: &GameDir, m: &umm_store::ModMeta) -> Option<String> {
    let file = m
        .files
        .keys()
        .filter(|k| {
            let n = k.rsplit('/').next().unwrap_or(k).to_ascii_lowercase();
            n.ends_with(".mod") && n != DESCRIPTOR
        })
        .min_by_key(|k| k.matches('/').count())?;
    let bytes = fs::read(game.mod_files_dir(&m.id).join(file)).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `dlc_load.json` with these descriptors (`mod/<name>.mod`) enabled after the user's own,
/// without those in `drop` (Workshop items a profile turns off, or enables through UMM's own
/// descriptor), everything else in it kept.
pub fn dlc_load(original: Option<&str>, ours: &[String], drop: &[String]) -> Result<String> {
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
    enabled.retain(|e| {
        !e.as_str()
            .is_some_and(|s| drop.iter().any(|d| d.eq_ignore_ascii_case(s)))
    });
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
        let vars = self.path_vars(&def.def);
        def.def
            .folders
            .iter()
            .map(|(name, path)| {
                let p = umm_gamedef::expand_path_with(path, &vars).ok_or_else(|| {
                    CoreError::Invalid(format!(
                        "[folders] {name}: cannot find {path} (an absolute path, with its variables set)"
                    ))
                })?;
                Ok((name.clone(), p))
            })
            .collect()
    }

    /// The game's arguments for its enabled mods, for a game that takes them so: Crusader
    /// Kings II, started past its launcher, loads only the mods given as
    /// `-mod=mod/<name>.mod`. Those are its `last_mods` (the user's and UMM's), which its
    /// launcher shows ticked, less the Workshop items the profile has off.
    pub fn paradox_mod_args(&self, def: &CompiledDef, profile: &Profile) -> Vec<String> {
        let Some(px) = def
            .def
            .paradox
            .as_ref()
            .filter(|p| p.list == ParadoxList::Settings)
        else {
            return Vec::new();
        };
        let Some(user) = self
            .resolve_folders(def)
            .ok()
            .and_then(|f| f.get(&px.folder).cloned())
        else {
            return Vec::new();
        };
        let Ok(bytes) = fs::read(user.join(SETTINGS)) else {
            return Vec::new();
        };
        let descriptors = self.workshop_descriptors(def);
        let off: Vec<&String> = profile
            .workshop()
            .filter(|(_, on)| !on)
            .filter_map(|(item, _)| descriptors.get(item))
            .collect();
        umm_deploy::block::get(&umm_deploy::block::decode(&bytes), LAST_MODS)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !off.iter().any(|o| o.eq_ignore_ascii_case(m)))
            .map(|m| format!("-mod={m}"))
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
            let meta = umm_store::load_mod(&game, id).ok();
            let own = plan
                .files
                .iter()
                .find(|f| {
                    f.target
                        .as_str()
                        .eq_ignore_ascii_case(&format!("{base}{DESCRIPTOR}"))
                })
                .and_then(|f| fs::read(&f.source).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .or_else(|| meta.as_ref().and_then(|m| launcher_descriptor(&game, m)));
            let name = meta.map(|m| m.name).unwrap_or_else(|| id.to_string());
            // Crusader Kings II's launcher cuts a descriptor's name at its first `-`.
            let file = match px.list {
                ParadoxList::DlcLoad => format!("umm-{id}.mod"),
                ParadoxList::Settings => format!("umm_{}.mod", id.replace('-', "_")),
            };
            let path = match px.list {
                // Paradox games read `/` in paths, on Windows too.
                ParadoxList::DlcLoad => user
                    .join("mod")
                    .join(id)
                    .display()
                    .to_string()
                    .replace('\\', "/"),
                ParadoxList::Settings => format!("mod/{id}"),
            };
            let text = descriptor(own.as_deref(), &name, &path);
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
            ours.push((id.to_string(), format!("mod/{file}")));
        }
        // Workshop items the profile has (games with `dlc_load.json`): a descriptor of UMM's
        // for each one on, its folder where Steam keeps it, and the launcher's own entry for
        // each (`mod/ugc_<id>.mod`) left out. Crusader Kings II, on Steam's older Workshop:
        // the descriptor the game wrote for each one on (those off it is not given, see
        // `paradox_mod_args`). Items the profile does not have stay as they are.
        let mut drop = Vec::new();
        if px.list == ParadoxList::Settings && profile.workshop().next().is_some() {
            let descriptors = self.workshop_descriptors(def);
            for (item, on) in profile.workshop() {
                if let Some(d) = descriptors.get(item).filter(|_| on) {
                    ours.push((format!("{}{item}", umm_store::WORKSHOP_PREFIX), d.clone()));
                }
            }
        }
        if px.list == ParadoxList::DlcLoad && profile.workshop().next().is_some() {
            let folders = self.workshop_folders(&def.def);
            for (item, on) in profile.workshop() {
                drop.push(format!("mod/ugc_{item}.mod"));
                let Some(folder) = folders.get(item).filter(|_| on) else {
                    continue;
                };
                let own = fs::read(folder.join(DESCRIPTOR))
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
                let path = folder.display().to_string().replace('\\', "/");
                let text = descriptor(own.as_deref(), &format!("Workshop item {item}"), &path);
                let file = format!("umm-workshop-{item}.mod");
                let src = out.join(&file);
                write_atomic(&src, text.as_bytes()).ctx(|| format!("writing {}", src.display()))?;
                let target = RelPath::parse(&format!("{prefix}{file}"))?;
                plan.files.retain(|f| f.target.key() != target.key());
                plan.files.push(PlannedFile {
                    hash: hash_file(&src).ctx(|| format!("hashing {}", src.display()))?,
                    size: text.len() as u64,
                    source: src,
                    owner: format!("{}{item}", umm_store::WORKSHOP_PREFIX),
                    writable: false,
                    target,
                });
                ours.push((
                    format!("{}{item}", umm_store::WORKSHOP_PREFIX),
                    format!("mod/{file}"),
                ));
            }
        }
        // In the profile's order, Workshop items with its other mods.
        ours.sort_by_key(|(owner, _)| profile.position(owner).unwrap_or(usize::MAX));
        let ours: Vec<String> = ours.into_iter().map(|(_, m)| m).collect();
        plan.files.sort_by_key(|f| f.target.key());
        if ours.is_empty() && drop.is_empty() {
            return Ok(());
        }
        if px.list == ParadoxList::Settings {
            plan.lists.push(PlannedList {
                path: user.join(SETTINGS),
                key: LAST_MODS.into(),
                entries: ours,
                format: umm_deploy::ListFormat::Block,
            });
            return Ok(());
        }
        let list = user.join(DLC_LOAD);
        let original = fs::read(&list)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let text = dlc_load(original.as_deref(), &ours, &drop)?;
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

pub(crate) struct Paradox;

impl crate::games::GameIntegration for Paradox {
    /// Crusader Kings II, on Steam's older Workshop: its user folder's `mod/`.
    fn workshop_downloads(&self, s: &Session, def: &CompiledDef) -> Option<PathBuf> {
        let px = def
            .def
            .paradox
            .as_ref()
            .filter(|p| p.list == ParadoxList::Settings)?;
        Some(s.resolve_folders(def).ok()?.get(&px.folder)?.join("mod"))
    }

    /// The descriptor the game wrote for each item, as its mod list names it (`mod/<name>.mod`).
    fn workshop_downloaded(
        &self,
        folder: &Path,
        items: &BTreeMap<String, crate::workshop::WorkshopDetails>,
    ) -> BTreeMap<String, String> {
        workshop_descriptors_in(folder, items)
    }

    fn mod_root(
        &self,
        rels: &[RelPath],
        def: &CompiledDef,
    ) -> Option<Option<umm_store::Placement>> {
        Some(mod_root(rels, def))
    }

    /// The launcher's `<mod>.mod` beside a mod's folder: UMM writes the game its own.
    fn is_mod_descriptor(&self, file: &str) -> bool {
        file.to_ascii_lowercase().ends_with(".mod") && !file.eq_ignore_ascii_case(DESCRIPTOR)
    }

    fn workshop(&self) -> crate::workshop::WorkshopControl {
        crate::workshop::WorkshopControl::Profile
    }

    fn plan(
        &self,
        s: &Session,
        def: &CompiledDef,
        profile: &umm_store::Profile,
        plan: &mut Plan,
    ) -> Result<()> {
        s.plan_paradox(def, profile, plan)
    }

    fn launch_args(
        &self,
        s: &Session,
        def: &CompiledDef,
        profile: &umm_store::Profile,
        _journal: &umm_deploy::Journal,
    ) -> Vec<String> {
        s.paradox_mod_args(def, profile)
    }
}

/// A Paradox game: its exe (relative to the game folder) and its user folder's name under
/// `Documents\\Paradox Interactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParadoxGame {
    pub exe: &'static str,
    pub user: &'static str,
    /// Where it lists its enabled mods.
    pub list: ParadoxList,
    /// Its exe's arguments to start the game directly.
    pub args: &'static [&'static str],
}

/// Paradox games whose mods go in their user folder.
pub const PARADOX_GAMES: &[ParadoxGame] = &[
    ParadoxGame {
        exe: "binaries/ck3.exe",
        user: "Crusader Kings III",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "binaries/victoria3.exe",
        user: "Victoria 3",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "binaries/imperator.exe",
        user: "Imperator",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "stellaris.exe",
        user: "Stellaris",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "hoi4.exe",
        user: "Hearts of Iron IV",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    ParadoxGame {
        exe: "eu4.exe",
        user: "Europa Universalis IV",
        list: ParadoxList::DlcLoad,
        args: &[],
    },
    // Its own launcher, built into the exe, is skipped: the game is then given its mods as
    // `-mod=` arguments, from `last_mods` in `settings.txt` (what that launcher shows ticked).
    ParadoxGame {
        exe: "CK2game.exe",
        user: "Crusader Kings II",
        list: ParadoxList::Settings,
        args: &["-skiplauncher"],
    },
];

/// The Paradox game installed in `root`, by its exe.
pub fn paradox_game(root: &std::path::Path) -> Option<&'static ParadoxGame> {
    PARADOX_GAMES
        .iter()
        .find(|g| umm_gamedef::kind::has_file_ci(root, g.exe))
}

/// Install rules for a Paradox game: each mod in a folder of its own in the user folder's
/// `mod/` (named `folder` in the definition's folders).
pub fn paradox_rules(folder: &str) -> Vec<umm_gamedef::InstallRule> {
    vec![umm_gamedef::kind::rule(
        "**",
        &format!("@{folder}/mod/{{mod}}/"),
    )]
}

impl umm_gamedef::GameKind for Paradox {
    fn key(&self) -> &'static str {
        "paradox"
    }

    /// By its exe, often in `binaries/` (the top holds its launcher).
    fn recognise(
        &self,
        root: &std::path::Path,
        _exes: &[String],
    ) -> Option<umm_gamedef::Recognised> {
        let g = paradox_game(root)?;
        Some(umm_gamedef::Recognised {
            key: "paradox",
            label: Some("Clausewitz / Jomini (Paradox)"),
            exe: Some(g.exe.to_string()),
            own_mods: Some(umm_gamedef::BUILT_IN_MODS),
        })
    }

    fn shape(
        &self,
        root: &std::path::Path,
        _found: &umm_gamedef::steam::Inspection,
        exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let Some(g) = paradox_game(root) else {
            return umm_gamedef::Shape::default();
        };
        let (list, launcher) = match g.list {
            ParadoxList::DlcLoad => (
                "dlc_load.json",
                "Paradox's launcher, which would replace the list with its playset",
            ),
            ParadoxList::Settings => ("settings.txt", "its launcher"),
        };
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "paradox",
                format!(
                    "Paradox game: each mod goes in its own folder in Documents\\Paradox Interactive\\{}\\mod, \
                     with a descriptor UMM writes, and is enabled in {list} while mods are deployed \
                     (your own list comes back afterwards). UMM starts the game directly, not through \
                     {launcher}.",
                    g.user,
                ),
            )],
            user_folder: Some(format!("%DOCUMENTS%/Paradox Interactive/{}", g.user)),
            install: paradox_rules(umm_gamedef::steam::USER_FOLDER),
            paradox: Some(umm_gamedef::Paradox {
                folder: umm_gamedef::steam::USER_FOLDER.into(),
                list: g.list,
            }),
            args: if exe.eq_ignore_ascii_case(g.exe) {
                g.args.iter().map(|a| a.to_string()).collect()
            } else {
                Vec::new()
            },
            // Started through Steam it runs Paradox's launcher, which writes the list of
            // enabled mods from its playset: started directly instead, Steam running.
            direct: true,
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.paradox.is_some()
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        paradox_rules(
            def.paradox
                .as_ref()
                .map_or(umm_gamedef::steam::USER_FOLDER, |p| p.folder.as_str()),
        )
    }
}

/// A Paradox mod's folder is the one holding its `descriptor.mod` (an archive often has the
/// launcher's `<mod>.mod` beside it, left out); without one, the folder beside the launcher's
/// `<mod>.mod` (Crusader Kings II's mods), else wrapper folders are dropped.
/// The install rules give it a folder of its own in the user folder's `mod/`.
fn mod_root(rels: &[RelPath], def: &CompiledDef) -> Option<umm_store::Placement> {
    let root = rels
        .iter()
        .filter(|r| {
            r.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(DESCRIPTOR))
        })
        .map(|r| r.parent().unwrap_or_else(RelPath::root))
        .min_by_key(|r| r.depth())
        .map(|r| r.as_str().to_string())
        .or_else(|| crate::placement::launcher_folder(rels))
        .unwrap_or_else(|| crate::placement::wrapper_chain(rels, def));
    let p = umm_store::Placement {
        root,
        ..Default::default()
    };
    (p != umm_store::Placement::default()).then_some(p)
}

/// The descriptors in `mods` (a game's `mod` folder) of the items Steam names the files of,
/// by item id, as `mod/<name>.mod`: the one whose `archive` (or `path`) is the item's file.
/// The game names them after the file, not the item.
pub(crate) fn workshop_descriptors_in(
    mods: &Path,
    items: &BTreeMap<String, crate::workshop::WorkshopDetails>,
) -> BTreeMap<String, String> {
    let norm = |s: &str| s.trim().trim_matches('"').replace('\\', "/").to_lowercase();
    let mut by_file = BTreeMap::new();
    for e in fs::read_dir(mods).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".mod") {
            continue;
        }
        let Ok(bytes) = fs::read(e.path()) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if matches!(k.trim().to_ascii_lowercase().as_str(), "archive" | "path") {
                by_file
                    .entry(norm(v))
                    .or_insert_with(|| format!("mod/{name}"));
            }
        }
    }
    items
        .iter()
        .filter(|(_, d)| !d.filename.is_empty())
        .filter_map(|(id, d)| Some((id.clone(), by_file.get(&norm(&d.filename))?.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_the_game_downloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path();
        // As Crusader Kings II writes them, named after the file, not the item.
        fs::write(
            mods.join("sketchycheatmenu.mod.mod"),
            "name=\"Sketchy Cheat Menu\"\narchive=\"mod/sketchycheatmenu.zip\"\n",
        )
        .unwrap();
        fs::write(mods.join("umm_x.mod"), "name=\"x\"\npath=\"mod/x\"\n").unwrap();
        let item = |filename: &str| crate::workshop::WorkshopDetails {
            title: String::new(),
            description: String::new(),
            preview_url: None,
            file_size: 0,
            time_updated: 0,
            time_created: 0,
            subscriptions: 0,
            favorited: 0,
            filename: filename.into(),
            tags: Vec::new(),
        };
        let items = BTreeMap::from([
            ("608738995".to_string(), item("mod/SketchyCheatMenu.zip")),
            ("2".to_string(), item("mod/elsewhere.zip")),
            ("3".to_string(), item("")),
        ]);
        assert_eq!(
            workshop_descriptors_in(mods, &items),
            BTreeMap::from([(
                "608738995".to_string(),
                "mod/sketchycheatmenu.mod.mod".to_string()
            )])
        );
    }

    #[test]
    fn writes_descriptors_and_the_enabled_list() {
        let own = "version=\"1.2\"\ntags={\n\t\"Gameplay\"\n}\nname=\"Better Barons\"\nsupported_version=\"1.12.*\"\npath=\"C:/old/place\"\nremote_file_id=\"123\"\n";
        let d = descriptor(
            Some(own),
            "x",
            "C:/Users/a/Documents/Paradox Interactive/Crusader Kings III/mod/better",
        );
        assert!(d.contains("name=\"Better Barons\""));
        assert!(!d.contains("C:/old/place"));
        assert!(
            d.ends_with(
                "path=\"C:/Users/a/Documents/Paradox Interactive/Crusader Kings III/mod/better\"\n"
            ),
            "{d}"
        );
        assert!(descriptor(None, "Plain", "/m").starts_with("name=\"Plain\"\n"));
        // As Age Customizer's has it: a brace closing nothing, which would hide `path`.
        let stray = "\tname=\"Age Customizer\"\r\n\tpath=\"mod/AgeCustomizer\"\r\n\tpicture=\"a.jpg\"\r\n}\r\n";
        assert_eq!(
            descriptor(Some(stray), "x", "mod/age"),
            "\tname=\"Age Customizer\"\n\tpicture=\"a.jpg\"\npath=\"mod/age\"\n"
        );
        let nested = "name=\"N\"\ntags={ \"Map\" } # a {\ndependencies={\n\t\"Other\"\n}\n";
        assert_eq!(
            descriptor(Some(nested), "x", "mod/n"),
            format!("{nested}path=\"mod/n\"\n")
        );

        let mine = ["mod/umm-a.mod".to_string()];
        let j: serde_json::Value = serde_json::from_str(
            &dlc_load(
                Some(r#"{"enabled_mods":["mod/ugc_1.mod"],"disabled_dlcs":["dlc/x.dlc"]}"#),
                &mine,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            j["enabled_mods"],
            serde_json::json!(["mod/ugc_1.mod", "mod/umm-a.mod"])
        );
        assert_eq!(j["disabled_dlcs"], serde_json::json!(["dlc/x.dlc"]));
        let j: serde_json::Value =
            serde_json::from_str(&dlc_load(None, &mine, &[]).unwrap()).unwrap();
        assert_eq!(j["enabled_mods"], serde_json::json!(["mod/umm-a.mod"]));
        // A Workshop item turned off: the launcher's entry for it goes.
        let j: serde_json::Value = serde_json::from_str(
            &dlc_load(
                Some(r#"{"enabled_mods":["mod/ugc_1.mod","mod/ugc_2.mod"]}"#),
                &[],
                &["mod/ugc_1.mod".to_string()],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(j["enabled_mods"], serde_json::json!(["mod/ugc_2.mod"]));
    }
}
