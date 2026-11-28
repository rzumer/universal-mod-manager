//! Games' mod lists: a game that only loads the plugins (or mods) its list names. While mods
//! are deployed UMM writes the list with the mods' entries enabled, after the user's own, in
//! the profile's load order. The journal puts the user's file back afterwards (see
//! `umm_deploy::Op::External`).
//!
//! What a list looks like is the game's own: each format is a [`ModList`], implemented in its
//! game's module and found by [`mod_list`]. What every list has in common (finding the file,
//! merging the mods' entries into the user's, Workshop items, planning it) is here. Only the
//! list changes; the rest of the file is kept as it is.

use crate::games::{bethesda, factorio, kenshi, morrowind, oni, rimworld, tmodloader, xcom2};
use crate::{Result, Session};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use umm_deploy::{Plan, PlannedExternal, PlannedFile, Skipped};
use umm_gamedef::{CompiledDef, PluginListFormat, RelPath};
use umm_store::fsutil::{hash_file, write_atomic, IoCtx};
use umm_store::Profile;

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
    /// A plugin's file name, a RimWorld mod's package id, a Factorio mod's name, or an Oxygen
    /// Not Included mod's folder (`<id>.<platform>` for one not installed locally).
    pub name: String,
    pub enabled: bool,
    /// The mod providing it; `None` for the user's own entries.
    pub owner: Option<String>,
}

/// One game's list format: how its file reads and writes, and which of a deploy's files are
/// entries in it.
pub(crate) trait ModList: Sync {
    /// The file's text.
    fn decode(&self, bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn encode(&self, text: &str) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// The user's list: its entries, in order.
    fn read(&self, text: &str) -> Vec<PluginEntry>;

    /// The file with these entries, made from the user's file (`original`, empty when there
    /// is none): everything but the list is kept.
    fn render(&self, original: &str, entries: &[PluginEntry]) -> String;

    /// The deployed mods' entries, from the files a deploy places under the list's folder
    /// (`folder`): (the mod, its entry).
    fn entries<'a>(&self, files: &'a [PlannedFile], folder: &RelPath) -> Vec<(&'a str, String)>;

    /// Whether a profile turns the game's Steam Workshop items on or off by their entries in
    /// this list ([`Self::workshop_entry`]); the others are left as the user's list has them.
    fn lists_workshop_items(&self) -> bool {
        false
    }

    /// A Workshop item's entry, from its downloaded folder.
    fn workshop_entry(&self, _folder: &Path) -> Option<String> {
        None
    }

    /// The entry the game itself writes for a Workshop item (`item`, its id), which a
    /// profile turns on or off where the user's list has it.
    fn workshop_toggle(&self, _item: &str) -> Option<String> {
        None
    }

    /// What the list names, for the interface: `plugins`, `mods` (a folder or file each), or
    /// `dated` (plugins the game loads in the order of their file dates).
    fn kind(&self) -> &'static str {
        "plugins"
    }

    /// What the game may write in the list itself while playing, lower-case, besides UMM's
    /// version (`rendered`): not taken for a change of the user's. `data`: the game's folder of
    /// mods.
    fn tolerated(&self, _rendered: &str, _entries: &[PluginEntry], _data: &Path) -> Vec<String> {
        Vec::new()
    }
}

/// The list format a game definition names.
pub(crate) fn mod_list(format: PluginListFormat) -> &'static dyn ModList {
    match format {
        PluginListFormat::Asterisk => &bethesda::ASTERISK,
        PluginListFormat::Plain => &bethesda::PLAIN,
        PluginListFormat::MorrowindIni => &morrowind::Morrowind,
        PluginListFormat::RimworldXml => &rimworld::RimWorld,
        PluginListFormat::FactorioJson => &factorio::Factorio,
        PluginListFormat::OniJson => &oni::Oni,
        PluginListFormat::KenshiCfg => &kenshi::Kenshi,
        PluginListFormat::XcomIni => &xcom2::Xcom2,
        PluginListFormat::TmodloaderJson => &tmodloader::TModLoader,
    }
}

/// The line ending the file uses; CRLF for a new one, as the games write.
pub(crate) fn eol(text: &str) -> &'static str {
    if text.contains('\n') && !text.contains("\r\n") {
        "\n"
    } else {
        "\r\n"
    }
}

/// An ini section header's name.
pub(crate) fn header(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim)
}

/// An ini file with these lines (`entries`, each ending in `nl`) in its `section`, in place
/// of the lines `is_entry` says are the list's there; the rest kept as it is. The section is
/// added at the end when the file has none.
pub(crate) fn ini_section(
    original: &str,
    section: &str,
    is_entry: impl Fn(&str) -> bool,
    entries: &str,
    nl: &str,
) -> String {
    let mut out = String::new();
    let mut in_section = false;
    let mut written = false;
    for line in original.split_inclusive('\n') {
        if let Some(h) = header(line) {
            in_section = h.eq_ignore_ascii_case(section);
            out.push_str(line);
            if in_section && !written {
                if !line.ends_with('\n') {
                    out.push_str(nl);
                }
                out.push_str(entries);
                written = true;
            }
        } else if !(in_section && is_entry(line)) {
            out.push_str(line);
        }
    }
    if !written {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(nl);
        }
        out.push_str(&format!("[{section}]{nl}{entries}"));
    }
    out
}

impl PluginList {
    /// What the list names: `plugins`, `mods` (a folder or file each), or `dated` (plugins the
    /// game loads in the order of their file dates).
    pub fn kind(&self) -> &'static str {
        mod_list(self.format).kind()
    }

    /// The file with this list, made from the user's file (`original`, empty when there is
    /// none): everything but the list is kept.
    pub fn render(&self, original: &str) -> String {
        mod_list(self.format).render(original, &self.entries)
    }
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
        let vars = self.path_vars(&def.def);
        let path = match (&root, cfg.relative_list()) {
            (Some(r), _) => cfg.list_path(r, &vars),
            (None, None) => cfg.list_path(Path::new(""), &vars),
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
        let list = mod_list(cfg.format);
        // The mods' entries, by load order. The loader pack (not in the profile) loads first.
        let order = |owner: &str| profile.position(owner).map_or(0, |p| p + 1);
        let mut ours = list.entries(files, &folder);
        // The Workshop items the profile has: on as it says, by their entries in the list;
        // the others are left as the user's list has them.
        let mut off: HashSet<String> = HashSet::new();
        let workshop_ids: Vec<(String, bool)> = profile
            .workshop()
            .map(|(i, on)| (format!("{}{i}", umm_store::WORKSHOP_PREFIX), on))
            .collect();
        if list.lists_workshop_items() && !workshop_ids.is_empty() {
            let folders = self.workshop_folders(&def.def);
            for (entry, on) in &workshop_ids {
                let item = &entry[umm_store::WORKSHOP_PREFIX.len()..];
                let Some(pid) = folders.get(item).and_then(|f| list.workshop_entry(f)) else {
                    continue;
                };
                if *on {
                    ours.push((entry.as_str(), pid));
                } else {
                    off.insert(pid.to_lowercase());
                }
            }
        }
        // Entries the game writes for its Workshop items: on or off as the profile says.
        let toggled: std::collections::HashMap<String, bool> = profile
            .workshop()
            .filter_map(|(i, on)| Some((list.workshop_toggle(i)?.to_lowercase(), on)))
            .collect();
        if ours.is_empty() && off.is_empty() && toggled.is_empty() {
            return Ok(None);
        }
        let rank = bethesda::rank;
        ours.sort_by(|(a, an), (b, bn)| {
            (order(a), rank(an), an.to_lowercase()).cmp(&(order(b), rank(bn), bn.to_lowercase()))
        });
        let text = match fs::read(&path) {
            Ok(b) => list.decode(&b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(
                    umm_store::fsutil::io_err(e, format!("reading {}", path.display())).into(),
                )
            }
        };
        let mut entries = list.read(&text);
        let names: HashSet<String> = ours.iter().map(|(_, n)| n.to_lowercase()).collect();
        entries.retain(|e| {
            let n = e.name.to_lowercase();
            !names.contains(&n) && !off.contains(&n)
        });
        for e in &mut entries {
            if let Some(on) = toggled.get(&e.name.to_lowercase()) {
                e.enabled = *on;
            }
        }
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
        let format = mod_list(list.format);
        let rendered = list.render(&original);
        write_atomic(&src, &format.encode(&rendered))
            .ctx(|| format!("writing {}", src.display()))?;
        // What the game may write in the list itself.
        let data = def.def.plugins.as_ref().map_or_else(
            || root.to_path_buf(),
            |p| root.join(p.folder.trim_matches('/')),
        );
        let tolerated = format.tolerated(&rendered, &list.entries, &data);
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
        let vars = self.path_vars(&def.def);
        def.def
            .ini
            .iter()
            .map(|s| {
                let path = umm_gamedef::expand_path_with(&s.file, &vars).ok_or_else(|| {
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
