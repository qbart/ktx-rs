// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Writes the golden transcodes of the JS binding tests (bindings/js/tests/golden/transcode.tsv)
//! with the native library: `ktxTexture2_TranscodeBasis` per target, one FNV-1a 64 hash per
//! image (level, layer, face). The ported transcode tests check that path against C++.
//!
//! cargo run -p ktx-rs-wasm --example js_golden > bindings/js/tests/golden/transcode.tsv

use ktx::texture::KtxTexture;
use ktx::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, TranscodeFmt};

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

fn main() -> std::process::ExitCode {
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
