//! Finding a game's install folder: `[detect] path` if set, then Steam libraries, then the
//! registry.

use crate::vdf::{self, Value};
use crate::{GameDef, RelPath};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Detected {
    pub path: PathBuf,
    pub via: &'static str,
}

pub fn detect_install(def: &GameDef) -> Option<Detected> {
    let d = &def.detect;
    let ok = |p: &Path| {
        p.is_dir()
            && d.marker.as_deref().is_none_or(|m| {
                RelPath::parse(m)
                    .map(|r| r.to_path(p).exists())
                    .unwrap_or(false)
            })
    };

    // A folder picked in UMM (or written by hand) wins, as long as it is still there.
    if let Some(p) = &d.path {
        if p.is_dir() {
            return Some(Detected {
                path: p.clone(),
                via: "manual",
            });
        }
    }

    if d.steam_app_id.is_some() || d.steam_dir.is_some() {
        for lib in steam_libraries() {
            let common = lib.join("steamapps").join("common");
            if let Some(id) = d.steam_app_id {
                if let Some(dir) = read_installdir(&lib, id) {
                    let p = common.join(dir);
                    if ok(&p) {
                        return Some(Detected {
                            path: p,
                            via: "steam",
                        });
                    }
                }
            }
            if let Some(dir) = &d.steam_dir {
                let p = common.join(dir);
                if ok(&p) {
                    return Some(Detected {
                        path: p,
                        via: "steam",
                    });
                }
            }
        }
    }

    if let Some(spec) = &d.registry {
        if let Some(p) = read_registry_string(spec) {
            let p = PathBuf::from(p);
            if ok(&p) {
                return Some(Detected {
                    path: p,
                    via: "registry",
                });
            }
        }
    }
    None
}

fn read_installdir(lib: &Path, app_id: u32) -> Option<String> {
    let text = fs::read_to_string(
        lib.join("steamapps")
            .join(format!("appmanifest_{app_id}.acf")),
    )
    .ok()?;
    let doc = vdf::parse(&text)?;
    let Value::Obj(state) = vdf::get(&doc, "AppState")? else {
        return None;
    };
    match vdf::get(state, "installdir")? {
        Value::Str(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// The Windows user's folder in a Steam game's Proton prefix,
/// `steamapps/compatdata/<app id>/pfx/drive_c/users/steamuser`: in the library the game is
/// installed in (`install`, in its `steamapps/common`), else in another of `libraries`.
/// `None` when it has none (run natively, or not started yet).
pub fn proton_user_in(
    app_id: u32,
    install: Option<&Path>,
    libraries: &[PathBuf],
) -> Option<PathBuf> {
    let own = install
        .and_then(Path::parent)
        .filter(|c| c.file_name().is_some_and(|n| n == "common"))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    own.into_iter()
        .chain(libraries.iter().cloned())
        .map(|lib| {
            lib.join("steamapps/compatdata")
                .join(app_id.to_string())
                .join("pfx/drive_c/users/steamuser")
        })
        .find(|p| p.is_dir())
}

/// [`proton_user_in`] the Steam libraries. Never on Windows, where games run as they are.
pub fn proton_user(app_id: u32, install: Option<&Path>) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    proton_user_in(app_id, install, &steam_libraries())
}

pub fn steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    // The registry and libraryfolders.vdf spell the same folder differently
    // (`c:/program files (x86)/steam` vs `C:\Program Files (x86)\Steam`).
    let mut push = |p: PathBuf| {
        let p = if cfg!(windows) {
            PathBuf::from(p.to_string_lossy().replace('/', "\\"))
        } else {
            p
        };
        let Ok(canon) = fs::canonicalize(&p) else {
            return;
        };
        if !seen.contains(&canon) {
            seen.push(canon);
            libs.push(p);
        }
    };
    for root in steam_roots() {
        if !root.is_dir() {
            continue;
        }
        push(root.clone());
        let Ok(text) = fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) else {
            continue;
        };
        let Some(doc) = vdf::parse(&text) else {
            continue;
        };
        let Some(Value::Obj(folders)) = vdf::get(&doc, "libraryfolders") else {
            continue;
        };
        for (key, v) in folders {
            match v {
                Value::Obj(fields) => {
                    if let Some(Value::Str(p)) = vdf::get(fields, "path") {
                        push(PathBuf::from(p));
                    }
                }
                // Older format: "1" "D:\\SteamLibrary"
                Value::Str(p) if key.chars().all(|c| c.is_ascii_digit()) => push(PathBuf::from(p)),
                _ => {}
            }
        }
    }
    libs
}

#[cfg(windows)]
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for spec in [
        r"HKCU\Software\Valve\Steam:SteamPath",
        r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam:InstallPath",
        r"HKLM\SOFTWARE\Valve\Steam:InstallPath",
    ] {
        if let Some(p) = read_registry_string(spec) {
            roots.push(PathBuf::from(p));
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots
}

#[cfg(not(windows))]
pub fn steam_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

/// Reads `HIVE\sub\key:ValueName`. Returns `None` on any other platform.
#[cfg(windows)]
pub fn read_registry_string(spec: &str) -> Option<String> {
    let (key, value) = spec.rsplit_once(':')?;
    let (hive, sub) = key.split_once('\\')?;
    crate::registry::read_string(crate::registry::Hive::parse(hive)?, sub, value)
}

#[cfg(not(windows))]
pub fn read_registry_string(_spec: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_is_in_the_games_library_else_another() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let install = a.join("steamapps/common/Game");
        let user = |lib: &Path| lib.join("steamapps/compatdata/42/pfx/drive_c/users/steamuser");
        let libs = [a.clone(), b.clone()];
        assert_eq!(proton_user_in(42, Some(&install), &libs), None);
        fs::create_dir_all(user(&b)).unwrap();
        assert_eq!(proton_user_in(42, Some(&install), &libs), Some(user(&b)));
        fs::create_dir_all(user(&a)).unwrap();
        assert_eq!(
            proton_user_in(42, Some(&install), &[b.clone()]),
            Some(user(&a))
        );
        assert_eq!(proton_user_in(7, Some(&install), &libs), None);
    }
}
