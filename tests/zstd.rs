// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! zstd deflate vs libzstd 1.5.7 (the copy bundled with basisu that libktx
//! links).
//!
//! Not a C test port: reference sizes and FNV-1a hashes come from
//! `scripts/golden/gen_zstd_golden.sh`, recorded in
//! `tests/resources/golden/zstd.txt`: `ZSTD_compressCCtx` frames of synthetic
//! inputs and test files at levels -5..22, and `ktxTexture2_DeflateZstd`
//! level data of uncompressed KTX2 files at levels 1..22.
#![allow(clippy::panic)]

mod common;

use common::synthetic::{fnv1a, synthetic_inputs};
use ktx::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, SupercmpScheme, Texture2};
use std::collections::HashMap;
use zstd_pure_rs::prelude as zstd;

const LEVELS: [i32; 25] = [
    -5, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
];

const FILES: [&str; 5] = [
    "ktx_app_astc_8x8.ktx2",
    "bc3_unorm_array_7.ktx2",
    "r8g8b8_srgb_mip.ktx2",
    "r8g8b8a8_srgb_array_7_mip.ktx2",
    "r8g8b8a8_srgb_3d_7.ktx2",
];

fn golden() -> HashMap<String, (usize, String)> {
    let path = common::resources_path().join("golden").join("zstd.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f.first()?.to_string(),
                (f.get(1)?.parse().ok()?, f.get(2)?.to_string()),
            ))
        })
        .collect()
}

fn check(golden: &HashMap<String, (usize, String)>, key: &str, out: &[u8]) {
    let Some((size, hash)) = golden.get(key) else {
        panic!("no golden for {key}")
    };
    assert_eq!(out.len(), *size, "{key}");
    assert_eq!(&format!("{:016x}", fnv1a(out)), hash, "{key}");
}

#[test]
fn compress_matches_libzstd() {
    let golden = golden();
    let mut inputs = synthetic_inputs();
    for f in FILES {
        match std::fs::read(common::ktx2_path().join(f)) {
            Ok(d) => inputs.push((f.to_string(), d)),
            Err(e) => panic!("{f}: {e}"),
        }
    }
    let Some(mut cctx) = zstd::ZSTD_createCCtx() else {
        panic!("ZSTD_createCCtx failed")
    };
    let mut checked = 0;
    for (name, data) in &inputs {
        let mut dst = vec![0u8; zstd::ZSTD_compressBound(data.len())];
        for level in LEVELS {
            let n = zstd::ZSTD_compressCCtx(&mut cctx, &mut dst, data, level);
            assert!(!zstd::ZSTD_isError(n), "{name} L{level}");
            let out = &dst[..n];
            check(&golden, &format!("{name}_L{level}"), out);
            let mut back = vec![0u8; data.len()];
            assert_eq!(
                ktx::zlib::ktx_zstd_decompress(&mut back, out),
                Ok(data.len())
            );
            assert!(&back == data, "{name} L{level} round trip");
            checked += 1;
        }
    }
    let ktx2 = golden.keys().filter(|k| k.contains("_deflate_")).count();
    assert_eq!(checked + ktx2, golden.len());
}

#[test]
fn deflate_zstd_matches_libktx() {
    let golden = golden();
    for f in FILES {
        for level in 1..=22u32 {
            let mut t = match Texture2::create_from_named_file(
                common::ktx2_path().join(f),
                KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
            ) {
                Ok(t) => t,
                Err(e) => panic!("{f}: {e:?}"),
            };
            assert_eq!(t.deflate_zstd(level), Ok(()), "{f} L{level}");
            assert_eq!(t.supercompression_scheme, SupercmpScheme::ZSTD);
            check(
                &golden,
                &format!("{f}_deflate_L{level}"),
                t.get_data().unwrap_or_default(),
            );
        }
    }
}

/// libzstd rejects nothing in 1..=22; an already supercompressed texture is
/// an invalid operation like in writer2.c.
#[test]
fn deflate_zstd_twice() {
    let mut t = match Texture2::create_from_named_file(
        common::ktx2_path().join("r8g8b8a8_srgb_3d_7.ktx2"),
        KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
    ) {
        Ok(t) => t,
        Err(e) => panic!("{e:?}"),
    };
    assert_eq!(t.deflate_zstd(3), Ok(()));
    assert_eq!(t.deflate_zstd(3), Err(ktx::ErrorCode::InvalidOperation));
}

/// Corrupted zstd level data fails with an error code, never a panic.
#[test]
fn inflate_corrupted_never_panics() {
    let mut state = 12345u32;
    let mut next = move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 8) as usize
    };
    let mut failed = 0;
    for f in [
        "color_grid_zstd_5.ktx2",
        "Desk_small_zstd_15.ktx2",
        "color_grid_uastc_zstd_5.ktx2",
    ] {
        let data = match std::fs::read(common::ktx2_path().join(f)) {
            Ok(d) => d,
            Err(e) => panic!("{f}: {e}"),
        };
        let Ok(t) = Texture2::create_from_memory(&data, 0) else {
            panic!("{f}: load failed")
        };
        let (start, end) = t.level_index().iter().fold((usize::MAX, 0), |(s, e), l| {
            let off = l.byte_offset as usize;
            (s.min(off), e.max(off + l.byte_length as usize))
        });
        // The level data is at the end of the file.
        let base = data.len() - (end - start);
        for i in 0..300 {
            let mut d = data.clone();
            for _ in 0..1 + i % 4 {
                let pos = base + next() % (data.len() - base);
                d[pos] ^= 1 + (next() % 255) as u8;
            }
            if i % 10 == 0 {
                d.truncate(base + next() % (data.len() - base));
            }
            if Texture2::create_from_memory(&d, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT).is_err() {
                failed += 1;
            }
        }
    }
    assert!(failed > 0);
}
