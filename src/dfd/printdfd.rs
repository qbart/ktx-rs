// SPDX-FileCopyrightText: 2019-2020 The Khronos Group Inc.
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software dfdutils and modified; see NOTICE.md.
//! Port of `external/dfdutils/printdfd.c`: human-readable and JSON printing
//! of data format descriptors.
//!
//! The `write_*` functions append to a `String`; `print_dfd`/`print_dfd_json`
//! write that text to stdout like the C `printf` based originals.

#![allow(non_upper_case_globals)]

use super::access::*;
use super::khr_df::*;
use std::fmt::Write as _;
use std::io::Write as _;

// These constraints are not mandated by the spec and only used as a
// reasonable upper limit to stop parsing garbage data during print.
const MAX_NUM_BDFD_SAMPLES: i32 = 16;
const MAX_NUM_DFD_BLOCKS: i32 = 10;

/// `match value { NAME => Some("NAME"), .. }` over `khr_df` constants.
macro_rules! names {
    ($v:expr; $($c:ident),* $(,)?) => {
        match $v {
            $($c => Some(stringify!($c)),)*
            _ => None,
        }
    };
}

/// Port of `dfdToStringVendorID`.
pub fn dfd_to_string_vendor_id(value: u32) -> Option<&'static str> {
    names!(value; KHR_DF_VENDORID_KHRONOS)
}

/// Port of `dfdToStringDescriptorType`.
pub fn dfd_to_string_descriptor_type(value: u32) -> Option<&'static str> {
    names!(value;
        KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT,
        KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_PLANES,
        KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_DIMENSIONS)
}

/// Port of `dfdToStringVersionNumber` (1.0 and 1.1 share a value).
pub fn dfd_to_string_version_number(value: u32) -> Option<&'static str> {
    names!(value; KHR_DF_VERSIONNUMBER_1_1, KHR_DF_VERSIONNUMBER_1_2, KHR_DF_VERSIONNUMBER_1_3)
}

/// Port of `dfdToStringFlagsBit`.
pub fn dfd_to_string_flags_bit(bit_index: u32, bit_value: bool) -> Option<&'static str> {
    match bit_index {
        0 if bit_value => Some("KHR_DF_FLAG_ALPHA_PREMULTIPLIED"),
        0 => Some("KHR_DF_FLAG_ALPHA_STRAIGHT"),
        _ => None,
    }
}

/// Port of `dfdToStringTransferFunction` (aliases print as the first name).
pub fn dfd_to_string_transfer_function(value: u32) -> Option<&'static str> {
    names!(value;
        KHR_DF_TRANSFER_UNSPECIFIED,
        KHR_DF_TRANSFER_LINEAR,
        KHR_DF_TRANSFER_SRGB,
        KHR_DF_TRANSFER_ITU,
        KHR_DF_TRANSFER_NTSC,
        KHR_DF_TRANSFER_SLOG,
        KHR_DF_TRANSFER_SLOG2,
        KHR_DF_TRANSFER_BT1886,
        KHR_DF_TRANSFER_HLG_OETF,
        KHR_DF_TRANSFER_HLG_EOTF,
        KHR_DF_TRANSFER_PQ_EOTF,
        KHR_DF_TRANSFER_PQ_OETF,
        KHR_DF_TRANSFER_DCIP3,
        KHR_DF_TRANSFER_PAL_OETF,
        KHR_DF_TRANSFER_PAL625_EOTF,
        KHR_DF_TRANSFER_ST240,
        KHR_DF_TRANSFER_ACESCC,
        KHR_DF_TRANSFER_ACESCCT,
        KHR_DF_TRANSFER_ADOBERGB,
        KHR_DF_TRANSFER_HLG_UNNORMALIZED_OETF)
}

/// Port of `dfdToStringColorPrimaries`.
pub fn dfd_to_string_color_primaries(value: u32) -> Option<&'static str> {
    names!(value;
        KHR_DF_PRIMARIES_UNSPECIFIED,
        KHR_DF_PRIMARIES_BT709,
        KHR_DF_PRIMARIES_BT601_EBU,
        KHR_DF_PRIMARIES_BT601_SMPTE,
        KHR_DF_PRIMARIES_BT2020,
        KHR_DF_PRIMARIES_CIEXYZ,
        KHR_DF_PRIMARIES_ACES,
        KHR_DF_PRIMARIES_ACESCC,
        KHR_DF_PRIMARIES_NTSC1953,
        KHR_DF_PRIMARIES_PAL525,
        KHR_DF_PRIMARIES_DISPLAYP3,
        KHR_DF_PRIMARIES_ADOBERGB)
}

/// Port of `dfdToStringColorModel`.
pub fn dfd_to_string_color_model(value: u32) -> Option<&'static str> {
    names!(value;
        KHR_DF_MODEL_UNSPECIFIED,
        KHR_DF_MODEL_RGBSDA,
        KHR_DF_MODEL_YUVSDA,
        KHR_DF_MODEL_YIQSDA,
        KHR_DF_MODEL_LABSDA,
        KHR_DF_MODEL_CMYKA,
        KHR_DF_MODEL_XYZW,
        KHR_DF_MODEL_HSVA_ANG,
        KHR_DF_MODEL_HSLA_ANG,
        KHR_DF_MODEL_HSVA_HEX,
        KHR_DF_MODEL_HSLA_HEX,
        KHR_DF_MODEL_YCGCOA,
        KHR_DF_MODEL_YCCBCCRC,
        KHR_DF_MODEL_ICTCP,
        KHR_DF_MODEL_CIEXYZ,
        KHR_DF_MODEL_CIEXYY,
        KHR_DF_MODEL_BC1A,
        KHR_DF_MODEL_BC2,
        KHR_DF_MODEL_BC3,
        KHR_DF_MODEL_BC4,
        KHR_DF_MODEL_BC5,
        KHR_DF_MODEL_BC6H,
        KHR_DF_MODEL_BC7,
        KHR_DF_MODEL_ETC1,
        KHR_DF_MODEL_ETC2,
        KHR_DF_MODEL_ASTC,
        KHR_DF_MODEL_ETC1S,
        KHR_DF_MODEL_PVRTC,
        KHR_DF_MODEL_PVRTC2,
        KHR_DF_MODEL_UASTC,
        KHR_DF_MODEL_UASTC_HDR_4x4,
        KHR_DF_MODEL_UASTC_HDR_6x6)
}

/// Port of `dfdToStringSampleDatatypeQualifiersBit`.
pub fn dfd_to_string_sample_datatype_qualifiers_bit(
    bit_index: u32,
    bit_value: bool,
) -> Option<&'static str> {
    if !bit_value {
        return None;
    }
    names!(1u32.checked_shl(bit_index).unwrap_or(0);
        KHR_DF_SAMPLE_DATATYPE_LINEAR,
        KHR_DF_SAMPLE_DATATYPE_EXPONENT,
        KHR_DF_SAMPLE_DATATYPE_SIGNED,
        KHR_DF_SAMPLE_DATATYPE_FLOAT)
}

/// Port of `dfdToStringChannelId`. Models without their own channel names
/// fall back to `KHR_DF_CHANNEL_UNSPECIFIED_*`.
pub fn dfd_to_string_channel_id(model: u32, value: u32) -> Option<&'static str> {
    match model {
        KHR_DF_MODEL_RGBSDA => names!(value;
            KHR_DF_CHANNEL_RGBSDA_RED, KHR_DF_CHANNEL_RGBSDA_GREEN, KHR_DF_CHANNEL_RGBSDA_BLUE,
            KHR_DF_CHANNEL_RGBSDA_STENCIL, KHR_DF_CHANNEL_RGBSDA_DEPTH, KHR_DF_CHANNEL_RGBSDA_ALPHA),
        KHR_DF_MODEL_YUVSDA => names!(value;
            KHR_DF_CHANNEL_YUVSDA_Y, KHR_DF_CHANNEL_YUVSDA_U, KHR_DF_CHANNEL_YUVSDA_V,
            KHR_DF_CHANNEL_YUVSDA_STENCIL, KHR_DF_CHANNEL_YUVSDA_DEPTH, KHR_DF_CHANNEL_YUVSDA_ALPHA),
        KHR_DF_MODEL_YIQSDA => names!(value;
            KHR_DF_CHANNEL_YIQSDA_Y, KHR_DF_CHANNEL_YIQSDA_I, KHR_DF_CHANNEL_YIQSDA_Q,
            KHR_DF_CHANNEL_YIQSDA_STENCIL, KHR_DF_CHANNEL_YIQSDA_DEPTH, KHR_DF_CHANNEL_YIQSDA_ALPHA),
        KHR_DF_MODEL_LABSDA => names!(value;
            KHR_DF_CHANNEL_LABSDA_L, KHR_DF_CHANNEL_LABSDA_A, KHR_DF_CHANNEL_LABSDA_B,
            KHR_DF_CHANNEL_LABSDA_STENCIL, KHR_DF_CHANNEL_LABSDA_DEPTH, KHR_DF_CHANNEL_LABSDA_ALPHA),
        KHR_DF_MODEL_CMYKA => names!(value;
            KHR_DF_CHANNEL_CMYKSDA_CYAN, KHR_DF_CHANNEL_CMYKSDA_MAGENTA,
            KHR_DF_CHANNEL_CMYKSDA_YELLOW, KHR_DF_CHANNEL_CMYKSDA_BLACK,
            KHR_DF_CHANNEL_CMYKSDA_ALPHA),
        KHR_DF_MODEL_XYZW => names!(value;
            KHR_DF_CHANNEL_XYZW_X, KHR_DF_CHANNEL_XYZW_Y, KHR_DF_CHANNEL_XYZW_Z,
            KHR_DF_CHANNEL_XYZW_W),
        KHR_DF_MODEL_HSVA_ANG => names!(value;
            KHR_DF_CHANNEL_HSVA_ANG_VALUE, KHR_DF_CHANNEL_HSVA_ANG_SATURATION,
            KHR_DF_CHANNEL_HSVA_ANG_HUE, KHR_DF_CHANNEL_HSVA_ANG_ALPHA),
        KHR_DF_MODEL_HSLA_ANG => names!(value;
            KHR_DF_CHANNEL_HSLA_ANG_LIGHTNESS, KHR_DF_CHANNEL_HSLA_ANG_SATURATION,
            KHR_DF_CHANNEL_HSLA_ANG_HUE, KHR_DF_CHANNEL_HSLA_ANG_ALPHA),
        KHR_DF_MODEL_HSVA_HEX => names!(value;
            KHR_DF_CHANNEL_HSVA_HEX_VALUE, KHR_DF_CHANNEL_HSVA_HEX_SATURATION,
            KHR_DF_CHANNEL_HSVA_HEX_HUE, KHR_DF_CHANNEL_HSVA_HEX_ALPHA),
        KHR_DF_MODEL_HSLA_HEX => names!(value;
            KHR_DF_CHANNEL_HSLA_HEX_LIGHTNESS, KHR_DF_CHANNEL_HSLA_HEX_SATURATION,
            KHR_DF_CHANNEL_HSLA_HEX_HUE, KHR_DF_CHANNEL_HSLA_HEX_ALPHA),
        KHR_DF_MODEL_YCGCOA => names!(value;
            KHR_DF_CHANNEL_YCGCOA_Y, KHR_DF_CHANNEL_YCGCOA_CG, KHR_DF_CHANNEL_YCGCOA_CO,
            KHR_DF_CHANNEL_YCGCOA_ALPHA),
        KHR_DF_MODEL_CIEXYZ => names!(value;
            KHR_DF_CHANNEL_CIEXYZ_X, KHR_DF_CHANNEL_CIEXYZ_Y, KHR_DF_CHANNEL_CIEXYZ_Z),
        KHR_DF_MODEL_CIEXYY => names!(value;
            KHR_DF_CHANNEL_CIEXYY_X, KHR_DF_CHANNEL_CIEXYY_YCHROMA, KHR_DF_CHANNEL_CIEXYY_YLUMA),
        KHR_DF_MODEL_BC1A => names!(value; KHR_DF_CHANNEL_BC1A_COLOR, KHR_DF_CHANNEL_BC1A_ALPHA),
        KHR_DF_MODEL_BC2 => names!(value; KHR_DF_CHANNEL_BC2_COLOR, KHR_DF_CHANNEL_BC2_ALPHA),
        KHR_DF_MODEL_BC3 => names!(value; KHR_DF_CHANNEL_BC3_COLOR, KHR_DF_CHANNEL_BC3_ALPHA),
        KHR_DF_MODEL_BC4 => names!(value; KHR_DF_CHANNEL_BC4_DATA),
        KHR_DF_MODEL_BC5 => names!(value; KHR_DF_CHANNEL_BC5_RED, KHR_DF_CHANNEL_BC5_GREEN),
        KHR_DF_MODEL_BC6H => names!(value; KHR_DF_CHANNEL_BC6H_COLOR),
        KHR_DF_MODEL_BC7 => names!(value; KHR_DF_CHANNEL_BC7_COLOR),
        KHR_DF_MODEL_ETC1 => names!(value; KHR_DF_CHANNEL_ETC1_COLOR),
        KHR_DF_MODEL_ETC2 => names!(value;
            KHR_DF_CHANNEL_ETC2_RED, KHR_DF_CHANNEL_ETC2_GREEN, KHR_DF_CHANNEL_ETC2_COLOR,
            KHR_DF_CHANNEL_ETC2_ALPHA),
        KHR_DF_MODEL_ASTC => names!(value; KHR_DF_CHANNEL_ASTC_DATA),
        KHR_DF_MODEL_ETC1S => names!(value;
            KHR_DF_CHANNEL_ETC1S_RGB, KHR_DF_CHANNEL_ETC1S_RRR, KHR_DF_CHANNEL_ETC1S_GGG,
            KHR_DF_CHANNEL_ETC1S_AAA),
        KHR_DF_MODEL_PVRTC => names!(value; KHR_DF_CHANNEL_PVRTC_COLOR),
        KHR_DF_MODEL_PVRTC2 => names!(value; KHR_DF_CHANNEL_PVRTC2_COLOR),
        KHR_DF_MODEL_UASTC => names!(value;
            KHR_DF_CHANNEL_UASTC_RGB, KHR_DF_CHANNEL_UASTC_RGBA, KHR_DF_CHANNEL_UASTC_RRR,
            KHR_DF_CHANNEL_UASTC_RRRG, KHR_DF_CHANNEL_UASTC_RG),
        KHR_DF_MODEL_UASTC_HDR_4x4 => names!(value; KHR_DF_CHANNEL_UASTC_HDR_4x4_RGB),
        KHR_DF_MODEL_UASTC_HDR_6x6 => names!(value; KHR_DF_CHANNEL_UASTC_HDR_6x6_RGB),
        _ => names!(value;
            KHR_DF_CHANNEL_UNSPECIFIED_0, KHR_DF_CHANNEL_UNSPECIFIED_1,
            KHR_DF_CHANNEL_UNSPECIFIED_2, KHR_DF_CHANNEL_UNSPECIFIED_3,
            KHR_DF_CHANNEL_UNSPECIFIED_4, KHR_DF_CHANNEL_UNSPECIFIED_5,
            KHR_DF_CHANNEL_UNSPECIFIED_6, KHR_DF_CHANNEL_UNSPECIFIED_7,
            KHR_DF_CHANNEL_UNSPECIFIED_8, KHR_DF_CHANNEL_UNSPECIFIED_9,
            KHR_DF_CHANNEL_UNSPECIFIED_10, KHR_DF_CHANNEL_UNSPECIFIED_11,
            KHR_DF_CHANNEL_UNSPECIFIED_12, KHR_DF_CHANNEL_UNSPECIFIED_13,
            KHR_DF_CHANNEL_UNSPECIFIED_14, KHR_DF_CHANNEL_UNSPECIFIED_15),
    }
}

type FlagToString = fn(u32, bool) -> Option<&'static str>;

/// Port of `printFlagBits`.
fn write_flag_bits(out: &mut String, flags: u32, to_string_fn: FlagToString) {
    let mut first = true;
    for bit_index in 0..32 {
        let bit_mask = 1u32 << bit_index;
        let bit_value = (bit_mask & flags) != 0;
        let comma = if first { "" } else { ", " };
        if let Some(s) = to_string_fn(bit_index, bit_value) {
            let _ = write!(out, "{comma}{s}");
            first = false;
        } else if bit_value {
            let _ = write!(out, "{comma}{bit_mask}");
            first = false;
        }
    }
}

/// Port of `printFlagBitsJSON`.
fn write_flag_bits_json(
    out: &mut String,
    indent: usize,
    nl: &str,
    flags: u32,
    to_string_fn: FlagToString,
) {
    let mut first = true;
    for bit_index in 0..32 {
        let bit_mask = 1u32 << bit_index;
        let bit_value = (bit_mask & flags) != 0;
        let (comma, sep) = if first { ("", "") } else { (",", nl) };
        if let Some(s) = to_string_fn(bit_index, bit_value) {
            let _ = write!(out, "{comma}{sep}{:indent$}\"{s}\"", "");
            first = false;
        } else if bit_value {
            let _ = write!(out, "{comma}{sep}{:indent$}{bit_mask}", "");
            first = false;
        }
    }
    if !first {
        out.push_str(nl);
    }
}

/// `PRINT_ENUM`: the name, or the number when there is none.
fn enum_str(value: u32, to_string_fn: fn(u32) -> Option<&'static str>) -> String {
    to_string_fn(value).map_or_else(|| value.to_string(), str::to_owned)
}

const SIZEOF_DFD_TOTAL_SIZE: u32 = 4;
const SIZEOF_DFDB_HEADER: u32 = 4 * 2;
const SIZEOF_BDFD: u32 = 4 * KHR_DF_WORD_SAMPLESTART;
const SIZEOF_BDFD_SAMPLE: u32 = 4 * KHR_DF_WORD_SAMPLEWORDS;

/// `(blockSize - sizeof_BDFD) / sizeof_BDFDSample` in C's unsigned math,
/// capped at `MAX_NUM_BDFD_SAMPLES`.
fn sample_count(block_size: u32) -> i32 {
    let samples = (block_size.wrapping_sub(SIZEOF_BDFD) / SIZEOF_BDFD_SAMPLE) as i32;
    samples.min(MAX_NUM_BDFD_SAMPLES)
}

/// Text of `printDFD`. `data_size` is the maximum size (bytes) that can be
/// considered part of the DFD.
pub fn write_dfd(out: &mut String, dfd: &[u32], data_size: u32) {
    let dfd_total_size = if data_size >= SIZEOF_DFD_TOTAL_SIZE {
        dfd.first().copied().unwrap_or(0)
    } else {
        0
    };
    let mut remaining_size = dfd_total_size.min(data_size);
    if remaining_size < SIZEOF_DFD_TOTAL_SIZE {
        return; // Invalid DFD: dfdTotalSize must be present
    }
    let mut block_ofs = 1usize;
    remaining_size -= SIZEOF_DFD_TOTAL_SIZE;

    let _ = writeln!(out, "DFD total bytes: {dfd_total_size}");

    for _ in 0..MAX_NUM_DFD_BLOCKS {
        if remaining_size < SIZEOF_DFDB_HEADER {
            break; // Invalid DFD: Missing or partial block header
        }
        let block = dfd.get(block_ofs..).unwrap_or(&[]);
        let vendor_id = dfdval(block, VENDORID);
        let descriptor_type = dfdval(block, DESCRIPTORTYPE);
        let version_number = dfdval(block, VERSIONNUMBER);
        let block_size = dfdval(block, DESCRIPTORBLOCKSIZE);

        let _ = write!(
            out,
            "Vendor ID: {}\nDescriptor type: {}\nVersion: {}\nDescriptor block size: {block_size}\n",
            enum_str(vendor_id, dfd_to_string_vendor_id),
            enum_str(descriptor_type, dfd_to_string_descriptor_type),
            enum_str(version_number, dfd_to_string_version_number),
        );

        if vendor_id == KHR_DF_VENDORID_KHRONOS
            && descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT
        {
            if remaining_size < SIZEOF_BDFD {
                break; // Invalid DFD: Missing or partial basic DFD block
            }
            let model = dfdval(block, MODEL);
            let flags = dfdval(block, FLAGS);
            let _ = write!(out, "Flags: 0x{flags:X} (");
            write_flag_bits(out, flags, dfd_to_string_flags_bit);
            let _ = write!(
                out,
                ")\nTransfer: {}\nPrimaries: {}\nModel: {}\n",
                enum_str(dfdval(block, TRANSFER), dfd_to_string_transfer_function),
                enum_str(dfdval(block, PRIMARIES), dfd_to_string_color_primaries),
                enum_str(model, dfd_to_string_color_model),
            );
            let _ = writeln!(
                out,
                "Dimensions: {}, {}, {}, {}",
                dfdval(block, TEXELBLOCKDIMENSION0) + 1,
                dfdval(block, TEXELBLOCKDIMENSION1) + 1,
                dfdval(block, TEXELBLOCKDIMENSION2) + 1,
                dfdval(block, TEXELBLOCKDIMENSION3) + 1
            );
            let _ = writeln!(
                out,
                "Plane bytes: {}, {}, {}, {}, {}, {}, {}, {}",
                dfdval(block, BYTESPLANE0),
                dfdval(block, BYTESPLANE1),
                dfdval(block, BYTESPLANE2),
                dfdval(block, BYTESPLANE3),
                dfdval(block, BYTESPLANE4),
                dfdval(block, BYTESPLANE5),
                dfdval(block, BYTESPLANE6),
                dfdval(block, BYTESPLANE7)
            );

            for sample in 0..sample_count(block_size) {
                if remaining_size < SIZEOF_BDFD + (sample as u32 + 1) * SIZEOF_BDFD_SAMPLE {
                    break; // Invalid DFD: Missing or partial basic DFD sample
                }
                let s = sample as u32;
                let channel_id = dfdsval(block, s, CHANNELID);
                let _ = writeln!(out, "Sample {sample}:");
                let qualifiers = dfdsval(block, s, QUALIFIERS);
                let _ = write!(out, "    Qualifiers: 0x{qualifiers:X} (");
                write_flag_bits(
                    out,
                    qualifiers,
                    dfd_to_string_sample_datatype_qualifiers_bit,
                );
                let _ = write!(out, ")\n    Channel Id: 0x{channel_id:X}");
                match dfd_to_string_channel_id(model, channel_id) {
                    Some(str) => {
                        let _ = writeln!(out, " ({str})");
                    }
                    None => {
                        let _ = writeln!(out, " ({channel_id})");
                    }
                }
                let _ = writeln!(
                    out,
                    "    Length: {} bits Offset: {}",
                    dfdsval(block, s, BITLENGTH) + 1,
                    dfdsval(block, s, BITOFFSET)
                );
                let _ = writeln!(
                    out,
                    "    Position: {}, {}, {}, {}",
                    dfdsval(block, s, SAMPLEPOSITION0),
                    dfdsval(block, s, SAMPLEPOSITION1),
                    dfdsval(block, s, SAMPLEPOSITION2),
                    dfdsval(block, s, SAMPLEPOSITION3)
                );
                let _ = write!(
                    out,
                    "    Lower: 0x{:08x}\n    Upper: 0x{:08x}\n",
                    dfdsval(block, s, SAMPLELOWER),
                    dfdsval(block, s, SAMPLEUPPER)
                );
            }
        } else if vendor_id == KHR_DF_VENDORID_KHRONOS
            && (descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_DIMENSIONS
                || descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_PLANES)
        {
            // TODO (upstream): DFD print for ADDITIONAL_DIMENSIONS/PLANES
        } else {
            out.push_str("Unknown block\n");
        }

        let advance = SIZEOF_DFDB_HEADER.max(block_size);
        if advance > remaining_size {
            break;
        }
        remaining_size -= advance;
        block_ofs += (advance / 4) as usize;
    }
}

/// JSON text of `printDFDJSON`.
pub fn write_dfd_json(
    out: &mut String,
    dfd: &[u32],
    data_size: u32,
    mut base_indent: u32,
    mut indent_width: u32,
    minified: bool,
) {
    if minified {
        base_indent = 0;
        indent_width = 0;
    }
    let space = if minified { "" } else { " " };
    let nl = if minified { "" } else { "\n" };
    let ind = |indent: u32| ((base_indent + indent) * indent_width) as usize;
    // PRINT_ENUM
    let print_enum = |out: &mut String,
                      indent: u32,
                      name: &str,
                      value: u32,
                      to_string_fn: fn(u32) -> Option<&'static str>,
                      comma: &str| {
        let _ = write!(out, "{:w$}\"{name}\":{space}", "", w = ind(indent));
        match to_string_fn(value) {
            Some(s) => {
                let _ = write!(out, "\"{s}\"");
            }
            None => {
                let _ = write!(out, "{value}");
            }
        }
        let _ = write!(out, "{comma}{nl}");
    };

    let dfd_total_size = if data_size >= SIZEOF_DFD_TOTAL_SIZE {
        dfd.first().copied().unwrap_or(0)
    } else {
        0
    };
    let _ = write!(
        out,
        "{:w$}\"totalSize\":{space}{dfd_total_size},{nl}",
        "",
        w = ind(0)
    );

    let mut remaining_size = dfd_total_size.min(data_size);
    if remaining_size < SIZEOF_DFD_TOTAL_SIZE {
        // Print empty blocks to conform to the json scheme
        let _ = write!(out, "{:w$}\"blocks\":{space}[]{nl}", "", w = ind(0));
        return;
    }
    let mut block_ofs = 1usize;
    remaining_size -= SIZEOF_DFD_TOTAL_SIZE;
    let _ = write!(out, "{:w$}\"blocks\":{space}[", "", w = ind(0));

    for i in 0..MAX_NUM_DFD_BLOCKS {
        if remaining_size < SIZEOF_DFDB_HEADER {
            break; // Invalid DFD: Missing or partial block header
        }
        let block = dfd.get(block_ofs..).unwrap_or(&[]);
        let vendor_id = dfdval(block, VENDORID);
        let descriptor_type = dfdval(block, DESCRIPTORTYPE);
        let version_number = dfdval(block, VERSIONNUMBER);
        let block_size = dfdval(block, DESCRIPTORBLOCKSIZE);

        if i == 0 {
            out.push_str(nl);
        } else {
            let _ = write!(out, ",{nl}");
        }
        let _ = write!(out, "{:w$}{{{nl}", "", w = ind(1));
        print_enum(out, 2, "vendorId", vendor_id, dfd_to_string_vendor_id, ",");
        print_enum(
            out,
            2,
            "descriptorType",
            descriptor_type,
            dfd_to_string_descriptor_type,
            ",",
        );
        print_enum(
            out,
            2,
            "versionNumber",
            version_number,
            dfd_to_string_version_number,
            ",",
        );
        let _ = write!(
            out,
            "{:w$}\"descriptorBlockSize\":{space}{block_size}",
            "",
            w = ind(2)
        );

        if vendor_id == KHR_DF_VENDORID_KHRONOS
            && descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT
        {
            if remaining_size < SIZEOF_BDFD {
                // Invalid DFD: Missing or partial basic DFD block
                out.push_str(nl);
                let _ = write!(out, "{:w$}}}{nl}", "", w = ind(1)); // End of block
                break;
            }
            let _ = write!(out, ",{nl}");
            let _ = write!(out, "{:w$}\"flags\":{space}[{nl}", "", w = ind(2));
            let flags = dfdval(block, FLAGS);
            write_flag_bits_json(out, ind(3), nl, flags, dfd_to_string_flags_bit);
            let _ = write!(out, "{:w$}],{nl}", "", w = ind(2));

            let model = dfdval(block, MODEL);
            print_enum(
                out,
                2,
                "transferFunction",
                dfdval(block, TRANSFER),
                dfd_to_string_transfer_function,
                ",",
            );
            print_enum(
                out,
                2,
                "colorPrimaries",
                dfdval(block, PRIMARIES),
                dfd_to_string_color_primaries,
                ",",
            );
            print_enum(out, 2, "colorModel", model, dfd_to_string_color_model, ",");
            let _ = write!(
                out,
                "{:w$}\"texelBlockDimension\":{space}[{},{space}{},{space}{},{space}{}],{nl}",
                "",
                dfdval(block, TEXELBLOCKDIMENSION0) + 1,
                dfdval(block, TEXELBLOCKDIMENSION1) + 1,
                dfdval(block, TEXELBLOCKDIMENSION2) + 1,
                dfdval(block, TEXELBLOCKDIMENSION3) + 1,
                w = ind(2)
            );
            let planes = [
                BYTESPLANE0,
                BYTESPLANE1,
                BYTESPLANE2,
                BYTESPLANE3,
                BYTESPLANE4,
                BYTESPLANE5,
                BYTESPLANE6,
                BYTESPLANE7,
            ]
            .map(|p| dfdval(block, p).to_string())
            .join(&format!(",{space}"));
            let _ = write!(
                out,
                "{:w$}\"bytesPlane\":{space}[{planes}],{nl}",
                "",
                w = ind(2)
            );

            let _ = write!(out, "{:w$}\"samples\":{space}[{nl}", "", w = ind(2));
            for sample in 0..sample_count(block_size) {
                if remaining_size < SIZEOF_BDFD + (sample as u32 + 1) * SIZEOF_BDFD_SAMPLE {
                    break; // Invalid DFD: Missing or partial basic DFD sample
                }
                let s = sample as u32;
                if sample != 0 {
                    let _ = write!(out, ",{nl}");
                }
                let _ = write!(out, "{:w$}{{{nl}", "", w = ind(3));

                let qualifiers = dfdsval(block, s, QUALIFIERS);
                if qualifiers == 0 {
                    let _ = write!(out, "{:w$}\"qualifiers\":{space}[],{nl}", "", w = ind(4));
                } else {
                    let _ = write!(out, "{:w$}\"qualifiers\":{space}[{nl}", "", w = ind(4));
                    write_flag_bits_json(
                        out,
                        ind(5),
                        nl,
                        qualifiers,
                        dfd_to_string_sample_datatype_qualifiers_bit,
                    );
                    let _ = write!(out, "{:w$}],{nl}", "", w = ind(4));
                }

                let channel_id = dfdsval(block, s, CHANNELID);
                match dfd_to_string_channel_id(model, channel_id) {
                    Some(str) => {
                        let _ = write!(
                            out,
                            "{:w$}\"channelId\":{space}\"{str}\",{nl}",
                            "",
                            w = ind(4)
                        );
                    }
                    None => {
                        let _ = write!(
                            out,
                            "{:w$}\"channelId\":{space}{channel_id},{nl}",
                            "",
                            w = ind(4)
                        );
                    }
                }
                let _ = write!(
                    out,
                    "{:w$}\"bitLength\":{space}{},{nl}",
                    "",
                    dfdsval(block, s, BITLENGTH),
                    w = ind(4)
                );
                let _ = write!(
                    out,
                    "{:w$}\"bitOffset\":{space}{},{nl}",
                    "",
                    dfdsval(block, s, BITOFFSET),
                    w = ind(4)
                );
                let _ = write!(
                    out,
                    "{:w$}\"samplePosition\":{space}[{},{space}{},{space}{},{space}{}],{nl}",
                    "",
                    dfdsval(block, s, SAMPLEPOSITION0),
                    dfdsval(block, s, SAMPLEPOSITION1),
                    dfdsval(block, s, SAMPLEPOSITION2),
                    dfdsval(block, s, SAMPLEPOSITION3),
                    w = ind(4)
                );
                let lower = dfdsval(block, s, SAMPLELOWER);
                let upper = dfdsval(block, s, SAMPLEUPPER);
                if qualifiers & KHR_DF_SAMPLE_DATATYPE_SIGNED != 0 {
                    let _ = write!(
                        out,
                        "{:w$}\"sampleLower\":{space}{},{nl}{:w$}\"sampleUpper\":{space}{}{nl}",
                        "",
                        lower as i32,
                        "",
                        upper as i32,
                        w = ind(4)
                    );
                } else {
                    let _ = write!(
                        out,
                        "{:w$}\"sampleLower\":{space}{lower},{nl}{:w$}\"sampleUpper\":{space}{upper}{nl}",
                        "",
                        "",
                        w = ind(4)
                    );
                }
                let _ = write!(out, "{:w$}}}", "", w = ind(3));
            }
            out.push_str(nl);
            let _ = write!(out, "{:w$}]{nl}", "", w = ind(2)); // End of samples
        } else {
            // ADDITIONAL_DIMENSIONS/PLANES and unknown blocks: no members yet.
            out.push_str(nl);
        }

        let _ = write!(out, "{:w$}}}", "", w = ind(1)); // End of block

        let advance = SIZEOF_DFDB_HEADER.max(block_size);
        if advance > remaining_size {
            break;
        }
        remaining_size -= advance;
        block_ofs += (advance / 4) as usize;
    }
    out.push_str(nl);
    let _ = write!(out, "{:w$}]{nl}", "", w = ind(0)); // End of blocks
}

/// Port of `printDFD`: prints [`write_dfd`]'s text to stdout.
pub fn print_dfd(dfd: &[u32], data_size: u32) {
    let mut s = String::new();
    write_dfd(&mut s, dfd, data_size);
    let _ = std::io::stdout().write_all(s.as_bytes());
}

/// Port of `printDFDJSON`: prints [`write_dfd_json`]'s text to stdout.
pub fn print_dfd_json(
    dfd: &[u32],
    data_size: u32,
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) {
    let mut s = String::new();
    write_dfd_json(&mut s, dfd, data_size, base_indent, indent_width, minified);
    let _ = std::io::stdout().write_all(s.as_bytes());
}
