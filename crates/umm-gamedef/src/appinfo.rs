//! Steam's own record of the apps it knows (`appcache/appinfo/appinfo_XX.vdf`, or before that
//! one `appcache/appinfo.vdf`), read for what it says of a game without asking Steam's
//! servers: whether the game has a Steam Workshop.
//!
//! Both are binary KeyValues. The split record keeps an app in the file named by the low byte
//! of MurmurHash3's 32-bit finaliser of its id, and names its keys by index in a table of key
//! names; the older single file lists apps one after another, naming keys inline (version 28)
//! or by index in a table at its end (version 29).

use std::fs;
use std::path::Path;

use crate::detect::steam_roots;

const SPLIT: u32 = 0x0756_442A;
const V27: u32 = 0x0756_4427;
const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;

/// Whether the game has a Steam Workshop, as Steam's record says: `None` when Steam has no
/// record of the app on this machine (or one UMM cannot read).
pub fn workshop_visible(app_id: u32) -> Option<bool> {
    steam_roots()
        .iter()
        .find_map(|root| workshop_visible_in(root, app_id))
}

/// [`workshop_visible`], in the Steam installation at `root`.
pub fn workshop_visible_in(root: &Path, app_id: u32) -> Option<bool> {
    let cache = root.join("appcache");
    let shard = cache
        .join("appinfo")
        .join(format!("appinfo_{:02x}.vdf", shard_of(app_id)));
    let found = match fs::read(&shard) {
        Ok(d) => split_lookup(&d, app_id, &["appinfo", "common", "workshop_visible"]),
        Err(_) => fs::read(cache.join("appinfo.vdf"))
            .ok()
            .and_then(|d| single_lookup(&d, app_id, &["appinfo", "common", "workshop_visible"])),
    }?;
    // The app is there; a game without a Workshop has no such key.
    Some(found.is_some_and(|v| v.truthy()))
}

/// The split record's file for an app: MurmurHash3's finaliser, low byte.
fn shard_of(app_id: u32) -> u8 {
    let mut k = app_id;
    k ^= k >> 16;
    k = k.wrapping_mul(0x85eb_ca6b);
    k ^= k >> 13;
    k = k.wrapping_mul(0xc2b2_ae35);
    k ^= k >> 16;
    (k & 0xff) as u8
}

/// A value read from binary KeyValues: what a lookup needs of it.
#[derive(Debug, Clone, PartialEq)]
enum Leaf {
    Text(String),
    Int(i64),
    Float(f32),
}

impl Leaf {
    fn truthy(&self) -> bool {
        match self {
            Leaf::Text(s) => !matches!(s.trim(), "" | "0" | "false"),
            Leaf::Int(i) => *i != 0,
            Leaf::Float(f) => *f != 0.0,
        }
    }
}

struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.at)?;
        self.at += 1;
        Some(b)
    }
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.d.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
    fn cstr(&mut self) -> Option<String> {
        let rest = self.d.get(self.at..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        self.at += end + 1;
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
    fn wstr(&mut self) -> Option<String> {
        let mut units = Vec::new();
        loop {
            let b = self.bytes(2)?;
            let u = u16::from_le_bytes([b[0], b[1]]);
            if u == 0 {
                break;
            }
            units.push(u);
        }
        Some(String::from_utf16_lossy(&units))
    }
}

/// How keys are named: inline, or by index in a table.
enum Keys<'a> {
    Inline,
    Table(&'a [String]),
}

impl Keys<'_> {
    fn read(&self, r: &mut Reader) -> Option<String> {
        match self {
            Keys::Inline => r.cstr(),
            Keys::Table(t) => t.get(r.u32()? as usize).cloned(),
        }
    }
}

/// Reads one KeyValues object from `r` (its first byte being a key's type), returning the
/// value at `path` within it: `Some(None)` when the object reads to its end without it.
fn find(r: &mut Reader, keys: &Keys, path: &[&str]) -> Option<Option<Leaf>> {
    // The path's keys matched so far, and how deep the reader is.
    let mut depth = 0usize;
    let mut matched = 0usize;
    loop {
        let t = r.u8()?;
        if t == 0x08 || t == 0x0B {
            if depth == 0 {
                return Some(None);
            }
            if matched == depth {
                // Left the section the path goes through: it has no such key.
                return Some(None);
            }
            depth -= 1;
            continue;
        }
        let key = keys.read(r)?;
        let on_path = matched == depth && path.get(depth) == Some(&key.as_str());
        let leaf = match t {
            0x00 => {
                depth += 1;
                if on_path {
                    matched += 1;
                }
                continue;
            }
            0x01 => Leaf::Text(r.cstr()?),
            0x02 => Leaf::Int(r.u32()? as i32 as i64),
            0x03 => Leaf::Float(f32::from_le_bytes(r.bytes(4)?.try_into().ok()?)),
            0x04 | 0x06 => Leaf::Int(r.u32()? as i64),
            0x05 => Leaf::Text(r.wstr()?),
            0x07 | 0x0A => Leaf::Int(r.u64()? as i64),
            _ => return None,
        };
        if on_path && depth + 1 == path.len() {
            return Some(Some(leaf));
        }
    }
}

/// The value at `path` (from the app's root object) of an app in a file of the split
/// record: `None` when the app is not there.
fn split_lookup(d: &[u8], app_id: u32, path: &[&str]) -> Option<Option<Leaf>> {
    let mut r = Reader { d, at: 0 };
    if r.u32()? != SPLIT {
        return None;
    }
    let mut head = [0u32; 10];
    for h in &mut head {
        *h = r.u32()?;
    }
    // Universe, file, files, apps, index size, keys, key table size, data size, two hashes.
    let (index_size, key_count, table_size) =
        (head[4] as usize, head[5] as usize, head[6] as usize);
    let table_at = 44usize.checked_add(index_size)?;
    let table = d.get(table_at..table_at.checked_add(table_size)?)?;
    let names: Vec<String> = table
        .split(|&b| b == 0)
        .take(key_count)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    let index = |k: &str| names.iter().position(|n| n == k).map(|i| i as u32);
    // Each app's object begins `appinfo { appid = <id>`.
    let mut start = Vec::with_capacity(14);
    start.push(0x00);
    start.extend(index("appinfo")?.to_le_bytes());
    start.push(0x02);
    start.extend(index("appid")?.to_le_bytes());
    start.extend(app_id.to_le_bytes());
    let data_at = table_at + table_size;
    let data = d.get(data_at..)?;
    let at = data
        .windows(start.len())
        .position(|w| w == start.as_slice())?;
    let mut r = Reader {
        d,
        at: data_at + at,
    };
    find(&mut r, &Keys::Table(&names), path)
}

/// [`split_lookup`], in the older single file.
fn single_lookup(d: &[u8], app_id: u32, path: &[&str]) -> Option<Option<Leaf>> {
    let mut r = Reader { d, at: 0 };
    let magic = r.u32()?;
    let _universe = r.u32()?;
    let table: Vec<String>;
    let keys = match magic {
        V27 | V28 => Keys::Inline,
        V29 => {
            let table_at = usize::try_from(r.u64()?).ok()?;
            let mut t = Reader { d, at: table_at };
            let count = t.u32()? as usize;
            table = (0..count.min(1 << 20)).map_while(|_| t.cstr()).collect();
            Keys::Table(&table)
        }
        _ => return None,
    };
    // Per app: id, size, then (state, updated, token, hash, change[, hash of the binary])
    // before its object.
    let before = if magic == V27 { 40 } else { 60 };
    loop {
        let id = r.u32()?;
        if id == 0 {
            return None;
        }
        let size = r.u32()? as usize;
        let next = r.at.checked_add(size)?;
        if id == app_id {
            r.at = r.at.checked_add(before)?;
            // The object is the root's one section: `appinfo { … }`.
            return find(&mut r, &keys, path);
        }
        r.at = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Binary KeyValues, keys by index: `appinfo { appid = id; common { name; [workshop] } }`.
    fn app(id: u32, workshop: Option<&str>) -> Vec<u8> {
        let mut o = vec![0x00];
        o.extend(0u32.to_le_bytes()); // appinfo
        o.push(0x02);
        o.extend(1u32.to_le_bytes()); // appid
        o.extend(id.to_le_bytes());
        o.push(0x00);
        o.extend(2u32.to_le_bytes()); // common
        o.push(0x01);
        o.extend(3u32.to_le_bytes()); // name
        o.extend(b"A game\0");
        if let Some(w) = workshop {
            o.push(0x01);
            o.extend(4u32.to_le_bytes()); // workshop_visible
            o.extend(w.as_bytes());
            o.push(0);
        }
        o.extend([0x08, 0x00]); // a section after it: depots { … }
        o.extend(5u32.to_le_bytes());
        o.push(0x02);
        o.extend(4u32.to_le_bytes()); // a workshop_visible elsewhere does not count
        o.extend(1u32.to_le_bytes());
        o.extend([0x08, 0x08, 0x08]);
        o
    }

    const KEYS: &[&str] = &[
        "appinfo",
        "appid",
        "common",
        "name",
        "workshop_visible",
        "depots",
    ];

    #[test]
    fn finds_the_split_records_file() {
        // As Steam names them.
        assert_eq!(shard_of(2212330), 0xf1);
        assert_eq!(shard_of(1145360), 0xf7);
        assert_eq!(shard_of(3146520), 0xfb);
        assert_eq!(shard_of(100), 0xea);
    }

    #[test]
    fn reads_the_split_record() {
        let table: Vec<u8> = KEYS
            .iter()
            .flat_map(|k| [k.as_bytes(), b"\0"].concat())
            .collect();
        let index = vec![0u8; 160];
        let mut d = Vec::new();
        d.extend(SPLIT.to_le_bytes());
        for h in [
            1u32,
            0xf1,
            256,
            2,
            160,
            KEYS.len() as u32,
            table.len() as u32,
            0,
            0,
            0,
        ] {
            d.extend(h.to_le_bytes());
        }
        d.extend(&index);
        d.extend(&table);
        d.extend([0u8; 20]);
        d.extend(app(7, None));
        d.extend([0u8; 20]);
        d.extend(app(2212330, Some("1")));
        let path = ["appinfo", "common", "workshop_visible"];
        assert_eq!(
            split_lookup(&d, 2212330, &path),
            Some(Some(Leaf::Text("1".into())))
        );
        assert_eq!(
            split_lookup(&d, 7, &path),
            Some(None),
            "no key: no Workshop"
        );
        assert_eq!(split_lookup(&d, 8, &path), None, "not in the record");
        assert_eq!(split_lookup(b"nonsense", 7, &path), None);

        // On disk, under a Steam folder.
        let dir = tempfile::tempdir().unwrap();
        let shards = dir.path().join("appcache").join("appinfo");
        fs::create_dir_all(&shards).unwrap();
        fs::write(shards.join("appinfo_f1.vdf"), &d).unwrap();
        assert_eq!(workshop_visible_in(dir.path(), 2212330), Some(true));
        assert_eq!(
            workshop_visible_in(dir.path(), 1145360),
            None,
            "no such file"
        );
    }

    #[test]
    fn reads_the_older_single_record() {
        let mut body = Vec::new();
        let entry = |id: u32, ws: Option<&str>| {
            let obj = app(id, ws);
            let mut e = Vec::new();
            e.extend(id.to_le_bytes());
            e.extend(((60 + obj.len()) as u32).to_le_bytes());
            e.extend([0u8; 60]);
            e.extend(obj);
            e
        };
        body.extend(entry(10, Some("0")));
        body.extend(entry(20, Some("1")));
        body.extend(0u32.to_le_bytes());
        let mut d = Vec::new();
        d.extend(V29.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        let table_at = 16 + body.len() as u64;
        d.extend(table_at.to_le_bytes());
        d.extend(body);
        d.extend((KEYS.len() as u32).to_le_bytes());
        for k in KEYS {
            d.extend(k.as_bytes());
            d.push(0);
        }
        let path = ["appinfo", "common", "workshop_visible"];
        assert_eq!(
            single_lookup(&d, 20, &path).flatten().map(|v| v.truthy()),
            Some(true)
        );
        assert_eq!(
            single_lookup(&d, 10, &path).flatten().map(|v| v.truthy()),
            Some(false)
        );
        assert_eq!(single_lookup(&d, 30, &path), None);
    }
}
