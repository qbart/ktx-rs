//! basisu UASTC RDO post-pass (`uastc_rdo` in `basisu_uastc_enc.cpp`) vs. golden output of the
//! C++ original (`scripts/golden/gen_basisu_uastc_rdo_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::enc::{ColorRgba, color_rgba};
use ktx::basisu::uastc_enc::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

struct H(u64);

impl H {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

const BW: u32 = 24;
const BH: u32 = 16;

fn make_image(img: u32) -> Vec<ColorRgba> {
    let mut l = Lcg(img.wrapping_mul(7919).wrapping_add(17));
    let mut px = vec![[0u8; 4]; (BW * BH * 16) as usize];
    let kind = img % 4;
    if img == 10 {
        // Blocks tiled from 4 tiles (tile 0 very noisy, so skipped) to repeat selector patterns.
        let mut tiles = [[[0i32; 3]; 16]; 4];
        for t in 0..4 {
            for i in 0..16 {
                for c in 0..3 {
                    tiles[t][i][c] = if t != 0 {
                        (l.next() % 40) as i32 + 60 * t as i32
                    } else {
                        (l.next() & 255) as i32
                    };
                }
            }
        }
        for b in 0..(BW * BH) as usize {
            let (t, o) = ((l.next() % 4) as usize, (l.next() % 3) as i32);
            for i in 0..16 {
                let p = tiles[t][i];
                px[b * 16 + i] = color_rgba(p[0] + o, p[1], p[2] + o, 255);
            }
        }
        return px;
    }
    let noise = if img >= 8 {
        24 + (img - 8) * 16
    } else {
        1 + (img % 3) * 6
    } as i32;
    let alpha = img % 5 == 3 || img == 9;
    for by in 0..BH {
        for bx in 0..BW {
            for i in 0..16u32 {
                let (x, y) = ((bx * 4 + (i & 3)) as i32, (by * 4 + (i >> 2)) as i32);
                let (r, g, b) = match kind {
                    0 => (x * 2, y * 3, x + y),
                    1 => {
                        let c = if (x / 8 + y / 8) & 1 != 0 { 200 } else { 40 };
                        (c, c / 2 + x, 255 - c)
                    }
                    2 => {
                        let s = (x * x + y * y) / 37;
                        ((s * 13) & 255, (s * 7) & 255, (s * 3) & 255)
                    }
                    _ => {
                        let c = ((x ^ y) & 15) * 16;
                        (c, c, c)
                    }
                };
                let m = (2 * noise + 1) as u32;
                let n0 = (l.next() % m) as i32 - noise;
                let n1 = (l.next() % m) as i32 - noise;
                let n2 = (l.next() % m) as i32 - noise;
                let a = if alpha { (x * 5 + y * 3) & 255 } else { 255 };
                px[((by * BW + bx) * 16 + i) as usize] = color_rgba(r + n0, g + n1, b + n2, a);
            }
        }
    }
    px
}

struct Cfg {
    enc_flags: u32,
    rdo_flags: u32,
    jobs: u32,
    dict: u32,
    literal: u32,
    lambda: f32,
    ratio: f32,
    skip: f32,
    smooth_dev: f32,
    smooth_scale: f32,
    refine: bool,
}

const fn c(
    enc_flags: u32,
    rdo_flags: u32,
    jobs: u32,
    dict: u32,
    literal: u32,
    f: [f32; 5],
    refine: bool,
) -> Cfg {
    Cfg {
        enc_flags,
        rdo_flags,
        jobs,
        dict,
        literal,
        lambda: f[0],
        ratio: f[1],
        skip: f[2],
        smooth_dev: f[3],
        smooth_scale: f[4],
        refine,
    }
}

const D: [f32; 5] = [2.0, 10.0, 8.0, 18.0, 10.0];

const CFGS: [Cfg; 16] = [
    c(2, 2, 1, 4096, 100, [0.5, 10.0, 8.0, 18.0, 10.0], true),
    c(2, 2, 1, 4096, 100, [1.0, 10.0, 8.0, 18.0, 10.0], true),
    c(2, 2, 1, 4096, 100, [3.0, 10.0, 8.0, 18.0, 10.0], true),
    c(2, 2, 1, 4096, 100, [10.0, 10.0, 30.0, 18.0, 10.0], true),
    c(2, 2, 1, 4096, 100, [3.0, 10.0, 8.0, 18.0, 10.0], false),
    c(2, 2, 1, 256, 100, [2.0, 1.25, 8.0, 18.0, 10.0], true),
    c(2, 2, 1, 32768, 50, [2.0, 10.0, 40.0, 40.0, 1.0], true),
    c(2, 2, 4, 4096, 100, D, true),
    c(2, 2, 7, 4096, 100, D, true),
    c(0, 0, 1, 4096, 100, D, true),
    c(1, 1, 1, 4096, 100, D, true),
    c(3, 3, 1, 4096, 100, D, true),
    c(2, 4, 1, 4096, 100, D, true),
    c(2, 2 | C_PACK_UASTC_FAVOR_UASTC_ERROR, 1, 4096, 100, D, true),
    c(2, 2 | C_PACK_UASTC_ETC1_FASTER_HINTS, 1, 4096, 100, D, true),
    c(
        2 | C_PACK_UASTC_FAVOR_SIMPLER_MODES,
        2 | C_PACK_UASTC_ETC1_DISABLE_FLIP_AND_INDIVIDUAL,
        1,
        4096,
        200,
        [1.5, 3.0, 12.0, 18.0, 10.0],
        true,
    ),
];

#[test]
fn uastc_rdo_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_uastc_rdo.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let n = (BW * BH) as usize;
    let mut fails = Vec::new();
    let mut total = 0;
    for img in 0..11u32 {
        let px = make_image(img);
        let mut last_enc = u32::MAX;
        let mut enc = vec![[0u8; 16]; n];
        for (ci, cfg) in CFGS.iter().enumerate() {
            if cfg.enc_flags != last_enc {
                for b in 0..n {
                    let mut raw = [0u8; 64];
                    for i in 0..16 {
                        raw[i * 4..i * 4 + 4].copy_from_slice(&px[b * 16 + i]);
                    }
                    encode_uastc(&raw, &mut enc[b], cfg.enc_flags);
                }
                last_enc = cfg.enc_flags;
            }
            let mut blocks = enc.clone();
            let p = UastcRdoParams {
                lz_dict_size: cfg.dict,
                lz_literal_cost: cfg.literal,
                lambda: cfg.lambda,
                max_allowed_rms_increase_ratio: cfg.ratio,
                skip_block_rms_thresh: cfg.skip,
                max_smooth_block_std_dev: cfg.smooth_dev,
                smooth_block_max_error_scale: cfg.smooth_scale,
                endpoint_refinement: cfg.refine,
            };
            let (ok, stats) = uastc_rdo(n as u32, &mut blocks, &px, &p, cfg.rdo_flags, cfg.jobs);
            let changed = blocks.iter().zip(&enc).filter(|(a, b)| a != b).count();
            let mut h = H(0xcbf29ce484222325);
            h.bytes(&[ok as u8]);
            for b in &blocks {
                h.bytes(b);
            }
            let got = format!("{:016x} {changed}", h.0);
            let key = format!("{img}_{ci}");
            total += 1;
            match golden.get(&key) {
                Some(w) if *w == got => {}
                Some(w) => fails.push(format!("{key}: want {w} got {got} ({stats:?})")),
                None => fails.push(format!("{key}: missing")),
            }
        }
    }
    if total != golden.len() {
        fails.push(format!("golden has {} cases, ran {total}", golden.len()));
    }
    if !fails.is_empty() {
        panic!("{} mismatches:\n{}", fails.len(), fails.join("\n"));
    }
}
