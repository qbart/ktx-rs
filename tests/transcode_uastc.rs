//! UASTC LDR 4x4 transcoding vs the original Basis Universal transcoder.
//!
//! Not a C test port (libktx has no UASTC golden test): reference outputs come
//! from `scripts/golden/gen_transcode_golden.sh` (libktx's transcodeUastcLDR4x4
//! call path on the C++ basisu transcoder), recorded per level as SHA-256 + size
//! in `tests/resources/golden/transcode_uastc.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::{
    ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KtxTexture, Texture2, TranscodeFmt,
    ktx_error_string, transcode_flags,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const FILES: [&str; 4] = [
    "color_grid_uastc_zstd_5.ktx2",
    "ktx_document_uastc_rdo_4_zstd_5.ktx2",
    "Iron_Bars_001_normal_uastc_zstd_10.ktx2",
    "cubemap_goldengate_uastc_rdo_4_zstd_5.ktx2",
];

const FORMATS: [TranscodeFmt; 18] = [
    TranscodeFmt::ETC1_RGB,
    TranscodeFmt::ETC2_RGBA,
    TranscodeFmt::BC1_RGB,
    TranscodeFmt::BC3_RGBA,
    TranscodeFmt::BC4_R,
    TranscodeFmt::BC5_RG,
    TranscodeFmt::BC7_RGBA,
    TranscodeFmt::PVRTC1_4_RGB,
    TranscodeFmt::PVRTC1_4_RGBA,
    TranscodeFmt::ASTC_4x4_RGBA,
    TranscodeFmt::PVRTC2_4_RGB,
    TranscodeFmt::PVRTC2_4_RGBA,
    TranscodeFmt::ETC2_EAC_R11,
    TranscodeFmt::ETC2_EAC_RG11,
    TranscodeFmt::RGBA32,
    TranscodeFmt::RGB565,
    TranscodeFmt::BGR565,
    TranscodeFmt::RGBA4444,
];

fn golden() -> HashMap<String, (usize, String)> {
    let path = common::resources_path()
        .join("golden")
        .join("transcode_uastc.txt");
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

#[test]
fn uastc_ldr_transcode_matches_basisu() {
    let golden = golden();
    let mut checked = 0;
    let mut expected = 0;
    for file in FILES {
        let data = match std::fs::read(common::ktx2_path().join(file)) {
            Ok(d) => d,
            Err(e) => panic!("{file}: {e}"),
        };
        for fmt in FORMATS {
            for flags in [
                0,
                transcode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS,
                transcode_flags::HIGH_QUALITY,
            ] {
                let mut tex = match Texture2::create_from_memory(
                    &data,
                    KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
                ) {
                    Ok(t) => t,
                    Err(e) => panic!("{file}: {}", ktx_error_string(e)),
                };
                let num_levels = tex.num_levels;
                let key = |level: u32| format!("{file}_{}_{flags}_L{level}.bin", fmt.0);
                let has_golden = golden.contains_key(&key(0));
                let res = tex.transcode_basis(fmt, flags);
                if !has_golden {
                    // basisu has no UASTC -> PVRTC2 path; libktx rejects it.
                    assert!(
                        matches!(
                            res,
                            Err(ErrorCode::UnsupportedFeature | ErrorCode::TranscodeFailed)
                        ),
                        "{file} fmt {} should fail",
                        fmt.0
                    );
                    continue;
                }
                if let Err(e) = res {
                    panic!(
                        "{file} fmt {} flags {flags}: {}",
                        fmt.0,
                        ktx_error_string(e)
                    );
                }
                let all = tex.get_data().unwrap_or_default();
                for level in 0..num_levels {
                    expected += 1;
                    let Some((size, hash)) = golden.get(&key(level)) else {
                        panic!("no golden for {}", key(level))
                    };
                    let ofs = match tex.get_image_offset(level, 0, 0) {
                        Ok(o) => o,
                        Err(e) => panic!("{}", ktx_error_string(e)),
                    };
                    let len = tex.get_level_size(level);
                    assert_eq!(len, *size, "{}", key(level));
                    let bytes = all.get(ofs..ofs + len).unwrap_or_default();
                    assert_eq!(
                        &hex(&Sha256::digest(bytes)),
                        hash,
                        "{}: differs from basisu",
                        key(level)
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(expected > 0);
    assert_eq!(checked, expected);
}
