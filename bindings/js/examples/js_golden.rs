// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Writes the goldens of the JS binding tests with the native library (whose paths the ported
//! tests check against C++):
//! - `transcode.tsv`: `ktxTexture2_TranscodeBasis` per target, one FNV-1a 64 hash per image
//!   (level, layer, face);
//! - `write.tsv`: the `libktx.js` write scenarios of `tests/libktx.test.js`, done the way
//!   `ktx_wrapper.cpp` calls libktx (zeroed params, one thread), hash of `writeToMemory`.
//!
//! cargo run -p ktx-rs-wasm --example js_golden > bindings/js/tests/golden/transcode.tsv
//! cargo run -p ktx-rs-wasm --example js_golden write > bindings/js/tests/golden/write.tsv

use ktx::encode_params::{AstcParams, BasisParams, KTX_ASTC_PARAMS_SIZE, KTX_BASIS_PARAMS_SIZE};
use ktx::texture::KtxTexture;
use ktx::vkformat::VkFormat;
use ktx::{
    KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KtxResult, Texture2, TextureCreateInfo,
    TextureCreateStorage, TranscodeFmt,
};

/// Paths relative to the repository root.
const FILES: [&str; 4] = [
    "tests/resources/ktx2/alpha_simple_blze.ktx2",
    "tests/resources/ktx2/kodim17_blze.ktx2",
    "tests/resources/ktx2/r8g8b8a8_srgb_mip_blze.ktx2",
    "bindings/js/tests/resources/r8g8b8a8_srgb_mip_uastc.ktx2",
];

/// `TranscodeTarget` names (msc_basis_transcoder.js) and values.
const TARGETS: [(&str, u32); 18] = [
    ("ETC1_RGB", 0),
    ("ETC2_RGBA", 1),
    ("BC1_RGB", 2),
    ("BC3_RGBA", 3),
    ("BC4_R", 4),
    ("BC5_RG", 5),
    ("BC7_RGBA", 6),
    ("PVRTC1_4_RGB", 8),
    ("PVRTC1_4_RGBA", 9),
    ("ASTC_4x4_RGBA", 10),
    ("RGBA32", 13),
    ("RGB565", 14),
    ("BGR565", 15),
    ("RGBA4444", 16),
    ("PVRTC2_4_RGB", 18),
    ("PVRTC2_4_RGBA", 19),
    ("EAC_R11", 20),
    ("EAC_RG11", 21),
];

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The 32x32 R8G8B8A8_SRGB source image of the write scenarios.
fn source_image() -> Vec<u8> {
    (0..32 * 32 * 4u32)
        .map(|i| ((i * 7 + (i >> 7)) & 0xff) as u8)
        .collect()
}

/// `new texture(createInfo, ALLOC_STORAGE)`, `setImageFromMemory`,
/// `addKVPairString(ORIENTATION_KEY, "rd")`.
fn raw_texture() -> KtxResult<Texture2> {
    let info = TextureCreateInfo {
        vk_format: VkFormat::R8G8B8A8_SRGB,
        base_width: 32,
        base_height: 32,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    let mut t = Texture2::create(&info, TextureCreateStorage::AllocStorage)?;
    t.set_image_from_memory(0, 0, 0, &source_image())?;
    t.kv_data_head.add_kv_pair("KTXorientation", b"rd\0")?;
    Ok(t)
}

/// `compressBasis` as `ktx_wrapper.cpp` calls it.
fn compress_basis(t: &mut Texture2, mut p: BasisParams) -> KtxResult<()> {
    p.struct_size = KTX_BASIS_PARAMS_SIZE;
    p.thread_count = 1;
    p.verbose = false;
    p.no_sse = true;
    t.compress_basis_ex(&p)
}

fn write_scenarios() -> KtxResult<()> {
    let out = |name: &str, t: &mut Texture2| -> KtxResult<()> {
        let data = t.write_to_memory()?;
        println!("{name}\t{}\t{:016x}", data.len(), fnv1a64(&data));
        Ok(())
    };
    out("raw", &mut raw_texture()?)?;

    let mut t = raw_texture()?;
    let p = BasisParams {
        codec: 1,
        quality_level: 128,
        etc1s_compression_level: 2,
        ..Default::default()
    };
    compress_basis(&mut t, p)?;
    out("etc1s", &mut t)?;

    let mut t = raw_texture()?;
    let p = BasisParams {
        codec: 2,
        uastc_flags: 0,
        ..Default::default()
    };
    compress_basis(&mut t, p)?;
    t.deflate_zstd(5)?;
    out("uastc_zstd", &mut t)?;

    let mut t = raw_texture()?;
    let p = AstcParams {
        struct_size: KTX_ASTC_PARAMS_SIZE,
        thread_count: 1,
        block_dimension: 4,
        mode: 1,
        quality_level: 10,
        ..Default::default()
    };
    t.compress_astc_ex(&p)?;
    t.deflate_zlib(3)?;
    out("astc_zlib", &mut t)?;

    let mut t = raw_texture()?;
    t.kv_data_head.add_kv_pair("mykey", &[1, 2, 3])?;
    t.kv_data_head.delete_kv_pair("KTXorientation")?;
    // The JS property setter ignores the result (LINEAR is refused for an sRGB format).
    let _ = t.set_oetf(1);
    t.set_primaries(10)?;
    out("kv_colorspace", &mut t)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("write") {
        return match write_scenarios() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e:?}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for file in FILES {
        for (name, value) in TARGETS {
            let path = format!("{root}/{file}");
            let mut texture = match Texture2::create_from_named_file(
                &path,
                KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
            ) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{file}: {e:?}");
                    return std::process::ExitCode::FAILURE;
                }
            };
            if texture.transcode_basis(TranscodeFmt(value), 0).is_err() {
                println!("{file}\t{name}\tfail");
                continue;
            }
            let base = texture.base();
            let data = base.p_data.as_deref().unwrap_or_default();
            for level in 0..base.num_levels {
                for layer in 0..base.num_layers {
                    for face in 0..base.num_faces {
                        let size = texture.get_image_size(level);
                        let image = texture
                            .get_image_offset(level, layer, face)
                            .ok()
                            .and_then(|o| data.get(o..o + size))
                            .unwrap_or_default();
                        println!(
                            "{file}\t{name}\t{level}.{layer}.{face}\t{}\t{:016x}",
                            image.len(),
                            fnv1a64(image)
                        );
                    }
                }
            }
        }
    }
    std::process::ExitCode::SUCCESS
}
