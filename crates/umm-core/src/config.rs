//! Mods' config files, to read and edit from UMM: those the mod loader keeps in a profile's
//! runtime folder (`BepInEx/config/*.cfg`, MelonLoader's `UserData/`, …). Plugins create
//! them on the game's first run; UMM's sync leaves files the user edited alone, so edits made
//! here stay.
//!
//! BepInEx's files get a structured view: each setting with its description, type, default
//! and the values it accepts. Saving one only rewrites the lines of the settings changed.

use crate::{CoreError, Result, Session};
use globset::{GlobBuilder, GlobSetBuilder};
use std::fs;
use std::path::{Path, PathBuf};
use umm_gamedef::RelPath;
use umm_store::fsutil::{write_atomic, IoCtx};

/// A config file in a profile's runtime folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    /// Relative to the runtime folder, `/`-separated.
    pub path: String,
    /// The plugin that made it (from a BepInEx file's header), else the file's name.
    pub name: String,
    pub version: Option<String>,
    pub guid: Option<String>,
    pub size: u64,
    pub modified_unix: u64,
    /// In BepInEx's format: editable setting by setting.
    pub bepinex: bool,
}

/// A config file's content.
#[derive(Debug, Clone)]
pub struct ConfigView {
    pub file: ConfigFile,
    pub text: String,
    /// Its settings, for a BepInEx file.
    pub sections: Option<Vec<ConfigSection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSection {
    pub name: String,
    pub entries: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub description: String,
    /// `Boolean`, `Int32`, `Single`, `String`, an enum's name…
    pub kind: Option<String>,
    pub default: Option<String>,
    /// The values it accepts (an enum's, or a list the plugin gave).
    pub acceptable: Vec<String>,
    /// The range it accepts, inclusive.
    pub range: Option<(String, String)>,
}

/// A setting to change.
#[derive(Debug, Clone)]
pub struct ConfigChange {
    pub section: String,
    pub key: String,
    pub value: String,
}

/// The loader's config folders, as the game definition's writable paths name them (not the
/// catch-all `**/*.cfg` kind, which would list packages' manifests).
fn config_globs(def: &umm_gamedef::CompiledDef) -> Result<globset::GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in &def.def.writable.paths {
        if p.starts_with("**") {
            continue;
        }
        let g = GlobBuilder::new(p)
            .literal_separator(true)
            .case_insensitive(true)
            .build()
            .map_err(|e| CoreError::Invalid(e.to_string()))?;
        b.add(g);
    }
    b.build().map_err(|e| CoreError::Invalid(e.to_string()))
}

/// Whether a file in a loader's config folder holds settings: not a log, and not in a
/// folder of dumps or logs (Lovely's `mods/lovely/dump/` holds the game's code as patched,
/// for debugging; `log/` its logs).
fn is_settings(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    let mut parts: Vec<&str> = lower.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    !name.ends_with(".log")
        && !parts
            .iter()
            .any(|d| matches!(*d, "dump" | "dumps" | "log" | "logs" | "cache"))
}

/// A BepInEx file's header: `## Settings file was created by plugin <name> v<version>` and
/// `## Plugin GUID: <guid>`.
fn bepinex_header(text: &str) -> Option<(String, Option<String>, Option<String>)> {
    let mut name = None;
    let mut guid = None;
    for line in text.lines().take(6) {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("## Settings file was created by plugin ") {
            name = Some(match rest.rsplit_once(" v") {
                Some((n, v)) => (n.trim().to_string(), Some(v.trim().to_string())),
                None => (rest.trim().to_string(), None),
            });
        } else if let Some(g) = l.strip_prefix("## Plugin GUID:") {
            guid = Some(g.trim().to_string());
        }
    }
    name.map(|(n, v)| (n, v, guid))
}

/// Whether a file is in BepInEx's config format: a `.cfg` of `[Section]`s and `Key = Value`
/// settings with `#` comments.
fn is_bepinex(path: &str, text: &str) -> bool {
    path.to_lowercase().ends_with(".cfg")
        && text.lines().any(|l| {
            let l = l.trim();
            l.starts_with('[') && l.ends_with(']')
        })
}

/// A BepInEx file's settings, section by section.
pub fn parse_bepinex(text: &str) -> Vec<ConfigSection> {
    let mut sections: Vec<ConfigSection> = Vec::new();
    let mut pending = ConfigEntry {
        key: String::new(),
        value: String::new(),
        description: String::new(),
        kind: None,
        default: None,
        acceptable: Vec::new(),
        range: None,
    };
    let reset = |e: &mut ConfigEntry| {
        e.description.clear();
        e.kind = None;
        e.default = None;
        e.acceptable.clear();
        e.range = None;
    };
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            sections.push(ConfigSection {
                name: l[1..l.len() - 1].trim().to_string(),
                entries: Vec::new(),
            });
            reset(&mut pending);
        } else if let Some(d) = l.strip_prefix("##") {
            // The file's own header comes before any section: not a setting's description.
            if sections.is_empty() {
                continue;
            }
            if !pending.description.is_empty() {
                pending.description.push('\n');
            }
            pending.description.push_str(d.trim());
        } else if let Some(c) = l.strip_prefix('#') {
            let c = c.trim();
            if let Some(t) = c.strip_prefix("Setting type:") {
                pending.kind = Some(t.trim().to_string());
            } else if let Some(d) = c.strip_prefix("Default value:") {
                pending.default = Some(d.trim().to_string());
            } else if let Some(v) = c.strip_prefix("Acceptable values:") {
                pending.acceptable = v.split(',').map(|s| s.trim().to_string()).collect();
            } else if let Some(r) = c.strip_prefix("Acceptable value range:") {
                // "From 0 to 100"
                let r = r.trim().trim_start_matches("From").trim();
                if let Some((a, b)) = r.split_once(" to ") {
                    pending.range = Some((a.trim().to_string(), b.trim().to_string()));
                }
            }
        } else if let Some((k, v)) = l.split_once('=') {
            let Some(section) = sections.last_mut() else {
                continue;
            };
            let mut e = pending.clone();
            e.key = k.trim().to_string();
            e.value = v.trim().to_string();
            section.entries.push(e);
            reset(&mut pending);
        }
    }
    sections
}

/// The text with these settings set, every other line (comments, spacing, settings not
/// changed) as it was. A setting not in the file is an error.
pub fn set_bepinex(text: &str, changes: &[ConfigChange]) -> Result<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut done = vec![false; changes.len()];
    let mut section = String::new();
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].trim().to_string();
        } else if !l.starts_with('#') {
            if let Some((k, _)) = line.split_once('=') {
                let key = k.trim();
                if let Some(i) = changes
                    .iter()
                    .position(|c| c.section == section && c.key == key)
                {
                    out.push(format!("{} = {}", k.trim_end(), changes[i].value));
                    done[i] = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if let Some(i) = done.iter().position(|d| !d) {
        return Err(CoreError::Invalid(format!(
            "this file has no setting {} in [{}]",
            changes[i].key, changes[i].section
        )));
    }
    let mut s = out.join(newline);
    if text.ends_with('\n') {
        s.push_str(newline);
    }
    Ok(s)
}

impl Session {
    /// The profile's runtime folder, where its loader keeps its configs.
    pub fn config_dir(&self, game_id: &str, profile: &str) -> PathBuf {
        self.data.game(game_id).runtime_dir(profile)
    }

    /// The profile's config files, by name. Empty when the game has not run with the
    /// profile yet (plugins create their files then) or keeps no configs there.
    pub fn config_files(&self, game_id: &str, profile: &str) -> Result<Vec<ConfigFile>> {
        let def = self.def(game_id)?;
        let dir = self.config_dir(game_id, profile);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let globs = config_globs(&def)?;
        let mut out = Vec::new();
        for e in walkdir::WalkDir::new(&dir)
            .min_depth(1)
            .into_iter()
            .flatten()
        {
            if !e.file_type().is_file() {
                continue;
            }
            let Ok(rel) = e.path().strip_prefix(&dir) else {
                continue;
            };
            let Ok(rel) = RelPath::from_path(rel) else {
                continue;
            };
            if !globs.is_match(rel.as_str()) || !is_settings(rel.as_str()) {
                continue;
            }
            out.push(describe(e.path(), rel.as_str()));
        }
        out.sort_by_key(|f| (f.name.to_lowercase(), f.path.to_lowercase()));
        Ok(out)
    }

    /// A config file's path, checked: inside the runtime folder, and one of its configs.
    fn config_path(&self, game_id: &str, profile: &str, path: &str) -> Result<PathBuf> {
        let def = self.def(game_id)?;
        let rel = RelPath::parse(path).map_err(|e| CoreError::Invalid(e.to_string()))?;
        if !config_globs(&def)?.is_match(rel.as_str()) || !is_settings(rel.as_str()) {
            return Err(CoreError::Invalid(format!("{path} is not a config file")));
        }
        Ok(rel.to_path(&self.config_dir(game_id, profile)))
    }

    pub fn read_config(&self, game_id: &str, profile: &str, path: &str) -> Result<ConfigView> {
        let p = self.config_path(game_id, profile, path)?;
        let bytes = fs::read(&p).ctx(|| format!("reading {}", p.display()))?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let file = describe(&p, path);
        let sections = file.bepinex.then(|| parse_bepinex(&text));
        Ok(ConfigView {
            file,
            text,
            sections,
        })
    }

    /// Sets settings of a BepInEx config file, leaving the rest of it as it was.
    pub fn set_config_values(
        &self,
        game_id: &str,
        profile: &str,
        path: &str,
        changes: &[ConfigChange],
    ) -> Result<ConfigView> {
        if changes.iter().any(|c| c.value.contains(['\r', '\n'])) {
            return Err(CoreError::Invalid("a setting's value is one line".into()));
        }
        let p = self.config_path(game_id, profile, path)?;
        let text = fs::read_to_string(&p).ctx(|| format!("reading {}", p.display()))?;
        let new = set_bepinex(&text, changes)?;
        write_config(&p, &new)?;
        self.read_config(game_id, profile, path)
    }

    /// Replaces a config file's text.
    pub fn set_config_text(
        &self,
        game_id: &str,
        profile: &str,
        path: &str,
        text: &str,
    ) -> Result<ConfigView> {
        let p = self.config_path(game_id, profile, path)?;
        if !p.is_file() {
            return Err(CoreError::Invalid(format!("{path} does not exist")));
        }
        write_config(&p, text)?;
        self.read_config(game_id, profile, path)
    }
}

/// Written through a temporary file: never into a file shared with the mod's stored copy
/// (a hard link), which the rename leaves as it was.
fn write_config(p: &Path, text: &str) -> Result<()> {
    write_atomic(p, text.as_bytes()).ctx(|| format!("writing {}", p.display()))?;
    Ok(())
}

fn describe(p: &Path, rel: &str) -> ConfigFile {
    let meta = fs::metadata(p).ok();
    let text = fs::read(p)
        .ok()
        .map(|b| String::from_utf8_lossy(&b[..b.len().min(64 * 1024)]).into_owned())
        .unwrap_or_default();
    let bepinex = is_bepinex(rel, &text);
    let file_name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let (name, version, guid) = bepinex_header(&text)
        .filter(|_| bepinex)
        .unwrap_or((file_name, None, None));
    ConfigFile {
        path: rel.to_string(),
        name,
        version,
        guid,
        size: meta.as_ref().map_or(0, |m| m.len()),
        modified_unix: meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs()),
        bepinex,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "## Settings file was created by plugin Better UI v2.1.0\r\n\
## Plugin GUID: com.example.betterui\r\n\
\r\n\
[General]\r\n\
\r\n\
## Show the clock.\r\n\
## Top right.\r\n\
# Setting type: Boolean\r\n\
# Default value: true\r\n\
Show Clock = true\r\n\
\r\n\
## How big.\r\n\
# Setting type: Single\r\n\
# Default value: 1\r\n\
# Acceptable value range: From 0.5 to 2\r\n\
Scale = 1.25\r\n\
\r\n\
[Colors]\r\n\
\r\n\
# Setting type: Theme\r\n\
# Default value: Dark\r\n\
# Acceptable values: Dark, Light, System\r\n\
Theme = Dark\r\n";

    #[test]
    fn reads_bepinex_settings() {
        assert_eq!(
            bepinex_header(CFG),
            Some((
                "Better UI".into(),
                Some("2.1.0".into()),
                Some("com.example.betterui".into())
            ))
        );
        let s = parse_bepinex(CFG);
        assert_eq!(s.len(), 2);
        let clock = &s[0].entries[0];
        assert_eq!(
            (
                clock.key.as_str(),
                clock.value.as_str(),
                clock.kind.as_deref()
            ),
            ("Show Clock", "true", Some("Boolean"))
        );
        assert_eq!(clock.description, "Show the clock.\nTop right.");
        assert_eq!(s[0].entries[1].range, Some(("0.5".into(), "2".into())));
        assert_eq!(s[1].entries[0].acceptable, ["Dark", "Light", "System"]);
        assert_eq!(s[1].entries[0].description, "");
    }

    #[test]
    fn sets_only_the_lines_changed() {
        let change = |section: &str, key: &str, value: &str| ConfigChange {
            section: section.into(),
            key: key.into(),
            value: value.into(),
        };
        let out = set_bepinex(
            CFG,
            &[
                change("General", "Scale", "1.5"),
                change("Colors", "Theme", "Light"),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            CFG.replace("Scale = 1.25", "Scale = 1.5")
                .replace("Theme = Dark", "Theme = Light")
        );
        assert!(set_bepinex(CFG, &[change("Colors", "Scale", "1")]).is_err());
    }
}
