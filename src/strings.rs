// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of lib/src/strings.c (string helpers other than `ktxErrorString`).

use crate::defs::SupercmpScheme;

/// Port of `ktxSupercompressionSchemeString`.
pub fn ktx_supercompression_scheme_string(scheme: SupercmpScheme) -> &'static str {
    match scheme {
        SupercmpScheme::NONE => "KTX_SS_NONE",
        SupercmpScheme::BASIS_LZ => "KTX_SS_BASIS_LZ",
        SupercmpScheme::ZSTD => "KTX_SS_ZSTD",
        SupercmpScheme::ZLIB => "KTX_SS_ZLIB",
        SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE => "KTX_SS_UASTC_HDR_6x6_INTERMEDIATE",
        s => {
            if s.0 < SupercmpScheme::BEGIN_VENDOR_RANGE.0 || s.0 >= SupercmpScheme::BEGIN_RESERVED.0
            {
                "Invalid scheme value"
            } else {
                "Vendor or reserved scheme"
            }
        }
    }
}

/// `ETC1S_P_FRAME` bit of BasisLZ image flags (basis_sgd.h).
pub const ETC1S_P_FRAME: u32 = 0x02;

/// Port of `ktxBUImageFlagsBitString`.
pub fn ktx_bu_image_flags_bit_string(bit_index: u32, bit_value: bool) -> Option<&'static str> {
    if !bit_value {
        return None;
    }
    match 1u32.checked_shl(bit_index) {
        Some(ETC1S_P_FRAME) => Some("ETC1S_P_FRAME"),
        _ => None,
    }
}

/// Port of the library's `strnstr` fallback (`STRNSTR`). Finds `needle` in the first
/// `len` bytes of `haystack` (stopping at NUL). Returns the byte offset of the match.
pub fn strnstr(haystack: &[u8], needle: &[u8], len: usize) -> Option<usize> {
    let hay_len = haystack
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(haystack.len());
    let needle_len = needle.iter().position(|&c| c == 0).unwrap_or(needle.len());
    let limit = len.min(hay_len);
    if needle_len == 0 {
        return Some(0);
    }
    if needle_len > limit {
        return None;
    }
    (0..=limit - needle_len).find(|&i| haystack[i..i + needle_len] == needle[..needle_len])
}

/// C `printf("%#x", value)`: `0` for zero, otherwise `0x` + lowercase hex.
pub fn c_hash_x(value: impl Into<u64>) -> String {
    match value.into() {
        0 => "0".to_string(),
        v => format!("{v:#x}"),
    }
}
