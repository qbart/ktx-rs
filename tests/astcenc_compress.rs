//! astcenc compressor (`astcenc_compress_image`) vs. golden output of the C++ original
//! (`scripts/golden/gen_astcenc_compress_golden.sh`): whole images over block sizes,
//! profiles, qualities, flag sets, swizzles and data types.
#![allow(clippy::panic, clippy::needless_range_loop)]

mod common;

use std::collections::BTreeMap;

use common::astcenc_gen::Lcg;
use ktx::astcenc::*;

fn fnv(p: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in p {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Texel value in [0, 1] for content `kind` (the harness `texel`).
fn texel(l: &mut Lcg, kind: u32, x: u32, y: u32, z: u32, c: u32) -> f32 {
    let n = l.unit();
    match kind {
        0 => {
            if c == 3 {
                1.0
            } else {
                let a = (x * (c + 1) + y * 3 + z * 5) as f32 / 48.0;
                (a + n * 0.02) - (a as i32) as f32
            }
        }
        1 => n,
        2 => {
            if (x + 2 * y) % 11 < 5 {
                if c == 0 { 0.9 } else { 0.1 + n * 0.05 }
            } else if c == 2 {
                0.8
            } else {
                0.3 + n * 0.05
            }
        }
        3 => {
            if c == 3 {
                ((x + y) % 9) as f32 / 8.0
            } else {
                0.25 + 0.5 * ((x * 7 + y * 3) % 13) as f32 / 12.0
            }
        }
        _ => {
            if c == 1 {
                0.5
            } else {
                0.25
            }
        }
    }
}

/// The harness float to half conversion (round to nearest even, tiny values flushed).
fn to_half(f: f32) -> u16 {
    let u = f.to_bits();
    let e = (u >> 23) & 0xff;
    let m = u & 0x7fffff;
    if f == 0.0 || e < 113 {
        return 0;
    }
    let mut he = e - 112;
    let mut hm = m >> 13;
    let rest = m & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && hm & 1 == 1) {
        hm += 1;
    }
    if hm == 0x400 {
        hm = 0;
        he += 1;
    }
    ((he << 10) | hm) as u16
}

#[test]
fn compress_matches_astcenc() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/astcenc_compress.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    const SIZES: [[u32; 3]; 8] = [
        [4, 4, 1],
        [5, 4, 1],
        [6, 6, 1],
        [8, 8, 1],
        [10, 5, 1],
        [12, 12, 1],
        [3, 3, 3],
        [4, 4, 4],
    ];
    let profiles = [
        AstcencProfile::Ldr,
        AstcencProfile::LdrSrgb,
        AstcencProfile::HdrRgbLdrA,
        AstcencProfile::Hdr,
    ];
    let qualities = [0.0f32, 60.0, 98.0];
    let flag_sets = [
        0,
        ASTCENC_FLG_MAP_NORMAL,
        ASTCENC_FLG_USE_ALPHA_WEIGHT | ASTCENC_FLG_USE_PERCEPTUAL,
        ASTCENC_FLG_USE_DECODE_UNORM8,
    ];
    let mut fails = Vec::new();
    let mut n = 0;
    for (si, s) in SIZES.iter().enumerate() {
        for (pi, &prof) in profiles.iter().enumerate() {
            for (qi, &q) in qualities.iter().enumerate() {
                for fi in 0..4usize {
                    let mut flags = flag_sets[fi];
                    let hdr = pi >= 2;
                    if hdr && fi == 3 {
                        continue;
                    }
                    if pi == 0 && fi == 3 {
                        flags = ASTCENC_FLG_MAP_RGBM;
                    }
                    let dt = (si + pi + qi + fi) % 3;
                    let kind = ((si * 3 + pi + qi * 2 + fi) % 5) as u32;
                    let Ok(c) = astcenc_config_init(prof, s[0], s[1], s[2], q, flags) else {
                        panic!("config_init")
                    };
                    let Ok(mut ctx) = astcenc_context_alloc(&c, 1) else {
                        panic!("context_alloc")
                    };
                    let dx = s[0] * 2 + 1;
                    let dy = s[1] * 2 - 1;
                    let dz = if s[2] == 1 { 1 } else { s[2] + 1 };
                    let mut l = Lcg((si * 1000 + pi * 100 + qi * 10 + fi) as u32);
                    let mut p8 = vec![Vec::new(); dz as usize];
                    let mut p16 = vec![Vec::new(); dz as usize];
                    let mut p32 = vec![Vec::new(); dz as usize];
                    for z in 0..dz {
                        for y in 0..dy {
                            for x in 0..dx {
                                for ch in 0..4 {
                                    let v = texel(&mut l, kind, x, y, z, ch).min(1.0);
                                    let hv = if hdr && ch != 3 { v * 6.0 } else { v };
                                    p8[z as usize].push((v * 255.0 + 0.5) as u8);
                                    p16[z as usize].push(to_half(hv));
                                    p32[z as usize].push(hv);
                                }
                            }
                        }
                    }
                    let img = AstcencImage {
                        dim_x: dx,
                        dim_y: dy,
                        dim_z: dz,
                        data: match dt {
                            0 => AstcencImageData::U8(p8),
                            1 => AstcencImageData::F16(p16),
                            _ => AstcencImageData::F32(p32),
                        },
                    };
                    let swz = if fi == 1 {
                        AstcencSwizzle {
                            r: AstcencSwz::R,
                            g: AstcencSwz::R,
                            b: AstcencSwz::R,
                            a: AstcencSwz::G,
                        }
                    } else {
                        AstcencSwizzle::RGBA
                    };
                    let blocks = dx.div_ceil(s[0]) * dy.div_ceil(s[1]) * dz.div_ceil(s[2]);
                    let mut out = vec![0u8; blocks as usize * 16];
                    if let Err(e) = astcenc_compress_image(&mut ctx, &img, &swz, &mut out, 0) {
                        panic!("compress: {e:?}");
                    }
                    n += 1;
                    let key = format!("s{si} p{pi} q{qi} f{fi}");
                    let got = format!("{:016x}", fnv(&out));
                    match golden.get(&key) {
                        Some(w) if *w == got => {}
                        Some(w) => fails.push(format!("{key}: want {w} got {got}")),
                        None => fails.push(format!("{key}: missing")),
                    }
                }
            }
        }
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
