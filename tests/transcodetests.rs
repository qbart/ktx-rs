// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of tests/transcodetests/transcodetests.cc.
//!
//! The C test transcodes the reference `.basis` files with the original Basis
//! Universal transcoder and compares byte-for-byte with libktx's transcode of the
//! matching `.ktx2` file. Here the reference outputs are recorded once by
//! `scripts/golden/gen_transcode_golden.sh` (C++ basisu) as SHA-256 + size in
//! `tests/resources/golden/transcode_basis.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, TranscodeFmt, ktx_error_string};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

struct TextureSet {
    ktx_file: &'static str,
    basisu_file: &'static str,
    is_po2: bool,
    has_alpha: bool,
}

struct FormatFeature {
    format: TranscodeFmt,
    supports_non_po2: bool,
    supports_non_alpha: bool,
}

const ALL_TEXTURE_SETS: [TextureSet; 3] = [
    TextureSet {
        ktx_file: "color_grid_blze.ktx2",
        basisu_file: "color_grid.basis",
        is_po2: true,
        has_alpha: false,
    },
    TextureSet {
        ktx_file: "kodim17_blze.ktx2",
        basisu_file: "kodim17.basis",
        is_po2: false,
        has_alpha: false,
    },
    TextureSet {
        ktx_file: "alpha_simple_blze.ktx2",
        basisu_file: "alpha_simple.basis",
        is_po2: true,
        has_alpha: true,
    },
];

fn ff(format: TranscodeFmt, supports_non_po2: bool, supports_non_alpha: bool) -> FormatFeature {
    FormatFeature {
        format,
        supports_non_po2,
        supports_non_alpha,
    }
}

fn all_formats() -> Vec<FormatFeature> {
    vec![
        ff(TranscodeFmt::ETC1_RGB, true, true),
        ff(TranscodeFmt::ETC2_RGBA, true, true),
        ff(TranscodeFmt::BC1_RGB, true, true),
        ff(TranscodeFmt::BC3_RGBA, true, true),
        ff(TranscodeFmt::BC4_R, true, true),
        ff(TranscodeFmt::BC5_RG, true, true),
        ff(TranscodeFmt::BC7_RGBA, true, true),
        ff(TranscodeFmt::PVRTC1_4_RGB, false, true),
        ff(TranscodeFmt::PVRTC1_4_RGBA, false, false),
        ff(TranscodeFmt::ASTC_4x4_RGBA, true, true),
        ff(TranscodeFmt::PVRTC2_4_RGB, true, true),
        ff(TranscodeFmt::PVRTC2_4_RGBA, true, true),
        ff(TranscodeFmt::ETC2_EAC_RG11, true, true),
        ff(TranscodeFmt::RGBA32, true, true),
        ff(TranscodeFmt::RGB565, true, true),
        ff(TranscodeFmt::BGR565, true, true),
        ff(TranscodeFmt::RGBA4444, true, true),
    ]
}

fn golden() -> HashMap<String, (usize, String)> {
    let path = common::resources_path()
        .join("golden")
        .join("transcode_basis.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?.to_string();
            let size = it.next()?.parse().ok()?;
            let hash = it.next()?.to_string();
            Some((name, (size, hash)))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn test_texture_set(
    golden: &HashMap<String, (usize, String)>,
    ts: &TextureSet,
    format: &FormatFeature,
) -> bool {
    if !ts.has_alpha && !format.supports_non_alpha {
        return false;
    }
    if !ts.is_po2 && !format.supports_non_po2 {
        return false;
    }
    let key = format!("{}_{}.bin", ts.basisu_file, format.format.0);
    let Some((final_size, hash)) = golden.get(&key) else {
        panic!("no golden for {key}")
    };

    let path = common::ktx2_path().join(ts.ktx_file);
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => panic!("Could not open texture file {}: {e}", path.display()),
    };
    let mut new_tex =
        match Texture2::create_from_memory(&data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => t,
            Err(e) => panic!("{}: {}", ts.ktx_file, ktx_error_string(e)),
        };
    if let Err(e) = new_tex.transcode_basis(format.format, 0) {
        panic!(
            "{} format {}: {}",
            ts.ktx_file,
            format.format.0,
            ktx_error_string(e)
        );
    }
    assert_eq!(*final_size, new_tex.data_size, "{key}");
    let out = new_tex.get_data().unwrap_or_default();
    assert_eq!(
        &hex(&Sha256::digest(out)),
        hash,
        "{key}: transcoded data differs from basisu"
    );
    true
}

#[test]
fn texture_combinations_test_basic() {
    let golden = golden();
    let mut checked = 0;
    for ts in &ALL_TEXTURE_SETS {
        for format in all_formats().iter() {
            if test_texture_set(&golden, ts, format) {
                checked += 1;
            }
        }
    }
    // Guard against silently skipping comparisons.
    let expected = ALL_TEXTURE_SETS
        .iter()
        .map(|ts| {
            all_formats()
                .iter()
                .filter(|f| {
                    (ts.has_alpha || f.supports_non_alpha) && (ts.is_po2 || f.supports_non_po2)
                })
                .count()
        })
        .sum::<usize>();
    assert_eq!(checked, expected);
}

/// Not in the C suite: proves the ETC1S->BC7 chroma filter is exercised by the
/// golden comparison (disabling it must change the output of at least one set).
#[test]
fn bc7_chroma_filter_is_exercised() {
    let golden = golden();
    let mut differs = 0;
    for ts in &ALL_TEXTURE_SETS {
        let Some((_, hash)) = golden.get(&format!("{}_6.bin", ts.basisu_file)) else {
            panic!("no golden")
        };
        let data = std::fs::read(common::ktx2_path().join(ts.ktx_file)).unwrap_or_default();
        let mut tex =
            match Texture2::create_from_memory(&data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
                Ok(t) => t,
                Err(e) => panic!("{}", ktx_error_string(e)),
            };
        const NO_ETC1S_CHROMA_FILTERING: u32 = 64;
        if let Err(e) = tex.transcode_basis(TranscodeFmt::BC7_RGBA, NO_ETC1S_CHROMA_FILTERING) {
            panic!("{}: {}", ts.ktx_file, ktx_error_string(e));
        }
        if &hex(&Sha256::digest(tex.get_data().unwrap_or_default())) != hash {
            differs += 1;
        }
    }
    assert!(differs > 0, "chroma filter never changed any output");
}

//////////////////////////////
// UASTC HDR 6x6 intermediate SGD image description indexing
//////////////////////////////

// The image description table in the supercompression global data is written
// in level order, level 0 first, each level contributing
// numLayers * numFaces * depth(level) descriptions. For 3D textures depth
// halves with each level, so indexing the table with
// level * levelImageCount selected descriptions belonging to other levels
// and transcoding failed. The 2D case covers the constant-image-count path.

/// Port of `floatToHalf` (truncating, no denormals).
fn float_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    let mantissa = (bits >> 13) & 0x3FF;
    if exponent <= 0 {
        return sign as u16;
    }
    if exponent >= 31 {
        return (sign | 0x7C00) as u16;
    }
    (sign | ((exponent as u32) << 10) | mantissa) as u16
}

/// Port of `roundTripUastcHdr6x6i`.
fn round_trip_uastc_hdr_6x6i(num_dimensions: u32) {
    use ktx::encode_params::{
        BasisParams, KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE, KTX_BASIS_PARAMS_SIZE,
    };
    use ktx::{SupercmpScheme, TextureCreateInfo, TextureCreateStorage, VkFormat};

    let create_info = TextureCreateInfo {
        vk_format: VkFormat::R16G16B16A16_SFLOAT,
        base_width: 24,
        base_height: 24,
        base_depth: if num_dimensions == 3 { 8 } else { 1 },
        num_dimensions,
        num_levels: 4,
        num_layers: 1,
        num_faces: 1,
        is_array: false,
        generate_mipmaps: false,
        ..Default::default()
    };
    let mut texture = match Texture2::create(&create_info, TextureCreateStorage::AllocStorage) {
        Ok(t) => t,
        Err(e) => panic!("{}", ktx_error_string(e)),
    };

    for level in 0..create_info.num_levels {
        let width = (create_info.base_width >> level).max(1);
        let height = (create_info.base_height >> level).max(1);
        let depth = (create_info.base_depth >> level).max(1);
        for slice in 0..depth {
            let mut pixels = vec![0u16; (width * height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let i = ((y * width + x) * 4) as usize;
                    pixels[i] = float_to_half(0.1 + 2.0 * x as f32 / width as f32 + level as f32);
                    pixels[i + 1] =
                        float_to_half(0.2 + 1.5 * y as f32 / height as f32 + slice as f32);
                    pixels[i + 2] = float_to_half(0.4 + 0.5 * level as f32);
                    pixels[i + 3] = float_to_half(1.0);
                }
            }
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            if let Err(e) = texture.set_image_from_memory(level, 0, slice, &bytes) {
                panic!("{}", ktx_error_string(e));
            }
        }
    }

    let cparams = BasisParams {
        struct_size: KTX_BASIS_PARAMS_SIZE,
        thread_count: 1,
        codec: KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE,
        ..Default::default()
    };
    if let Err(e) = texture.compress_basis_ex(&cparams) {
        panic!("{}", ktx_error_string(e));
    }
    assert_eq!(
        texture.supercompression_scheme,
        SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
    );

    if let Err(e) = texture.transcode_basis(TranscodeFmt::ASTC_HDR_6x6_RGBA, 0) {
        panic!("{}", ktx_error_string(e));
    }
    assert_eq!(texture.vk_format, VkFormat::ASTC_6x6_SFLOAT_BLOCK);
    assert_eq!(texture.supercompression_scheme, SupercmpScheme::NONE);
    assert!(texture.get_data().is_some());
}

#[test]
fn transcode_uastc_hdr_6x6i_round_trip_2d() {
    round_trip_uastc_hdr_6x6i(2);
}

#[test]
fn transcode_uastc_hdr_6x6i_round_trip_3d_mip_levels() {
    round_trip_uastc_hdr_6x6i(3);
}
