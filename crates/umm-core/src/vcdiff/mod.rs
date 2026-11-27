//! A VCDIFF decoder (RFC 3284), for the delta patches xdelta3 writes (`.xdelta`): a patch
//! rebuilds a file (the target) from another (the source) and the patch's instructions.
//!
//! Besides the standard format, it reads xdelta3's extensions: its application header (the
//! files' names) and a checksum of each window's output, which tells a patch applied to a
//! source it was not made from, and its secondary compression of the patch's sections: LZMA
//! (the default of xdelta3 3.1 when built with liblzma), DJW and FGK. Custom code tables are
//! not read: such patches are refused, with the reason.

mod djw;
mod fgk;
mod lzma;

use std::borrow::Cow;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcdiffError(pub String);

impl fmt::Display for VcdiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VcdiffError {}

type Result<T> = std::result::Result<T, VcdiffError>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(VcdiffError(msg.into()))
}

const MAGIC: [u8; 3] = [0xD6, 0xC3, 0xC4];

// Header indicator bits.
const VCD_DECOMPRESS: u8 = 0x01;
const VCD_CODETABLE: u8 = 0x02;
const VCD_APPHEADER: u8 = 0x04;
// Window indicator bits.
const VCD_SOURCE: u8 = 0x01;
const VCD_TARGET: u8 = 0x02;
const VCD_ADLER32: u8 = 0x04;

/// Whether the bytes start like a VCDIFF patch.
pub fn is_vcdiff(patch: &[u8]) -> bool {
    patch.len() >= 4 && patch[..3] == MAGIC
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, i: 0 }
    }

    fn done(&self) -> bool {
        self.i >= self.b.len()
    }

    fn byte(&mut self) -> Result<u8> {
        let Some(&c) = self.b.get(self.i) else {
            return err("the patch ends early (truncated?)");
        };
        self.i += 1;
        Ok(c)
    }

    /// A variable-length integer: base 128, most significant digit first.
    fn int(&mut self) -> Result<u64> {
        let mut v: u64 = 0;
        for _ in 0..10 {
            let c = self.byte()?;
            v = v
                .checked_mul(128)
                .ok_or_else(|| VcdiffError("a number in the patch is too large".into()))?
                | u64::from(c & 0x7F);
            if c & 0x80 == 0 {
                return Ok(v);
            }
        }
        err("a number in the patch is too large")
    }

    fn size(&mut self) -> Result<usize> {
        usize::try_from(self.int()?).or_else(|_| err("a size in the patch is too large"))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .i
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(|| VcdiffError("the patch ends early (truncated?)".into()))?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
}

/// The patch's header: its secondary compressor, if any, and xdelta3's application header.
struct Header<'a> {
    secondary: Option<u8>,
    app: Option<&'a [u8]>,
}

fn header<'a>(r: &mut Reader<'a>) -> Result<Header<'a>> {
    if !is_vcdiff(r.b) {
        return err("not a VCDIFF (xdelta) patch");
    }
    r.take(4)?;
    let ind = r.byte()?;
    let secondary = if ind & VCD_DECOMPRESS != 0 {
        Some(r.byte()?)
    } else {
        None
    };
    if ind & VCD_CODETABLE != 0 {
        return err("the patch uses its own code table, which UMM cannot read");
    }
    let app = if ind & VCD_APPHEADER != 0 {
        let n = r.size()?;
        Some(r.take(n)?)
    } else {
        None
    };
    Ok(Header { secondary, app })
}

/// The name of the file the patch was made from, as xdelta3 records it in its application
/// header (`target/compression/source/compression`), without its folder.
pub fn source_name(patch: &[u8]) -> Option<String> {
    let mut r = Reader::new(patch);
    let app = header(&mut r).ok()?.app?;
    let text = String::from_utf8_lossy(app);
    let parts: Vec<&str> = text.split('/').collect();
    let name = match parts.len() {
        4 => parts[2],
        _ => return None,
    };
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name).trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn secondary_name(id: u8) -> String {
    match id {
        1 => "DJW".into(),
        2 => "LZMA".into(),
        16 => "FGK".into(),
        n => format!("number {n}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Noop,
    Add,
    Run,
    Copy,
}

#[derive(Clone, Copy)]
struct Inst {
    kind: Kind,
    size: u8,
    mode: u8,
}

const NOOP: Inst = Inst {
    kind: Kind::Noop,
    size: 0,
    mode: 0,
};

const fn inst(kind: Kind, size: u8, mode: u8) -> Inst {
    Inst { kind, size, mode }
}

/// RFC 3284's default code table: each code's one or two instructions.
fn code_table() -> Vec<[Inst; 2]> {
    let mut t = Vec::with_capacity(256);
    t.push([inst(Kind::Run, 0, 0), NOOP]);
    for size in 0..=17 {
        t.push([inst(Kind::Add, size, 0), NOOP]);
    }
    for mode in 0..9 {
        t.push([inst(Kind::Copy, 0, mode), NOOP]);
        for size in 4..=18 {
            t.push([inst(Kind::Copy, size, mode), NOOP]);
        }
    }
    for mode in 0..6 {
        for add in 1..=4 {
            for copy in 4..=6 {
                t.push([inst(Kind::Add, add, 0), inst(Kind::Copy, copy, mode)]);
            }
        }
    }
    for mode in 6..9 {
        for add in 1..=4 {
            t.push([inst(Kind::Add, add, 0), inst(Kind::Copy, 4, mode)]);
        }
    }
    for mode in 0..9 {
        t.push([inst(Kind::Copy, 4, mode), inst(Kind::Add, 1, 0)]);
    }
    debug_assert_eq!(t.len(), 256);
    t
}

const NEAR: usize = 4;
const SAME: usize = 3;

/// The address caches, reset for each window.
struct Cache {
    near: [u64; NEAR],
    next: usize,
    same: [u64; SAME * 256],
}

impl Cache {
    fn new() -> Self {
        Cache {
            near: [0; NEAR],
            next: 0,
            same: [0; SAME * 256],
        }
    }

    fn decode(&mut self, mode: u8, here: u64, addrs: &mut Reader) -> Result<u64> {
        let mode = mode as usize;
        let addr = match mode {
            0 => addrs.int()?,
            1 => here
                .checked_sub(addrs.int()?)
                .ok_or_else(|| VcdiffError("the patch copies from before its start".into()))?,
            m if m < 2 + NEAR => self.near[m - 2]
                .checked_add(addrs.int()?)
                .ok_or_else(|| VcdiffError("an address in the patch is too large".into()))?,
            m if m < 2 + NEAR + SAME => self.same[(m - 2 - NEAR) * 256 + addrs.byte()? as usize],
            _ => return err("the patch uses an unknown address mode"),
        };
        self.near[self.next] = addr;
        self.next = (self.next + 1) % NEAR;
        self.same[(addr % (SAME as u64 * 256)) as usize] = addr;
        Ok(addr)
    }
}

/// Adler-32, as xdelta3 checks each window's output with.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &c in chunk {
            a += u32::from(c);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// The secondary decoders: LZMA's, one per kind of section, last from window to window.
#[derive(Default)]
struct Secondary {
    lzma: [lzma::Section; 3],
    fgk: [Option<Box<fgk::Fgk>>; 3],
}

impl Secondary {
    /// A compressed section (its decompressed size, then the compressed bytes) decoded with
    /// the patch's compressor; `kind` 0 for data, 1 for instructions, 2 for addresses.
    fn decode(&mut self, id: Option<u8>, kind: usize, section: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(section);
        let size = r.size()?;
        let rest = &section[r.i..];
        match id {
            Some(1) => djw::decode(rest, size),
            Some(2) => self.lzma[kind].decode(rest, size),
            Some(16) => self.fgk(kind).decode(rest, size),
            Some(n) => err(format!(
                "the patch uses xdelta3's {} secondary compression, which UMM cannot read",
                secondary_name(n)
            )),
            None => err("the patch is damaged (a compressed section, but no compressor)"),
        }
    }

    fn fgk(&mut self, kind: usize) -> &mut fgk::Fgk {
        self.fgk[kind].get_or_insert_with(|| Box::new(fgk::Fgk::new()))
    }

    /// A section left uncompressed: FGK's encoder had still run it through its tree when it
    /// was large enough to try.
    fn uncompressed(&mut self, id: Option<u8>, kind: usize, section: &[u8]) -> Result<()> {
        if id == Some(16) && section.len() >= fgk::MIN_INPUT {
            self.fgk(kind).replay(section)?;
        }
        Ok(())
    }
}

const MISMATCH: &str = "the patch does not fit this file: it was made from another version of it";

/// Applies a patch to the source it was made from, returning the target.
pub fn apply(source: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(patch);
    let h = header(&mut r)?;
    let table = code_table();
    let mut secondary = Secondary::default();
    let mut out: Vec<u8> = Vec::new();
    while !r.done() {
        let win = r.byte()?;
        if win & VCD_SOURCE != 0 && win & VCD_TARGET != 0 {
            return err("the patch is damaged (a window copies from both files)");
        }
        // The segment of the source (or of the output so far) that the window copies from.
        let segment: Vec<u8>;
        let seg: &[u8] = if win & (VCD_SOURCE | VCD_TARGET) != 0 {
            let len = r.size()?;
            let pos = r.size()?;
            let from: &[u8] = if win & VCD_SOURCE != 0 { source } else { &out };
            let end = pos.checked_add(len).filter(|&e| e <= from.len());
            let Some(end) = end else {
                return err(if win & VCD_SOURCE != 0 {
                    MISMATCH
                } else {
                    "the patch is damaged (it copies past what it has written)"
                });
            };
            if win & VCD_SOURCE != 0 {
                &source[pos..end]
            } else {
                segment = out[pos..end].to_vec();
                &segment
            }
        } else {
            &[]
        };
        let _delta_len = r.size()?;
        let target_len = r.size()?;
        let delta = r.byte()?;
        let data_len = r.size()?;
        let inst_len = r.size()?;
        let addr_len = r.size()?;
        let checksum = if win & VCD_ADLER32 != 0 {
            let b = r.take(4)?;
            Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        } else {
            None
        };
        let mut sections = [
            Cow::Borrowed(r.take(data_len)?),
            Cow::Borrowed(r.take(inst_len)?),
            Cow::Borrowed(r.take(addr_len)?),
        ];
        for (kind, section) in sections.iter_mut().enumerate() {
            if delta & (1 << kind) != 0 {
                *section = Cow::Owned(secondary.decode(h.secondary, kind, section)?);
            } else {
                secondary.uncompressed(h.secondary, kind, section)?;
            }
        }
        let [data, insts, addrs] = &sections;
        let mut data = Reader::new(data);
        let mut insts = Reader::new(insts);
        let mut addrs = Reader::new(addrs);

        let start = out.len();
        out.reserve(target_len);
        let mut cache = Cache::new();
        let seg_len = seg.len() as u64;
        while !insts.done() {
            let code = insts.byte()?;
            for i in table[code as usize] {
                if i.kind == Kind::Noop {
                    continue;
                }
                let size = match i.size {
                    0 => insts.size()?,
                    s => s as usize,
                };
                if (out.len() - start).saturating_add(size) > target_len {
                    return err("the patch is damaged (it writes past the end of a window)");
                }
                match i.kind {
                    Kind::Noop => {}
                    Kind::Add => out.extend_from_slice(data.take(size)?),
                    Kind::Run => {
                        let b = data.byte()?;
                        out.resize(out.len() + size, b);
                    }
                    Kind::Copy => {
                        let here = seg_len + (out.len() - start) as u64;
                        let addr = cache.decode(i.mode, here, &mut addrs)?;
                        if addr >= here {
                            return err("the patch is damaged (it copies from ahead of itself)");
                        }
                        let addr = addr as usize;
                        if addr + size <= seg.len() {
                            out.extend_from_slice(&seg[addr..addr + size]);
                        } else {
                            // From the window's own output too, which may overlap what is
                            // being written: byte by byte.
                            for k in 0..size {
                                let p = addr + k;
                                let b = if p < seg.len() {
                                    seg[p]
                                } else {
                                    out[start + p - seg.len()]
                                };
                                out.push(b);
                            }
                        }
                    }
                }
            }
        }
        if out.len() - start != target_len {
            return err("the patch is damaged (a window is short)");
        }
        if let Some(sum) = checksum {
            if adler32(&out[start..]) != sum {
                return err(MISMATCH);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: u64) -> Vec<u8> {
        let mut digits = vec![(v & 0x7F) as u8];
        let mut v = v >> 7;
        while v > 0 {
            digits.push((v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
        digits.reverse();
        digits
    }

    /// A one-window patch: its source segment, target length, sections and checksum.
    fn patch(
        app: Option<&str>,
        seg: Option<(usize, usize)>,
        target: &[u8],
        data: &[u8],
        inst: &[u8],
        addr: &[u8],
    ) -> Vec<u8> {
        let mut p = vec![0xD6, 0xC3, 0xC4, 0x00];
        match app {
            Some(a) => {
                p.push(VCD_APPHEADER);
                p.extend(int(a.len() as u64));
                p.extend(a.as_bytes());
            }
            None => p.push(0),
        }
        let mut win = VCD_ADLER32;
        let mut seg_bytes = Vec::new();
        if let Some((len, pos)) = seg {
            win |= VCD_SOURCE;
            seg_bytes.extend(int(len as u64));
            seg_bytes.extend(int(pos as u64));
        }
        p.push(win);
        p.extend(seg_bytes);
        let mut enc = int(target.len() as u64);
        enc.push(0);
        enc.extend(int(data.len() as u64));
        enc.extend(int(inst.len() as u64));
        enc.extend(int(addr.len() as u64));
        enc.extend(adler32(target).to_be_bytes());
        enc.extend(data);
        enc.extend(inst);
        enc.extend(addr);
        p.extend(int(enc.len() as u64));
        p.extend(enc);
        p
    }

    #[test]
    fn adler32_matches_the_reference() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn copies_adds_and_runs() {
        let source = b"Hello, world! Goodbye.";
        let target = b"Hello, Undertale! zzzzzz Goodbye.";
        // COPY 7 bytes from 0 (code 19 + 7 - 3 = size 7, mode 0: code 19 is size 0; sizes
        // 4..=18 are codes 20..=34, so size 7 is code 23), ADD "Undertale" (9 bytes: code
        // 10), ADD "! " (code 3), RUN 6 'z' (code 0, size given), COPY " Goodbye." (9 bytes
        // from 13: code 25, mode 0).
        let inst = [23, 10, 3, 0, 6, 25];
        let data = b"Undertale! z";
        let addr = [0, 13];
        let p = patch(
            Some("new.win//data.win/"),
            Some((source.len(), 0)),
            target,
            data,
            &inst,
            &addr,
        );
        assert_eq!(apply(source, &p).unwrap(), target);
        assert_eq!(source_name(&p).as_deref(), Some("data.win"));
    }

    #[test]
    fn copies_overlapping_its_own_output() {
        // ADD "ab" (code 3), then COPY 6 from address 0 of the target window (its address
        // is the source segment's length, none here): "abababab".
        let p = patch(None, None, b"abababab", b"ab", &[3, 22], &[0]);
        assert_eq!(apply(b"", &p).unwrap(), b"abababab");
        assert_eq!(source_name(&p), None);
    }

    #[test]
    fn a_patch_for_another_source_is_refused() {
        let source = b"Hello, world!";
        let target = b"Hello";
        let p = patch(None, Some((source.len(), 0)), target, b"", &[21], &[0]);
        assert_eq!(apply(source, &p).unwrap(), target);
        let e = apply(b"Howdy, world!", &p).unwrap_err();
        assert!(e.0.contains("another version"), "{e}");
        // Shorter than the segment the patch copies from.
        let e = apply(b"Hi", &p).unwrap_err();
        assert!(e.0.contains("another version"), "{e}");
    }

    #[test]
    fn an_unknown_compressor_is_reported() {
        let mut p = patch(None, None, b"ab", b"ab", &[3], &[]);
        // Header: decompress flag with compressor 7, and the window's data marked compressed.
        p[4] = VCD_DECOMPRESS;
        p.insert(5, 7);
        let delta_at = p.len() - (2 + 1) - 4 - 3 - 1;
        p[delta_at] = 0x01;
        let e = apply(b"", &p).unwrap_err();
        assert!(e.0.contains("number 7"), "{e}");
    }

    #[test]
    fn not_a_patch() {
        assert!(apply(b"", b"PK\x03\x04").is_err());
        assert!(!is_vcdiff(b"xx"));
    }
}
