//! Bethesda games (Oblivion and later; Morrowind is [`crate::games::morrowind`]): plugins
//! (`.esm`, `.esl`, `.esp`) load only when `plugins.txt` names them. Newer games list every
//! plugin, the enabled ones starting with `*`; older ones list the enabled plugins only.

use crate::games::plugins::{eol, ModList, PluginEntry};
use std::fs;
use std::path::Path;
use umm_deploy::PlannedFile;
use umm_gamedef::{PluginListFormat, RelPath, PLUGIN_EXTENSIONS};

/// A Bethesda game's `plugins.txt`: with `*` for enabled plugins (Skyrim Special Edition and
/// later), or plain (enabled plugins only: Oblivion, Fallout 3, New Vegas).
pub(crate) struct PluginsTxt {
    pub asterisk: bool,
}

pub(crate) static ASTERISK: PluginsTxt = PluginsTxt { asterisk: true };
pub(crate) static PLAIN: PluginsTxt = PluginsTxt { asterisk: false };

/// A plugin's rank within one mod: masters first, then light plugins, then the rest.
/// Anything else ranks last.
pub(crate) fn rank(name: &str) -> usize {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    PLUGIN_EXTENSIONS
        .iter()
        .position(|e| *e == ext)
        .unwrap_or(PLUGIN_EXTENSIONS.len())
}

/// The plugins a deploy places directly in the plugin folder: (the mod, the file's name).
pub(crate) fn plugin_files<'a>(
    files: &'a [PlannedFile],
    folder: &RelPath,
) -> Vec<(&'a str, String)> {
    files
        .iter()
        .filter(|f| f.target.parent().is_some_and(|p| p.key() == folder.key()))
        .filter_map(|f| Some((f.owner.as_str(), f.target.file_name()?.to_string())))
        .filter(|(_, n)| rank(n) < PLUGIN_EXTENSIONS.len())
        .collect()
}

/// A list of one name per line, `#` comments kept at the top; with `asterisk`, every entry
/// listed and the enabled ones marked, else the enabled ones only.
pub(crate) fn read_lines(text: &str, asterisk: bool) -> Vec<PluginEntry> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let (name, enabled) = match (asterisk, line.strip_prefix('*')) {
                (true, Some(n)) => (n.trim(), true),
                (true, None) => (line, false),
                (false, _) => (line, true),
            };
            PluginEntry {
                name: name.to_string(),
                enabled,
                owner: None,
            }
        })
        .collect()
}

pub(crate) fn render_lines(original: &str, entries: &[PluginEntry], asterisk: bool) -> String {
    let nl = eol(original);
    let mut out = String::new();
    for c in original
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('#'))
    {
        out.push_str(c);
        out.push_str(nl);
    }
    for e in entries {
        if asterisk && e.enabled {
            out.push('*');
        } else if !asterisk && !e.enabled {
            continue;
        }
        out.push_str(&e.name);
        out.push_str(nl);
    }
    out
}

impl ModList for PluginsTxt {
    fn read(&self, text: &str) -> Vec<PluginEntry> {
        read_lines(text, self.asterisk)
    }

    fn render(&self, original: &str, entries: &[PluginEntry]) -> String {
        render_lines(original, entries, self.asterisk)
    }

    fn entries<'a>(&self, files: &'a [PlannedFile], folder: &RelPath) -> Vec<(&'a str, String)> {
        plugin_files(files, folder)
    }

    /// UMM's entries, and the plugins already in the data folder (the game's own masters and
    /// DLC, which some games add on every start).
    fn tolerated(&self, rendered: &str, _entries: &[PluginEntry], data: &Path) -> Vec<String> {
        let mut t: Vec<String> = rendered
            .lines()
            .map(|l| l.trim().trim_start_matches('*').to_lowercase())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        if let Ok(rd) = fs::read_dir(data) {
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
}

/// What UMM shows as these games' engine.
pub(crate) const LABEL: &str = "Gamebryo / Creation Engine";

/// A Bethesda game keeping `plugins.txt`, recognised by its own exe in the game folder: every
/// edition has it, regional ones (Fallout: New Vegas PCR) included, whichever store it came
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListGame {
    pub exe: &'static str,
    pub list: ListAt,
    /// The folder holding plugins, relative to the game folder.
    pub folder: &'static str,
    /// Editions keeping their `plugins.txt` in another folder under local app data: a file
    /// only that edition has in the game folder, and the folder. As LOOT's libloadorder tells
    /// them apart.
    pub editions: &'static [(&'static str, &'static str)],
    pub format: PluginListFormat,
    /// Where its `Custom.ini` (and `Prefs.ini`) live, under Documents\\My Games, when it needs
    /// ini settings for mods to load.
    pub ini: Option<&'static str>,
}

/// Where a game keeps its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAt {
    /// `plugins.txt` in this folder under the user's local app data.
    LocalAppData(&'static str),
    /// This file in the game folder.
    Game(&'static str),
}

const fn game(
    exe: &'static str,
    dir: &'static str,
    editions: &'static [(&'static str, &'static str)],
    format: PluginListFormat,
    ini: Option<&'static str>,
) -> ListGame {
    ListGame {
        exe,
        list: ListAt::LocalAppData(dir),
        folder: "Data",
        editions,
        format,
        ini,
    }
}

use PluginListFormat::{Asterisk, Plain};

/// Bethesda games keeping `plugins.txt`, VR editions before the flat ones they share files
/// with.
pub const LIST_GAMES: &[ListGame] = &[
    game("SkyrimVR.exe", "Skyrim VR", &[], Asterisk, None),
    game(
        "SkyrimSE.exe",
        "Skyrim Special Edition",
        &[
            // Only GOG's installer installs Galaxy64.dll.
            ("Galaxy64.dll", "Skyrim Special Edition GOG"),
            ("EOSSDK-Win64-Shipping.dll", "Skyrim Special Edition EPIC"),
            ("appxmanifest.xml", "Skyrim Special Edition MS"),
        ],
        Asterisk,
        None,
    ),
    game("Fallout4VR.exe", "Fallout4VR", &[], Asterisk, None),
    game(
        "Fallout4.exe",
        "Fallout4",
        &[
            ("appxmanifest.xml", "Fallout4 MS"),
            ("EOSSDK-Win64-Shipping.dll", "Fallout4 EPIC"),
        ],
        Asterisk,
        Some("Fallout4"),
    ),
    game(
        "Starfield.exe",
        "Starfield",
        &[],
        Asterisk,
        Some("Starfield"),
    ),
    game("TESV.exe", "Skyrim", &[], Plain, None),
    game(
        "FalloutNV.exe",
        "FalloutNV",
        &[("EOSSDK-Win32-Shipping.dll", "FalloutNV_Epic")],
        Plain,
        None,
    ),
    game("Fallout3.exe", "Fallout3", &[], Plain, None),
    game("Oblivion.exe", "Oblivion", &[], Plain, None),
    // Its plugins and their list are in the Unreal project's content folder.
    ListGame {
        exe: "OblivionRemastered.exe",
        list: ListAt::Game("OblivionRemastered/Content/Dev/ObvData/Data/Plugins.txt"),
        folder: "OblivionRemastered/Content/Dev/ObvData/Data",
        editions: &[],
        format: Plain,
        ini: None,
    },
];

/// The game keeping a list whose exe is at the top of a game folder (`exes`, relative paths,
/// as an inspection lists them).
pub fn list_game(exes: &[String]) -> Option<&'static ListGame> {
    LIST_GAMES
        .iter()
        .find(|g| umm_gamedef::kind::has_top_exe(exes, g.exe))
}

impl ListGame {
    /// Its plugin list, for the edition installed in `folder`.
    pub fn plugins(&self, folder: &Path) -> umm_gamedef::Plugins {
        let list = match self.list {
            ListAt::LocalAppData(dir) => {
                let dir = self
                    .editions
                    .iter()
                    .find(|(marker, _)| folder.join(marker).exists())
                    .map_or(dir, |(_, dir)| *dir);
                format!("%LOCALAPPDATA%/{dir}/plugins.txt")
            }
            ListAt::Game(p) => p.into(),
        };
        umm_gamedef::Plugins {
            list,
            format: self.format,
            folder: self.folder.into(),
            // Oblivion set not to use My Games keeps its list in its own folder.
            in_game_folder_when: (self.exe == "Oblivion.exe").then(|| {
                umm_gamedef::GameIniCondition {
                    ini: "Oblivion.ini".into(),
                    section: "General".into(),
                    key: "bUseMyGamesDirectory".into(),
                    value: "0".into(),
                }
            }),
        }
    }

    /// The ini settings it needs for mods to load: Fallout 4 reads its plugin list only with
    /// `bEnableFileSelection`, and Fallout 4 and Starfield load loose files (outside `.ba2`
    /// archives) only with archive invalidation in their `Custom.ini`, a file of overrides UMM
    /// may create.
    pub fn ini(&self) -> Vec<umm_gamedef::IniSetting> {
        let Some(dir) = self.ini else {
            return Vec::new();
        };
        let set = |file: String, section: &str, key: &str, value: &str, create: bool| {
            umm_gamedef::IniSetting {
                file,
                section: section.into(),
                key: key.into(),
                value: value.into(),
                create,
            }
        };
        let base = format!("%DOCUMENTS%/My Games/{dir}");
        let mut v = Vec::new();
        if dir == "Fallout4" {
            v.push(set(
                format!("{base}/Fallout4Prefs.ini"),
                "Launcher",
                "bEnableFileSelection",
                "1",
                false,
            ));
        }
        let custom = format!("{base}/{dir}Custom.ini");
        v.push(set(
            custom.clone(),
            "Archive",
            "bInvalidateOlderFiles",
            "1",
            true,
        ));
        v.push(set(custom, "Archive", "sResourceDataDirsFinal", "", true));
        v
    }
}

/// The folder at the top of a game folder named `Data` or `Data Files` that holds an `.esm`
/// master, as spelled on disk.
pub fn top_data_folder(root: &Path) -> Option<String> {
    let rd = fs::read_dir(root).ok()?;
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.eq_ignore_ascii_case("Data") || n.eq_ignore_ascii_case("Data Files"))
        .find(|n| {
            fs::read_dir(root.join(n)).is_ok_and(|rd| {
                rd.flatten().any(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".esm") && e.path().is_file()
                })
            })
        })
}

/// Install rules for a Bethesda game without a mod loader: plugins, archives and the data
/// folder's usual folders go to the data folder (`Data`); everything else, a mod laid out from
/// the game folder (`Data/…`) included, keeps its path.
pub fn bethesda_rules(data: &str) -> Vec<umm_gamedef::InstallRule> {
    use umm_gamedef::kind::rule;
    use umm_gamedef::{DATA_ARCHIVE_EXTENSIONS, DATA_DIRS, EXTENDER_DIRS};
    let data = data.trim_matches('/');
    let mut v: Vec<umm_gamedef::InstallRule> = PLUGIN_EXTENSIONS
        .iter()
        .chain(DATA_ARCHIVE_EXTENSIONS)
        .map(|e| rule(&format!("*.{e}"), &format!("{data}/")))
        .collect();
    v.extend(
        DATA_DIRS
            .iter()
            .map(|d| rule(&format!("{d}/**"), &format!("{data}/{d}/"))),
    );
    v.extend(
        EXTENDER_DIRS
            .iter()
            .map(|d| rule(&format!("{d}/plugins/**"), &format!("{data}/{d}/plugins/"))),
    );
    v.push(rule("**", ""));
    v
}

/// Without a mod loader: the data folder's rules, for a definition whose plugin list names
/// it; else every file keeps its path.
pub fn own_rules(def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
    umm_gamedef::setup::bethesda_data_folder(def)
        .map_or_else(|| vec![umm_gamedef::kind::rule("**", "")], bethesda_rules)
}

/// What a game with a data folder at its top gets without a mod loader: its rules and a note.
pub(crate) fn data_shape(root: &Path, setup: umm_gamedef::LoaderSetup) -> umm_gamedef::Shape {
    let mut shape = umm_gamedef::Shape::default();
    if let Some(d) = top_data_folder(root).filter(|_| setup == umm_gamedef::LoaderSetup::Generic) {
        shape.install = bethesda_rules(&d);
        shape.notes.push(umm_gamedef::steam::Note::new(
            "bethesda",
            format!(
                "Bethesda game: plugins, archives and asset folders (meshes, textures, \
                 sound, script extender plugins…) go to {d}/, and FOMOD installers \
                 install there. Everything else keeps its path."
            ),
        ));
    }
    shape
}

/// Bethesda's games from Oblivion on (Morrowind is [`crate::games::morrowind`]), recognised by
/// their data folder or their exe.
pub(crate) struct Bethesda;

impl umm_gamedef::GameKind for Bethesda {
    fn key(&self) -> &'static str {
        "bethesda"
    }

    fn recognise(&self, root: &Path, exes: &[String]) -> Option<umm_gamedef::Recognised> {
        let data = top_data_folder(root);
        let listed = list_game(exes);
        if data.is_none() && listed.is_none() {
            return None;
        }
        Some(umm_gamedef::Recognised {
            key: "bethesda",
            label: data.map(|_| LABEL),
            exe: None,
            own_mods: listed.map(|_| umm_gamedef::BUILT_IN_MODS),
        })
    }

    fn shape(
        &self,
        root: &Path,
        found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let mut shape = data_shape(root, setup);
        let Some(g) = list_game(&found.exes) else {
            return shape;
        };
        let plugins = g.plugins(root);
        shape.notes.push(umm_gamedef::steam::Note::new(
            "plugins",
            format!(
                "Bethesda game: while mods are deployed, UMM enables their plugins in {}.",
                plugins.list
            ),
        ));
        shape.ini = g.ini();
        if !shape.ini.is_empty() {
            shape.notes.push(umm_gamedef::steam::Note::new(
                "ini",
                "Bethesda game: UMM sets the ini settings it needs to load mods (listed in this file) while they are deployed, and puts them back afterwards.",
            ));
        }
        shape.plugins = Some(plugins);
        shape
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| matches!(p.format, Asterisk | Plain))
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        own_rules(def)
    }
}

impl crate::games::GameIntegration for Bethesda {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(list: &[(&str, bool)]) -> Vec<PluginEntry> {
        list.iter()
            .map(|(n, enabled)| PluginEntry {
                name: n.to_string(),
                enabled: *enabled,
                owner: None,
            })
            .collect()
    }

    #[test]
    fn reads_and_writes_bethesda_lists() {
        let original = "# This file is used by the game\r\n*Mine.esp\r\nOff.esp\r\n\r\n";
        let e = ASTERISK.read(original);
        assert_eq!(
            (e[0].name.as_str(), e[0].enabled, e[1].enabled),
            ("Mine.esp", true, false)
        );
        let l = entries(&[("Mine.esp", true), ("Off.esp", false)]);
        assert_eq!(
            ASTERISK.render(original, &l),
            "# This file is used by the game\r\n*Mine.esp\r\nOff.esp\r\n"
        );
        assert_eq!(
            PLAIN.render("", &l),
            "Mine.esp\r\n",
            "a plain list names enabled plugins only"
        );
        assert!(rank("a.esm") < rank("b.ESL") && rank("b.esl") < rank("c.esp"));
        assert_eq!(rank("readme.txt"), PLUGIN_EXTENSIONS.len());
    }
}
