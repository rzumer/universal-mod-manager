//! Patches written by xdelta3 3.1.1 itself (in `fixtures/xdelta`), with each of its
//! secondary compressors: none, DJW, LZMA (its default) and FGK. The files they
//! patch are generated here as they were for xdelta3:
//!
//! - `source`: 200,000 bytes; `target`: `source` with `<mod N>` after every 10,000 bytes.
//!   Patches `none`, `djw`, `lzma`, `fgk` (`-W 16384`: several windows) and `default`.
//!   xdelta3 cannot read its own `fgk`: see [`fgk_patches_xdelta3_cannot_read_apply`].
//! - `text`: words, as a mod's new data would be; patched from nothing (no source).
//!   Patches `text-<compressor>` and `text-<compressor>-w16k` (several windows).
//! - `skew`: letters of uneven frequencies, their case changing every 4,000; patched from
//!   nothing. Patches `skew-djw` (`-S djw9`: several Huffman tables, chosen sector by
//!   sector) and `skew-fgk`.

use std::path::PathBuf;
use umm_core::vcdiff;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xdelta")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn lcg(x: &mut u64) -> u64 {
    *x = (*x * 1_103_515_245 + 12_345) % (1 << 31);
    *x >> 16
}

fn source() -> Vec<u8> {
    let mut x = 12_345;
    (0..200_000)
        .map(|_| b"abcdefgh \n"[(lcg(&mut x) % 10) as usize])
        .collect()
}

fn target() -> Vec<u8> {
    source()
        .chunks(10_000)
        .enumerate()
        .flat_map(|(i, c)| [c.to_vec(), format!("<mod {i}>").into_bytes()].concat())
        .collect()
}

fn text() -> Vec<u8> {
    let words: [&[u8]; 12] = [
        b"the",
        b"undertale",
        b"data",
        b"win",
        b"patch",
        b"mod",
        b"sans",
        b"papyrus",
        b"toriel",
        b"a",
        b"of",
        b"and",
    ];
    let mut x = 777;
    let mut out = Vec::new();
    for i in 0..12_000 {
        out.extend_from_slice(words[(lcg(&mut x) % 12) as usize]);
        out.push(if i % 13 == 12 { b'\n' } else { b' ' });
    }
    out
}

/// Letter `k` with probability about 1/2^(k+1).
fn skew() -> Vec<u8> {
    let mut x = 4_242;
    (0..60_000)
        .map(|i| {
            let mut r = lcg(&mut x) % 1000;
            let mut k = 0;
            while k < 20 && r >= 500 >> k {
                r -= 500 >> k;
                k += 1;
            }
            97 + k as u8 + if (i / 4_000) % 2 == 1 { 32 } else { 0 }
        })
        .collect()
}

#[test]
fn the_generated_files_match_xdelta3s() {
    assert_eq!(text().len(), 62_035);
    assert_eq!(
        target().len(),
        200_000 + (0..20).map(|i| format!("<mod {i}>").len()).sum::<usize>()
    );
}

#[test]
fn xdelta3_patches_apply_with_each_compressor() {
    let (source, target) = (source(), target());
    for name in ["none", "djw", "lzma", "default"] {
        let patch = fixture(&format!("{name}.xdelta"));
        let out = vcdiff::apply(&source, &patch).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out == target, "{name}: wrong output");
        assert_eq!(vcdiff::source_name(&patch).as_deref(), Some("source.bin"));
    }
    let text = text();
    for name in [
        "text-none",
        "text-djw",
        "text-djw-w16k",
        "text-lzma",
        "text-lzma-w16k",
        "text-fgk",
        "text-fgk-w16k",
    ] {
        let patch = fixture(&format!("{name}.xdelta"));
        let out = vcdiff::apply(&[], &patch).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out == text, "{name}: wrong output");
    }
    for name in ["skew-djw", "skew-fgk"] {
        let out = vcdiff::apply(&[], &fixture(&format!("{name}.xdelta"))).unwrap();
        assert!(out == skew(), "{name}: wrong output");
    }
}

#[test]
fn compressed_patches_still_tell_a_wrong_source() {
    let mut other = source();
    other[5_000] ^= 1;
    for name in ["djw", "lzma", "fgk"] {
        let e = vcdiff::apply(&other, &fixture(&format!("{name}.xdelta"))).unwrap_err();
        assert!(e.0.contains("another version"), "{name}: {e}");
    }
}

/// Most of `fgk`'s sections saved too little compressed, so xdelta3 left them uncompressed,
/// though its encoder had run them through its adaptive tree; its decoder then works from
/// another tree and fails ("target window checksum mismatch"). UMM replays them.
#[test]
fn fgk_patches_xdelta3_cannot_read_apply() {
    let out = vcdiff::apply(&source(), &fixture("fgk.xdelta")).unwrap();
    assert!(out == target(), "fgk: wrong output");
}
