//! Lays a Thunderstore package's zip out relative to the game folder, the way r2modman does,
//! so the stored mod deploys without further install rules.
//!
//! - A mod loader package (the BepInEx pack): the contents of its root folder
//!   (`BepInExPack/`) go to the game folder as they are.
//! - Any other package: a folder named like a route's last part (`plugins`, `config`, ...)
//!   goes to that route; other folders are looked into; a loose file goes to the route whose
//!   extension it has (longest match), else to the default route. `subdir` routes put the
//!   package's files under `<route>/<Owner-Name>/`.

use crate::ecosystem::{InstallRule, ModloaderPackage, TrackingMethod};
use crate::{io_err, Result, TsError};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use umm_gamedef::RelPath;

/// Files every package has at its root, which never go to the game.
const PACKAGE_FILES: &[&str] = &["manifest.json", "readme.md", "changelog.md", "icon.png"];

/// A package's `manifest.json`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
struct Route {
    path: String,
    last: String,
    extensions: Vec<String>,
    tracking: TrackingMethod,
    default: bool,
}

fn flatten(rules: &[InstallRule], prefix: &str, out: &mut Vec<Route>) {
    for r in rules {
        let path = if prefix.is_empty() {
            r.route.trim_matches('/').to_string()
        } else {
            format!("{prefix}/{}", r.route.trim_matches('/'))
        };
        out.push(Route {
            last: path.rsplit('/').next().unwrap_or(&path).to_lowercase(),
            path: path.clone(),
            extensions: r
                .default_file_extensions
                .iter()
                .map(|e| e.to_lowercase())
                .collect(),
            tracking: r.tracking_method,
            default: r.is_default_location,
        });
        flatten(&r.sub_routes, &path, out);
    }
}

/// The zip's files by their path inside it, `/`-separated, unsafe paths refused.
fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let bad = |m: String| TsError::Data {
        what: "package zip".into(),
        message: m,
    };
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| bad(e.to_string()))?;
    let mut files = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().replace('\\', "/");
        let rel = RelPath::parse(&name).map_err(|e| bad(format!("{name}: {e}")))?;
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).map_err(|e| bad(e.to_string()))?;
        files.insert(rel.as_str().to_string(), buf);
    }
    Ok(files)
}

/// The manifest at the zip's root, if any.
pub fn manifest(zip: &[u8]) -> Result<Option<Manifest>> {
    let files = read_zip(zip)?;
    let Some((_, body)) = files
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("manifest.json"))
    else {
        return Ok(None);
    };
    // Some manifests start with a byte-order mark.
    let text = String::from_utf8_lossy(body);
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).map(Some).map_err(|e| TsError::Data {
        what: "manifest.json".into(),
        message: e.to_string(),
    })
}

/// Where each file of the package goes, relative to the game folder.
pub fn plan(
    zip: &[u8],
    full_name: &str,
    rules: &[InstallRule],
    loader: Option<&ModloaderPackage>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let files = read_zip(zip)?;
    let mut out = BTreeMap::new();

    if let Some(l) = loader {
        let root = format!("{}/", l.root_folder.trim_matches('/').to_lowercase());
        for (path, body) in files {
            if path.to_lowercase().starts_with(&root) {
                out.insert(path[root.len()..].to_string(), body);
            }
        }
        if out.is_empty() {
            return Err(TsError::Data {
                what: format!("{full_name} (mod loader)"),
                message: format!("no `{}` folder in the package", l.root_folder),
            });
        }
        return Ok(out);
    }

    let mut routes = Vec::new();
    flatten(rules, "", &mut routes);
    if routes.is_empty() {
        return Err(TsError::Invalid(
            "this game's install rules are missing from Thunderstore's data".into(),
        ));
    }
    let default = routes
        .iter()
        .find(|r| r.default)
        .or(routes.first())
        .cloned()
        .expect("routes is not empty");

    let dest = |route: &Route, rel: &str| -> Result<String> {
        match route.tracking {
            TrackingMethod::Subdir | TrackingMethod::SubdirNoFlatten => {
                Ok(format!("{}/{full_name}/{rel}", route.path))
            }
            TrackingMethod::None | TrackingMethod::State => Ok(format!("{}/{rel}", route.path)),
            TrackingMethod::PackageZip | TrackingMethod::Unknown => Err(TsError::Invalid(format!(
                "{full_name}: this game installs packages to {} in a way UMM does not support yet",
                route.path
            ))),
        }
    };

    for (path, body) in files {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() == 1 && PACKAGE_FILES.contains(&parts[0].to_lowercase().as_str()) {
            continue;
        }
        // The first folder on the way down that names a route takes the rest of the path.
        let routed = parts[..parts.len() - 1]
            .iter()
            .enumerate()
            .find_map(|(i, dir)| {
                let d = dir.to_lowercase();
                routes
                    .iter()
                    .find(|r| r.last == d)
                    .map(|r| (r, parts[i + 1..].join("/")))
            });
        let target = match routed {
            Some((route, rel)) => dest(route, &rel)?,
            None => {
                let file = parts[parts.len() - 1];
                let lower = file.to_lowercase();
                let route = routes
                    .iter()
                    .flat_map(|r| r.extensions.iter().map(move |e| (r, e)))
                    .filter(|(_, e)| lower.ends_with(e.as_str()))
                    .max_by_key(|(_, e)| e.len())
                    .map(|(r, _)| r)
                    .unwrap_or(&default);
                let rel = if route.tracking == TrackingMethod::SubdirNoFlatten {
                    path.clone()
                } else {
                    file.to_string()
                };
                dest(route, &rel)?
            }
        };
        out.insert(target, body);
    }
    if out.is_empty() {
        return Err(TsError::Data {
            what: full_name.into(),
            message: "the package has no files to install".into(),
        });
    }
    Ok(out)
}

/// Writes planned files under `dir`.
pub fn write(files: &BTreeMap<String, Vec<u8>>, dir: &Path) -> Result<()> {
    for (rel, body) in files {
        let rel = RelPath::parse(rel).map_err(|e| TsError::Invalid(format!("{rel}: {e}")))?;
        let p = rel.to_path(dir);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(io_err(format!("creating {}", parent.display())))?;
        }
        fs::write(&p, body).map_err(io_err(format!("writing {}", p.display())))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ecosystem::{tests::SAMPLE, Ecosystem};
    use std::io::Write;

    pub fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn rules() -> Vec<InstallRule> {
        let eco = Ecosystem::parse(SAMPLE.as_bytes()).unwrap();
        eco.community("lethal-company").unwrap().r2.install_rules.clone()
    }

    #[test]
    fn plugin_packages_follow_the_routes() {
        let zip = zip_of(&[
            ("manifest.json", r#"{"name":"MoreThings","version_number":"1.0.0","dependencies":["BepInEx-BepInExPack-5.4.2100"]}"#),
            ("README.md", "hi"),
            ("icon.png", "png"),
            ("MoreThings.dll", "a"),
            ("Patch.mm.dll", "m"),
            ("BepInEx/plugins/Sub/Extra.dll", "b"),
            ("config/more.cfg", "c"),
            ("patchers/P.dll", "p"),
            ("assets/sound.wav", "s"),
        ]);
        let m = manifest(&zip).unwrap().unwrap();
        assert_eq!(m.dependencies, ["BepInEx-BepInExPack-5.4.2100"]);
        let out = plan(&zip, "Ann-MoreThings", &rules(), None).unwrap();
        let keys: Vec<&str> = out.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "BepInEx/config/more.cfg",
                "BepInEx/monomod/Ann-MoreThings/Patch.mm.dll",
                "BepInEx/patchers/Ann-MoreThings/P.dll",
                "BepInEx/plugins/Ann-MoreThings/MoreThings.dll",
                "BepInEx/plugins/Ann-MoreThings/Sub/Extra.dll",
                "BepInEx/plugins/Ann-MoreThings/sound.wav",
            ]
        );
    }

    #[test]
    fn the_loader_pack_goes_to_the_game_folder() {
        let eco = Ecosystem::parse(SAMPLE.as_bytes()).unwrap();
        let zip = zip_of(&[
            ("manifest.json", "{}"),
            ("BepInExPack/winhttp.dll", "w"),
            ("BepInExPack/doorstop_config.ini", "[General]"),
            ("BepInExPack/BepInEx/core/BepInEx.Preloader.dll", "p"),
        ]);
        let out = plan(
            &zip,
            "BepInEx-BepInExPack",
            &rules(),
            eco.modloader("BepInEx-BepInExPack"),
        )
        .unwrap();
        let keys: Vec<&str> = out.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "BepInEx/core/BepInEx.Preloader.dll",
                "doorstop_config.ini",
                "winhttp.dll"
            ]
        );
    }

    #[test]
    fn unsafe_paths_are_refused() {
        let zip = zip_of(&[("../evil.dll", "x")]);
        assert!(plan(&zip, "A-B", &rules(), None).is_err());
    }
}
