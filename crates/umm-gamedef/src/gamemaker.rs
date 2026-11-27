//! GameMaker data files (`data.win`, `game.unx`, `game.ios`, `game.droid`): the game's code
//! and assets in one IFF-style file, `FORM` and its size, then chunks, `GEN8` first. A game
//! may have several (DELTARUNE: one for its menu, one per chapter).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The extensions of GameMaker data files, lower-case.
pub const DATA_EXTENSIONS: &[&str] = &["win", "unx", "ios", "droid"];

/// What a data file's `GEN8` chunk says about its game.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DataInfo {
    /// The game's name (`DELTARUNE`).
    pub name: Option<String>,
    /// The name it shows (`DELTARUNE Chapter 2`), in files recent enough to have one.
    pub display_name: Option<String>,
}

fn has_data_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DATA_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Whether the file is a GameMaker data file: named like one, starting with `FORM`, its
/// size, and the `GEN8` chunk.
pub fn is_data_file(path: &Path) -> bool {
    if !has_data_extension(path) {
        return false;
    }
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut h = [0u8; 12];
    if f.read_exact(&mut h).is_err() {
        return false;
    }
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    &h[..4] == b"FORM"
        && u64::from(u32::from_le_bytes([h[4], h[5], h[6], h[7]])) + 8 == len
        && &h[8..12] == b"GEN8"
}

/// Whether bytes (a whole file, or its start) begin like a GameMaker data file.
pub fn looks_like_data(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"FORM" && &bytes[8..12] == b"GEN8"
}

/// Reads a GameMaker string: `ptr` points at its characters, its length just before them.
fn string_at(f: &mut File, ptr: u32, len: u64) -> Option<String> {
    let ptr = u64::from(ptr);
    if ptr < 4 || ptr >= len {
        return None;
    }
    f.seek(SeekFrom::Start(ptr - 4)).ok()?;
    let mut n = [0u8; 4];
    f.read_exact(&mut n).ok()?;
    let n = u32::from_le_bytes(n) as usize;
    if n > 1024 || ptr + n as u64 > len {
        return None;
    }
    let mut s = vec![0u8; n];
    f.read_exact(&mut s).ok()?;
    String::from_utf8(s).ok().filter(|s| !s.trim().is_empty())
}

/// The game's names from a data file's `GEN8` chunk. `None` for a file that is not one.
pub fn data_info(path: &Path) -> Option<DataInfo> {
    if !is_data_file(path) {
        return None;
    }
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = [0u8; 16];
    f.read_exact(&mut head).ok()?;
    let gen_len = u32::from_le_bytes([head[12], head[13], head[14], head[15]]) as usize;
    let mut gen = vec![0u8; gen_len.min(4096)];
    f.read_exact(&mut gen).ok()?;
    let ptr = |off: usize| {
        gen.get(off..off + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    // GEN8: debug flag, bytecode version, 2 bytes, then string pointers (file name, config),
    // last object, last tile, game id, a 16-byte GUID, the name (at 40); the display name is
    // at 100.
    let (name, display) = (ptr(40), ptr(100));
    Some(DataInfo {
        name: name.and_then(|p| string_at(&mut f, p, len)),
        display_name: display.and_then(|p| string_at(&mut f, p, len)),
    })
}

/// The GameMaker data files in a game folder, at its top and in the folders below it (two
/// levels: DELTARUNE's `chapter1_windows/data.win`), relative to it with `/`, sorted.
pub fn data_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, "", 0, &mut out);
    out.sort_by_key(|p| (p.matches('/').count(), p.to_lowercase()));
    out
}

fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let here = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        match e.file_type() {
            Ok(t) if t.is_dir() && depth < 2 => walk(&path, &here, depth + 1, out),
            Ok(t) if t.is_file() && is_data_file(&path) => out.push(here),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data file: `FORM`, its size, and a `GEN8` chunk naming the game, its display name
    /// in a string after it.
    pub(crate) fn data(name: &str, display: &str) -> Vec<u8> {
        let mut gen = vec![0u8; 120];
        let mut strings = Vec::new();
        let add = |s: &str, at: usize, gen: &mut Vec<u8>, strings: &mut Vec<u8>| {
            // Characters start after FORM(8) + GEN8 header(8) + chunk + earlier strings + length.
            let ptr = (8 + 8 + 120 + strings.len() + 4) as u32;
            gen[at..at + 4].copy_from_slice(&ptr.to_le_bytes());
            strings.extend((s.len() as u32).to_le_bytes());
            strings.extend(s.as_bytes());
            strings.push(0);
        };
        add(name, 40, &mut gen, &mut strings);
        add(display, 100, &mut gen, &mut strings);
        let mut body = b"GEN8".to_vec();
        body.extend((gen.len() as u32).to_le_bytes());
        body.extend(gen);
        body.extend(strings);
        let mut f = b"FORM".to_vec();
        f.extend((body.len() as u32).to_le_bytes());
        f.extend(body);
        f
    }

    #[test]
    fn reads_data_files_and_their_names() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("data.win"), data("DELTARUNE", "DELTARUNE")).unwrap();
        std::fs::create_dir_all(root.join("chapter2_windows")).unwrap();
        std::fs::write(
            root.join("chapter2_windows/data.win"),
            data("DELTARUNE", "DELTARUNE Chapter 2"),
        )
        .unwrap();
        std::fs::write(root.join("fake.win"), b"FORM nope").unwrap();
        std::fs::write(root.join("other.bin"), data("X", "X")).unwrap();
        assert_eq!(data_files(root), ["data.win", "chapter2_windows/data.win"]);
        let info = data_info(&root.join("chapter2_windows/data.win")).unwrap();
        assert_eq!(info.name.as_deref(), Some("DELTARUNE"));
        assert_eq!(info.display_name.as_deref(), Some("DELTARUNE Chapter 2"));
        assert!(data_info(&root.join("fake.win")).is_none());
    }
}
