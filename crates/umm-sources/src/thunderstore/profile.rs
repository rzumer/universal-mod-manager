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
use base64::Engine as _;
use serde::{Deserialize, Serialize};
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct R2x {
    profile_name: String,
    #[serde(default)]
    mods: Vec<R2xMod>,
}

#[derive(Serialize, Deserialize)]
struct R2xMod {
    name: String,
    version: R2xVersion,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct R2xVersion {
    major: u64,
    minor: u64,
    patch: u64,
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
    let r2x = R2x {
        profile_name: export.profile_name.clone(),
        mods: export
            .mods
            .iter()
            .map(|m| {
                let mut parts = m.version.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
                R2xMod {
                    name: m.name.clone(),
                    version: R2xVersion {
                        major: parts.next().unwrap_or(0),
                        minor: parts.next().unwrap_or(0),
                        patch: parts.next().unwrap_or(0),
                    },
                    enabled: m.enabled,
                }
            })
            .collect(),
    };
    let yaml = serde_yaml::to_string(&r2x).map_err(|e| bad("export.r2x", e))?;
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
    let mut r2x: Option<R2x> = None;
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
            r2x = Some(serde_yaml::from_str(&text).map_err(|e| bad("export.r2x", e))?);
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
    let r2x = r2x.ok_or_else(|| bad("profile export", "no export.r2x in it"))?;
    out.profile_name = r2x.profile_name;
    out.mods = r2x
        .mods
        .into_iter()
        .map(|m| ExportMod {
            name: m.name,
            version: format!(
                "{}.{}.{}",
                m.version.major, m.version.minor, m.version.patch
            ),
            enabled: m.enabled,
        })
        .collect();
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
    let body = format!(
        "{CODE_PREFIX}\n{}",
        base64::engine::general_purpose::STANDARD.encode(r2z)
    );
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
    base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| bad("profile code", e))
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
    fn codes_wrap_the_zip_in_base64() {
        let zip = build_r2z(&sample()).unwrap();
        let body = format!(
            "{CODE_PREFIX}\n{}",
            base64::engine::general_purpose::STANDARD.encode(&zip)
        );
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
