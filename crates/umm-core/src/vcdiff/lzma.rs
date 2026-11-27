//! xdelta3's LZMA secondary compressor: for each kind of section (data, instructions,
//! addresses), one xz stream (LZMA2, no check) running through the whole patch, flushed at
//! the end of each window's section and never finished. The decoder for a kind of section
//! is kept from one window to the next, as xdelta3's is, and fed each window's section.

use super::{err, Result, VcdiffError};
use lzma_rust2::Lzma2Reader;
use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

/// The compressed bytes fed so far and not yet read.
#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    pos: usize,
}

/// The decoder's input: what has been fed to it.
#[derive(Clone, Default)]
struct Feed(Rc<RefCell<Buffer>>);

impl Read for Feed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut b = self.0.borrow_mut();
        let n = buf.len().min(b.bytes.len() - b.pos);
        let start = b.pos;
        buf[..n].copy_from_slice(&b.bytes[start..start + n]);
        b.pos += n;
        Ok(n)
    }
}

/// One kind of section's decoder.
#[derive(Default)]
pub(super) struct Section {
    feed: Feed,
    reader: Option<Lzma2Reader<Feed>>,
}

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const LZMA2: u64 = 0x21;

/// An xz multibyte integer: 7 bits a byte, least significant first.
fn xz_int(b: &[u8], at: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let Some(&c) = b.get(*at) else {
            return err("the patch is damaged (LZMA: a short block header)");
        };
        *at += 1;
        v |= u64::from(c & 0x7F) << (7 * i);
        if c & 0x80 == 0 {
            return Ok(v);
        }
    }
    err("the patch is damaged (LZMA: a block header number is too large)")
}

/// The dictionary size an LZMA2 filter's property byte gives.
fn dict_size(p: u8) -> Result<u32> {
    match p {
        40 => Ok(u32::MAX),
        p if p < 40 => Ok((2 | u32::from(p & 1)) << (p / 2 + 11)),
        _ => err("the patch is damaged (LZMA: an invalid dictionary size)"),
    }
}

/// Reads the xz stream's header and its block's, at the start of `b`: the LZMA2 data's
/// offset and dictionary size.
fn xz_headers(b: &[u8]) -> Result<(usize, u32)> {
    if b.len() < 12 || b[..6] != XZ_MAGIC {
        return err("the patch is damaged (LZMA: no xz stream header)");
    }
    let block = &b[12..];
    let Some(&size_byte) = block.first() else {
        return err("the patch is damaged (LZMA: no block)");
    };
    if size_byte == 0 {
        return err("the patch is damaged (LZMA: no block)");
    }
    let header_len = (usize::from(size_byte) + 1) * 4;
    if block.len() < header_len {
        return err("the patch is damaged (LZMA: a short block header)");
    }
    let flags = block[1];
    let mut at = 2;
    if flags & 0x40 != 0 {
        xz_int(block, &mut at)?;
    }
    if flags & 0x80 != 0 {
        xz_int(block, &mut at)?;
    }
    let filters = usize::from(flags & 0x03) + 1;
    let mut dict = None;
    for _ in 0..filters {
        let id = xz_int(block, &mut at)?;
        let len = xz_int(block, &mut at)? as usize;
        let props = block.get(at..at + len).ok_or_else(|| {
            VcdiffError("the patch is damaged (LZMA: a short block header)".into())
        })?;
        at += len;
        match (id, props) {
            (LZMA2, [p]) => dict = Some(dict_size(*p)?),
            _ => {
                return err(format!(
                    "the patch uses an xz filter UMM cannot read (number {id:#x})"
                ))
            }
        }
    }
    let dict =
        dict.ok_or_else(|| VcdiffError("the patch is damaged (LZMA: no LZMA2 filter)".into()))?;
    Ok((12 + header_len, dict))
}

impl Section {
    /// Decodes this window's section into `size` bytes, all its input used.
    pub(super) fn decode(&mut self, input: &[u8], size: usize) -> Result<Vec<u8>> {
        {
            let mut b = self.feed.0.borrow_mut();
            let used = b.pos;
            b.bytes.drain(..used);
            b.pos = 0;
            b.bytes.extend_from_slice(input);
        }
        if self.reader.is_none() {
            let (offset, dict) = xz_headers(&self.feed.0.borrow().bytes)?;
            self.feed.0.borrow_mut().pos = offset;
            self.reader = Some(Lzma2Reader::new(self.feed.clone(), dict, None));
        }
        let reader = self.reader.as_mut().expect("set above");
        let mut out = vec![0u8; size];
        reader
            .read_exact(&mut out)
            .map_err(|e| VcdiffError(format!("the patch is damaged (LZMA: {e})")))?;
        let b = self.feed.0.borrow();
        if b.pos != b.bytes.len() {
            return err("the patch is damaged (LZMA: unused input)");
        }
        Ok(out)
    }
}
