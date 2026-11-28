//! r2modman's profile exports, so mod lists move both ways between UMM and r2modman.
//!
//! An export (`.r2z`) is a zip: `export.r2x` (YAML: `profileName`, and `mods` with `name`
//! `Owner-Name`, `version` as `major`/`minor`/`patch`, `enabled`), the profile's
//! `BepInEx/config` folder stored as `config/`, and other config-type files at their path in
//! the profile. A profile code is an export uploaded to Thunderstore as `#r2modman`, a
//! newline and the zip in base64, answered with a key. Matches r2modman's
//! `ProfileModList.createExport` and `ProfileImportExport`.

use crate::net::Http;
use crate::thunderstore::{net, Result, SourceError, BASE_URL};
use crate::Site;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use umm_gamedef::RelPath;

pub const CODE_PREFIX: &str = "#r2modman";
/// r2modman refuses to upload exports larger than this.
pub const MAX_CODE_SIZE: usize = 20_000_000;
/// The file types r2modman copies into exports besides `BepInEx/config`.
pub const CONFIG_EXTENSIONS: &[&str] = &[".cfg", ".txt", ".json", ".yml", ".yaml", ".ini"];
/// Never written from an import, whatever the export holds.
const BLOCKED_EXTENSIONS: &[&str] = &[
    ".dll", ".exe", ".scr", ".com", ".pif", ".bat", ".cmd", ".ps1", ".vbs", ".vbe", ".js", ".jse",
    ".wsf", ".wsh", ".hta", ".msi", ".msix", ".sys", ".drv", ".cpl", ".ocx", ".lnk", ".reg",
    ".inf",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportMod {
    /// `Owner-Name`.
    pub name: String,
    /// `1.2.3`.
    pub version: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Export {
    pub profile_name: String,
    pub mods: Vec<ExportMod>,
    /// Path in the profile (`BepInEx/config/x.cfg`) → contents.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Files in the export that were not taken (unsafe paths or blocked types).
    pub skipped: Vec<String>,
}

/// A YAML scalar: plain when that reads back the same, else double-quoted (as JSON, which
/// YAML reads).
fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '(' | ')'))
        && !s.starts_with([' ', '-'])
        && !s.ends_with(' ')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        )
        && s.parse::<f64>().is_err();
    if plain {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap_or_default()
    }
}

/// `export.r2x`, laid out as r2modman writes it.
fn write_r2x(export: &Export) -> String {
    let mut out = format!(
        "profileName: {}\nmods:\n",
        yaml_scalar(&export.profile_name)
    );
    for m in &export.mods {
        let mut parts = m.version.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        let mut part = || parts.next().unwrap_or(0);
        let (major, minor, patch) = (part(), part(), part());
        out.push_str(&format!(
            "  - name: {}\n    version:\n      major: {major}\n      minor: {minor}\n      \
             patch: {patch}\n    enabled: {}\n",
            yaml_scalar(&m.name),
            m.enabled
        ));
    }
    out
}

/// A YAML scalar's value: unquoted, or plain without a trailing comment.
fn read_scalar(v: &str) -> std::result::Result<String, String> {
    let v = v.trim();
    if v.starts_with('"') {
        // Double-quoted YAML is JSON's string syntax for what r2modman writes.
        let end = v.rfind('"').filter(|&e| e > 0).ok_or("an unclosed quote")?;
        return serde_json::from_str(&v[..=end]).map_err(|e| e.to_string());
    }
    if let Some(rest) = v.strip_prefix('\'') {
        let end = rest.rfind('\'').ok_or("an unclosed quote")?;
        return Ok(rest[..end].replace("''", "'"));
    }
    let v = v.split(" #").next().unwrap_or(v);
    Ok(v.trim().to_string())
}

/// Reads `export.r2x`: the profile's name and its mods. Every key r2modman writes is unique
/// to its level (`profileName`, `mods`, a mod's `name`, `version` and `enabled`, a version's
/// `major`, `minor` and `patch`), so lines are read by key, each `- ` starting a mod. A
/// version written on one line (`{major: 1, minor: 0, patch: 3}`) is read too.
fn read_r2x(text: &str) -> std::result::Result<(String, Vec<ExportMod>), String> {
    struct Partial {
        name: Option<String>,
        version: [Option<u64>; 3],
        enabled: bool,
    }
    let mut profile = None;
    let mut mods: Vec<Partial> = Vec::new();
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t == "---" {
            continue;
        }
        let t = match t.strip_prefix("- ").or_else(|| (t == "-").then_some("")) {
            Some(rest) => {
                mods.push(Partial {
                    name: None,
                    version: [None; 3],
                    enabled: true,
                });
                rest.trim()
            }
            None => t,
        };
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if let Some(flow) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
            for pair in flow.split(',') {
                if let Some((k, v)) = pair.split_once(':') {
                    entries.push((k.trim().into(), v.trim().into()));
                }
            }
        }
        entries.push((key.trim().into(), value.into()));
        for (key, value) in entries.drain(..) {
            let number = || {
                read_scalar(&value)?
                    .parse::<u64>()
                    .map_err(|e| e.to_string())
            };
            let current = mods.last_mut();
            match (key.as_str(), current) {
                ("profileName", _) => profile = Some(read_scalar(&value)?),
                ("name", Some(m)) => m.name = Some(read_scalar(&value)?),
                ("enabled", Some(m)) => m.enabled = read_scalar(&value)? != "false",
                ("major", Some(m)) => m.version[0] = Some(number()?),
                ("minor", Some(m)) => m.version[1] = Some(number()?),
                ("patch", Some(m)) => m.version[2] = Some(number()?),
                _ => {}
            }
        }
    }
    let profile = profile.ok_or("no profileName")?;
    let mods = mods
        .into_iter()
        .map(|m| match (m.name, m.version) {
            (Some(name), [Some(a), Some(b), Some(c)]) => Ok(ExportMod {
                name,
                version: format!("{a}.{b}.{c}"),
                enabled: m.enabled,
            }),
            (name, _) => Err(format!(
                "a mod without a name or a full version ({})",
                name.unwrap_or_default()
            )),
        })
        .collect::<std::result::Result<_, _>>()?;
    Ok((profile, mods))
}

fn bad(what: &str, m: impl std::fmt::Display) -> SourceError {
    SourceError::Data {
        site: Site::Thunderstore,
        what: what.into(),
        message: m.to_string(),
    }
}

/// Whether a profile file belongs in an export.
pub fn is_config_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    CONFIG_EXTENSIONS.iter().any(|e| lower.ends_with(e)) && !lower.ends_with("mods.yml")
}

/// Builds the `.r2z` zip. `files` are paths in the profile; `BepInEx/config/...` is stored
/// as `config/...`, as r2modman does.
pub fn build_r2z(export: &Export) -> Result<Vec<u8>> {
    let yaml = write_r2x(export);
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let zerr = |e: zip::result::ZipError| bad("profile export", e);
    w.start_file("export.r2x", opts).map_err(zerr)?;
    w.write_all(yaml.as_bytes())
        .map_err(|e| bad("profile export", e))?;
    for (path, body) in &export.files {
        let name = match path.strip_prefix("BepInEx/config/") {
            Some(rest) => format!("config/{rest}"),
            None => path.clone(),
        };
        w.start_file(name, opts).map_err(zerr)?;
        w.write_all(body).map_err(|e| bad("profile export", e))?;
    }
    Ok(w.finish().map_err(zerr)?.into_inner())
}

/// Reads a `.r2z`: its mod list, and the files to copy into the profile.
pub fn read_r2z(bytes: &[u8]) -> Result<Export> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| bad("profile export", e))?;
    let mut out = Export::default();
    let mut r2x = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| bad("profile export", e))?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().replace('\\', "/");
        let mut body = Vec::new();
        f.read_to_end(&mut body)
            .map_err(|e| bad("profile export", e))?;
        let lower = name.to_lowercase();
        if lower == "export.r2x" {
            let text = String::from_utf8_lossy(&body);
            r2x = Some(read_r2x(&text).map_err(|e| bad("export.r2x", e))?);
            continue;
        }
        if lower == "mods.yml" {
            continue;
        }
        if BLOCKED_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
            out.skipped.push(name);
            continue;
        }
        let path = match name.strip_prefix("config/") {
            Some(rest) => format!("BepInEx/config/{rest}"),
            None => name.clone(),
        };
        match RelPath::parse(&path) {
            Ok(p) if !p.is_root() => {
                out.files.insert(p.as_str().to_string(), body);
            }
            _ => out.skipped.push(name),
        }
    }
    let (profile_name, mods) = r2x.ok_or_else(|| bad("profile export", "no export.r2x in it"))?;
    out.profile_name = profile_name;
    out.mods = mods;
    Ok(out)
}

/// Uploads an export and returns its profile code.
pub fn upload(http: &dyn Http, r2z: &[u8]) -> Result<String> {
    if r2z.len() > MAX_CODE_SIZE {
        return Err(SourceError::Invalid(format!(
            "the export is {} MB; profile codes are limited to 20 MB (save it as a file instead)",
            r2z.len() / 1_000_000
        )));
    }
    let body = format!("{CODE_PREFIX}\n{}", crate::base64::encode(r2z));
    let resp = http
        .post(
            &format!("{BASE_URL}/api/experimental/legacyprofile/create/"),
            "application/octet-stream",
            body.as_bytes(),
        )
        .map_err(net)?;
    #[derive(Deserialize)]
    struct Key {
        key: String,
    }
    let k: Key = serde_json::from_slice(&resp).map_err(|e| bad("profile code", e))?;
    Ok(k.key)
}

/// Downloads the export behind a profile code.
pub fn download(http: &dyn Http, code: &str) -> Result<Vec<u8>> {
    let code = code.trim();
    if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(SourceError::Invalid(format!(
            "`{code}` does not look like a profile code"
        )));
    }
    let resp = http
        .get(&format!(
            "{BASE_URL}/api/experimental/legacyprofile/get/{code}/"
        ))
        .map_err(net)?;
    let text = String::from_utf8_lossy(&resp);
    let b64 = text
        .strip_prefix(CODE_PREFIX)
        .ok_or_else(|| bad("profile code", "not an r2modman profile"))?
        .trim();
    crate::base64::decode(b64).ok_or_else(|| bad("profile code", "not base64"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeHttp;

    fn sample() -> Export {
        Export {
            profile_name: "Default".into(),
            mods: vec![
                ExportMod {
                    name: "BepInEx-BepInExPack".into(),
                    version: "5.4.2100".into(),
                    enabled: true,
                },
                ExportMod {
                    name: "Ann-Lantern_Tweaks".into(),
                    version: "1.2.0".into(),
                    enabled: false,
                },
            ],
            files: BTreeMap::from([
                (
                    "BepInEx/config/lanterns.cfg".to_string(),
                    b"bright=true".to_vec(),
                ),
                (
                    "BepInEx/plugins/Ann-Lantern_Tweaks/colors.json".to_string(),
                    b"{}".to_vec(),
                ),
            ]),
            skipped: vec![],
        }
    }

    #[test]
    fn round_trips_through_an_r2z() {
        let zip = build_r2z(&sample()).unwrap();
        let back = read_r2z(&zip).unwrap();
        assert_eq!(back.profile_name, "Default");
        assert_eq!(back.mods, sample().mods);
        assert_eq!(back.files, sample().files);

        // The YAML is laid out as r2modman writes it.
        let mut z = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        let mut yaml = String::new();
        z.by_name("export.r2x")
            .unwrap()
            .read_to_string(&mut yaml)
            .unwrap();
        assert!(yaml.contains("profileName: Default"), "{yaml}");
        assert!(yaml.contains("major: 5"), "{yaml}");
        assert!(z.by_name("config/lanterns.cfg").is_ok());
    }

    #[test]
    fn reads_r2modman_yaml_and_skips_unsafe_files() {
        let yaml = "profileName: Friends\nmods:\n  - name: A-B\n    version:\n      major: 1\n      minor: 0\n      patch: 3\n    enabled: true\n  - name: C-D\n    version:\n      major: 2\n      minor: 1\n      patch: 0\n";
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, b) in [
            ("export.r2x", yaml),
            ("config/a.cfg", "x"),
            ("evil.dll", "x"),
            ("../escape.cfg", "x"),
        ] {
            w.start_file(n, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b.as_bytes()).unwrap();
        }
        let e = read_r2z(&w.finish().unwrap().into_inner()).unwrap();
        assert_eq!(e.profile_name, "Friends");
        assert_eq!(e.mods[0].version, "1.0.3");
        assert!(e.mods[1].enabled, "enabled when not stated");
        assert_eq!(e.files.keys().collect::<Vec<_>>(), ["BepInEx/config/a.cfg"]);
        assert_eq!(e.skipped.len(), 2);
    }

    #[test]
    fn r2x_names_that_yaml_reads_otherwise_are_quoted() {
        let mut e = sample();
        e.profile_name = "Run #2: \"hard\"".into();
        e.mods[0].name = "true".into();
        let yaml = write_r2x(&e);
        assert!(
            yaml.starts_with("profileName: \"Run #2: \\\"hard\\\"\"\n"),
            "{yaml}"
        );
        assert!(yaml.contains("  - name: \"true\"\n"), "{yaml}");
        let (name, mods) = read_r2x(&yaml).unwrap();
        assert_eq!(name, e.profile_name);
        assert_eq!(mods, e.mods);
        // Single quotes, comments, a flow version, a list not indented under its key.
        let other = "# made elsewhere\nprofileName: 'It''s mine' # a comment\nmods:\n- name: A-B\n  version: {major: 1, minor: 2, patch: 3}\n  enabled: false\n";
        let (name, mods) = read_r2x(other).unwrap();
        assert_eq!(name, "It's mine");
        assert_eq!(
            mods,
            [ExportMod {
                name: "A-B".into(),
                version: "1.2.3".into(),
                enabled: false
            }]
        );
        assert!(read_r2x("mods:\n  - name: A\n").is_err());
        assert!(read_r2x("profileName: x\nmods:\n  - name: A\n").is_err());
    }

    #[test]
    fn codes_wrap_the_zip_in_base64() {
        let zip = build_r2z(&sample()).unwrap();
        let body = format!("{CODE_PREFIX}\n{}", crate::base64::encode(&zip));
        let http = FakeHttp::default()
            .with(
                "https://thunderstore.io/api/experimental/legacyprofile/get/0192-abc/",
                body,
            )
            .with(
                "https://thunderstore.io/api/experimental/legacyprofile/create/",
                r#"{"key":"0192-abc"}"#,
            );
        assert_eq!(download(&http, " 0192-abc ").unwrap(), zip);
        assert_eq!(upload(&http, &zip).unwrap(), "0192-abc");
        assert!(download(&http, "../x").is_err());
    }
}
