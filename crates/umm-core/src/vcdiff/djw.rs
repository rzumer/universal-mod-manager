//! xdelta3's DJW secondary compressor ("static Huffman"): each compressed section stands
//! alone. A port of `xd3_decode_huff` (xdelta3-djw.h): one to eight Huffman tables, their
//! code lengths themselves Huffman coded after move-to-front and run-length (1/2) coding, and
//! with several tables, which one codes each sector of the output.

use super::{err, Result, VcdiffError};

const ALPHABET_SIZE: usize = 256;
const MAX_CODELEN: usize = 20;
const TOTAL_CODES: usize = MAX_CODELEN + 2;
const RUN_1: usize = 1;
const BASIC_CODES: usize = 5;
const RUN_CODES: usize = 2;
const EXTRA_12OFFSET: usize = BASIC_CODES + RUN_CODES;
const EXTRA_CODE_BITS: u32 = 4;
const GROUP_BITS: u32 = 3;
const SECTORSZ_MULT: usize = 5;
const SECTORSZ_BITS: u32 = 5;
const MAX_CLCLEN: usize = 15;
const CLCLEN_BITS: u32 = 4;
const MAX_GBCLEN: usize = 7;
const GBCLEN_BITS: u32 = 3;

const ENCODE_12EXTRA: [u8; 15] = [9, 10, 3, 11, 2, 12, 13, 1, 14, 15, 16, 17, 18, 19, 20];
const ENCODE_12BASIC: [u8; BASIC_CODES] = [4, 5, 6, 7, 8];

fn corrupt<T>() -> Result<T> {
    err("the patch is damaged (DJW: an invalid code)")
}

/// Bits, read from each byte's lowest first.
struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    byte: u8,
    mask: u32,
}

impl<'a> Bits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Bits {
            input,
            pos: 0,
            byte: 0,
            mask: 0x100,
        }
    }

    fn bit(&mut self) -> Result<bool> {
        if self.mask == 0x100 {
            let Some(&b) = self.input.get(self.pos) else {
                return err("the patch ends early (DJW: end of input)");
            };
            self.pos += 1;
            self.byte = b;
            self.mask = 1;
        }
        let set = u32::from(self.byte) & self.mask != 0;
        self.mask <<= 1;
        Ok(set)
    }

    /// An `n`-bit value, most significant bit first.
    fn bits(&mut self, n: u32) -> Result<usize> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | usize::from(self.bit()?);
        }
        Ok(v)
    }

    fn symbol(&mut self, h: &Huffman, max_sym: usize) -> Result<usize> {
        let (mut code, mut bits) = (0i64, 0usize);
        loop {
            if bits == h.max_clen {
                return corrupt();
            }
            bits += 1;
            code = (code << 1) | i64::from(self.bit()?);
            if bits >= h.min_clen && code <= h.limit[bits] {
                break;
            }
        }
        if h.base[bits] <= code {
            let offset = (code - h.base[bits]) as usize;
            if offset <= max_sym {
                if let Some(&s) = h.inorder.get(offset) {
                    return Ok(s);
                }
            }
        }
        corrupt()
    }
}

/// A canonical Huffman decoder, as `djw_build_decoder` builds it.
struct Huffman {
    inorder: Vec<usize>,
    base: Vec<i64>,
    limit: Vec<i64>,
    min_clen: usize,
    max_clen: usize,
}

impl Huffman {
    fn new(clen: &[u8], abs_max: usize) -> Result<Self> {
        let mut nr = vec![0i64; abs_max + 2];
        for &c in clen {
            if c as usize > abs_max {
                return corrupt();
            }
            nr[c as usize] += 1;
        }
        let min_clen = (1..=abs_max).find(|&i| nr[i] != 0).unwrap_or(abs_max + 1);
        let max_clen = (1..=abs_max).rev().find(|&i| nr[i] != 0).unwrap_or(0);
        let mut tmp_base = vec![0i64; abs_max + 2];
        let mut base = vec![0i64; abs_max + 2];
        let mut limit = vec![0i64; abs_max + 2];
        if min_clen <= abs_max {
            limit[min_clen] = nr[min_clen] - 1;
            for i in min_clen + 1..=max_clen {
                let last_limit = (limit[i - 1] + 1) << 1;
                tmp_base[i] = tmp_base[i - 1] + nr[i - 1];
                limit[i] = last_limit + nr[i] - 1;
                base[i] = last_limit - tmp_base[i];
            }
        }
        let mut inorder = vec![0usize; clen.iter().filter(|&&c| c != 0).count()];
        for (sym, &c) in clen.iter().enumerate() {
            if c != 0 {
                let at = &mut tmp_base[c as usize];
                inorder[*at as usize] = sym;
                *at += 1;
            }
        }
        Ok(Huffman {
            inorder,
            base,
            limit,
            min_clen,
            max_clen,
        })
    }
}

fn update_mtf(mtf: &mut [u8], i: usize) -> Result<u8> {
    if i >= mtf.len() {
        return corrupt();
    }
    let sym = mtf[i];
    mtf.copy_within(0..i, 1);
    mtf[0] = sym;
    Ok(sym)
}

/// `djw_decode_1_2`: `elts` values, move-to-front coded with runs of the front value in
/// bijective base 2 (RUN_0, RUN_1). With `skip_offset`, a value whose counterpart that far
/// back is zero is zero too (code lengths after the first table's).
fn decode_1_2(
    bits: &mut Bits,
    h: &Huffman,
    mtf: &mut [u8],
    elts: usize,
    skip_offset: usize,
    values: &mut [u8],
) -> Result<()> {
    let (mut n, mut rep, mut next, mut s) = (0usize, 0usize, 0usize, 0u32);
    while n < elts {
        if skip_offset != 0 && n >= skip_offset && values[n - skip_offset] == 0 {
            values[n] = 0;
            n += 1;
            continue;
        }
        if rep != 0 {
            values[n] = mtf[0];
            n += 1;
            rep -= 1;
            continue;
        }
        if next != 0 {
            values[n] = update_mtf(mtf, next)?;
            n += 1;
            next = 0;
            continue;
        }
        let sym = bits.symbol(h, TOTAL_CODES)?;
        if sym <= RUN_1 {
            rep = (sym + 1)
                .checked_shl(s)
                .filter(|r| *r <= elts)
                .ok_or_else(|| VcdiffError("the patch is damaged (DJW: a long run)".into()))?;
            s += 1;
        } else {
            next = sym - 1;
            s = 0;
        }
    }
    if rep != 0 {
        return err("the patch is damaged (DJW: an invalid repeat code)");
    }
    Ok(())
}

/// Decodes a DJW section into `size` bytes, all its input used.
pub(super) fn decode(input: &[u8], size: usize) -> Result<Vec<u8>> {
    if size == 0 {
        return err("the patch is damaged (DJW: an empty section)");
    }
    let mut bits = Bits::new(input);
    let groups = bits.bits(GROUP_BITS)? + 1;
    let sector_size = if groups > 1 {
        (bits.bits(SECTORSZ_BITS)? + 1) * SECTORSZ_MULT
    } else {
        size
    };
    let sectors = 1 + (size - 1) / sector_size;

    // The code lengths' decoder, then each table's code lengths.
    let extra = bits.bits(EXTRA_CODE_BITS)? + EXTRA_12OFFSET;
    let mut cl_clen = [0u8; TOTAL_CODES];
    for c in cl_clen.iter_mut().take(extra) {
        *c = bits.bits(CLCLEN_BITS)? as u8;
    }
    let cl = Huffman::new(&cl_clen, MAX_CLCLEN)?;
    let mut cl_mtf = [0u8; TOTAL_CODES];
    cl_mtf[1..1 + BASIC_CODES].copy_from_slice(&ENCODE_12BASIC);
    cl_mtf[1 + BASIC_CODES..1 + BASIC_CODES + ENCODE_12EXTRA.len()]
        .copy_from_slice(&ENCODE_12EXTRA);
    let mut clen = vec![0u8; ALPHABET_SIZE * groups];
    decode_1_2(
        &mut bits,
        &cl,
        &mut cl_mtf,
        ALPHABET_SIZE * groups,
        ALPHABET_SIZE,
        &mut clen,
    )?;
    let tables: Vec<Huffman> = clen
        .chunks(ALPHABET_SIZE)
        .map(|c| Huffman::new(c, MAX_CODELEN))
        .collect::<Result<_>>()?;

    // Which table codes each sector.
    let mut selected = vec![0u8; sectors];
    if groups > 1 {
        let mut sel_clen = vec![0u8; groups + 1];
        let mut sel_mtf = vec![0u8; groups + 1];
        for (gp, c) in sel_clen.iter_mut().enumerate() {
            *c = bits.bits(GBCLEN_BITS)? as u8;
            sel_mtf[gp] = gp as u8;
        }
        let sel = Huffman::new(&sel_clen, MAX_GBCLEN)?;
        decode_1_2(&mut bits, &sel, &mut sel_mtf, sectors, 0, &mut selected)?;
    }

    let mut out = Vec::with_capacity(size);
    for &gp in &selected {
        let Some(table) = tables.get(gp as usize) else {
            return corrupt();
        };
        let n = sector_size.min(size - out.len());
        for _ in 0..n {
            out.push(bits.symbol(table, ALPHABET_SIZE)? as u8);
        }
    }
    if bits.pos != input.len() {
        return err("the patch is damaged (DJW: unused input)");
    }
    Ok(out)
}
