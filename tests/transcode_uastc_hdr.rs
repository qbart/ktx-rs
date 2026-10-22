//! UASTC HDR transcoding vs the original Basis Universal transcoder.
//!
//! Not a C test port (libktx has no UASTC HDR golden test): reference outputs
//! come from `scripts/golden/gen_transcode_golden.sh` (libktx's
//! transcodeUastcHDR4x4 and transcodeUastcHDR6x6_intermediate call paths on the
//! C++ basisu transcoder), recorded per level as SHA-256 + size in
//! `tests/resources/golden/transcode_uastc_hdr.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::{
    ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KtxTexture, Texture2, TranscodeFmt,
    VkFormat, ktx_error_string, transcode_flags,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const HDR4X4: &str = "Desk_uastc_hdr4x4_zstd_15.ktx2";
const HDR6X6I: &str = "Desk_uastc_hdr6x6i.ktx2";
const FILES: [&str; 1] = [HDR4X4];

/// KTX transcode target and the matching basisu `transcoder_texture_format`
/// used in the golden file names.
const FORMATS: [(TranscodeFmt, u32); 5] = [
    (TranscodeFmt::BC6HU_RGB, 22),
    (TranscodeFmt::ASTC_HDR_4x4_RGBA, 23),
    (TranscodeFmt::RGB_HALF, 24),
    (TranscodeFmt::RGBA_HALF, 25),
    (TranscodeFmt::RGB_9E5, 26),
];

const FORMATS_6X6I: [(TranscodeFmt, u32); 5] = [
    (TranscodeFmt::BC6HU_RGB, 22),
    (TranscodeFmt::ASTC_HDR_6x6_RGBA, 27),
    (TranscodeFmt::RGB_HALF, 24),
    (TranscodeFmt::RGBA_HALF, 25),
    (TranscodeFmt::RGB_9E5, 26),
];

/// (file, targets, transcode flags) covered by the goldens.
type Case = (&'static str, &'static [(TranscodeFmt, u32)], &'static [u32]);
const CASES: [Case; 2] = [
    (HDR4X4, &FORMATS, &[0]),
    (HDR6X6I, &FORMATS_6X6I, &[0, transcode_flags::HIGH_QUALITY]),
];

fn golden() -> HashMap<String, (usize, String)> {
    let path = common::resources_path()
        .join("golden")
        .join("transcode_uastc_hdr.txt");
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

fn load(file: &str) -> Texture2 {
    let data = match std::fs::read(common::ktx2_path().join(file)) {
        Ok(d) => d,
        Err(e) => panic!("{file}: {e}"),
    };
    match Texture2::create_from_memory(&data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
        Ok(t) => t,
        Err(e) => panic!("{file}: {}", ktx_error_string(e)),
    }
}

#[test]
fn uastc_hdr_transcode_matches_basisu() {
    let golden = golden();
    let mut checked = 0;
    let mut expected = 0;
    for (file, formats, flagsets) in CASES {
        for &(fmt, basis_fmt) in formats {
            for &flags in flagsets {
                let mut tex = load(file);
                if let Err(e) = tex.transcode_basis(fmt, flags) {
                    panic!(
                        "{file} fmt {} flags {flags}: {}",
                        fmt.0,
                        ktx_error_string(e)
                    );
                }
                let all = tex.get_data().unwrap_or_default();
                for level in 0..tex.num_levels {
                    expected += 1;
                    let key = format!("{file}_{basis_fmt}_{flags}_L{level}.bin");
                    let Some((size, hash)) = golden.get(&key) else {
                        panic!("no golden for {key}")
                    };
                    let ofs = match tex.get_image_offset(level, 0, 0) {
                        Ok(o) => o,
                        Err(e) => panic!("{}", ktx_error_string(e)),
                    };
                    let len = tex.get_level_size(level);
                    assert_eq!(len, *size, "{key}");
                    let bytes = all.get(ofs..ofs + len).unwrap_or_default();
                    assert_eq!(
                        &hex(&Sha256::digest(bytes)),
                        hash,
                        "{key}: differs from basisu"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, expected);
    assert_eq!(checked, golden.len());
}

#[test]
fn uastc_hdr_transcode_output_formats() {
    for (fmt, vk) in [
        (TranscodeFmt::BC6HU_RGB, VkFormat::BC6H_UFLOAT_BLOCK),
        (
            TranscodeFmt::ASTC_HDR_4x4_RGBA,
            VkFormat::ASTC_4x4_SFLOAT_BLOCK,
        ),
        (TranscodeFmt::RGB_HALF, VkFormat::R16G16B16_SFLOAT),
        (TranscodeFmt::RGBA_HALF, VkFormat::R16G16B16A16_SFLOAT),
        (TranscodeFmt::RGB_9E5, VkFormat::E5B9G9R9_UFLOAT_PACK32),
    ] {
        let mut tex = load(FILES[0]);
        if let Err(e) = tex.transcode_basis(fmt, 0) {
            panic!("fmt {}: {}", fmt.0, ktx_error_string(e));
        }
        assert_eq!(tex.vk_format, vk, "fmt {}", fmt.0);
    }
}

#[test]
fn uastc_hdr_rejects_ldr_targets() {
    for fmt in [
        TranscodeFmt::BC7_RGBA,
        TranscodeFmt::ETC1_RGB,
        TranscodeFmt::RGBA32,
        TranscodeFmt::ASTC_4x4_RGBA,
        TranscodeFmt::ASTC_HDR_6x6_RGBA,
    ] {
        let mut tex = load(FILES[0]);
        assert_eq!(
            tex.transcode_basis(fmt, 0),
            Err(ErrorCode::UnsupportedFeature),
            "fmt {}",
            fmt.0
        );
    }
}

/// `astc_helpers::pack_astc_block` inverts `unpack_block` on every block
/// basisu's encoder produced (the UASTC HDR 4x4 file holds plain ASTC HDR).
#[test]
fn astc_pack_round_trips_uastc_hdr_blocks() {
    use ktx::basisu::astc_helpers::{LogAstcBlock, pack_astc_block, unpack_block};
    let tex = load(FILES[0]);
    let data = tex.get_data().unwrap_or_default();
    let (blocks, rest) = data.as_chunks::<16>();
    assert!(rest.is_empty() && !blocks.is_empty());
    let mut solid = 0;
    for (i, blk) in blocks.iter().enumerate() {
        let mut log_blk = LogAstcBlock::default();
        assert!(unpack_block(blk, &mut log_blk, 4, 4), "block {i}");
        solid += usize::from(log_blk.solid_color_flag_hdr);
        let mut packed = [0u8; 16];
        assert!(pack_astc_block(&mut packed, &log_blk), "block {i}");
        assert_eq!(&packed, blk, "block {i}");
    }
    assert!(solid < blocks.len());
}

#[test]
fn uastc_hdr_6x6i_output_formats() {
    for (fmt, vk) in [
        (TranscodeFmt::BC6HU_RGB, VkFormat::BC6H_UFLOAT_BLOCK),
        (
            TranscodeFmt::ASTC_HDR_6x6_RGBA,
            VkFormat::ASTC_6x6_SFLOAT_BLOCK,
        ),
        (TranscodeFmt::RGB_9E5, VkFormat::E5B9G9R9_UFLOAT_PACK32),
    ] {
        let mut tex = load(HDR6X6I);
        if let Err(e) = tex.transcode_basis(fmt, 0) {
            panic!("fmt {}: {}", fmt.0, ktx_error_string(e));
        }
        assert_eq!(tex.vk_format, vk, "fmt {}", fmt.0);
    }
    for fmt in [
        TranscodeFmt::ASTC_HDR_4x4_RGBA,
        TranscodeFmt::BC7_RGBA,
        TranscodeFmt::RGBA32,
    ] {
        let mut tex = load(HDR6X6I);
        assert_eq!(
            tex.transcode_basis(fmt, 0),
            Err(ErrorCode::UnsupportedFeature),
            "fmt {}",
            fmt.0
        );
    }
}

/// A corrupt 6x6i stream fails cleanly (no panic) as `TranscodeFailed`.
#[test]
fn uastc_hdr_6x6i_corrupt_data_fails() {
    let mut tex = load(HDR6X6I);
    let ofs = tex.level_data_offset(0) as usize;
    if let Some(data) = tex.p_data.as_mut() {
        for b in data.iter_mut().skip(ofs + 12).step_by(7) {
            *b ^= 0x5A;
        }
    }
    assert_eq!(
        tex.transcode_basis(TranscodeFmt::RGBA_HALF, 0),
        Err(ErrorCode::TranscodeFailed)
    );
}
