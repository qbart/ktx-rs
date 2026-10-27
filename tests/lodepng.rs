// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! lodepng encoder and decoder ports vs the original external/lodepng.
//!
//! Not a C test port: reference sizes and FNV-1a hashes come from
//! `scripts/golden/gen_lodepng_golden.sh` (synthetic images in every raw
//! color type / bit depth and size, option variants, and zlib streams of the
//! shared synthetic inputs), recorded in `tests/resources/golden/lodepng.txt`;
//! decoding of those PNGs with extra chunks and corruptions under several raw
//! modes and settings in `tests/resources/golden/lodepng_decode.txt`.
#![allow(clippy::panic)]

mod common;

use common::synthetic::{fnv1a, synthetic_inputs};
use ktx::lodepng::{
    self, ColorType, CompressSettings, FilterStrategy, LCT_GREY, LCT_GREY_ALPHA, LCT_RGB, LCT_RGBA,
    State,
};
use std::collections::HashMap;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

const KINDS: [&str; 8] = [
    "noise",
    "gradient",
    "fewcolors",
    "grey",
    "key",
    "flat",
    "grey4",
    "alphagrad",
];
const FEW: [[u16; 4]; 5] = [
    [10, 200, 30, 255],
    [250, 250, 250, 255],
    [0, 0, 0, 255],
    [90, 10, 160, 255],
    [33, 66, 99, 255],
];

/// 16-bit RGBA of pixel (x, y) for a kind (`pixel` in lodepng_golden.cpp).
fn pixel(rng: &mut Lcg, kind: usize, x: u32, y: u32) -> [u16; 4] {
    match kind {
        0 => [0; 4].map(|_: u16| (rng.next() & 0xffff) as u16),
        1 => {
            let mut p = [0u16; 4];
            for (c, v) in p.iter_mut().enumerate() {
                *v = (((x * 7 + y * 13 + c as u32 * 50) & 255) * 257) as u16;
            }
            p
        }
        2 | 4 => {
            let k = (rng.next() % 5) as usize;
            let mut p = FEW[k].map(|v| v * 257);
            if kind == 4 && k == 0 {
                p[3] = 0;
            }
            p
        }
        3 => {
            let v = ((rng.next() & 255) * 257) as u16;
            [v, v, v, 65535]
        }
        5 => [0; 4],
        6 => {
            let v = ((rng.next() % 16) * 17 * 257) as u16;
            [v, v, v, 65535]
        }
        _ => {
            let mut p = [0u16; 4];
            for (c, v) in p.iter_mut().take(3).enumerate() {
                *v = (((x * 3 + y * 5 + c as u32 * 40) & 255) * 257) as u16;
            }
            p[3] = (((x * 5) & 255) * 257) as u16;
            p
        }
    }
}

fn channels(ct: ColorType) -> usize {
    match ct {
        LCT_GREY => 1,
        LCT_GREY_ALPHA => 2,
        LCT_RGB => 3,
        _ => 4,
    }
}

/// Raw image buffer in the given mode (`make_image`).
fn make_image(kind: usize, ct: ColorType, bd: u32, w: u32, h: u32) -> Vec<u8> {
    let mut rng = Lcg(kind as u32 * 1000 + ct * 100 + bd);
    let nc = channels(ct);
    let mut out = vec![0u8; (w as usize * h as usize * nc * bd as usize).div_ceil(8)];
    let mut bit = 0usize;
    for y in 0..h {
        for x in 0..w {
            let p = pixel(&mut rng, kind, x, y);
            let v: Vec<u16> = match ct {
                LCT_GREY => vec![p[0]],
                LCT_GREY_ALPHA => vec![p[0], p[3]],
                LCT_RGB => vec![p[0], p[1], p[2]],
                _ => p.to_vec(),
            };
            for c in v {
                let val = u32::from(c) >> (16 - bd);
                for b in (0..bd).rev() {
                    if (val >> b) & 1 != 0 {
                        out[bit >> 3] |= 0x80 >> (bit & 7);
                    }
                    bit += 1;
                }
            }
        }
    }
    out
}

fn state_for(ct: ColorType, bd: u32, variant: &str) -> State {
    let mut state = State::new();
    state.info_raw.colortype = ct;
    state.info_raw.bitdepth = bd;
    state.info_png.color.colortype = ct;
    state.info_png.color.bitdepth = bd;
    let info = &mut state.info_png;
    let enc = &mut state.encoder;
    match variant {
        "sbit" => {
            info.sbit_defined = true;
            let v = bd.min(5);
            (info.sbit_r, info.sbit_g, info.sbit_b, info.sbit_a) = (v, v, v, bd);
        }
        "sbitfull" => {
            info.sbit_defined = true;
            (info.sbit_r, info.sbit_g, info.sbit_b, info.sbit_a) = (bd, bd, bd, bd);
        }
        "srgb" => {
            info.srgb_defined = true;
            info.srgb_intent = 0;
            info.chrm_defined = true;
            (info.chrm_white_x, info.chrm_white_y) = (31270, 32900);
            (info.chrm_red_x, info.chrm_red_y) = (64000, 33000);
            (info.chrm_green_x, info.chrm_green_y) = (30000, 60000);
            (info.chrm_blue_x, info.chrm_blue_y) = (15000, 6000);
        }
        "gama" => {
            info.gama_defined = true;
            info.gama_gamma = 100000;
        }
        "f0" => enc.filter_strategy = FilterStrategy::Zero,
        "f1" => enc.filter_strategy = FilterStrategy::One,
        "f2" => enc.filter_strategy = FilterStrategy::Two,
        "f3" => enc.filter_strategy = FilterStrategy::Three,
        "f4" => enc.filter_strategy = FilterStrategy::Four,
        "entropy" => enc.filter_strategy = FilterStrategy::Entropy,
        "brute" => enc.filter_strategy = FilterStrategy::BruteForce,
        "interlace" => info.interlace_method = 1,
        "btype0" => enc.zlibsettings.btype = 0,
        "btype1" => enc.zlibsettings.btype = 1,
        "noauto" => enc.auto_convert = false,
        "noautointerlace" => {
            enc.auto_convert = false;
            info.interlace_method = 1;
        }
        "win32k" => {
            enc.zlibsettings.windowsize = 32768;
            enc.zlibsettings.nicematch = 258;
        }
        "nolz77" => enc.zlibsettings.use_lz77 = false,
        _ => {}
    }
    state
}

fn golden() -> HashMap<String, (u32, usize, String)> {
    let path = common::resources_path().join("golden").join("lodepng.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f.first()?.to_string(),
                (
                    f.get(1)?.parse().ok()?,
                    f.get(2)?.parse().ok()?,
                    f.get(3)?.to_string(),
                ),
            ))
        })
        .collect()
}

fn check(golden: &HashMap<String, (u32, usize, String)>, key: &str, result: Result<Vec<u8>, u32>) {
    let Some((error, size, hash)) = golden.get(key) else {
        panic!("no golden for {key}")
    };
    let (e, out) = match result {
        Ok(v) => (0, v),
        Err(e) => (e, Vec::new()),
    };
    assert_eq!(e, *error, "{key}");
    assert_eq!(out.len(), *size, "{key}");
    assert_eq!(&format!("{:016x}", fnv1a(&out)), hash, "{key}");
}

const MODES: [(ColorType, u32); 11] = [
    (LCT_GREY, 1),
    (LCT_GREY, 2),
    (LCT_GREY, 4),
    (LCT_GREY, 8),
    (LCT_GREY, 16),
    (LCT_GREY_ALPHA, 8),
    (LCT_GREY_ALPHA, 16),
    (LCT_RGB, 8),
    (LCT_RGB, 16),
    (LCT_RGBA, 8),
    (LCT_RGBA, 16),
];
const SIZES: [(u32, u32); 5] = [(1, 1), (3, 2), (17, 9), (64, 48), (150, 100)];
const VARIANTS: [&str; 17] = [
    "sbit",
    "sbitfull",
    "srgb",
    "gama",
    "f0",
    "f1",
    "f2",
    "f3",
    "f4",
    "entropy",
    "brute",
    "interlace",
    "btype0",
    "btype1",
    "noauto",
    "win32k",
    "nolz77",
];

#[test]
fn encode_matches_lodepng() {
    let golden = golden();
    let mut checked = 0;
    for (kind, kind_name) in KINDS.iter().enumerate() {
        for (ct, bd) in MODES {
            for (w, h) in SIZES {
                let base = format!("{kind_name}_{ct}_{bd}_{w}x{h}");
                let image = make_image(kind, ct, bd, w, h);
                let mut variants = vec!["default"];
                if w == 17 || w == 64 {
                    variants.extend_from_slice(&VARIANTS);
                }
                for variant in variants {
                    let mut state = state_for(ct, bd, variant);
                    let result = lodepng::encode(&image, w, h, &mut state);
                    check(&golden, &format!("{base}_{variant}"), result);
                    checked += 1;
                }
            }
        }
    }
    for (name, data) in synthetic_inputs() {
        for variant in 0..4 {
            let mut settings = CompressSettings::default();
            match variant {
                1 if data.is_empty() => continue, // C divides by zero (blocksize 0)
                1 => settings.btype = 1,
                2 => {
                    settings.windowsize = 32768;
                    settings.nicematch = 258;
                }
                3 => settings.lazymatching = false,
                _ => {}
            }
            let result = lodepng::lodepng_zlib_compress(&data, &settings);
            check(&golden, &format!("{name}_zlib{variant}"), result);
            checked += 1;
        }
    }
    assert_eq!(checked, golden.len());
}

/// Encoded PNGs decode back to the input with an independent inflater.
#[test]
fn zlib_round_trips() {
    for (name, data) in synthetic_inputs() {
        let Ok(z) = lodepng::lodepng_zlib_compress(&data, &CompressSettings::default()) else {
            panic!("{name}: compress failed")
        };
        let mut back = vec![0u8; data.len()];
        assert_eq!(
            ktx::zlib::ktx_uncompress_zlib_int(&mut back, &z),
            Ok(data.len()),
            "{name}"
        );
        assert!(back == data, "{name}");
    }
}

#[test]
fn error_text() {
    assert_eq!(
        lodepng::lodepng_error_text(84),
        "given image too small to contain all pixels to be encoded"
    );
    assert_eq!(lodepng::lodepng_error_text(9999), "unknown error code");
    let mut state = State::new();
    assert_eq!(lodepng::encode(&[0u8; 3], 2, 2, &mut state), Err(84));
}

// ---------------------------------------------------------------------------
// Decoder (lodepng_decode_golden.cpp)

fn put32(s: &mut String, v: u32) {
    s.push_str(&format!("{v},"));
}

fn put_bytes(s: &mut String, b: &[u8]) {
    s.push_str(&format!("{}:{:016x},", b.len(), fnv1a(b)));
}

/// `strlen` view of a stored C string.
fn c_str(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

fn put_color(s: &mut String, c: &lodepng::ColorMode) {
    put32(s, c.colortype);
    put32(s, c.bitdepth);
    put32(s, c.palettesize as u32);
    if c.palettesize != 0 {
        put_bytes(s, &c.palette[..c.palettesize * 4]);
    }
    put32(s, u32::from(c.key_defined));
    put32(s, c.key_r);
    put32(s, c.key_g);
    put32(s, c.key_b);
}

/// `info_text` of the harness.
fn info_text(st: &State) -> String {
    let i = &st.info_png;
    let b = |v: bool| u32::from(v);
    let mut s = String::new();
    put_color(&mut s, &i.color);
    s.push('|');
    put_color(&mut s, &st.info_raw);
    s.push('|');
    // Values only when defined: the C leaves them uninitialized otherwise
    let opt = |s: &mut String, defined: bool, values: &[u32]| {
        put32(s, b(defined));
        if defined {
            for &v in values {
                put32(s, v);
            }
        }
    };
    for v in [i.compression_method, i.filter_method, i.interlace_method] {
        put32(&mut s, v);
    }
    opt(
        &mut s,
        i.background_defined,
        &[i.background_r, i.background_g, i.background_b],
    );
    put32(&mut s, i.texts.len() as u32);
    for (k, v) in &i.texts {
        put_bytes(&mut s, c_str(k));
        put_bytes(&mut s, c_str(v));
    }
    put32(&mut s, i.itexts.len() as u32);
    for t in &i.itexts {
        for part in t {
            put_bytes(&mut s, c_str(part));
        }
    }
    opt(&mut s, i.time_defined, &i.time);
    opt(&mut s, i.phys_defined, &[i.phys_x, i.phys_y, i.phys_unit]);
    opt(&mut s, i.gama_defined, &[i.gama_gamma]);
    opt(
        &mut s,
        i.chrm_defined,
        &[
            i.chrm_white_x,
            i.chrm_white_y,
            i.chrm_red_x,
            i.chrm_red_y,
            i.chrm_green_x,
            i.chrm_green_y,
            i.chrm_blue_x,
            i.chrm_blue_y,
        ],
    );
    opt(&mut s, i.srgb_defined, &[i.srgb_intent]);
    put32(&mut s, b(i.iccp_defined));
    if i.iccp_defined {
        put_bytes(&mut s, c_str(&i.iccp_name));
        put_bytes(&mut s, &i.iccp_profile);
    }
    opt(&mut s, i.cicp_defined, &i.cicp);
    opt(&mut s, i.mdcv_defined, &i.mdcv);
    opt(&mut s, i.clli_defined, &i.clli);
    put32(&mut s, b(i.exif_defined));
    if i.exif_defined {
        put_bytes(&mut s, &i.exif);
    }
    opt(
        &mut s,
        i.sbit_defined,
        &[i.sbit_r, i.sbit_g, i.sbit_b, i.sbit_a],
    );
    for u in &i.unknown_chunks_data {
        put_bytes(&mut s, u);
    }
    s
}

#[derive(Clone, Copy)]
struct Settings {
    ignore_crc: bool,
    ignore_adler32: bool,
    ignore_end: bool,
    ignore_critical: bool,
    remember_unknown: bool,
    read_text: bool,
}

const fn settings(
    crc: bool,
    adler: bool,
    end: bool,
    critical: bool,
    unknown: bool,
    text: bool,
) -> Settings {
    Settings {
        ignore_crc: crc,
        ignore_adler32: adler,
        ignore_end: end,
        ignore_critical: critical,
        remember_unknown: unknown,
        read_text: text,
    }
}

const DEF: Settings = settings(false, false, false, false, false, true);

fn apply(st: &mut State, s: Settings) {
    st.decoder.ignore_crc = s.ignore_crc;
    st.decoder.zlibsettings.ignore_adler32 = s.ignore_adler32;
    st.decoder.ignore_end = s.ignore_end;
    st.decoder.ignore_critical = s.ignore_critical;
    st.decoder.remember_unknown_chunks = s.remember_unknown;
    st.decoder.read_text_chunks = s.read_text;
}

/// The width and height `lodepng_inspect` stores before failing.
fn header_dims(png: &[u8]) -> (u32, u32) {
    let be = |p: usize| u32::from_be_bytes([png[p], png[p + 1], png[p + 2], png[p + 3]]);
    if png.len() < 33
        || png[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
        || be(8) != 13
        || &png[12..16] != b"IHDR"
    {
        (0, 0)
    } else {
        (be(16), be(20))
    }
}

/// `decode_case` of the harness: (error, w, h, output, info text).
fn decode_case(png: &[u8], mode: u32, s: Settings) -> (u32, u32, u32, Vec<u8>, String) {
    let mut st = State::new();
    apply(&mut st, s);
    match mode {
        0 => st.decoder.color_convert = false,
        2 => st.info_raw.colortype = LCT_RGB,
        3 => st.info_raw.bitdepth = 16,
        4 => st.info_raw.colortype = LCT_GREY,
        _ => {}
    }
    let (hw, hh) = header_dims(png);
    if mode != 5 {
        return match lodepng::lodepng_decode(&mut st, png) {
            Ok((out, w, h)) => (0, w, h, out, info_text(&st)),
            Err(e) => (e, hw, hh, Vec::new(), info_text(&st)),
        };
    }
    let (idat, w, h) = match lodepng::lodepng_decode_chunks(&mut st, png) {
        Ok(r) => r,
        Err(e) => return (e, hw, hh, Vec::new(), info_text(&st)),
    };
    st.info_raw = st.info_png.color.clone();
    if st.info_png.color.colortype == LCT_GREY && st.info_png.color.key_defined {
        st.info_raw.colortype = LCT_GREY_ALPHA;
    }
    if st.info_png.color.colortype == LCT_RGB && st.info_png.color.key_defined {
        st.info_raw.colortype = LCT_RGBA;
    }
    if st.info_png.color.colortype == lodepng::LCT_PALETTE {
        let trns = lodepng::lodepng_chunk_find_const(png, 33, png.len() - 1, b"tRNS");
        st.info_raw.colortype = if trns.is_some() { LCT_RGBA } else { LCT_RGB };
        st.info_raw.bitdepth = 8;
    }
    let size = lodepng::get_raw_size(w, h, &st.info_raw);
    if size > 1 << 26 {
        return (9999, w, h, Vec::new(), info_text(&st));
    }
    let mut buf = vec![0u8; size + 1];
    match lodepng::lodepng_finish_decode(&mut buf, w, h, &mut st, &idat) {
        Ok(()) => {
            buf.truncate(size);
            (0, w, h, buf, info_text(&st))
        }
        Err(e) => (e, w, h, Vec::new(), info_text(&st)),
    }
}

fn chunk(t: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    c.extend_from_slice(t);
    c.extend_from_slice(data);
    let crc = lodepng::lodepng_crc32(&c[4..]);
    c.extend_from_slice(&crc.to_be_bytes());
    c
}

fn zlib(d: &[u8]) -> Vec<u8> {
    match lodepng::lodepng_zlib_compress(d, &CompressSettings::default()) {
        Ok(v) => v,
        Err(e) => panic!("zlib {e}"),
    }
}

/// `extra_chunks` of the harness.
fn extra_chunks() -> Vec<(&'static str, Vec<u8>)> {
    let z: &[u8] = &[0];
    let cat = |parts: &[&[u8]]| parts.concat();
    let bad_crc = |mut c: Vec<u8>| {
        if let Some(l) = c.last_mut() {
            *l ^= 1;
        }
        c
    };
    vec![
        ("text", chunk(b"tEXt", &cat(&[b"Title", z, b"Hello world"]))),
        ("text_nonull", chunk(b"tEXt", b"Comment")),
        ("text_emptykey", chunk(b"tEXt", &cat(&[z, b"x"]))),
        (
            "text_longkey",
            chunk(b"tEXt", &cat(&[&[b'k'; 80], z, b"x"])),
        ),
        (
            "text_embedded_nul",
            chunk(b"tEXt", &cat(&[b"K", z, b"ab", z, b"cd"])),
        ),
        (
            "ztxt",
            chunk(
                b"zTXt",
                &cat(&[b"Desc", z, z, &zlib(b"compressed text payload")]),
            ),
        ),
        (
            "ztxt_method",
            chunk(b"zTXt", &cat(&[b"Desc", z, &[1], &zlib(b"x")])),
        ),
        ("ztxt_short", chunk(b"zTXt", &cat(&[b"Desc", z]))),
        (
            "ztxt_badzlib",
            chunk(b"zTXt", &cat(&[b"Desc", z, z, b"garbage!"])),
        ),
        (
            "itxt",
            chunk(
                b"iTXt",
                &cat(&[b"Key", z, z, z, b"en", z, b"Schl", z, b"plain"]),
            ),
        ),
        (
            "itxt_z",
            chunk(
                b"iTXt",
                &cat(&[b"Key", z, &[1], z, b"pl", z, z, &zlib(b"zipped itext")]),
            ),
        ),
        ("itxt_short", chunk(b"iTXt", b"Ke")),
        (
            "itxt_method",
            chunk(b"iTXt", &cat(&[b"Key", z, z, &[1], z, z, b"x"])),
        ),
        (
            "itxt_notrans",
            chunk(b"iTXt", &cat(&[b"Key", z, z, z, b"en"])),
        ),
        ("time", chunk(b"tIME", &[7, 230, 10, 23, 12, 30, 59])),
        ("time_bad", chunk(b"tIME", &[7, 230, 10])),
        ("phys", chunk(b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1])),
        ("phys_bad", chunk(b"pHYs", &[0, 0, 11])),
        ("gama", chunk(b"gAMA", &[0, 0, 177, 143])),
        ("gama_bad", chunk(b"gAMA", &[0, 0])),
        ("chrm", chunk(b"cHRM", &[7; 32])),
        ("chrm_bad", chunk(b"cHRM", &[7; 31])),
        ("srgb", chunk(b"sRGB", &[2])),
        ("srgb_bad", chunk(b"sRGB", &[2, 3])),
        (
            "iccp",
            chunk(b"iCCP", &cat(&[b"icc", z, z, &zlib(&[42; 300])])),
        ),
        (
            "iccp_empty",
            chunk(b"iCCP", &cat(&[b"icc", z, z, &zlib(&[])])),
        ),
        (
            "iccp_method",
            chunk(b"iCCP", &cat(&[b"icc", z, &[3], &zlib(&[1])])),
        ),
        (
            "iccp_twice",
            cat(&[
                &chunk(b"iCCP", &cat(&[b"a", z, z, &zlib(&[1, 2])])),
                &chunk(b"iCCP", &cat(&[b"bb", z, z, &zlib(&[3])])),
            ]),
        ),
        ("cicp", chunk(b"cICP", &[1, 13, 0, 1])),
        ("cicp_bad", chunk(b"cICP", &[1, 13, 0])),
        ("mdcv", chunk(b"mDCV", &(1..=24).collect::<Vec<u8>>())),
        ("mdcv_bad", chunk(b"mDCV", &[1, 2])),
        ("clli", chunk(b"cLLI", &[0, 0, 3, 232, 0, 0, 1, 144])),
        ("clli_bad", chunk(b"cLLI", &[0])),
        ("exif", chunk(b"eXIf", &[b'M', b'M', 0, 42, 1, 2, 3])),
        ("exif_empty", chunk(b"eXIf", &[])),
        ("unknown_anc", chunk(b"abCd", &[1, 2, 3])),
        (
            "unknown_two",
            cat(&[&chunk(b"prVt", &[9]), &chunk(b"xyZw", &[])]),
        ),
        ("unknown_critical", chunk(b"ABCD", &[1])),
        ("bad_name", chunk(b"ab1d", &[1])),
        ("reserved", chunk(b"abcd", &[1])),
        ("badcrc", bad_crc(chunk(b"tEXt", &cat(&[b"A", z, b"b"])))),
        ("badcrc_unknown", bad_crc(chunk(b"abCd", &[5]))),
        ("sbit1", chunk(b"sBIT", &[1])),
        ("sbit2", chunk(b"sBIT", &[1, 1])),
        ("sbit3", chunk(b"sBIT", &[1, 2, 3])),
        ("sbit4", chunk(b"sBIT", &[1, 2, 3, 4])),
        ("sbit_zero", chunk(b"sBIT", &[0, 0, 0])),
        ("sbit_big", chunk(b"sBIT", &[9, 9, 9, 9])),
        ("bkgd1", chunk(b"bKGD", &[0])),
        ("bkgd2", chunk(b"bKGD", &[0, 9])),
        ("bkgd6", chunk(b"bKGD", &[0, 1, 0, 2, 0, 3])),
        ("bkgd_badindex", chunk(b"bKGD", &[200])),
        ("trns_grey", chunk(b"tRNS", &[0, 5])),
        ("trns_rgb", chunk(b"tRNS", &[0, 1, 0, 2, 0, 3])),
        ("trns_pal", chunk(b"tRNS", &[1, 2, 3])),
        ("plte", chunk(b"PLTE", &[1, 2, 3, 4, 5, 6])),
        ("plte_bad", chunk(b"PLTE", &[1, 2])),
        ("empty_idat", chunk(b"IDAT", &[])),
        ("iend_early", chunk(b"IEND", &[])),
        ("signature", vec![137, 80, 78, 71, 13, 10, 26, 10]),
    ]
}

fn insert(png: &[u8], extra: &[u8], at_end: bool) -> Vec<u8> {
    let pos = if at_end { png.len() - 12 } else { 33 };
    [&png[..pos], extra, &png[pos..]].concat()
}

fn encode_png(kind: usize, ct: ColorType, bd: u32, w: u32, h: u32, variant: &str) -> Vec<u8> {
    let image = make_image(kind, ct, bd, w, h);
    let mut state = state_for(ct, bd, variant);
    lodepng::encode(&image, w, h, &mut state).unwrap_or_default()
}

#[test]
fn decode_matches_lodepng() {
    let path = common::resources_path()
        .join("golden")
        .join("lodepng_decode.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let golden: HashMap<&str, &str> = text.lines().filter_map(|l| l.split_once(' ')).collect();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    let mut check = |name: String, png: &[u8], mode: u32, s: Settings| {
        let (e, w, h, out, info) = decode_case(png, mode, s);
        let got = format!(
            "{e} {w} {h} {} {:016x} {:016x}",
            out.len(),
            fnv1a(&out),
            fnv1a(info.as_bytes())
        );
        match golden.get(name.as_str()) {
            Some(want) if *want == got => {}
            want => failures.push(format!("{name}: got {got}, want {want:?}\n  {info}")),
        }
        checked += 1;
    };

    const SIZES: [(u32, u32); 4] = [(1, 1), (3, 2), (17, 9), (64, 48)];
    const VARIANTS: [&str; 12] = [
        "sbit",
        "srgb",
        "gama",
        "f1",
        "f2",
        "f3",
        "f4",
        "interlace",
        "btype0",
        "btype1",
        "noauto",
        "noautointerlace",
    ];
    for (kind, kind_name) in KINDS.iter().enumerate() {
        for (ct, bd) in MODES {
            for (w, h) in SIZES {
                let base = format!("{kind_name}_{ct}_{bd}_{w}x{h}");
                let png = encode_png(kind, ct, bd, w, h, "default");
                for mode in 0..6 {
                    check(format!("{base}_default_m{mode}"), &png, mode, DEF);
                }
                if w == 3 || w == 17 {
                    for v in VARIANTS {
                        let p = encode_png(kind, ct, bd, w, h, v);
                        for mode in [0, 1, 5] {
                            check(format!("{base}_{v}_m{mode}"), &p, mode, DEF);
                        }
                    }
                }
            }
        }
    }

    let bases: [(&str, usize, ColorType, u32, &str); 4] = [
        ("pal", 2, LCT_RGB, 8, "default"),
        ("grey", 3, lodepng::LCT_GREY, 8, "noauto"),
        ("rgb16", 1, LCT_RGB, 16, "noauto"),
        ("rgba", 7, LCT_RGBA, 8, "noauto"),
    ];
    let extra_settings = [
        DEF,
        settings(false, false, false, true, true, true),
        settings(false, false, false, false, false, false),
        settings(true, false, false, false, false, true),
    ];
    for (name, kind, ct, bd, variant) in bases {
        let png = encode_png(kind, ct, bd, 5, 4, variant);
        for (ename, extra) in extra_chunks() {
            for at_end in [false, true] {
                let p = insert(&png, &extra, at_end);
                for (si, s) in extra_settings.iter().enumerate() {
                    for mode in [0, 5] {
                        check(
                            format!("extra_{name}_{ename}_w{}_s{si}_m{mode}", u32::from(at_end)),
                            &p,
                            mode,
                            *s,
                        );
                    }
                }
            }
        }
    }

    let corrupt_settings = [
        DEF,
        settings(true, false, false, false, false, true),
        settings(true, true, false, false, false, true),
        settings(true, true, true, true, false, true),
    ];
    for (kind, kind_name) in KINDS.iter().enumerate() {
        for (m, (ct, bd)) in MODES.iter().enumerate() {
            for v in ["default", "interlace", "btype1"] {
                let png = encode_png(kind, *ct, *bd, 17, 9, v);
                let mut rng = Lcg(kind as u32 * 100 + m as u32 * 7 + v.len() as u32);
                for c in 0..8 {
                    let mut p = png.clone();
                    let r = rng.next();
                    if c < 5 {
                        let pos = r as usize % p.len();
                        p[pos] ^= (1 + (r >> 12) % 255) as u8;
                    } else {
                        p.truncate(r as usize % p.len());
                    }
                    for (si, s) in corrupt_settings.iter().enumerate() {
                        let mode = if si == 3 { 5 } else { 1 };
                        check(
                            format!("corrupt_{kind_name}_{m}_{v}_c{c}_s{si}"),
                            &p,
                            mode,
                            *s,
                        );
                    }
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} decode cases differ:\n{}",
        failures.len(),
        failures
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(checked, golden.len());
}

/// lodepng's inflater decodes the zlib streams of its own compressor.
#[test]
fn zlib_decompress_round_trips() {
    for (name, data) in synthetic_inputs() {
        let Ok(z) = lodepng::lodepng_zlib_compress(&data, &CompressSettings::default()) else {
            panic!("{name}: compress failed")
        };
        let back = lodepng::lodepng_zlib_decompress(&z, &lodepng::DecompressSettings::default());
        assert!(back.as_deref() == Ok(&data[..]), "{name}");
    }
}

/// Random corruptions of every color type never panic the decoder.
#[test]
fn decode_corrupted_never_panics() {
    let mut rng = Lcg(77);
    for (kind, (ct, bd)) in MODES.iter().enumerate() {
        for v in ["default", "interlace", "btype0", "noauto"] {
            let png = encode_png(kind % KINDS.len(), *ct, *bd, 23, 11, v);
            for _ in 0..200 {
                let mut p = png.clone();
                for _ in 0..1 + rng.next() % 4 {
                    let r = rng.next();
                    let pos = 33 + r as usize % (p.len() - 33);
                    p[pos] = (r >> 13) as u8;
                }
                for mode in [0, 1, 4, 5] {
                    let _ = decode_case(&p, mode, settings(true, true, true, true, true, true));
                }
            }
        }
    }
}
