//! Morrowind: its plugins load when `Morrowind.ini` lists them as `GameFile0=`, `GameFile1=`…
//! in its `[Game Files]` section. The game loads them in the order of their file dates, which
//! UMM does not set.

use crate::games::bethesda::plugin_files;
use crate::games::plugins::{eol, header, ini_section, ModList, PluginEntry};
use std::fmt::Write;
use umm_deploy::PlannedFile;
use umm_gamedef::RelPath;

pub(crate) struct Morrowind;

const GAME_FILES: &str = "Game Files";

/// A `GameFileN=` line's value.
fn game_file(line: &str) -> Option<&str> {
    let (k, v) = line.trim().split_once('=')?;
    let k = k.trim();
    let digits = k
        .get(8..)
        .filter(|_| k[..8].eq_ignore_ascii_case("GameFile"))?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| v.trim())
}

impl ModList for Morrowind {
    fn kind(&self) -> &'static str {
        "dated"
    }

    /// The ini is in the system code page: it is read byte for byte, so the rest of the file
    /// is written back exactly as it was.
    fn decode(&self, bytes: &[u8]) -> String {
        bytes.iter().map(|&b| b as char).collect()
    }

    fn encode(&self, text: &str) -> Vec<u8> {
        text.chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect()
    }

    fn read(&self, text: &str) -> Vec<PluginEntry> {
        let mut section = "";
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(h) = header(line) {
                section = h;
            } else if section.eq_ignore_ascii_case(GAME_FILES) {
                if let Some(v) = game_file(line).filter(|v| !v.is_empty()) {
                    out.push(PluginEntry {
                        name: v.to_string(),
                        enabled: true,
                        owner: None,
                    });
                }
            }
        }
        out
    }

    fn render(&self, original: &str, entries: &[PluginEntry]) -> String {
        let nl = eol(original);
        let mut lines = String::new();
        for (i, e) in entries.iter().filter(|e| e.enabled).enumerate() {
            let _ = write!(lines, "GameFile{i}={}{nl}", e.name);
        }
        ini_section(original, GAME_FILES, |l| game_file(l).is_some(), &lines, nl)
    }

    fn entries<'a>(&self, files: &'a [PlannedFile], folder: &RelPath) -> Vec<(&'a str, String)> {
        plugin_files(files, folder)
    }
}

const EXE: &str = "Morrowind.exe";
const DATA: &str = "Data Files";

impl umm_gamedef::GameKind for Morrowind {
    fn key(&self) -> &'static str {
        "morrowind"
    }

    fn recognise(
        &self,
        root: &std::path::Path,
        exes: &[String],
    ) -> Option<umm_gamedef::Recognised> {
        umm_gamedef::kind::has_top_exe(exes, EXE).then(|| umm_gamedef::Recognised {
            key: "morrowind",
            label: crate::games::bethesda::top_data_folder(root)
                .map(|_| crate::games::bethesda::LABEL),
            exe: None,
            own_mods: Some(umm_gamedef::BUILT_IN_MODS),
        })
    }

    fn shape(
        &self,
        root: &std::path::Path,
        _found: &umm_gamedef::steam::Inspection,
        _exe: &str,
        setup: umm_gamedef::LoaderSetup,
    ) -> umm_gamedef::Shape {
        let mut shape = crate::games::bethesda::data_shape(root, setup);
        shape.notes.push(umm_gamedef::steam::Note::new(
            "plugins",
            "Morrowind: while mods are deployed, UMM enables their plugins in the [Game Files] section of Morrowind.ini. The game loads plugins in the order of their file dates.",
        ));
        shape.plugins = Some(umm_gamedef::Plugins {
            list: "Morrowind.ini".into(),
            format: umm_gamedef::PluginListFormat::MorrowindIni,
            folder: DATA.into(),
            in_game_folder_when: None,
        });
        shape
    }

    fn applies(&self, def: &umm_gamedef::GameDef) -> bool {
        def.plugins
            .as_ref()
            .is_some_and(|p| p.format == umm_gamedef::PluginListFormat::MorrowindIni)
    }

    fn own_rules(&self, def: &umm_gamedef::GameDef) -> Vec<umm_gamedef::InstallRule> {
        crate::games::bethesda::own_rules(def)
    }
}

impl crate::games::GameIntegration for Morrowind {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morrowind_lists_its_game_files_in_its_ini() {
        // Bytes outside UTF-8 elsewhere in the file come back as they were.
        let original: &[u8] = b"[General]\r\nName=Caf\xe9\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n";
        let text = Morrowind.decode(original);
        let e = Morrowind.read(&text);
        assert_eq!(
            e.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["Morrowind.esm", "Tribunal.esm"]
        );
        let l: Vec<PluginEntry> = ["Morrowind.esm", "Tribunal.esm", "Mine.esp"]
            .iter()
            .map(|n| PluginEntry {
                name: n.to_string(),
                enabled: true,
                owner: None,
            })
            .collect();
        assert_eq!(
            Morrowind.encode(&Morrowind.render(&text, &l)),
            b"[General]\r\nName=Caf\xe9\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=Mine.esp\r\n\r\n[Archives]\r\nArchive 0=Tribunal.bsa\r\n"
        );
        // An ini without the section gets one.
        assert_eq!(
            Morrowind.render("[General]\r\nA=1", &l),
            "[General]\r\nA=1\r\n[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=Mine.esp\r\n"
        );
        assert_eq!(game_file("GameFile12 = x.esp"), Some("x.esp"));
        assert_eq!(game_file("GameFiles=x"), None);
        assert_eq!(game_file("Game"), None);
    }
}
