// SPDX-FileCopyrightText: 2019-2020 The Khronos Group Inc.
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software dfdutils and modified; see NOTICE.md.
//! Port of external/dfdutils/createdfd.c.
//!
//! Each function returns the full DFD (`dfd[0]` = total size in bytes) or `None`
//! when the arguments are outside what the C code handles.

use super::access::*;
use super::khr_df::*;
use super::{VkCompScheme, VkSuffix};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChannelsInfoType {
    Color,
    NonColor,
}

fn write_header(
    num_samples: u32,
    bytes: u32,
    suffix: VkSuffix,
    infotype: ChannelsInfoType,
) -> Vec<u32> {
    let words = 1 + KHR_DF_WORD_SAMPLESTART + num_samples * KHR_DF_WORD_SAMPLEWORDS;
    let mut dfd = vec![0u32; words as usize];
    dfd[0] = 4 * words;
    let b = bdb_mut(&mut dfd);
    b[KHR_DF_WORD_VENDORID as usize] = (KHR_DF_VENDORID_KHRONOS << KHR_DF_SHIFT_VENDORID)
        | (KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT << KHR_DF_SHIFT_DESCRIPTORTYPE);
    b[KHR_DF_WORD_VERSIONNUMBER as usize] = (KHR_DF_VERSIONNUMBER_LATEST
        << KHR_DF_SHIFT_VERSIONNUMBER)
        | ((4 * (KHR_DF_WORD_SAMPLESTART + num_samples * KHR_DF_WORD_SAMPLEWORDS))
            << KHR_DF_SHIFT_DESCRIPTORBLOCKSIZE);
    b[KHR_DF_WORD_MODEL as usize] = (KHR_DF_MODEL_RGBSDA << KHR_DF_SHIFT_MODEL)
        | (KHR_DF_FLAG_ALPHA_STRAIGHT << KHR_DF_SHIFT_FLAGS);
    if infotype == ChannelsInfoType::Color {
        b[KHR_DF_WORD_PRIMARIES as usize] |= KHR_DF_PRIMARIES_BT709 << KHR_DF_SHIFT_PRIMARIES;
    } else {
        b[KHR_DF_WORD_PRIMARIES as usize] |= KHR_DF_PRIMARIES_UNSPECIFIED << KHR_DF_SHIFT_PRIMARIES;
    }
    if suffix == VkSuffix::Srgb {
        b[KHR_DF_WORD_TRANSFER as usize] |= KHR_DF_TRANSFER_SRGB << KHR_DF_SHIFT_TRANSFER;
    } else {
        b[KHR_DF_WORD_TRANSFER as usize] |= KHR_DF_TRANSFER_LINEAR << KHR_DF_SHIFT_TRANSFER;
    }
    b[KHR_DF_WORD_TEXELBLOCKDIMENSION0 as usize] = 0;
    b[KHR_DF_WORD_BYTESPLANE0 as usize] = bytes;
    b[KHR_DF_WORD_BYTESPLANE4 as usize] = 0;
    dfd
}

fn set_channel_flags(mut channel: u32, suffix: VkSuffix) -> u32 {
    match suffix {
        VkSuffix::Unorm | VkSuffix::Uscaled | VkSuffix::Uint => {}
        VkSuffix::Snorm | VkSuffix::Sscaled | VkSuffix::Sint | VkSuffix::Sfixed5 => {
            channel |= KHR_DF_SAMPLE_DATATYPE_SIGNED;
        }
        VkSuffix::Sfloat => channel |= KHR_DF_SAMPLE_DATATYPE_FLOAT | KHR_DF_SAMPLE_DATATYPE_SIGNED,
        VkSuffix::Ufloat => channel |= KHR_DF_SAMPLE_DATATYPE_FLOAT,
        VkSuffix::Srgb => {
            if channel == KHR_DF_CHANNEL_RGBSDA_ALPHA {
                channel |= KHR_DF_SAMPLE_DATATYPE_LINEAR;
            }
        }
    }
    channel
}

/// `(1 << bits) - 1` for 1..=32 bits, saturating.
fn low_mask(bits: u32) -> u32 {
    if bits >= 32 {
        0xFFFF_FFFF
    } else {
        (1u32 << bits) - 1
    }
}

#[allow(clippy::too_many_arguments)]
fn write_sample(
    dfd: &mut [u32],
    sample_no: u32,
    channel: u32,
    bits: u32,
    offset: u32,
    top_sample: bool,
    bottom_sample: bool,
    suffix: VkSuffix,
) {
    let base = (1 + KHR_DF_WORD_SAMPLESTART + sample_no * KHR_DF_WORD_SAMPLEWORDS) as usize;
    let mut channel = channel;
    if channel == 3 {
        channel = KHR_DF_CHANNEL_RGBSDA_ALPHA;
    }
    channel = set_channel_flags(channel, suffix);
    let word0 = (offset << KHR_DF_SAMPLESHIFT_BITOFFSET)
        | (bits.wrapping_sub(1) << KHR_DF_SAMPLESHIFT_BITLENGTH)
        | (channel << KHR_DF_SAMPLESHIFT_CHANNELID);

    let (lower, upper): (u32, u32) = match suffix {
        VkSuffix::Unorm | VkSuffix::Srgb => (0, low_mask(bits)),
        VkSuffix::Snorm => {
            let upper = if bits > 32 {
                0x7FFF_FFFF
            } else if top_sample {
                low_mask(bits.saturating_sub(1))
            } else {
                low_mask(bits)
            };
            let mut lower = !upper;
            if bottom_sample {
                lower = lower.wrapping_add(1);
            }
            (lower, upper)
        }
        VkSuffix::Uscaled | VkSuffix::Uint => (0, u32::from(bottom_sample)),
        VkSuffix::Sscaled | VkSuffix::Sint => (!0u32, u32::from(bottom_sample)),
        VkSuffix::Sfloat => ((-1.0f32).to_bits(), 1.0f32.to_bits()),
        VkSuffix::Ufloat => (0.0f32.to_bits(), 1.0f32.to_bits()),
        VkSuffix::Sfixed5 => {
            let upper = 32u32;
            (upper.wrapping_neg(), upper)
        }
    };
    let words = [word0, 0, lower, upper];
    if let Some(dst) = dfd.get_mut(base..base + 4) {
        dst.copy_from_slice(&words);
    }
}

/// Port of `createDFDUnpacked`.
pub fn create_dfd_unpacked(
    big_endian: bool,
    num_channels: u32,
    bytes: u32,
    red_blue_swap: bool,
    suffix: VkSuffix,
) -> Option<Vec<u32>> {
    if num_channels == 0 || num_channels > 4 || bytes == 0 || bytes > 8 {
        return None;
    }
    let mut dfd;
    if big_endian {
        dfd = write_header(
            num_channels * bytes,
            num_channels * bytes,
            suffix,
            ChannelsInfoType::Color,
        );
        for channel_counter in 0..num_channels {
            let mut channel = channel_counter;
            if red_blue_swap && (channel == 0 || channel == 2) {
                channel ^= 2;
            }
            for channel_byte in 0..bytes {
                write_sample(
                    &mut dfd,
                    channel_counter * bytes + channel_byte,
                    channel,
                    8,
                    8 * (channel_counter * bytes + bytes - channel_byte - 1),
                    channel_byte == bytes - 1,
                    channel_byte == 0,
                    suffix,
                );
            }
        }
    } else {
        dfd = write_header(
            num_channels,
            num_channels * bytes,
            suffix,
            ChannelsInfoType::Color,
        );
        for sample_counter in 0..num_channels {
            let mut channel = sample_counter;
            if red_blue_swap && (channel == 0 || channel == 2) {
                channel ^= 2;
            }
            write_sample(
                &mut dfd,
                sample_counter,
                channel,
                8 * bytes,
                8 * sample_counter * bytes,
                true,
                true,
                suffix,
            );
        }
    }
    Some(dfd)
}

/// Port of `createDFDPackedShifted`.
pub fn create_dfd_packed_shifted(
    big_endian: bool,
    num_channels: u32,
    bits: &[u32],
    shift_bits: &[u32],
    channels: &[u32],
    suffix: VkSuffix,
) -> Option<Vec<u32>> {
    let n = num_channels as usize;
    if num_channels == 6 {
        // Special case E5B9G9R9
        let mut dfd = write_header(num_channels, 4, VkSuffix::Ufloat, ChannelsInfoType::Color);
        write_sample(&mut dfd, 0, 0, 9, 0, true, true, VkSuffix::Unorm);
        dfdsetsval(bdb_mut(&mut dfd), 0, SAMPLEUPPER, 8448);
        write_sample(
            &mut dfd,
            1,
            KHR_DF_SAMPLE_DATATYPE_EXPONENT,
            5,
            27,
            true,
            true,
            VkSuffix::Unorm,
        );
        dfdsetsval(bdb_mut(&mut dfd), 1, SAMPLELOWER, 15);
        dfdsetsval(bdb_mut(&mut dfd), 1, SAMPLEUPPER, 31);
        write_sample(&mut dfd, 2, 1, 9, 9, true, true, VkSuffix::Unorm);
        dfdsetsval(bdb_mut(&mut dfd), 2, SAMPLEUPPER, 8448);
        write_sample(
            &mut dfd,
            3,
            1 | KHR_DF_SAMPLE_DATATYPE_EXPONENT,
            5,
            27,
            true,
            true,
            VkSuffix::Unorm,
        );
        dfdsetsval(bdb_mut(&mut dfd), 3, SAMPLELOWER, 15);
        dfdsetsval(bdb_mut(&mut dfd), 3, SAMPLEUPPER, 31);
        write_sample(&mut dfd, 4, 2, 9, 18, true, true, VkSuffix::Unorm);
        dfdsetsval(bdb_mut(&mut dfd), 4, SAMPLEUPPER, 8448);
        write_sample(
            &mut dfd,
            5,
            2 | KHR_DF_SAMPLE_DATATYPE_EXPONENT,
            5,
            27,
            true,
            true,
            VkSuffix::Unorm,
        );
        dfdsetsval(bdb_mut(&mut dfd), 5, SAMPLELOWER, 15);
        dfdsetsval(bdb_mut(&mut dfd), 5, SAMPLEUPPER, 31);
        return Some(dfd);
    }
    if num_channels == 0
        || num_channels > 4
        || bits.len() < n
        || shift_bits.len() < n
        || channels.len() < n
    {
        return None;
    }
    if big_endian {
        // No packed format is larger than 32 bits.
        // No packed channel crosses more than two bytes.
        let mut total_bits: u32 = 0;
        let mut bit_channel = [-1i32; 32];
        let mut be_channel_start = [0u32; 4];
        for c in 0..n {
            be_channel_start[c] = total_bits;
            total_bits += shift_bits[c] + bits[c];
        }
        if total_bits == 0 || total_bits > 32 {
            return None;
        }
        let be_mask = (total_bits - 1) & 0x18;
        let mut bit_offset: u32 = 0;
        let mut num_samples = num_channels;
        for c in 0..n {
            bit_offset += shift_bits[c];
            *bit_channel.get_mut(((bit_offset ^ be_mask) & 31) as usize)? = c as i32;
            let last = bit_offset + bits[c] - 1;
            if (last & !7) != (bit_offset & !7) {
                // Continuation sample
                *bit_channel.get_mut((((last & !7) ^ be_mask) & 31) as usize)? = c as i32;
                num_samples += 1;
            }
            bit_offset += bits[c];
        }
        let mut dfd = write_header(
            num_samples,
            total_bits >> 3,
            suffix,
            ChannelsInfoType::Color,
        );
        let mut sample_counter = 0u32;
        bit_offset = 0;
        while bit_offset < total_bits {
            let bc = *bit_channel.get(bit_offset as usize)?;
            if bc == -1 {
                // Done this bit, so this is the lower half of something.
                bit_offset = (bit_offset + 8) & !7;
            } else {
                let this_channel = bc as usize;
                if (be_channel_start[this_channel] ^ be_mask) == bit_offset {
                    // Must be just one sample if we hit it first.
                    write_sample(
                        &mut dfd,
                        sample_counter,
                        channels[this_channel],
                        bits[this_channel],
                        bit_offset,
                        true,
                        true,
                        suffix,
                    );
                    sample_counter += 1;
                    bit_offset += bits[this_channel];
                } else {
                    // Two samples. Move to the end of the first one we hit when we're done.
                    let first_sample_bits = 8 - (be_channel_start[this_channel] & 0x7);
                    let second_sample_bits = bits[this_channel].checked_sub(first_sample_bits)?;
                    write_sample(
                        &mut dfd,
                        sample_counter,
                        channels[this_channel],
                        first_sample_bits,
                        be_channel_start[this_channel] ^ be_mask,
                        false,
                        true,
                        suffix,
                    );
                    sample_counter += 1;
                    // Mark that we've already handled this sample
                    *bit_channel
                        .get_mut(((be_channel_start[this_channel] ^ be_mask) & 31) as usize)? = -1;
                    write_sample(
                        &mut dfd,
                        sample_counter,
                        channels[this_channel],
                        second_sample_bits,
                        bit_offset,
                        true,
                        false,
                        suffix,
                    );
                    sample_counter += 1;
                    bit_offset += second_sample_bits;
                }
            }
        }
        Some(dfd)
    } else {
        let mut total_bits = 0u32;
        for c in 0..n {
            total_bits += shift_bits[c] + bits[c];
        }
        let mut dfd = write_header(
            num_channels,
            total_bits >> 3,
            suffix,
            ChannelsInfoType::Color,
        );
        let mut bit_offset = 0u32;
        for c in 0..n {
            bit_offset += shift_bits[c];
            write_sample(
                &mut dfd,
                c as u32,
                channels[c],
                bits[c],
                bit_offset,
                true,
                true,
                suffix,
            );
            bit_offset += bits[c];
        }
        Some(dfd)
    }
}

/// Port of `createDFDPacked`.
pub fn create_dfd_packed(
    big_endian: bool,
    num_channels: u32,
    bits: &[u32],
    channels: &[u32],
    suffix: VkSuffix,
) -> Option<Vec<u32>> {
    if num_channels > 6 {
        return None;
    }
    let shift_bits = [0u32; 6];
    let shift_bits = &shift_bits[..(num_channels as usize).max(bits.len()).min(6)];
    create_dfd_packed_shifted(big_endian, num_channels, bits, shift_bits, channels, suffix)
}

/// Port of `createDFD422`. Only little-endian UNORM is supported (asserted in C).
#[allow(clippy::too_many_arguments)]
pub fn create_dfd_422(
    big_endian: bool,
    num_samples: u32,
    bits: &[u32],
    shift_bits: &[u32],
    channels: &[u32],
    position_xs: &[u32],
    position_ys: &[u32],
    suffix: VkSuffix,
) -> Option<Vec<u32>> {
    let n = num_samples as usize;
    if big_endian
        || suffix != VkSuffix::Unorm
        || bits.len() < n
        || shift_bits.len() < n
        || channels.len() < n
        || position_xs.len() < n
        || position_ys.len() < n
    {
        return None;
    }
    let total_bits: u32 = (0..n).map(|i| shift_bits[i] + bits[i]).sum();
    if !total_bits.is_multiple_of(8) {
        return None;
    }
    let bdfd_size = 4 * (KHR_DF_WORD_SAMPLESTART + num_samples * KHR_DF_WORD_SAMPLEWORDS);
    let dfd_size = 4 + bdfd_size;
    let mut dfd = vec![0u32; (dfd_size / 4) as usize];
    dfd[0] = dfd_size;
    let b = bdb_mut(&mut dfd);
    dfdsetval(b, VENDORID, KHR_DF_VENDORID_KHRONOS);
    dfdsetval(b, DESCRIPTORTYPE, KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT);
    dfdsetval(b, VERSIONNUMBER, KHR_DF_VERSIONNUMBER_LATEST);
    dfdsetval(b, DESCRIPTORBLOCKSIZE, bdfd_size);
    dfdsetval(b, MODEL, KHR_DF_MODEL_YUVSDA);
    dfdsetval(b, PRIMARIES, KHR_DF_PRIMARIES_UNSPECIFIED);
    dfdsetval(b, TRANSFER, KHR_DF_TRANSFER_LINEAR);
    dfdsetval(b, FLAGS, KHR_DF_FLAG_ALPHA_STRAIGHT);
    dfdsetval(b, TEXELBLOCKDIMENSION0, 2 - 1); // 422 contains 2 x 1 blocks
    dfdsetval(b, TEXELBLOCKDIMENSION1, 0);
    dfdsetval(b, TEXELBLOCKDIMENSION2, 0);
    dfdsetval(b, TEXELBLOCKDIMENSION3, 0);
    dfdsetval(b, BYTESPLANE0, total_bits / 8);
    for f in [
        BYTESPLANE1,
        BYTESPLANE2,
        BYTESPLANE3,
        BYTESPLANE4,
        BYTESPLANE5,
        BYTESPLANE6,
        BYTESPLANE7,
    ] {
        dfdsetval(b, f, 0);
    }
    let mut bit_offset = 0u32;
    for i in 0..n {
        let s = i as u32;
        bit_offset += shift_bits[i];
        dfdsetsval(b, s, BITOFFSET, bit_offset);
        dfdsetsval(b, s, BITLENGTH, bits[i].wrapping_sub(1));
        dfdsetsval(b, s, CHANNELID, channels[i]);
        dfdsetsval(b, s, QUALIFIERS, 0);
        dfdsetsval(b, s, SAMPLEPOSITION0, position_xs[i]);
        dfdsetsval(b, s, SAMPLEPOSITION1, position_ys[i]);
        dfdsetsval(b, s, SAMPLEPOSITION2, 0);
        dfdsetsval(b, s, SAMPLEPOSITION3, 0);
        dfdsetsval(b, s, SAMPLELOWER, 0);
        dfdsetsval(b, s, SAMPLEUPPER, low_mask(bits[i]));
        bit_offset += bits[i];
    }
    Some(dfd)
}

struct CompInfo {
    model: u32,
    sample_count: u32,
    first_channel: u32,
    second_channel: u32,
    second_channel_offset: u32,
    channel_bits: u32,
    bytes: u32,
}

const fn ci(
    model: u32,
    sample_count: u32,
    first: u32,
    second: u32,
    off: u32,
    bits: u32,
    bytes: u32,
) -> CompInfo {
    CompInfo {
        model,
        sample_count,
        first_channel: first,
        second_channel: second,
        second_channel_offset: off,
        channel_bits: bits,
        bytes,
    }
}

fn comp_info(scheme: VkCompScheme) -> CompInfo {
    use VkCompScheme::*;
    match scheme {
        Bc1Rgb => ci(
            KHR_DF_MODEL_BC1A,
            1,
            KHR_DF_CHANNEL_BC1A_COLOR,
            KHR_DF_CHANNEL_BC1A_COLOR,
            0,
            64,
            8,
        ),
        Bc1Rgba => ci(
            KHR_DF_MODEL_BC1A,
            1,
            KHR_DF_CHANNEL_BC1A_ALPHAPRESENT,
            KHR_DF_CHANNEL_BC1A_ALPHAPRESENT,
            0,
            64,
            8,
        ),
        Bc2 => ci(
            KHR_DF_MODEL_BC2,
            2,
            KHR_DF_CHANNEL_BC2_ALPHA,
            KHR_DF_CHANNEL_BC2_COLOR,
            64,
            64,
            16,
        ),
        Bc3 => ci(
            KHR_DF_MODEL_BC3,
            2,
            KHR_DF_CHANNEL_BC3_ALPHA,
            KHR_DF_CHANNEL_BC3_COLOR,
            64,
            64,
            16,
        ),
        Bc4 => ci(
            KHR_DF_MODEL_BC4,
            1,
            KHR_DF_CHANNEL_BC4_DATA,
            KHR_DF_CHANNEL_BC4_DATA,
            0,
            64,
            8,
        ),
        Bc5 => ci(
            KHR_DF_MODEL_BC5,
            2,
            KHR_DF_CHANNEL_BC5_RED,
            KHR_DF_CHANNEL_BC5_GREEN,
            64,
            64,
            16,
        ),
        Bc6h => ci(
            KHR_DF_MODEL_BC6H,
            1,
            KHR_DF_CHANNEL_BC6H_COLOR,
            KHR_DF_CHANNEL_BC6H_COLOR,
            0,
            128,
            16,
        ),
        Bc7 => ci(
            KHR_DF_MODEL_BC7,
            1,
            KHR_DF_CHANNEL_BC7_COLOR,
            KHR_DF_CHANNEL_BC7_COLOR,
            0,
            128,
            16,
        ),
        Etc2R8G8B8 => ci(
            KHR_DF_MODEL_ETC2,
            1,
            KHR_DF_CHANNEL_ETC2_COLOR,
            KHR_DF_CHANNEL_ETC2_COLOR,
            0,
            64,
            8,
        ),
        Etc2R8G8B8A1 => ci(
            KHR_DF_MODEL_ETC2,
            2,
            KHR_DF_CHANNEL_ETC2_COLOR,
            KHR_DF_CHANNEL_ETC2_ALPHA,
            0,
            64,
            8,
        ),
        Etc2R8G8B8A8 => ci(
            KHR_DF_MODEL_ETC2,
            2,
            KHR_DF_CHANNEL_ETC2_ALPHA,
            KHR_DF_CHANNEL_ETC2_COLOR,
            64,
            64,
            16,
        ),
        EacR11 => ci(
            KHR_DF_MODEL_ETC2,
            1,
            KHR_DF_CHANNEL_ETC2_RED,
            KHR_DF_CHANNEL_ETC2_RED,
            0,
            64,
            8,
        ),
        EacR11G11 => ci(
            KHR_DF_MODEL_ETC2,
            2,
            KHR_DF_CHANNEL_ETC2_RED,
            KHR_DF_CHANNEL_ETC2_GREEN,
            64,
            64,
            16,
        ),
        Astc => ci(
            KHR_DF_MODEL_ASTC,
            1,
            KHR_DF_CHANNEL_ASTC_DATA,
            KHR_DF_CHANNEL_ASTC_DATA,
            0,
            128,
            16,
        ),
        Etc1s => ci(
            KHR_DF_MODEL_ETC1S,
            1,
            KHR_DF_CHANNEL_ETC1S_RGB,
            KHR_DF_CHANNEL_ETC1S_RGB,
            0,
            64,
            8,
        ),
        Pvrtc => ci(
            KHR_DF_MODEL_PVRTC,
            1,
            KHR_DF_CHANNEL_PVRTC_COLOR,
            KHR_DF_CHANNEL_PVRTC_COLOR,
            0,
            64,
            8,
        ),
        Pvrtc2 => ci(
            KHR_DF_MODEL_PVRTC2,
            1,
            KHR_DF_CHANNEL_PVRTC2_COLOR,
            KHR_DF_CHANNEL_PVRTC2_COLOR,
            0,
            64,
            8,
        ),
    }
}

/// Port of `createDFDCompressed`.
pub fn create_dfd_compressed(
    comp_scheme: VkCompScheme,
    bwidth: u32,
    bheight: u32,
    bdepth: u32,
    suffix: VkSuffix,
) -> Option<Vec<u32>> {
    if bwidth == 0 || bheight == 0 || bdepth == 0 {
        return None;
    }
    let info = comp_info(comp_scheme);
    let num_samples = info.sample_count;
    let words = 1 + KHR_DF_WORD_SAMPLESTART + num_samples * KHR_DF_WORD_SAMPLEWORDS;
    let mut dfd = vec![0u32; words as usize];
    dfd[0] = 4 * words;
    let b = bdb_mut(&mut dfd);
    b[KHR_DF_WORD_VENDORID as usize] = (KHR_DF_VENDORID_KHRONOS << KHR_DF_SHIFT_VENDORID)
        | (KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT << KHR_DF_SHIFT_DESCRIPTORTYPE);
    b[KHR_DF_WORD_VERSIONNUMBER as usize] = (KHR_DF_VERSIONNUMBER_LATEST
        << KHR_DF_SHIFT_VERSIONNUMBER)
        | ((4 * (KHR_DF_WORD_SAMPLESTART + num_samples * KHR_DF_WORD_SAMPLEWORDS))
            << KHR_DF_SHIFT_DESCRIPTORBLOCKSIZE);
    b[KHR_DF_WORD_MODEL as usize] = (info.model << KHR_DF_SHIFT_MODEL)
        | (KHR_DF_PRIMARIES_BT709 << KHR_DF_SHIFT_PRIMARIES)
        | (KHR_DF_FLAG_ALPHA_STRAIGHT << KHR_DF_SHIFT_FLAGS);
    if suffix == VkSuffix::Srgb {
        b[KHR_DF_WORD_TRANSFER as usize] |= KHR_DF_TRANSFER_SRGB << KHR_DF_SHIFT_TRANSFER;
    } else {
        b[KHR_DF_WORD_TRANSFER as usize] |= KHR_DF_TRANSFER_LINEAR << KHR_DF_SHIFT_TRANSFER;
    }
    b[KHR_DF_WORD_TEXELBLOCKDIMENSION0 as usize] = (bwidth - 1)
        | ((bheight - 1) << KHR_DF_SHIFT_TEXELBLOCKDIMENSION1)
        | ((bdepth - 1) << KHR_DF_SHIFT_TEXELBLOCKDIMENSION2);
    b[KHR_DF_WORD_BYTESPLANE0 as usize] = info.bytes;
    b[KHR_DF_WORD_BYTESPLANE4 as usize] = 0;

    let (lower, upper): (u32, u32) = match suffix {
        VkSuffix::Snorm => (!0x7FFF_FFFFu32, 0x7FFF_FFFF),
        VkSuffix::Uscaled | VkSuffix::Uint => (0, 1),
        VkSuffix::Sscaled | VkSuffix::Sint => (!0u32, 1),
        VkSuffix::Sfloat => ((-1.0f32).to_bits(), 1.0f32.to_bits()),
        VkSuffix::Ufloat => (0.0f32.to_bits(), 1.0f32.to_bits()),
        VkSuffix::Unorm | VkSuffix::Srgb | VkSuffix::Sfixed5 => (0, 0xFFFF_FFFF),
    };

    let s0 = KHR_DF_WORD_SAMPLESTART as usize;
    let channel = set_channel_flags(info.first_channel, suffix);
    b[s0] = ((info.channel_bits - 1) << KHR_DF_SAMPLESHIFT_BITLENGTH)
        | (channel << KHR_DF_SAMPLESHIFT_CHANNELID);
    b[s0 + 1] = 0;
    b[s0 + 2] = lower;
    b[s0 + 3] = upper;
    if info.sample_count > 1 {
        let s1 = s0 + KHR_DF_WORD_SAMPLEWORDS as usize;
        let channel = set_channel_flags(info.second_channel, suffix);
        b[s1] = (info.second_channel_offset << KHR_DF_SAMPLESHIFT_BITOFFSET)
            | ((info.channel_bits - 1) << KHR_DF_SAMPLESHIFT_BITLENGTH)
            | (channel << KHR_DF_SAMPLESHIFT_CHANNELID);
        b[s1 + 1] = 0;
        b[s1 + 2] = lower;
        b[s1 + 3] = upper;
    }
    Some(dfd)
}

/// Port of `createDFDDepthStencil`. Little-endian is assumed.
pub fn create_dfd_depth_stencil(
    depth_bits: u32,
    stencil_bits: u32,
    size_bytes: u32,
) -> Option<Vec<u32>> {
    let num_samples = u32::from(depth_bits > 0) + u32::from(stencil_bits > 0);
    let mut dfd = write_header(
        num_samples,
        size_bytes,
        VkSuffix::Unorm,
        ChannelsInfoType::NonColor,
    );
    // Special case D24_UNORM_S8_UINT: stencil is in the LSBs.
    if depth_bits == 24 && stencil_bits == 8 {
        write_sample(
            &mut dfd,
            0,
            KHR_DF_CHANNEL_RGBSDA_STENCIL,
            8,
            0,
            true,
            true,
            VkSuffix::Uint,
        );
        write_sample(
            &mut dfd,
            1,
            KHR_DF_CHANNEL_RGBSDA_DEPTH,
            24,
            8,
            true,
            true,
            VkSuffix::Unorm,
        );
        return Some(dfd);
    }
    if depth_bits == 32 {
        write_sample(
            &mut dfd,
            0,
            KHR_DF_CHANNEL_RGBSDA_DEPTH,
            32,
            0,
            true,
            true,
            VkSuffix::Sfloat,
        );
    } else if depth_bits > 0 {
        write_sample(
            &mut dfd,
            0,
            KHR_DF_CHANNEL_RGBSDA_DEPTH,
            depth_bits,
            0,
            true,
            true,
            VkSuffix::Unorm,
        );
    }
    if stencil_bits > 0 {
        if depth_bits > 0 {
            write_sample(
                &mut dfd,
                1,
                KHR_DF_CHANNEL_RGBSDA_STENCIL,
                stencil_bits,
                depth_bits,
                true,
                true,
                VkSuffix::Uint,
            );
        } else {
            write_sample(
                &mut dfd,
                0,
                KHR_DF_CHANNEL_RGBSDA_STENCIL,
                stencil_bits,
                0,
                true,
                true,
                VkSuffix::Uint,
            );
        }
    }
    Some(dfd)
}

/// Port of `createDFDAlpha`.
pub fn create_dfd_alpha(big_endian: bool, bytes: u32, suffix: VkSuffix) -> Option<Vec<u32>> {
    if bytes == 0 || bytes > 8 {
        return None;
    }
    let channel = 3; // alpha channel
    let mut dfd;
    if big_endian {
        dfd = write_header(bytes, bytes, suffix, ChannelsInfoType::Color);
        for channel_byte in 0..bytes {
            write_sample(
                &mut dfd,
                channel_byte,
                channel,
                8,
                8 * (bytes - channel_byte - 1),
                channel_byte == bytes - 1,
                channel_byte == 0,
                suffix,
            );
        }
    } else {
        dfd = write_header(1, bytes, suffix, ChannelsInfoType::Color);
        write_sample(&mut dfd, 0, channel, 8 * bytes, 0, true, true, suffix);
    }
    Some(dfd)
}
