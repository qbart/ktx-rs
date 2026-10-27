// SPDX-FileCopyrightText: 2019-2020 The Khronos Group Inc.
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software dfdutils and modified; see NOTICE.md.
//! Port of external/dfdutils/queries.c.

use super::access::*;
use super::khr_df::*;

/// Port of `getDFDComponentInfoUnpacked`. Returns `(numComponents, componentByteLength)`.
pub fn get_dfd_component_info_unpacked(dfd: &[u32]) -> (u32, u32) {
    let b = bdb(dfd);
    let num_samples = dfdsamplecount(b);
    let mut current_channel = !0u32;
    let mut num_components = 0u32;
    let mut component_byte_length = 0u32;
    for s in 0..num_samples {
        let sample_byte_length = (dfdsval(b, s, BITLENGTH) + 1) >> 3;
        let sample_channel = dfdsval(b, s, CHANNELID);
        if sample_channel == current_channel {
            component_byte_length += sample_byte_length;
        } else {
            current_channel = sample_channel;
            num_components += 1;
            component_byte_length = sample_byte_length;
        }
    }
    (num_components, component_byte_length)
}

/// Port of `getDFDNumComponents`.
pub fn get_dfd_num_components(dfd: &[u32]) -> u32 {
    let b = bdb(dfd);
    let mut current_channel = !0u32;
    let mut num_components = 0u32;
    for s in 0..dfdsamplecount(b) {
        let sample_channel = dfdsval(b, s, CHANNELID);
        if sample_channel != current_channel {
            num_components += 1;
            current_channel = sample_channel;
        }
    }
    num_components
}

/// Port of `reconstructDFDBytesPlane0FromSamples`.
pub fn reconstruct_dfd_bytes_plane0_from_samples(dfd: &[u32]) -> u32 {
    let b = bdb(dfd);
    let num_samples = dfdsamplecount(b);

    // Special case depth{,-stencil} formats whose unused bits are in the MSBs.
    if dfdsval(b, 0, CHANNELID) == KHR_DF_CHANNEL_COMMON_DEPTH {
        if num_samples == 1 {
            if dfdsval(b, 0, BITLENGTH) + 1 == 24 {
                return 4; // X8_D24_UNORM_PACK32
            }
        } else if num_samples == 2 {
            if dfdsval(b, 0, BITLENGTH) + 1 == 16 {
                return 4; // D16_UNORM_S8_UINT
            }
            if dfdsval(b, 0, BITLENGTH) + 1 == 32
                && dfdsval(b, 1, CHANNELID) == KHR_DF_CHANNEL_COMMON_STENCIL
            {
                return 8; // D32_SFLOAT_S8_UINT
            }
        }
    }
    if dfdval(b, MODEL) == KHR_DF_MODEL_ETC1S {
        return 8; // Size of the first plane.
    }
    let mut largest_offset = 0u32;
    let mut sample_number_with_largest_offset = 0u32;
    for s in 0..num_samples {
        let sample_bit_offset = dfdsval(b, s, BITOFFSET);
        if sample_bit_offset > largest_offset {
            largest_offset = sample_bit_offset;
            sample_number_with_largest_offset = s;
        }
    }
    let sample_bit_length = dfdsval(b, sample_number_with_largest_offset, BITLENGTH) + 1;
    (largest_offset + sample_bit_length) >> 3
}

/// Port of `reconstructDFDBytesPlanesFromSamples`.
pub fn reconstruct_dfd_bytes_planes_from_samples(dfd: &mut [u32]) {
    let bp0 = reconstruct_dfd_bytes_plane0_from_samples(dfd);
    let b = bdb_mut(dfd);
    dfdsetval(b, BYTESPLANE0, bp0);
    if dfdval(b, MODEL) == KHR_DF_MODEL_ETC1S && dfdsamplecount(b) == 2 {
        dfdsetval(b, BYTESPLANE1, 8);
    }
}

/// Port of `recreateBytesPlane0FromSampleInfo` (deprecated).
pub fn recreate_bytes_plane0_from_sample_info(dfd: &[u32]) -> u32 {
    reconstruct_dfd_bytes_plane0_from_samples(dfd)
}
