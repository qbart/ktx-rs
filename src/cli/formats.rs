// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/formats.h. The VkFormat switch predicates are generated
//! into `formats_gen.rs` by `scripts/gen_cli_tables.py`.

use crate::defs::SupercmpScheme;
use crate::dfd::khr_df::KHR_DF_MODEL_DXT1A;
use crate::dfd::khr_df::KHR_DF_VENDORID_KHRONOS;
use crate::dfd::printdfd::{
    dfd_to_string_channel_id, dfd_to_string_color_model, dfd_to_string_color_primaries,
    dfd_to_string_descriptor_type, dfd_to_string_transfer_function, dfd_to_string_vendor_id,
    dfd_to_string_version_number,
};
use crate::strings::ktx_supercompression_scheme_string;
use crate::vkformat::{VkFormat, is_valid_format, string_to_vk_format, vk_format_string};

pub use super::formats_gen::*;

/// Port of `toString(VkFormat)`.
pub fn to_string_vk_format(format: VkFormat) -> String {
    match vk_format_string(format) {
        "VK_UNKNOWN_FORMAT" => format!("(0x{:08X})", format.0),
        s => s.to_string(),
    }
}

/// Port of `toString(ktxSupercmpScheme)`.
pub fn to_string_scheme(scheme: SupercmpScheme) -> String {
    match ktx_supercompression_scheme_string(scheme) {
        "Invalid scheme value" => format!("(0x{:08X})", scheme.0),
        "Vendor or reserved scheme" => format!("Vendor or reserved scheme (0x{:08X})", scheme.0),
        s => s.to_string(),
    }
}

/// Port of `toString(khr_df_vendorid_e)`.
pub fn to_string_vendor_id(vendor_id: u32) -> String {
    match dfd_to_string_vendor_id(vendor_id) {
        Some(s) => s.to_string(),
        None => format!("(0x{vendor_id:05X})"),
    }
}

/// Port of `toString(khr_df_vendorid_e, khr_df_khr_descriptortype_e)`.
pub fn to_string_descriptor_type(vendor_id: u32, desc_type: u32) -> String {
    if vendor_id == KHR_DF_VENDORID_KHRONOS
        && let Some(s) = dfd_to_string_descriptor_type(desc_type)
    {
        return s.to_string();
    }
    format!("(0x{desc_type:04X})")
}

/// Port of `toString(khr_df_versionnumber_e)`.
pub fn to_string_version_number(version: u32) -> String {
    match dfd_to_string_version_number(version) {
        Some(s) => s.to_string(),
        None => format!("(0x{version:04X})"),
    }
}

/// Port of `toString(khr_df_model_e)`.
pub fn to_string_model(model: u32) -> String {
    match dfd_to_string_color_model(model) {
        Some(s) => s.to_string(),
        None => format!("(0x{model:02X})"),
    }
}

/// Port of `toString(khr_df_primaries_e)`.
pub fn to_string_primaries(primaries: u32) -> String {
    match dfd_to_string_color_primaries(primaries) {
        Some(s) => s.to_string(),
        None => format!("(0x{primaries:02X})"),
    }
}

/// Port of `toString(khr_df_transfer_e)`.
pub fn to_string_transfer(transfer: u32) -> String {
    match dfd_to_string_transfer_function(transfer) {
        Some(s) => s.to_string(),
        None => format!("(0x{transfer:02X})"),
    }
}

/// Port of `toString(khr_df_model_e, khr_df_model_channels_e)`.
pub fn to_string_channel(model: u32, channel_id: u32) -> String {
    match dfd_to_string_channel_id(model, channel_id) {
        Some(s) => s.to_string(),
        None => format!("(0x{channel_id:01X})"),
    }
}

/// Port of `parseVkFormat`. `VK_FORMAT_` prefix optional, case insensitive.
pub fn parse_vk_format(s: &str) -> Option<VkFormat> {
    match string_to_vk_format(s) {
        VkFormat::UNDEFINED => None,
        f => Some(f),
    }
}

/// Port of `isSupercompressionWithGlobalData`.
pub fn is_supercompression_with_global_data(scheme: SupercmpScheme) -> bool {
    matches!(
        scheme,
        SupercmpScheme::BASIS_LZ | SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
    )
}

/// Port of `isSupercompressionWithNoGlobalData`.
pub fn is_supercompression_with_no_global_data(scheme: SupercmpScheme) -> bool {
    matches!(scheme, SupercmpScheme::ZSTD | SupercmpScheme::ZLIB)
}

/// Port of `isSupercompressionBlockCompressed`.
pub fn is_supercompression_block_compressed(scheme: SupercmpScheme) -> bool {
    matches!(
        scheme,
        SupercmpScheme::BASIS_LZ | SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
    )
}

/// Port of `isColorModelBlockCompressed`.
pub fn is_color_model_block_compressed(color_model: u32) -> bool {
    color_model >= KHR_DF_MODEL_DXT1A
}

/// Port of `isColorPrimariesValid`.
pub fn is_color_primaries_valid(primaries: u32) -> bool {
    dfd_to_string_color_primaries(primaries).is_some()
}

/// Port of `isChannelIdValid`.
pub fn is_channel_id_valid(color_model: u32, channel_id: u32) -> bool {
    dfd_to_string_channel_id(color_model, channel_id).is_some()
}

/// Port of `isFormatValid` (rejects negative values).
pub fn is_format_valid(format: VkFormat) -> bool {
    is_valid_format(format) && (format.0 as i32) >= 0
}

/// Port of `isFormatKnown`.
pub fn is_format_known(format: VkFormat) -> bool {
    vk_format_string(format) != "VK_UNKNOWN_FORMAT"
}

/// Port of `isFormatDepthStencil`.
pub fn is_format_depth_stencil(format: VkFormat) -> bool {
    is_format_depth(format) || is_format_stencil(format)
}
