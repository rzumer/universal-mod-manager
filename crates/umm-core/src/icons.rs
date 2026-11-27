//! Pictures for the interface: a game's icon (Steam's, else its exe's), and a mod's (the
//! icon its Thunderstore package came with, or its Nexus Mods picture, fetched once).
//!
//! Each is returned as a `data:` URL, and kept in UMM's data folder once found.

use crate::{Result, Session};
use base64::Engine;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use umm_gamedef::RelPath;

/// Where a game's icon is kept once found, by its kind.
const GAME_ICON: &str = "icon";
/// Where a mod's icon is kept, beside its package files.
const MOD_ICON: &str = "icon";
/// Marks a mod with no picture on Nexus Mods, so UMM does not ask again.
const NO_ICON: &str = ".no-icon";
/// Image kinds an icon can be, with their media types.
const KINDS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("ico", "image/x-icon"),
];
/// Icons larger than this are not shown.
const MAX_ICON: u64 = 2 << 20;

fn data_url(bytes: &[u8], mime: &str) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The first `<stem>.<kind>` file in `dir`, as a `data:` URL.
fn read_icon(dir: &Path, stem: &str) -> Option<String> {
    KINDS.iter().find_map(|(ext, mime)| {
        let p = dir.join(format!("{stem}.{ext}"));
        let meta = fs::metadata(&p).ok()?;
        if meta.len() > MAX_ICON {
            return None;
        }
        Some(data_url(&fs::read(&p).ok()?, mime))
    })
}

/// The file extension for an image's bytes.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        [b'G', b'I', b'F', ..] => Some("gif"),
        [0, 0, 1, 0, ..] => Some("ico"),
        _ => None,
    }
}

/// Steam's own small icon for an app, from its library cache: a file named by a 40-character
/// hash in `appcache/librarycache/<app>/` (or, in older Steam, `<app>_icon.jpg` beside them).
fn steam_icon(roots: &[PathBuf], app: u32) -> Option<PathBuf> {
    for root in roots {
        let cache = root.join("appcache").join("librarycache");
        if let Ok(rd) = fs::read_dir(cache.join(app.to_string())) {
            let found = rd.flatten().map(|e| e.path()).find(|p| {
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned());
                let ext = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase());
                stem.is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    && matches!(ext.as_deref(), Some("jpg" | "png" | "ico"))
            });
            if found.is_some() {
                return found;
            }
        }
        let old = cache.join(format!("{app}_icon.jpg"));
        if old.is_file() {
            return Some(old);
        }
    }
    None
}

// ----- An exe's icon -----

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The entries of a resource directory at `at`: `(name or id, offset, is a directory)`.
fn res_entries(res: &[u8], at: usize) -> Option<Vec<(u32, usize, bool)>> {
    let count = u16_at(res, at + 12)? as usize + u16_at(res, at + 14)? as usize;
    (0..count)
        .map(|i| {
            let e = at + 16 + i * 8;
            let name = u32_at(res, e)?;
            let off = u32_at(res, e + 4)?;
            Some((name, (off & 0x7FFF_FFFF) as usize, off & 0x8000_0000 != 0))
        })
        .collect()
}

/// The first data entry under a resource directory (any name, then any language):
/// `(rva, size)`.
fn res_first_data(res: &[u8], mut at: usize, mut is_dir: bool) -> Option<(u32, u32)> {
    while is_dir {
        let (_, off, dir) = *res_entries(res, at)?.first()?;
        at = off;
        is_dir = dir;
    }
    Some((u32_at(res, at)?, u32_at(res, at + 4)?))
}

/// The icon of a Windows exe, as a single-image `.ico` (or the PNG one holds): the first icon
/// group's image closest to 64 pixels, from the exe's resources. `None` for anything that is
/// not a PE file, or has no icon.
pub(crate) fn exe_icon(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    if head.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(&head, 0x3C)? as usize;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = u16_at(&head, pe + 6)? as usize;
    let opt_size = u16_at(&head, pe + 20)? as usize;
    let opt = pe + 24;
    let dirs = match u16_at(&head, opt)? {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return None,
    };
    let res_rva = u32_at(&head, dirs + 2 * 8)?;
    if res_rva == 0 {
        return None;
    }
    // The section holding the resources.
    let table = opt + opt_size;
    let (va, raw_size, raw_at) = (0..sections).find_map(|i| {
        let s = table + i * 40;
        let vsize = u32_at(&head, s + 8)?;
        let va = u32_at(&head, s + 12)?;
        let raw_size = u32_at(&head, s + 16)?;
        let raw_at = u32_at(&head, s + 20)?;
        // By its size in memory: a section can carry far more raw data (Godot's `pck`).
        let span = if vsize == 0 { raw_size } else { vsize };
        (res_rva >= va && res_rva < va + span).then_some((va, raw_size, raw_at))
    })?;
    if raw_size as u64 > 64 << 20 {
        return None;
    }
    let mut section = vec![0u8; raw_size as usize];
    f.seek(SeekFrom::Start(raw_at as u64)).ok()?;
    f.read_exact(&mut section).ok()?;
    let res = section.get((res_rva - va) as usize..)?;
    let at_rva = |rva: u32, size: u32| -> Option<&[u8]> {
        let start = rva.checked_sub(va)? as usize;
        section.get(start..start + size as usize)
    };

    let types = res_entries(res, 0)?;
    let of_type = |t: u32| {
        types
            .iter()
            .find(|(n, _, dir)| *n == t && *dir)
            .map(|e| e.1)
    };
    const RT_ICON: u32 = 3;
    const RT_GROUP_ICON: u32 = 14;
    let groups = of_type(RT_GROUP_ICON)?;
    let icons = of_type(RT_ICON)?;
    let (g_rva, g_size) = res_first_data(res, groups, true)?;
    let group = at_rva(g_rva, g_size)?;
    let count = u16_at(group, 4)? as usize;
    // Entries: width, height, colours, reserved, planes, bits, bytes, id (14 bytes each).
    let best = (0..count)
        .filter_map(|i| {
            let e = 6 + i * 14;
            let w = match *group.get(e)? {
                0 => 256,
                w => w as u32,
            };
            Some((w, e))
        })
        .min_by_key(|(w, _)| (w.abs_diff(64), u32::MAX - w))?;
    let entry = group.get(best.1..best.1 + 14)?;
    let id = u16_at(entry, 12)? as u32;
    let (i_rva, i_size) = res_entries(res, icons)?
        .into_iter()
        .find(|(n, _, _)| *n == id)
        .and_then(|(_, off, dir)| res_first_data(res, off, dir))?;
    let image = at_rva(i_rva, i_size)?;
    if image.starts_with(b"\x89PNG") {
        return Some((image.to_vec(), "png"));
    }
    // One-image .ico: its header, the group's entry (with the image's offset in place of
    // its id), then the image.
    let mut ico = vec![0, 0, 1, 0, 1, 0];
    ico.extend_from_slice(&entry[..8]);
    ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(image);
    Some((ico, "ico"))
}

impl Session {
    /// The game's icon as a `data:` URL: Steam's, else its exe's. `None` when there is
    /// neither (a game not installed, or an exe without one).
    pub fn game_icon(&self, game_id: &str) -> Result<Option<String>> {
        let game = self.data.game(game_id);
        let dir = game.generated_dir();
        if let Some(url) = read_icon(&dir, GAME_ICON) {
            return Ok(Some(url));
        }
        let def = self.def(game_id)?.def;
        let found: Option<(Vec<u8>, &str)> = def
            .detect
            .steam_app_id
            .and_then(|app| steam_icon(&umm_gamedef::detect::steam_roots(), app))
            .and_then(|p| {
                let bytes = fs::read(p).ok()?;
                let kind = sniff(&bytes)?;
                Some((bytes, kind))
            })
            .or_else(|| {
                let (root, _) = self.install_path(&def).ok()??;
                let exe = RelPath::parse(&def.launch.as_ref()?.exe).ok()?;
                exe_icon(&exe.to_path(&root))
            });
        let Some((bytes, kind)) = found else {
            return Ok(None);
        };
        // Kept, so the exe is not read again; a failure to keep it only costs that.
        if fs::create_dir_all(&dir).is_ok() {
            let _ =
                umm_store::fsutil::write_atomic(&dir.join(format!("{GAME_ICON}.{kind}")), &bytes);
        }
        let mime = KINDS
            .iter()
            .find(|(e, _)| *e == kind)
            .map_or("image/png", |k| k.1);
        Ok(Some(data_url(&bytes, mime)))
    }

    /// The mod's icon as a `data:` URL: the one its Thunderstore package came with (or the one
    /// Thunderstore lists, for a package installed before UMM kept them), or its picture on
    /// Nexus Mods or GameBanana, fetched the first time and kept. `None` for a mod without
    /// one.
    pub fn mod_icon(&self, game_id: &str, mod_id: &str) -> Result<Option<String>> {
        let game = self.data.game(game_id);
        let dir = game.mod_package_dir(mod_id);
        if let Some(url) = read_icon(&dir, MOD_ICON) {
            return Ok(Some(url));
        }
        let meta = umm_store::load_mod(&game, mod_id)?;
        if dir.join(NO_ICON).exists() {
            return Ok(None);
        }
        // Network failures are not remembered: the next look tries again. A store that says
        // there is no picture is.
        let url = if let Some(ts) = &meta.thunderstore {
            // Installed before UMM kept package files: the icon Thunderstore lists for it.
            let Ok((packages, _)) = self.ts_packages(game_id, false) else {
                return Ok(None);
            };
            packages
                .iter()
                .find(|p| p.full_name.eq_ignore_ascii_case(&ts.full_name))
                .map(|p| p.icon.clone())
                .filter(|u| !u.is_empty())
        } else if let Some(nexus) = &meta.nexus {
            let Some(game_nr) = nexus.game_id else {
                return Ok(None);
            };
            let Ok(info) = self.graph().mod_info(game_nr, nexus.mod_id) else {
                return Ok(None);
            };
            info.thumbnail_url
        } else if let Some(gb) = &meta.gamebanana {
            let Ok(d) = self.gamebanana().mod_details(gb.mod_id) else {
                return Ok(None);
            };
            d.summary.thumbnail_url
        } else {
            return Ok(None);
        };
        Ok(self.keep_icon(&dir, url))
    }

    /// Downloads a mod's picture from `url` and keeps it beside its package files, as a
    /// `data:` URL. With no `url`, or not an image, the mod is marked as having none.
    fn keep_icon(&self, dir: &Path, url: Option<String>) -> Option<String> {
        fs::create_dir_all(dir).ok();
        let none = || {
            let _ = fs::write(dir.join(NO_ICON), "");
            None
        };
        let Some(url) = url else {
            return none();
        };
        let bytes = self.http().get(&url).ok()?;
        let Some(kind) = sniff(&bytes).filter(|_| bytes.len() as u64 <= MAX_ICON) else {
            return none();
        };
        let _ = umm_store::fsutil::write_atomic(&dir.join(format!("{MOD_ICON}.{kind}")), &bytes);
        let mime = KINDS
            .iter()
            .find(|(e, _)| *e == kind)
            .map_or("image/png", |k| k.1);
        Some(data_url(&bytes, mime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_keeps_a_small_icon_named_by_its_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("appcache/librarycache/77");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("header.jpg"), "h").unwrap();
        fs::write(dir.join("logo.png"), "l").unwrap();
        let icon = dir.join("ca69719555a9dc1006f9cdc1ea537aaf7cdff725.jpg");
        fs::write(&icon, "i").unwrap();
        assert_eq!(steam_icon(&[tmp.path().to_path_buf()], 77), Some(icon));
        assert_eq!(steam_icon(&[tmp.path().to_path_buf()], 78), None);
    }

    #[test]
    fn images_are_told_apart_by_their_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n"), Some("png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8"), Some("webp"));
        assert_eq!(sniff(b"<html>"), None);
    }

    /// Windows' own exes carry icons: one comes out as a valid single-image icon.
    #[cfg(windows)]
    #[test]
    fn an_exe_icon_is_read_from_its_resources() {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap();
        let exe = windir.join("explorer.exe");
        if !exe.is_file() {
            return;
        }
        let (bytes, kind) = exe_icon(&exe).expect("explorer.exe has an icon");
        match kind {
            "png" => assert!(bytes.starts_with(b"\x89PNG")),
            _ => {
                assert_eq!(&bytes[..6], &[0, 0, 1, 0, 1, 0]);
                let size = u32_at(&bytes, 14).unwrap() as usize;
                assert_eq!(u32_at(&bytes, 18), Some(22));
                assert_eq!(bytes.len(), 22 + size);
            }
        }
        assert!(exe_icon(&windir.join("win.ini")).is_none(), "not an exe");
    }
}
