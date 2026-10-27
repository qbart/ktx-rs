// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! tinyexr writer port vs the tinyexr bundled with basis_universal.
//!
//! Not a C test port: reference sizes and FNV-1a hashes come from
//! `scripts/golden/gen_tinyexr_golden.sh` (scanline images for every input /
//! requested pixel type pair, special float values, optional chromaticities),
//! recorded in `tests/resources/golden/tinyexr.txt`.
#![allow(clippy::panic)]

mod common;

use common::synthetic::fnv1a;
use ktx::tinyexr::{
    ExrAttribute, ExrChannel, TINYEXR_PIXELTYPE_FLOAT as F, TINYEXR_PIXELTYPE_HALF as H,
    TINYEXR_PIXELTYPE_UINT as U, save_exr_image_to_memory,
};
use std::collections::HashMap;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

const SPECIAL: [u32; 18] = [
    0x00000000, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00000001,
    0x387fe000, 0x33800000, 0x477fe000, 0x477ff000, 0x47800000, 0x38800000, 0x38000000, 0x3f801000,
    0x3f802000, 0xc7123456,
];

/// `value` in tinyexr_golden.cpp.
fn value(rng: &mut Lcg, pixel_type: i32, c: usize, i: usize) -> u32 {
    let r = rng.next();
    if pixel_type == U {
        return r.wrapping_mul(2654435761);
    }
    if pixel_type == H {
        return (r ^ c as u32) & 0xffff;
    }
    if i.is_multiple_of(3) {
        return SPECIAL[((r >> 4) as usize) % SPECIAL.len()];
    }
    let f = ((r % 200001) as i32 - 100000) as f32 / (1u32 << (r % 24)) as f32;
    f.to_bits()
}

fn run(w: i32, h: i32, pixel_types: &[i32], requested: &[i32], chroma: bool) -> Vec<u8> {
    let mut rng = Lcg((w * 131 + h * 7) as u32 + pixel_types.len() as u32);
    const NAMES: [&str; 4] = ["A", "B", "G", "R"];
    let n = pixel_types.len();
    let count = (w * h) as usize;
    let mut images = vec![vec![0u32; count]; n];
    for i in 0..count {
        for (c, image) in images.iter_mut().enumerate() {
            image[i] = value(&mut rng, pixel_types[c], c, i);
        }
    }
    let channels: Vec<ExrChannel> = images
        .into_iter()
        .enumerate()
        .map(|(c, data)| ExrChannel {
            name: NAMES[c % 4].to_string(),
            pixel_type: pixel_types[c],
            requested_pixel_type: requested[c],
            data,
        })
        .collect();
    let mut attributes = Vec::new();
    if chroma {
        let prim: [f32; 8] = [0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290];
        attributes.push(ExrAttribute {
            name: "chromaticities".into(),
            attr_type: "chromaticities".into(),
            value: prim.iter().flat_map(|v| v.to_le_bytes()).collect(),
        });
    }
    match save_exr_image_to_memory(w, h, &channels, &attributes) {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn save_matches_tinyexr() {
    let path = common::resources_path().join("golden").join("tinyexr.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let golden: HashMap<&str, (usize, &str)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((*f.first()?, (f.get(1)?.parse().ok()?, *f.get(2)?)))
        })
        .collect();
    let mut checked = 0;
    let mut check = |name: &str, out: Vec<u8>| {
        let Some((size, hash)) = golden.get(name) else {
            panic!("no golden for {name}")
        };
        assert_eq!(out.len(), *size, "{name}");
        assert_eq!(format!("{:016x}", fnv1a(&out)), *hash, "{name}");
        checked += 1;
    };
    for (w, h) in [(1, 1), (7, 3), (32, 17)] {
        for n in 1..=4usize {
            for (p, r) in [(F, H), (F, F), (U, U), (H, H), (H, F)] {
                for chroma in [false, true] {
                    let name =
                        format!("{w}x{h}_n{n}_{p}{r}{}", if chroma { "_chroma" } else { "" });
                    check(&name, run(w, h, &vec![p; n], &vec![r; n], chroma));
                }
            }
        }
    }
    check("mixed_ds", run(9, 5, &[F, F], &[F, H], false));
    check("mixed_uf", run(9, 5, &[U, F, F], &[U, F, H], true));
    assert_eq!(checked, golden.len());
}
