//! lodepng encoder port vs the original external/lodepng.
//!
//! Not a C test port: reference sizes and FNV-1a hashes come from
//! `scripts/golden/gen_lodepng_golden.sh` (synthetic images in every raw
//! color type / bit depth and size, option variants, and zlib streams of the
//! shared synthetic inputs), recorded in `tests/resources/golden/lodepng.txt`.
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
