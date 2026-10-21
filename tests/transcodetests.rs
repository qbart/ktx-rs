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

/// Target formats whose ETC1S transcoders are ported so far.
/// TODO(basis): grow this list until it covers `all_formats()`.
fn implemented(f: TranscodeFmt) -> bool {
    matches!(
        f,
        TranscodeFmt::ETC1_RGB
            | TranscodeFmt::ETC2_RGBA
            | TranscodeFmt::ETC2_EAC_RG11
            | TranscodeFmt::BC1_RGB
            | TranscodeFmt::BC3_RGBA
            | TranscodeFmt::BC4_R
            | TranscodeFmt::BC5_RG
            | TranscodeFmt::BC7_RGBA
            | TranscodeFmt::RGBA32
            | TranscodeFmt::RGB565
            | TranscodeFmt::BGR565
            | TranscodeFmt::RGBA4444
            | TranscodeFmt::ASTC_4x4_RGBA
    )
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
        for format in all_formats().iter().filter(|f| implemented(f.format)) {
            if test_texture_set(&golden, ts, format) {
                checked += 1;
            }
        }
    }
    // Guard against silently skipping comparisons.
    let expected = ALL_TEXTURE_SETS.len()
        * all_formats()
            .iter()
            .filter(|f| implemented(f.format))
            .count();
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
