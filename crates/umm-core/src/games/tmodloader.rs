//! tModLoader (Terraria's mod loader, a game of its own on Steam): each mod is a `.tmod`
//! file, which it loads from the top of its mods folder (in its save folder, in Documents)
//! when named after the mod inside it, and enables by that name in `enabled.json` there. It
//! also loads the Workshop items it is subscribed to, straight from Steam's folder, when that
//! list names them.
//!
//! UMM puts each mod's `.tmod` at the top of that folder, named after its mod, and lists it
//! there while deployed (see [`crate::games::plugins`]); the player's own list comes back after.
//! (The `-modpath` flag that would point it elsewhere is for its servers only.)
//!
//! Resource packs (Terraria's own, which tModLoader loads too) are folders holding a
//! `pack.json`, or `.zip` files of one, in `ResourcePacks/` beside the mods folder, enabled in
//! the `ResourcePacks` array of `config.json` there. UMM puts each pack a mod has there, in a
//! folder named after the mod, and adds it to that array, on, while deployed; only UMM's
//! entries are taken out afterwards, the rest of the file being the game's settings.

use crate::games::plugins::PluginEntry;
use crate::{CoreError, Result, Session};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use umm_deploy::{ListFormat, Plan, PlannedList, Skipped};
use umm_gamedef::{tml_packs_prefix, CompiledDef, PluginListFormat, RelPath, TML_EXT, TML_PACK};

/// The file holding the game's settings and its list of resource packs, in its save folder,
/// and that list's key.
pub const CONFIG: &str = "config.json";
pub const PACKS_KEY: &str = "ResourcePacks";

/// Whether a `.zip` file is a resource pack: a `pack.json` at its top.
fn zip_pack(path: &Path) -> bool {
    File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|z| z.file_names().any(|n| n.eq_ignore_ascii_case(TML_PACK)))
}

/// A resource pack's entry in the game's list, on.
fn pack_entry(file_name: &str, order: usize) -> String {
    serde_json::json!({"FileName": file_name, "Enabled": true, "SortingOrder": order}).to_string()
}

fn is_zip(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

/// A string as .NET's `BinaryWriter` writes it: its length in bytes (7 bits a byte, low
/// first), then UTF-8.
fn dotnet_string(r: &mut impl Read) -> Option<String> {
    let mut len = 0usize;
    for shift in (0..35).step_by(7) {
        let mut b = [0u8];
        r.read_exact(&mut b).ok()?;
        len |= usize::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
    }
    if len > 4096 {
        return None;
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// A `.tmod` file's mod name and version, from its header: `TMOD`, the tModLoader version
/// it was built for, its hash (20 bytes), signature (256 bytes) and data length (4), then
/// the name and version. `None` for anything else, or files from before tModLoader 0.11,
/// which compressed those.
pub fn mod_info(path: &Path) -> Option<(String, String)> {
    let mut r = BufReader::new(File::open(path).ok()?);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"TMOD" {
        return None;
    }
    let built_for = dotnet_string(&mut r)?;
    let mut parts = built_for.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    if (parts.next()?, parts.next().unwrap_or(0)) < (0, 11) {
        return None;
    }
    let mut skip = [0u8; 20 + 256 + 4];
    r.read_exact(&mut skip).ok()?;
    let name = dotnet_string(&mut r)?;
    let version = dotnet_string(&mut r)?;
    fit_name(&name).then_some((name, version))
}

/// The mod name of a `.tmod` file: from its header, or else its file name.
pub fn mod_name(path: &Path, file: &str) -> String {
    mod_info(path)
        .map(|(n, _)| n)
        .unwrap_or_else(|| file[..file.len() - TML_EXT.len()].to_string())
}

/// Whether a mod's name can name its file.
fn fit_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        && !name.chars().any(char::is_control)
}

/// Whether `name` ends in `.tmod`.
fn is_tmod(name: &str) -> bool {
    name.len() > TML_EXT.len() && name.to_ascii_lowercase().ends_with(TML_EXT)
}

/// The mod name of a Workshop item's `.tmod`: at its top, or in a folder per tModLoader
/// version (`2025.9/`), as the Workshop has them now.
pub fn workshop_mod(folder: &Path) -> Option<String> {
    let mut files = Vec::new();
    let mut dirs = vec![(folder.to_path_buf(), 0)];
    while let Some((d, depth)) = dirs.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 1 {
                dirs.push((p, depth + 1));
            } else if is_tmod(&e.file_name().to_string_lossy()) {
                files.push(p);
            }
        }
    }
    // The newest version folder's first.
    files.sort();
    files.iter().rev().find_map(|f| mod_info(f)).map(|(n, _)| n)
}

/// The list's entries: the mods it names, all enabled.
pub fn read(text: &str) -> Vec<PluginEntry> {
    serde_json::from_str::<Vec<String>>(text.trim_start_matches('\u{feff}'))
        .unwrap_or_default()
        .into_iter()
        .map(|name| PluginEntry {
            name,
            enabled: true,
            owner: None,
        })
        .collect()
}

/// The list with these entries' enabled ones, as tModLoader writes it (Json.NET, indented,
/// with Windows line endings).
pub fn render(entries: &[PluginEntry]) -> String {
    let names: Vec<String> = entries
        .iter()
        .filter(|e| e.enabled)
        .map(|e| serde_json::Value::String(e.name.clone()).to_string())
        .collect();
    if names.is_empty() {
        return "[]".into();
    }
    format!("[\r\n  {}\r\n]", names.join(",\r\n  "))
}

/// The lines tModLoader may write in the list itself, lower-cased: its brackets, and these
/// names (in whatever order it keeps them).
pub fn tolerated(entries: &[PluginEntry]) -> Vec<String> {
    let mut t = vec!["[".to_string(), "]".to_string(), "[]".to_string()];
    for e in entries.iter().filter(|e| e.enabled) {
        let quoted = serde_json::Value::String(e.name.clone())
            .to_string()
            .to_lowercase();
        t.push(format!("{quoted},"));
        t.push(quoted);
    }
    t.sort();
    t.dedup();
    t
}

/// The name of the `.tmod` file `target` is in the mods folder (`prefix`, ending in `/`), if
/// it is one: in a folder of its own there too, as a mod shipped it.
pub fn tmod_in<'a>(target: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = target.get(prefix.len()..).filter(|_| {
        target
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    })?;
    let file = rest.rsplit('/').next()?;
    is_tmod(file).then_some(file)
}

/// The mods folder of a tModLoader definition, ending in `/`.
pub fn mods_prefix(def: &CompiledDef) -> Option<String> {
    def.def
        .plugins
        .as_ref()
        .filter(|p| p.format == PluginListFormat::TmodloaderJson)
        .map(|p| format!("{}/", p.folder.trim_matches('/')))
}

impl Session {
    /// For tModLoader: each `.tmod` at the top of the mods folder, named after its mod (it
    /// loads no other). Two mods of the same name cannot both be deployed.
    pub(crate) fn plan_tmodloader(&self, def: &CompiledDef, plan: &mut Plan) -> Result<()> {
        let Some(prefix) = mods_prefix(def) else {
            return Ok(());
        };
        // Mod name (lower-case) → the UMM mod it comes from.
        let mut owners: HashMap<String, String> = HashMap::new();
        let mut keep = Vec::with_capacity(plan.files.len());
        for mut f in std::mem::take(&mut plan.files) {
            let Some(file) = tmod_in(f.target.as_str(), &prefix) else {
                keep.push(f);
                continue;
            };
            let name = mod_name(&f.source, file);
            match owners.get(&name.to_lowercase()) {
                Some(other) if *other != f.owner => {
                    let (a, b) = if *other < f.owner {
                        (other.as_str(), f.owner.as_str())
                    } else {
                        (f.owner.as_str(), other.as_str())
                    };
                    return Err(CoreError::Invalid(format!(
                        "mods `{a}` and `{b}` are both the tModLoader mod `{name}`: disable one"
                    )));
                }
                // The same mod twice in one UMM mod: its first.
                Some(_) => continue,
                None => {}
            }
            owners.insert(name.to_lowercase(), f.owner.clone());
            f.target = RelPath::parse(&format!("{prefix}{name}{TML_EXT}"))?;
            keep.push(f);
        }
        plan.files = keep;
        let packs = plan_packs(plan, &tml_packs_prefix(&prefix))?;
        plan.files.sort_by_key(|f| f.target.key());
        // The packs on in the game's list, where its save folder is.
        let user = prefix
            .strip_prefix('@')
            .and_then(|p| p.split('/').next())
            .and_then(|name| plan.folders.get(name))
            .cloned();
        if let (Some(user), false) = (user, packs.is_empty()) {
            plan.lists.push(PlannedList {
                path: user.join(CONFIG),
                key: PACKS_KEY.into(),
                entries: packs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| pack_entry(p, i))
                    .collect(),
                format: ListFormat::Json,
            });
        }
        Ok(())
    }
}

/// Whether `path` is `root` or inside it (both lower-case, `root` empty for the top).
fn within(path: &str, root: &str) -> bool {
    root.is_empty() || path.starts_with(&format!("{root}/"))
}

/// The resource packs in each mod's folder in the packs folder (`prefix`, ending in `/`): its
/// folders holding a `pack.json` (one inside another is part of it), each moved to the top
/// (named after the mod, or after its own folder when the mod has several), and its `.zip`
/// packs; anything else is not for the game. Returns the packs' names in the packs folder.
fn plan_packs(plan: &mut Plan, prefix: &str) -> Result<Vec<String>> {
    let under = |t: &str| {
        t.get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    };
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut plan.files)
        .into_iter()
        .partition(|f| under(f.target.as_str()));
    plan.files = rest;
    // A target's mod folder and its path inside.
    let split = |t: &str| {
        let r = &t[prefix.len()..];
        r.split_once('/')
            .map(|(d, i)| (d.to_string(), i.to_string()))
    };
    // (owner, mod folder) → the folders inside holding a `pack.json`, as spelled.
    let mut roots: HashMap<(String, String), Vec<String>> = HashMap::new();
    for f in &mine {
        let Some((dir, inner)) = split(f.target.as_str()) else {
            continue;
        };
        let (parent, file) = inner.rsplit_once('/').unwrap_or(("", &inner));
        if file.eq_ignore_ascii_case(TML_PACK) {
            roots
                .entry((f.owner.clone(), dir.to_lowercase()))
                .or_default()
                .push(parent.to_string());
        }
    }
    for r in roots.values_mut() {
        let depth = |p: &String| {
            if p.is_empty() {
                0
            } else {
                p.matches('/').count() + 1
            }
        };
        r.sort_by_key(|p| (depth(p), p.clone()));
        let mut kept: Vec<String> = Vec::new();
        for p in r.drain(..) {
            if !kept
                .iter()
                .any(|k| within(&p.to_lowercase(), &k.to_lowercase()))
            {
                kept.push(p);
            }
        }
        *r = kept;
    }
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut claim = |name: &str, owner: &str| -> Result<()> {
        if let Some(other) = owners.get(&name.to_lowercase()).filter(|o| *o != owner) {
            return Err(CoreError::Invalid(format!(
                "mods `{other}` and `{owner}` both have the resource pack `{name}`: disable one"
            )));
        }
        owners.insert(name.to_lowercase(), owner.to_string());
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.to_string());
        }
        Ok(())
    };
    for mut f in mine {
        let target = f.target.as_str().to_string();
        let Some((dir, inner)) = split(&target) else {
            continue;
        };
        let packs = roots
            .get(&(f.owner.clone(), dir.to_lowercase()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let lower = inner.to_lowercase();
        let new = match packs.iter().find(|r| within(&lower, &r.to_lowercase())) {
            Some(r) => {
                let name = match r.rsplit('/').next() {
                    Some(own) if packs.len() > 1 && !own.is_empty() => {
                        // Its own folder's name, as the mod spells it.
                        let at = r.len() - own.len();
                        inner[at..r.len()].to_string()
                    }
                    _ => dir.clone(),
                };
                let inside = if r.is_empty() {
                    &inner[..]
                } else {
                    &inner[r.len() + 1..]
                };
                claim(&name, &f.owner)?;
                format!("{prefix}{name}/{inside}")
            }
            None if is_zip(&inner) && zip_pack(&f.source) => {
                let file = inner.rsplit('/').next().unwrap_or(&inner).to_string();
                claim(&file, &f.owner)?;
                format!("{prefix}{file}")
            }
            None => {
                plan.skipped.push(Skipped {
                    owner: f.owner.clone(),
                    file: inner.clone(),
                    reason: "neither a tModLoader mod nor part of a resource pack".into(),
                });
                continue;
            }
        };
        f.target = RelPath::parse(&new)?;
        plan.files.push(f);
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

pub(crate) struct TModLoader;

impl crate::games::GameIntegration for TModLoader {
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
        s.plan_tmodloader(def, plan)
    }
}

impl crate::games::plugins::ModList for TModLoader {
    fn kind(&self) -> &'static str {
        "mods"
    }

    fn read(&self, text: &str) -> Vec<PluginEntry> {
        read(text)
    }

    fn render(&self, _original: &str, entries: &[PluginEntry]) -> String {
        render(entries)
    }

    /// A `.tmod` each, named after the mod in it.
    fn entries<'a>(
        &self,
        files: &'a [umm_deploy::PlannedFile],
        folder: &umm_gamedef::RelPath,
    ) -> Vec<(&'a str, String)> {
        let prefix = format!("{}/", folder.as_str());
        files
            .iter()
            .filter_map(|f| {
                let file = tmod_in(f.target.as_str(), &prefix)?;
                Some((f.owner.as_str(), mod_name(&f.source, file)))
            })
            .collect()
    }

    fn lists_workshop_items(&self) -> bool {
        true
    }

    fn workshop_entry(&self, folder: &Path) -> Option<String> {
        workshop_mod(folder)
    }

    /// tModLoader rewrites its list on every start, in an order of its own.
    fn tolerated(&self, _rendered: &str, entries: &[PluginEntry], _data: &Path) -> Vec<String> {
        tolerated(entries)
    }
}

/// tModLoader's launch script, which starts it on the .NET it installs beside it, passing
/// its arguments on.
pub const TML_EXE: &str = "start-tModLoader.bat";
/// tModLoader's save folder (players, worlds, configs and its mods folder), and its list of
/// enabled mods in that folder.
const TML_USER: &str = "%DOCUMENTS%/My Games/Terraria/tModLoader";
const TML_ENABLED: &str = "enabled.json";

/// Install rules for tModLoader: each `.tmod` to its mods folder (`folder`), wherever it is
/// in the mod (UMM then puts it at the top, named after the mod, as the game wants), and
/// everything else to a folder of the mod's own among the resource packs beside it (UMM
/// keeps only the packs in it, each folder holding a `pack.json`).
pub fn tmodloader_rules(folder: &str) -> Vec<umm_gamedef::InstallRule> {
    use umm_gamedef::kind::rule;
    let folder = folder.trim_matches('/');
    vec![
        rule(&format!("**/*{TML_EXT}"), &format!("{folder}/")),
        rule("**", &format!("{}{{mod}}/", tml_packs_prefix(folder))),
    ]
}

impl umm_gamedef::GameKind for TModLoader {
    fn key(&self) -> &'static str {
        "tmodloader"
    }

    /// By its launch script beside its assembly (it runs on its own .NET).
    fn recognise(&self, root: &Path, _exes: &[String]) -> Option<umm_gamedef::Recognised> {
        use umm_gamedef::kind::has_file_ci;
        (has_file_ci(root, TML_EXE) && has_file_ci(root, "tModLoader.dll")).then(|| {
            umm_gamedef::Recognised {
                key: "tmodloader",
                label: Some("tModLoader (Terraria, XNA/FNA)"),
                exe: Some(TML_EXE.to_string()),
                own_mods: Some("tModLoader"),
            }
        })
    }

    fn shape(
        &self,
        _root: &Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        _setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let mods = format!("@{}/Mods", umm_gamedef::steam::USER_FOLDER);
        umm_gamedef::Shape {
            notes: vec![umm_gamedef::steam::Note::new(
                "tmodloader",
                "tModLoader: .tmod mods go to its Mods folder in Documents\\My Games\\Terraria\\\
                 tModLoader, named after the mod in each as it wants, and are enabled in its \
                 enabled.json there while mods are deployed (your own list comes back afterwards). \
                 Resource packs (folders or .zip files with a pack.json) go to its ResourcePacks \
                 folder beside it and are turned on in its config.json while deployed, the rest of \
                 your settings left as they are. Steam Workshop items can be turned on or off in a \
                 profile too. UMM starts tModLoader directly, not through Steam.",
            )],
            user_folder: Some(TML_USER.to_string()),
            install: tmodloader_rules(&mods),
            plugins: Some(umm_gamedef::Plugins {
                list: format!("{TML_USER}/Mods/{TML_ENABLED}"),
                format: PluginListFormat::TmodloaderJson,
                folder: mods,
                in_game_folder_when: None,
            }),
            // It runs as `dotnet.exe` under its script: started directly, UMM follows it as the
            // script's child, not by a name other programs share.
            direct: true,
            ..Default::default()
        }
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == PluginListFormat::TmodloaderJson)
    }

    fn own_mods(&self) -> Option<&'static str> {
        Some("tModLoader")
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        tmodloader_rules(def.plugins.as_ref().map_or("", |p| p.folder.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.tmod` header as tModLoader writes it.
    fn header(name: &str, version: &str, built_for: &str) -> Vec<u8> {
        let s = |out: &mut Vec<u8>, v: &str| {
            out.push(v.len() as u8);
            out.extend_from_slice(v.as_bytes());
        };
        let mut out = b"TMOD".to_vec();
        s(&mut out, built_for);
        out.extend_from_slice(&[0u8; 20 + 256 + 4]);
        s(&mut out, name);
        s(&mut out, version);
        out.extend_from_slice(&[0u8; 16]);
        out
    }

    #[test]
    fn reads_a_tmod_header() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.tmod");
        fs::write(&p, header("CalamityMod", "2.0.4.6", "2024.9.3.0")).unwrap();
        assert_eq!(mod_info(&p), Some(("CalamityMod".into(), "2.0.4.6".into())));
        fs::write(&p, header("Old", "1.0", "0.10.1.5")).unwrap();
        assert_eq!(mod_info(&p), None);
        fs::write(&p, b"PK\x03\x04").unwrap();
        assert_eq!(mod_info(&p), None);
        assert_eq!(mod_name(&p, "x.tmod"), "x");
        fs::write(&p, header("../evil", "1.0", "2024.9.3.0")).unwrap();
        assert_eq!(mod_info(&p), None);
    }

    #[test]
    fn workshop_items_newest_version_folder() {
        let dir = tempfile::tempdir().unwrap();
        let item = dir.path().join("2824688072");
        fs::create_dir_all(item.join("2023.8")).unwrap();
        fs::create_dir_all(item.join("2024.9")).unwrap();
        fs::write(
            item.join("2023.8/Old.tmod"),
            header("OldName", "1.0", "2023.8.3.0"),
        )
        .unwrap();
        fs::write(
            item.join("2024.9/New.tmod"),
            header("NewName", "1.1", "2024.9.3.0"),
        )
        .unwrap();
        fs::write(item.join("workshop.json"), "{}").unwrap();
        assert_eq!(workshop_mod(&item).as_deref(), Some("NewName"));
    }

    #[test]
    fn the_list_as_tmodloader_writes_it() {
        let e = |n: &str, on: bool| PluginEntry {
            name: n.into(),
            enabled: on,
            owner: None,
        };
        let entries = [e("A", true), e("Off", false), e("B", true)];
        let text = render(&entries);
        assert_eq!(text, "[\r\n  \"A\",\r\n  \"B\"\r\n]");
        let names: Vec<String> = read(&text).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(render(&[]), "[]");
        assert!(read("garbage").is_empty());
        assert_eq!(
            tolerated(&entries),
            ["\"a\"", "\"a\",", "\"b\"", "\"b\",", "[", "[]", "]"]
        );
    }
}
