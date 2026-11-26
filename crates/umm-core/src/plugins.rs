//! Game plugin lists: a game that only loads the plugins (or mods) its list names. While
//! mods are deployed UMM writes the list with the mods' entries enabled, after the user's own,
//! in the profile's load order. The journal puts the user's file back afterwards (see
//! `umm_deploy::Op::External`).
//!
//! The formats: Bethesda's `plugins.txt` (with or without `*`), Morrowind's `[Game Files]` in
//! `Morrowind.ini`, RimWorld's `ModsConfig.xml` and Factorio's `mod-list.json`. Only the list
//! changes; the rest of the file is kept as it is.

use crate::{Result, Session};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use umm_deploy::{Plan, PlannedExternal, PlannedFile, Skipped};
use umm_gamedef::{CompiledDef, PluginListFormat, RelPath, PLUGIN_EXTENSIONS};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::Profile;
use PluginListFormat::{Asterisk, FactorioJson, MorrowindIni, Plain, RimworldXml};

/// The plugin list a deploy writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginList {
    pub path: PathBuf,
    pub format: PluginListFormat,
    /// Every entry, in order.
    pub entries: Vec<PluginEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
    /// A plugin's file name, a RimWorld mod's package id, or a Factorio mod's name.
    pub name: String,
    pub enabled: bool,
    /// The mod providing it; `None` for the user's own entries.
    pub owner: Option<String>,
}

/// The file's text. Morrowind's ini is in the system code page: it is read byte for byte, so
/// the rest of the file is written back exactly as it was.
fn decode(bytes: &[u8], format: PluginListFormat) -> String {
    match format {
        MorrowindIni => bytes.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn encode(text: &str, format: PluginListFormat) -> Vec<u8> {
    match format {
        MorrowindIni => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect(),
        _ => text.as_bytes().to_vec(),
    }
}

/// The line ending the file uses; CRLF for a new one, as the games write.
fn eol(text: &str) -> &'static str {
    if text.contains('\n') && !text.contains("\r\n") {
        "\n"
    } else {
        "\r\n"
    }
}

/// An ini section header's name.
fn header(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim)
}

/// A `GameFileN=` line's value.
fn game_file(line: &str) -> Option<&str> {
    let (k, v) = line.trim().split_once('=')?;
    let k = k.trim();
    let digits = k
        .get(8..)
        .filter(|_| k[..8].eq_ignore_ascii_case("GameFile"))?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| v.trim())
}

const GAME_FILES: &str = "Game Files";
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

/// The user's list: its entries, in order.
fn read_list(text: &str, format: PluginListFormat) -> Vec<PluginEntry> {
    let entry = |name: &str, enabled: bool| PluginEntry {
        name: name.to_string(),
        enabled,
        owner: None,
    };
    match format {
        Asterisk | Plain => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|line| match (format, line.strip_prefix('*')) {
                (Asterisk, Some(n)) => entry(n.trim(), true),
                (Asterisk, None) => entry(line, false),
                _ => entry(line, true),
            })
            .collect(),
        MorrowindIni => {
            let mut section = "";
            let mut out = Vec::new();
            for line in text.lines() {
                if let Some(h) = header(line) {
                    section = h;
                } else if section.eq_ignore_ascii_case(GAME_FILES) {
                    if let Some(v) = game_file(line).filter(|v| !v.is_empty()) {
                        out.push(entry(v, true));
                    }
                }
            }
            out
        }
        RimworldXml => active_mods(text)
            .map(|(_, _, ids)| ids.iter().map(|id| entry(id, true)).collect())
            .unwrap_or_default(),
        FactorioJson => crate::factorio::read(text),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl PluginList {
    fn enabled(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.name.as_str())
    }

    /// The file with this list, made from the user's file (`original`, empty when there is
    /// none): everything but the list is kept.
    pub fn render(&self, original: &str) -> String {
        let nl = eol(original);
        match self.format {
            Asterisk | Plain => {
                let mut out = String::new();
                for c in original
                    .lines()
                    .map(str::trim)
                    .filter(|l| l.starts_with('#'))
                {
                    out.push_str(c);
                    out.push_str(nl);
                }
                for e in &self.entries {
                    match self.format {
                        Asterisk if e.enabled => out.push('*'),
                        Plain if !e.enabled => continue,
                        _ => {}
                    }
                    out.push_str(&e.name);
                    out.push_str(nl);
                }
                out
            }
            MorrowindIni => {
                let entries: String = self
                    .enabled()
                    .enumerate()
                    .map(|(i, n)| format!("GameFile{i}={n}{nl}"))
                    .collect();
                let mut out = String::new();
                let mut in_section = false;
                let mut written = false;
                for line in original.split_inclusive('\n') {
                    if let Some(h) = header(line) {
                        in_section = h.eq_ignore_ascii_case(GAME_FILES);
                        out.push_str(line);
                        if in_section && !written {
                            if !line.ends_with('\n') {
                                out.push_str(nl);
                            }
                            out.push_str(&entries);
                            written = true;
                        }
                    } else if !(in_section && game_file(line).is_some()) {
                        out.push_str(line);
                    }
                }
                if !written {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push_str(nl);
                    }
                    out.push_str(&format!("[{GAME_FILES}]{nl}{entries}"));
                }
                out
            }
            RimworldXml => {
                let items: String = self
                    .enabled()
                    .map(|id| format!("{nl}    <li>{}</li>", xml_escape(id)))
                    .collect();
                if let Some((start, end, _)) = active_mods(original) {
                    return format!("{}{items}{nl}  {}", &original[..start], &original[end..]);
                }
                let block = format!("{ACTIVE}{items}{nl}  {ACTIVE_END}");
                for empty in ["<activeMods />", "<activeMods/>"] {
                    if let Some(i) = original.find(empty) {
                        return format!(
                            "{}{block}{}",
                            &original[..i],
                            &original[i + empty.len()..]
                        );
                    }
                }
                if let Some(i) = original.find("</ModsConfigData>") {
                    return format!("{}  {block}{nl}{}", &original[..i], &original[i..]);
                }
                format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?>{nl}<ModsConfigData>{nl}  {block}{nl}</ModsConfigData>{nl}"
                )
            }
            FactorioJson => crate::factorio::render(original, &self.entries),
        }
    }
}

/// A RimWorld mod's package id, from its `About/About.xml`, lower-case as the game lists it.
fn package_id(about: &Path) -> Option<String> {
    let text = String::from_utf8_lossy(&fs::read(about).ok()?).into_owned();
    let lower = text.to_ascii_lowercase();
    let start = lower.find("<packageid>")? + "<packageid>".len();
    let end = start + lower[start..].find('<')?;
    let id = lower[start..end].trim().to_string();
    (!id.is_empty()).then_some(id)
}

/// An ini setting UMM sets while deployed, and what the file has now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniPreview {
    pub path: PathBuf,
    pub section: String,
    pub key: String,
    pub value: String,
    pub current: Option<String>,
    /// The file exists; UMM leaves a missing one for the game to write, unless `create`.
    pub exists: bool,
    pub create: bool,
}

/// The value of `key` in `section`, as the deploy reads it.
fn ini_get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = s.trim().to_string();
        } else if current.eq_ignore_ascii_case(section) && !t.starts_with([';', '#']) {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// A plugin's rank within one mod: masters first, then light plugins, then the rest.
fn rank(name: &str) -> usize {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    PLUGIN_EXTENSIONS
        .iter()
        .position(|e| *e == ext)
        .unwrap_or(PLUGIN_EXTENSIONS.len())
}

impl Session {
    /// The plugin list deploying `files` would write, with the user's file as it is now.
    /// `None` when the game keeps no list, or no mod has an entry to enable.
    pub(crate) fn plugin_list(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        files: &[PlannedFile],
    ) -> Result<Option<(PluginList, String)>> {
        let Some(cfg) = &def.def.plugins else {
            return Ok(None);
        };
        let root = self.install_path(&def.def)?.map(|(r, _)| r);
        let path = match (&root, cfg.relative_list()) {
            (Some(r), _) => cfg.list_path(r),
            (None, None) => cfg.list_path(Path::new("")),
            (None, Some(_)) => None,
        };
        let Some(mut path) = path else {
            return Err(crate::CoreError::Invalid(format!(
                "cannot find {}: [mod_list] list needs its environment variables set, or the game's folder",
                cfg.list
            )));
        };
        // Some games keep the list in their own folder when an ini of theirs says so.
        if let (Some(c), Some(root)) = (&cfg.in_game_folder_when, &root) {
            let ini = RelPath::parse(&c.ini)?.to_path(root);
            let set = fs::read(&ini)
                .ok()
                .and_then(|b| ini_get(&String::from_utf8_lossy(&b), &c.section, &c.key));
            if set.as_deref() == Some(c.value.as_str()) {
                if let Some(name) = path.file_name() {
                    path = root.join(name);
                }
            }
        }
        let folder = RelPath::parse(&cfg.folder)?;
        // The mods' entries, by load order. The loader pack (not in the profile) loads first.
        let order = |owner: &str| profile.position(owner).map_or(0, |p| p + 1);
        let mut ours: Vec<(&str, String)> = match cfg.format {
            // Plugins: files directly in the plugin folder.
            Asterisk | Plain | MorrowindIni => files
                .iter()
                .filter(|f| f.target.parent().is_some_and(|p| p.key() == folder.key()))
                .filter_map(|f| Some((f.owner.as_str(), f.target.file_name()?.to_string())))
                .filter(|(_, n)| rank(n) < PLUGIN_EXTENSIONS.len())
                .collect(),
            // RimWorld mods: a folder each, named by its About/About.xml.
            RimworldXml => {
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
            // Factorio mods: a folder each, named by its info.json.
            FactorioJson => {
                let prefix = format!("{}/", folder.as_str());
                files
                    .iter()
                    .filter(|f| crate::factorio::info_folder(&f.target, &prefix).is_some())
                    .filter_map(|f| Some((f.owner.as_str(), crate::factorio::mod_name(&f.source)?)))
                    .collect()
            }
        };
        // RimWorld's Workshop items the profile has: on as it says, by their package ids;
        // the others are left as the user's list has them.
        let mut off: HashSet<String> = HashSet::new();
        let workshop_ids: Vec<(String, bool)> = profile
            .workshop()
            .map(|(i, on)| (format!("{}{i}", umm_store::WORKSHOP_PREFIX), on))
            .collect();
        if cfg.format == RimworldXml && !workshop_ids.is_empty() {
            let folders = self.workshop_folders(&def.def);
            for (entry, on) in &workshop_ids {
                let item = &entry[umm_store::WORKSHOP_PREFIX.len()..];
                let Some(pid) = folders
                    .get(item)
                    .and_then(|f| package_id(&f.join("About").join("About.xml")))
                else {
                    continue;
                };
                if *on {
                    ours.push((entry.as_str(), pid));
                } else {
                    off.insert(pid);
                }
            }
        }
        if ours.is_empty() && off.is_empty() {
            return Ok(None);
        }
        ours.sort_by(|(a, an), (b, bn)| {
            (order(a), rank(an), an.to_lowercase()).cmp(&(order(b), rank(bn), bn.to_lowercase()))
        });
        let text = match fs::read(&path) {
            Ok(b) => decode(&b, cfg.format),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("reading {}", path.display())).into(),
                )
            }
        };
        let mut entries = read_list(&text, cfg.format);
        let names: HashSet<String> = ours.iter().map(|(_, n)| n.to_lowercase()).collect();
        entries.retain(|e| {
            let n = e.name.to_lowercase();
            !names.contains(&n) && !off.contains(&n)
        });
        let mut seen = HashSet::new();
        entries.extend(
            ours.into_iter()
                .filter(|(_, n)| seen.insert(n.to_lowercase()))
                .map(|(owner, name)| PluginEntry {
                    name,
                    enabled: true,
                    owner: Some(owner.to_string()),
                }),
        );
        Ok(Some((
            PluginList {
                path,
                format: cfg.format,
                entries,
            },
            text,
        )))
    }

    /// Adds the game's plugin list to a deploy: written to the data folder, then planned as a
    /// file UMM puts in place and takes back. A mod's own copy of a list kept in the game
    /// folder is left out, since UMM writes it.
    pub(crate) fn plan_plugin_list(
        &self,
        def: &CompiledDef,
        profile: &Profile,
        plan: &mut Plan,
        root: &Path,
    ) -> Result<()> {
        let Some((list, original)) = self.plugin_list(def, profile, &plan.files)? else {
            return Ok(());
        };
        let key = |p: &Path| p.to_string_lossy().to_lowercase();
        let (theirs, files): (Vec<PlannedFile>, Vec<PlannedFile>) = std::mem::take(&mut plan.files)
            .into_iter()
            .partition(|f| key(&f.target.to_path(root)) == key(&list.path));
        plan.files = files;
        plan.skipped.extend(theirs.into_iter().map(|f| Skipped {
            owner: f.owner,
            file: f.target.as_str().to_string(),
            reason: "UMM writes the game's plugin list itself".into(),
        }));
        let name = list
            .path
            .file_name()
            .map_or("plugins.txt".into(), |n| n.to_string_lossy().into_owned());
        let src = self.data.game(&def.def.id).generated_dir().join(name);
        write_atomic(&src, &encode(&list.render(&original), list.format))
            .ctx(|| format!("writing {}", src.display()))?;
        // What the game may write in the list itself: UMM's entries, and the plugins already
        // in its data folder (its own masters and DLC, which some games add on every start).
        let tolerated = match list.format {
            Plain | Asterisk => {
                let mut t: Vec<String> = list
                    .render(&original)
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').to_lowercase())
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .collect();
                let data = def
                    .def
                    .plugins
                    .as_ref()
                    .map(|p| root.join(p.folder.trim_matches('/')));
                if let Some(rd) = data.and_then(|d| fs::read_dir(d).ok()) {
                    t.extend(rd.flatten().filter_map(|e| {
                        let n = e.file_name().to_string_lossy().to_lowercase();
                        let ext = n.rsplit_once('.').map(|(_, x)| x.to_string())?;
                        PLUGIN_EXTENSIONS.contains(&ext.as_str()).then_some(n)
                    }));
                }
                t.sort();
                t.dedup();
                t
            }
            MorrowindIni | RimworldXml | FactorioJson => Vec::new(),
        };
        plan.external.push(PlannedExternal {
            hash: hash_file(&src).ctx(|| format!("hashing {}", src.display()))?,
            path: list.path,
            source: src,
            tolerated,
        });
        Ok(())
    }

    /// The game's ini settings, planned for a deploy.
    pub(crate) fn plan_ini(&self, def: &CompiledDef) -> Result<Vec<umm_deploy::PlannedIni>> {
        def.def
            .ini
            .iter()
            .map(|s| {
                let path = umm_gamedef::expand_path(&s.file).ok_or_else(|| {
                    crate::CoreError::Invalid(format!(
                        "cannot find {}: [[ini]] file needs an absolute path, and its variables set",
                        s.file
                    ))
                })?;
                Ok(umm_deploy::PlannedIni {
                    path,
                    section: s.section.clone(),
                    key: s.key.clone(),
                    value: s.value.clone(),
                    create: s.create,
                })
            })
            .collect()
    }

    /// The game's ini settings, each with the value the file has now (`None` when it does
    /// not set it) and whether the file exists, for showing before a deploy.
    pub fn ini_preview(&self, game_id: &str) -> Result<Vec<IniPreview>> {
        let def = self.def(game_id)?;
        Ok(self
            .plan_ini(&def)?
            .into_iter()
            .map(|s| {
                let text = fs::read(&s.path)
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
                IniPreview {
                    current: text.as_deref().and_then(|t| ini_get(t, &s.section, &s.key)),
                    exists: text.is_some(),
                    create: s.create,
                    path: s.path,
                    section: s.section,
                    key: s.key,
                    value: s.value,
                }
            })
            .collect())
    }

    /// The plugin list the profile would get, for showing before a deploy.
    pub fn plugin_list_preview(&self, game_id: &str, profile: &str) -> Result<Option<PluginList>> {
        let def = self.def(game_id)?;
        let game = self.data.game(game_id);
        let profile = umm_store::load_profile(&game, profile)?;
        let root = self.install_path(&def.def)?.map(|(p, _)| p);
        let plan = umm_deploy::build_plan(&game, &def, &profile, root.as_deref())?;
        Ok(self
            .plugin_list(&def, &profile, &plan.files)?
            .map(|(l, _)| l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(format: PluginListFormat, entries: &[(&str, bool)]) -> PluginList {
        PluginList {
            path: PathBuf::new(),
            format,
            entries: entries
                .iter()
                .map(|(n, enabled)| PluginEntry {
                    name: n.to_string(),
                    enabled: *enabled,
                    owner: None,
                })
                .collect(),
        }
    }

    #[test]
    fn reads_and_writes_bethesda_lists() {
        let original = "# This file is used by the game\r\n*Mine.esp\r\nOff.esp\r\n\r\n";
        let e = read_list(original, Asterisk);
        assert_eq!(
            (e[0].name.as_str(), e[0].enabled, e[1].enabled),
            ("Mine.esp", true, false)
        );
        let l = list(Asterisk, &[("Mine.esp", true), ("Off.esp", false)]);
        assert_eq!(
            l.render(original),
            "# This file is used by the game\r\n*Mine.esp\r\nOff.esp\r\n"
        );
        let p = PluginList { format: Plain, ..l };
        assert_eq!(
            p.render(""),
            "Mine.esp\r\n",
            "a plain list names enabled plugins only"
        );
        assert!(rank("a.esm") < rank("b.ESL") && rank("b.esl") < rank("c.esp"));
        assert_eq!(rank("readme.txt"), PLUGIN_EXTENSIONS.len());
    }

    #[test]
    fn morrowind_lists_its_game_files_in_its_ini() {
        // Bytes outside UTF-8 elsewhere in the file come back as they were.
        let original: &[u8] = b"[General]\r\nName=Caf\xe9\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n";
        let text = decode(original, MorrowindIni);
        let e = read_list(&text, MorrowindIni);
        assert_eq!(
            e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["Morrowind.esm", "Tribunal.esm"]
        );
        let l = list(
            MorrowindIni,
            &[
                ("Morrowind.esm", true),
                ("Tribunal.esm", true),
                ("Mine.esp", true),
            ],
        );
        assert_eq!(
            encode(&l.render(&text), MorrowindIni),
            b"[General]\r\nName=Caf\xe9\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=Mine.esp\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n"
        );
        // An ini without the section gets one.
        assert_eq!(
            l.render("[General]\r\nA=1"),
            "[General]\r\nA=1\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=Mine.esp\r\n"
        );
        assert_eq!(game_file("GameFile12 = x.esp"), Some("x.esp"));
        assert_eq!(game_file("GameFiles=x"), None);
        assert_eq!(game_file("Game"), None);
    }

    #[test]
    fn rimworld_lists_its_active_mods() {
        let original = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n  <version>1.5.4104 rev435</version>\n  <activeMods>\n    <li>brrainz.harmony</li>\n    <li>ludeon.rimworld</li>\n  </activeMods>\n  <knownExpansions>\n    <li>ludeon.rimworld.royalty</li>\n  </knownExpansions>\n</ModsConfigData>\n";
        let e = read_list(original, RimworldXml);
        assert_eq!(
            e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["brrainz.harmony", "ludeon.rimworld"]
        );
        let l = list(
            RimworldXml,
            &[
                ("brrainz.harmony", true),
                ("ludeon.rimworld", true),
                ("me.mine", true),
            ],
        );
        assert_eq!(
            l.render(original),
            original.replace(
                "    <li>ludeon.rimworld</li>\n",
                "    <li>ludeon.rimworld</li>\n    <li>me.mine</li>\n"
            )
        );
        assert_eq!(
            l.render("<ModsConfigData>\r\n  <activeMods />\r\n</ModsConfigData>"),
            "<ModsConfigData>\r\n  <activeMods>\r\n    <li>brrainz.harmony</li>\r\n    <li>ludeon.rimworld</li>\r\n    <li>me.mine</li>\r\n  </activeMods>\r\n</ModsConfigData>"
        );
        assert!(l
            .render("")
            .contains("<ModsConfigData>\r\n  <activeMods>\r\n    <li>brrainz.harmony</li>"));
    }
}
