//! astcenc decompressor port vs the original astc-encoder.
//!
//! Not a C test port: reference hashes come from
//! `scripts/golden/gen_astcenc_golden.sh` (pseudo random ASTC data for every
//! 2D and 3D block size, every profile, U8 / F16 / F32 output and two
//! swizzles), recorded in `tests/resources/golden/astcenc.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::astcenc::*;
use std::collections::HashMap;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

/// `make_block` in astcenc_golden.cpp.
fn make_block(rng: &mut Lcg, b: &mut [u8], i: u32) {
    for v in b.iter_mut().take(16) {
        *v = rng.next() as u8;
    }
    if i % 8 == 7 {
        let ve: u32 = if rng.next() & 1 != 0 { 0x1FC } else { 0x3FC };
        b[0] = (ve & 0xFF) as u8;
        b[1] = ((u32::from(b[1]) & 0xF8) | (ve >> 8) | 0x0C) as u8;
        if i % 16 == 7 {
            for (k, v) in b.iter_mut().enumerate().take(8).skip(1) {
                *v |= if k == 1 { 0xFE } else { 0xFF };
            }
            b[1] = ((ve >> 8) | 0xFC) as u8;
        }
    }
}

fn fnv1a(h: u64, data: &[u8]) -> u64 {
    data.iter()
        .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

#[test]
fn decompress_matches_astcenc() {
    let path = common::resources_path().join("golden").join("astcenc.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let golden: HashMap<&str, (i32, &str)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((*f.first()?, (f.get(1)?.parse().ok()?, *f.get(2)?)))
        })
        .collect();

    const SIZES: [(u32, u32, u32); 24] = [
        (4, 4, 1),
        (5, 4, 1),
        (5, 5, 1),
        (6, 5, 1),
        (6, 6, 1),
        (8, 5, 1),
        (8, 6, 1),
        (8, 8, 1),
        (10, 5, 1),
        (10, 6, 1),
        (10, 8, 1),
        (10, 10, 1),
        (12, 10, 1),
        (12, 12, 1),
        (3, 3, 3),
        (4, 3, 3),
        (4, 4, 3),
        (4, 4, 4),
        (5, 4, 4),
        (5, 5, 4),
        (5, 5, 5),
        (6, 5, 5),
        (6, 6, 5),
        (6, 6, 6),
    ];
    const PROFILES: [AstcencProfile; 4] = [
        AstcencProfile::LdrSrgb,
        AstcencProfile::Ldr,
        AstcencProfile::HdrRgbLdrA,
        AstcencProfile::Hdr,
    ];
    let swizzles = [
        AstcencSwizzle::RGBA,
        AstcencSwizzle {
            r: AstcencSwz::A,
            g: AstcencSwz::Z,
            b: AstcencSwz::One,
            a: AstcencSwz::Zero,
        },
    ];
    let mut checked = 0;
    for (bx, by, bz) in SIZES {
        let (blocks_x, blocks_y, blocks_z) = (7, 5, if bz > 1 { 2 } else { 1 });
        let dim_x = blocks_x * bx - 1;
        let dim_y = blocks_y * by - 2;
        let dim_z = if bz > 1 { blocks_z * bz - 1 } else { 1 };
        let nblocks = blocks_x * blocks_y * blocks_z;
        let mut data = vec![0u8; nblocks as usize * 16];
        let mut rng = Lcg(bx * 10000 + by * 100 + bz);
        for i in 0..nblocks {
            make_block(&mut rng, &mut data[i as usize * 16..], i);
        }
        for (p, profile) in PROFILES.iter().enumerate() {
            for t in 0..3 {
                for (sw, swizzle) in swizzles.iter().enumerate() {
                    let config = match astcenc_config_init(
                        *profile,
                        bx,
                        by,
                        bz,
                        ASTCENC_PRE_MEDIUM,
                        ASTCENC_FLG_DECOMPRESS_ONLY,
                    ) {
                        Ok(c) => c,
                        Err(e) => panic!("{e:?}"),
                    };
                    let mut ctx = match astcenc_context_alloc(&config, 1) {
                        Ok(c) => c,
                        Err(e) => panic!("{e:?}"),
                    };
                    let n = (dim_x * dim_y * 4) as usize;
                    let planes = dim_z as usize;
                    let mut image = AstcencImage {
                        dim_x,
                        dim_y,
                        dim_z,
                        data: match t {
                            0 => AstcencImageData::U8(vec![vec![0; n]; planes]),
                            1 => AstcencImageData::F16(vec![vec![0; n]; planes]),
                            _ => AstcencImageData::F32(vec![vec![0.0; n]; planes]),
                        },
                    };
                    let ec = match astcenc_decompress_image(&mut ctx, &data, &mut image, swizzle, 0)
                    {
                        Ok(()) => 0,
                        Err(e) => e as i32,
                    };
                    let mut h = 0xcbf29ce484222325u64;
                    match &image.data {
                        AstcencImageData::U8(p) => p.iter().for_each(|pl| h = fnv1a(h, pl)),
                        AstcencImageData::F16(p) => p.iter().for_each(|pl| {
                            let b: Vec<u8> = pl.iter().flat_map(|v| v.to_le_bytes()).collect();
                            h = fnv1a(h, &b);
                        }),
                        AstcencImageData::F32(p) => p.iter().for_each(|pl| {
                            let b: Vec<u8> = pl.iter().flat_map(|v| v.to_le_bytes()).collect();
                            h = fnv1a(h, &b);
                        }),
                    }
                    let key = format!("{bx}x{by}x{bz}_p{p}_t{t}_s{sw}");
                    let Some((gec, ghash)) = golden.get(key.as_str()) else {
                        panic!("no golden for {key}")
                    };
                    assert_eq!(ec, *gec, "{key}");
                    assert_eq!(format!("{h:016x}"), *ghash, "{key}");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, golden.len());
}

#[test]
fn config_errors() {
    assert_eq!(
        astcenc_config_init(AstcencProfile::Ldr, 4, 5, 1, 60.0, 0),
        Err(AstcencError::BadBlockSize)
    );
    assert_eq!(
        astcenc_config_init(
            AstcencProfile::Hdr,
            4,
            4,
            1,
            60.0,
            ASTCENC_FLG_USE_DECODE_UNORM8
        ),
        Err(AstcencError::BadDecodeMode)
    );
    assert_eq!(
        astcenc_get_error_string(AstcencError::BadBlockSize),
        "ASTCENC_ERR_BAD_BLOCK_SIZE"
    );
}
