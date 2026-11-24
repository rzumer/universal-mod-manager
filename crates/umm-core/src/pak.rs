//! Reading one file out of a Larian `.pak` package (LSPK versions 15, 16 and 18: Baldur's
//! Gate 3, Divinity: Original Sin 2), laid out as LSLib reads it: the `LSPK` signature and a
//! header at the start, and at the header's offset the list of files, LZ4-compressed. Only
//! the list and the one file are read, not the whole package.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SIGNATURE: &[u8; 4] = b"LSPK";
/// Header flag: the files are one LZ4 frame, not compressed one by one.
const SOLID: u8 = 0x04;
/// Larger than any file UMM reads from a package (a mod's `meta.lsx`).
const MAX_FILE: u64 = 64 << 20;

/// A file in a package's list.
struct Entry {
    name: String,
    offset: u64,
    part: u32,
    flags: u32,
    size_on_disk: u64,
    size: u64,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"))
}

fn read_at(f: &mut File, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0; len];
    f.read_exact(&mut buf)
        .map_err(|e| format!("reading {len} bytes at {offset}: {e}"))?;
    Ok(buf)
}

/// The first file in the package whose name (`/`-separated, as listed) `want` accepts: its
/// name and its contents. `None` when it has none.
pub fn read_matching(
    path: &Path,
    want: impl Fn(&str) -> bool,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let head = read_at(&mut f, 0, 8).map_err(|_| "not a package (too short)".to_string())?;
    if &head[..4] != SIGNATURE {
        return Err("not an LSPK package".into());
    }
    let version = u32_at(&head, 4);
    let entry_size = match version {
        18 => 272,
        15 | 16 => 296,
        v => return Err(format!("package version {v} is not supported")),
    };
    // After the signature: version, file list offset and size, flags, priority, MD5 and
    // (from 16) the number of parts.
    let h = read_at(&mut f, 4, 18)?;
    let list_offset = u64_at(&h, 4);
    let flags = h[16];
    if flags & SOLID != 0 {
        return Err("solid packages are not supported".into());
    }
    if list_offset.saturating_add(8) > len {
        return Err("its file list is past its end".into());
    }
    let counts = read_at(&mut f, list_offset, 8)?;
    let files = u32_at(&counts, 0) as u64;
    let packed = u32_at(&counts, 4) as u64;
    if packed > len - list_offset - 8 || files > 10_000_000 {
        return Err("its file list is damaged".into());
    }
    let packed = read_at(&mut f, list_offset + 8, packed as usize)?;
    let list = lz4_flex::block::decompress(&packed, files as usize * entry_size)
        .map_err(|e| format!("its file list: {e}"))?;
    if list.len() != files as usize * entry_size {
        return Err("its file list is damaged".into());
    }
    let Some(e) = list
        .chunks_exact(entry_size)
        .map(|c| entry(c, version))
        .find(|e| want(&e.name))
    else {
        return Ok(None);
    };
    if e.size_on_disk > MAX_FILE || e.size > MAX_FILE {
        return Err(format!("{} is too large", e.name));
    }
    let data = if e.part == 0 {
        read_at(&mut f, e.offset, e.size_on_disk as usize)?
    } else {
        let mut part = File::open(part_path(path, e.part))
            .map_err(|err| format!("part {} of the package: {err}", e.part))?;
        read_at(&mut part, e.offset, e.size_on_disk as usize)?
    };
    let bytes = decompress(&data, e.flags, e.size).map_err(|err| format!("{}: {err}", e.name))?;
    Ok(Some((e.name, bytes)))
}

fn entry(c: &[u8], version: u32) -> Entry {
    let end = c[..256].iter().position(|&b| b == 0).unwrap_or(256);
    let name = String::from_utf8_lossy(&c[..end]).replace('\\', "/");
    if version == 18 {
        Entry {
            name,
            offset: u32_at(c, 256) as u64 | (u16_at(c, 260) as u64) << 32,
            part: c[262] as u32,
            flags: c[263] as u32,
            size_on_disk: u32_at(c, 264) as u64,
            size: u32_at(c, 268) as u64,
        }
    } else {
        Entry {
            name,
            offset: u64_at(c, 256),
            size_on_disk: u64_at(c, 264),
            size: u64_at(c, 272),
            part: u32_at(c, 280),
            flags: u32_at(c, 284),
        }
    }
}

/// `Name_<part>.pak` beside `Name.pak`.
fn part_path(path: &Path, part: u32) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    path.with_file_name(format!("{stem}_{part}{ext}"))
}

fn decompress(data: &[u8], flags: u32, size: u64) -> Result<Vec<u8>, String> {
    match flags & 0x0f {
        0 => Ok(data.to_vec()),
        1 => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(data)
                .take(MAX_FILE)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(out)
        }
        2 => lz4_flex::block::decompress(data, size as usize).map_err(|e| e.to_string()),
        3 => {
            let mut out = Vec::new();
            ruzstd::streaming_decoder::StreamingDecoder::new(data)
                .map_err(|e| e.to_string())?
                .take(MAX_FILE)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(out)
        }
        m => Err(format!("compression method {m} is not supported")),
    }
}

/// A version 18 package of these files, LZ4-compressed as the game's are (for tests).
#[cfg(test)]
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(SIGNATURE);
    out.extend_from_slice(&18u32.to_le_bytes());
    let header = out.len();
    out.extend_from_slice(&[0; 32]); // list offset and size, flags, priority, MD5, parts
    let mut list = Vec::new();
    for (name, content) in files {
        let packed = lz4_flex::block::compress(content);
        let offset = out.len() as u64;
        out.extend_from_slice(&packed);
        let mut e = vec![0u8; 272];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[256..260].copy_from_slice(&(offset as u32).to_le_bytes());
        e[260..262].copy_from_slice(&((offset >> 32) as u16).to_le_bytes());
        e[263] = 2;
        e[264..268].copy_from_slice(&(packed.len() as u32).to_le_bytes());
        e[268..272].copy_from_slice(&(content.len() as u32).to_le_bytes());
        list.extend_from_slice(&e);
    }
    let list_offset = out.len() as u64;
    let packed = lz4_flex::block::compress(&list);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
    out.extend_from_slice(&packed);
    out[header..header + 8].copy_from_slice(&list_offset.to_le_bytes());
    out[header + 8..header + 12].copy_from_slice(&(packed.len() as u32 + 8).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_file_out_of_a_package() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("m.pak");
        std::fs::write(
            &p,
            build(&[
                ("Public/M/Stats/a.txt", b"stats".as_slice()),
                ("Mods/M/meta.lsx", b"<save>meta</save>".as_slice()),
            ]),
        )
        .unwrap();
        let (name, bytes) = read_matching(&p, |n| n.ends_with("meta.lsx"))
            .unwrap()
            .unwrap();
        assert_eq!(name, "Mods/M/meta.lsx");
        assert_eq!(bytes, b"<save>meta</save>");
        assert!(read_matching(&p, |n| n == "none").unwrap().is_none());
        std::fs::write(&p, b"PK\x03\x04 a zip").unwrap();
        assert!(read_matching(&p, |_| true).is_err());
    }
}
