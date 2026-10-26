//! ETC1S to ATC RGB / RGBA (`convert_etc1s_to_atc`) vs. the C++ basisu transcoder.
//!
//! KTX has no `ktx_transcode_fmt` for ATC (no VkFormat), so the original suite does not cover
//! it; libktx still compiles it into the transcoder. Level 0 of each BasisLZ texture of
//! `transcodetests` goes through `LowlevelEtc1sTranscoder::transcode_image` and is compared with
//! the C++ transcode of the matching `.basis` file (`tests/resources/golden/transcode_basis.txt`,
//! `scripts/golden/gen_transcode_golden.sh`). FXT1 is compiled out of libktx
//! (`BASISD_SUPPORT_FXT1=0`) and must be rejected.
#![allow(clippy::panic)]

mod common;

use ktx::basis_transcode::{
    BASIS_LZ_ETC1S_IMAGE_DESC_SIZE, BASIS_LZ_GLOBAL_HEADER_SIZE, BasisLzEtc1sImageDesc,
    BasisLzGlobalHeader,
};
use ktx::basisu::etc1s::{LowlevelEtc1sTranscoder, TranscodeStatus};
use ktx::basisu::{
    BasisTexFormat, TranscoderState, TranscoderTextureFormat, basis_get_bytes_per_block_or_pixel,
    basis_is_format_supported,
};
use ktx::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, ktx_error_string};
use sha2::{Digest, Sha256};

const SETS: [(&str, &str); 3] = [
    ("color_grid_blze.ktx2", "color_grid.basis"),
    ("kodim17_blze.ktx2", "kodim17.basis"),
    ("alpha_simple_blze.ktx2", "alpha_simple.basis"),
];

fn golden(key: &str) -> (usize, String) {
    let path = common::resources_path()
        .join("golden")
        .join("transcode_basis.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for l in text.lines() {
        let mut it = l.split_whitespace();
        if it.next() == Some(key) {
            let size = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            return (size, it.next().unwrap_or_default().to_string());
        }
    }
    panic!("no golden for {key}")
}

/// Transcodes image 0 of level 0 to `target`.
fn transcode_level0(file: &str, target: TranscoderTextureFormat) -> (TranscodeStatus, Vec<u8>) {
    let data = std::fs::read(common::ktx2_path().join(file)).unwrap_or_default();
    let tex = match Texture2::create_from_memory(&data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
        Ok(t) => t,
        Err(e) => panic!("{file}: {}", ktx_error_string(e)),
    };
    let bgd = tex.supercompression_global_data();
    let Some(h) = BasisLzGlobalHeader::from_bytes(bgd) else {
        panic!("{file}: no BasisLZ global header")
    };
    let image_count: usize = (0..tex.num_levels)
        .map(|l| (tex.num_layers * tex.num_faces * (tex.base_depth >> l).max(1)) as usize)
        .sum();
    let ep = BASIS_LZ_GLOBAL_HEADER_SIZE + BASIS_LZ_ETC1S_IMAGE_DESC_SIZE * image_count;
    let sp = ep + h.endpoints_byte_length as usize;
    let tp = sp + h.selectors_byte_length as usize;
    let te = tp + h.tables_byte_length as usize;
    let mut bit = LowlevelEtc1sTranscoder::new();
    assert!(bit.decode_palettes(
        h.endpoint_count as u32,
        &bgd[ep..sp],
        h.selector_count as u32,
        &bgd[sp..tp]
    ));
    assert!(bit.decode_tables(&bgd[tp..te]));
    let Some(desc) = BasisLzEtc1sImageDesc::from_bytes(&bgd[BASIS_LZ_GLOBAL_HEADER_SIZE..]) else {
        panic!("{file}: no image desc")
    };

    let (w, hgt) = (tex.base_width, tex.base_height);
    let (bx, by) = (w.div_ceil(4), hgt.div_ceil(4));
    let mut out = vec![0u8; (bx * by * basis_get_bytes_per_block_or_pixel(target)) as usize];
    let level_offset = tex.level_data_offset(0);
    let src = tex.p_data.as_deref().unwrap_or_default();
    let mut state = TranscoderState::new();
    let status = bit.transcode_image(
        target,
        &mut out,
        bx * by,
        src,
        bx,
        by,
        w,
        hgt,
        0,
        level_offset + desc.rgb_slice_byte_offset as u64,
        desc.rgb_slice_byte_length,
        level_offset + desc.alpha_slice_byte_offset as u64,
        desc.alpha_slice_byte_length,
        0,
        desc.alpha_slice_byte_length != 0,
        tex.is_video,
        0,
        &mut state,
        0,
    );
    (status, out)
}

#[test]
fn etc1s_to_atc_matches_basisu() {
    for (ktx_file, basis_file) in SETS {
        for target in [
            TranscoderTextureFormat::ATC_RGB,
            TranscoderTextureFormat::ATC_RGBA,
        ] {
            assert!(basis_is_format_supported(target, BasisTexFormat::Etc1s));
            let key = format!("{basis_file}_{}.bin", target.0);
            let (size, hash) = golden(&key);
            let (status, out) = transcode_level0(ktx_file, target);
            assert_eq!(status, TranscodeStatus::Ok, "{key}");
            assert_eq!(out.len(), size, "{key}");
            let got: String = Sha256::digest(&out)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(got, hash, "{key}: transcoded data differs from basisu");
        }
    }
}

#[test]
fn fxt1_is_compiled_out() {
    let fxt1 = TranscoderTextureFormat::FXT1_RGB;
    assert!(!basis_is_format_supported(fxt1, BasisTexFormat::Etc1s));
    assert!(!basis_is_format_supported(
        fxt1,
        BasisTexFormat::UastcLdr4x4
    ));
    let (status, _) = transcode_level0("color_grid_blze.ktx2", fxt1);
    assert_eq!(status, TranscodeStatus::Unsupported);
}
