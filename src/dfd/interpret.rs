//! Port of external/dfdutils/interpretdfd.c.

use super::access::*;
use super::khr_df::*;

/// Port of `enum InterpretDFDResult`: bit flags, or an error value (>= `I_UNSUPPORTED_ERROR_BIT`).
pub type InterpretDfdResult = u32;

pub const I_LITTLE_ENDIAN_FORMAT_BIT: u32 = 0;
pub const I_BIG_ENDIAN_FORMAT_BIT: u32 = 1 << 0;
pub const I_PACKED_FORMAT_BIT: u32 = 1 << 1;
pub const I_SRGB_FORMAT_BIT: u32 = 1 << 2;
pub const I_NORMALIZED_FORMAT_BIT: u32 = 1 << 3;
pub const I_SIGNED_FORMAT_BIT: u32 = 1 << 4;
pub const I_FIXED_FORMAT_BIT: u32 = 1 << 5;
pub const I_FLOAT_FORMAT_BIT: u32 = 1 << 6;
pub const I_COMPRESSED_FORMAT_BIT: u32 = 1 << 7;
pub const I_YUVSDA_FORMAT_BIT: u32 = 1 << 8;
pub const I_UNSUPPORTED_ERROR_BIT: u32 = 1 << 9;
pub const I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS: u32 = I_UNSUPPORTED_ERROR_BIT;
pub const I_UNSUPPORTED_MULTIPLE_SAMPLE_LOCATIONS: u32 = I_UNSUPPORTED_ERROR_BIT + 1;
pub const I_UNSUPPORTED_MULTIPLE_PLANES: u32 = I_UNSUPPORTED_ERROR_BIT + 2;
pub const I_UNSUPPORTED_CHANNEL_TYPES: u32 = I_UNSUPPORTED_ERROR_BIT + 3;
pub const I_UNSUPPORTED_MIXED_CHANNELS: u32 = I_UNSUPPORTED_ERROR_BIT + 4;
pub const I_UNSUPPORTED_BLOCK_DIMENSIONS: u32 = I_UNSUPPORTED_ERROR_BIT + 5;

/// Port of `InterpretedDFDChannel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterpretedDfdChannel {
    /// Offset in bits for packed, bytes for unpacked.
    pub offset: u32,
    /// Size in bits for packed, bytes for unpacked.
    pub size: u32,
}

fn bit_ceil(x: u32) -> u32 {
    let x = x.wrapping_sub(1);
    for i in 0..32u32 {
        if (1u32 << i) > x {
            return 1u32 << i;
        }
    }
    0
}

const R_IDX: usize = 0;
const G_IDX: usize = 1;
const B_IDX: usize = 2;
const A_IDX: usize = 3;

fn channel_index(sample_channel: u32) -> Option<usize> {
    match sample_channel {
        KHR_DF_CHANNEL_RGBSDA_RED => Some(R_IDX),
        KHR_DF_CHANNEL_RGBSDA_GREEN => Some(G_IDX),
        KHR_DF_CHANNEL_RGBSDA_BLUE => Some(B_IDX),
        KHR_DF_CHANNEL_RGBSDA_DEPTH => Some(R_IDX),
        KHR_DF_CHANNEL_RGBSDA_STENCIL => Some(G_IDX),
        KHR_DF_CHANNEL_RGBSDA_ALPHA => Some(A_IDX),
        _ => None,
    }
}

/// Port of `interpretDFD`. `dfd` is the whole descriptor (`dfd[0]` = size).
pub fn interpret_dfd(
    dfd: &[u32],
    r: &mut InterpretedDfdChannel,
    g: &mut InterpretedDfdChannel,
    b: &mut InterpretedDfdChannel,
    a: &mut InterpretedDfdChannel,
    word_bytes: &mut u32,
) -> InterpretDfdResult {
    let mut ch = [InterpretedDfdChannel::default(); 4];
    let result = interpret_inner(dfd, &mut ch, word_bytes);
    *r = ch[R_IDX];
    *g = ch[G_IDX];
    *b = ch[B_IDX];
    *a = ch[A_IDX];
    result
}

fn interpret_inner(dfd: &[u32], ch: &mut [InterpretedDfdChannel; 4], word_bytes: &mut u32) -> InterpretDfdResult {
    let bdfdb = bdb(dfd);
    let num_samples = dfdsamplecount(bdfdb);
    if num_samples == 0 {
        return I_UNSUPPORTED_CHANNEL_TYPES;
    }
    // Reject descriptors whose sample array is not present.
    if bdfdb.len() < dfdsizewords(num_samples) as usize {
        return I_UNSUPPORTED_CHANNEL_TYPES;
    }

    let mut determined_endianness = false;
    let mut result: u32 = 0;
    let mut is_depth_stencil = false;

    // Rule out the multiple planes case: only bytesPlane0 may be non-zero.
    let bp0 = bdfdb.get(KHR_DF_WORD_BYTESPLANE0 as usize).copied().unwrap_or(0);
    let bp4 = bdfdb.get(KHR_DF_WORD_BYTESPLANE4 as usize).copied().unwrap_or(0);
    if (bp0 & !KHR_DF_MASK_BYTESPLANE0) != 0 || bp4 != 0 {
        return I_UNSUPPORTED_MULTIPLE_PLANES;
    }

    // Packed formats have channels that aren't byte-aligned.
    for s in 0..num_samples {
        let offset = dfdsval(bdfdb, s, BITOFFSET);
        let length = dfdsval(bdfdb, s, BITLENGTH) + 1;
        if (offset & 0x7) != 0 || ((offset + length) & 0x7) != 0 {
            result |= I_PACKED_FORMAT_BIT;
            break;
        }
    }

    // Check data types.
    let mut has_signed = false;
    let mut has_float = false;
    let mut has_normalized = false;
    let mut has_fixed = false;
    let model = dfdval(bdfdb, MODEL);

    for i in 0..num_samples {
        let quals = dfdsval(bdfdb, i, QUALIFIERS);
        let is_signed = (quals & KHR_DF_SAMPLE_DATATYPE_SIGNED) != 0;
        let is_float = (quals & KHR_DF_SAMPLE_DATATYPE_FLOAT) != 0;
        let is_fixed;
        let is_normalized;
        if is_float {
            is_normalized = f32::from_bits(dfdsval(bdfdb, i, SAMPLEUPPER)) != 1.0f32;
            is_fixed = false;
        } else {
            let sample_upper = dfdsval(bdfdb, i, SAMPLEUPPER);
            let mut max_val = 1u32.wrapping_shl(dfdsval(bdfdb, i, BITLENGTH));
            if !is_signed {
                max_val = max_val.wrapping_shl(1);
            }
            max_val = max_val.wrapping_sub(1);
            is_fixed = 1 < sample_upper && sample_upper < max_val;
            is_normalized = !is_fixed && sample_upper != 1;
        }
        has_signed |= is_signed;
        has_fixed |= is_fixed;
        has_float |= is_float;
        // Normalizedness of a single bit channel is ambiguous; ignore those.
        if dfdsval(bdfdb, i, BITLENGTH) > 0 {
            has_normalized |= is_normalized;
        }
    }
    if has_signed {
        result |= I_SIGNED_FORMAT_BIT;
    }
    if has_float {
        result |= I_FLOAT_FORMAT_BIT;
    }
    if has_normalized {
        result |= I_NORMALIZED_FORMAT_BIT;
    }
    if has_fixed {
        result |= I_FIXED_FORMAT_BIT;
    }

    // Checks based on color model
    if model == KHR_DF_MODEL_YUVSDA {
        result |= I_NORMALIZED_FORMAT_BIT;
        result |= I_COMPRESSED_FORMAT_BIT;
        result |= I_YUVSDA_FORMAT_BIT;

        for i in 0..num_samples {
            match dfdsval(bdfdb, i, CHANNELID) {
                KHR_DF_CHANNEL_YUVSDA_Y
                | KHR_DF_CHANNEL_YUVSDA_U
                | KHR_DF_CHANNEL_YUVSDA_V
                | KHR_DF_CHANNEL_YUVSDA_A => {}
                KHR_DF_CHANNEL_YUVSDA_DEPTH | KHR_DF_CHANNEL_YUVSDA_STENCIL => is_depth_stencil = true,
                _ => return I_UNSUPPORTED_CHANNEL_TYPES,
            }
        }

        // Determine wordBytes
        let mut largest_sample_size = 0u32;
        for i in 0..num_samples {
            let length = dfdsval(bdfdb, i, BITLENGTH) + 1;
            if largest_sample_size < length {
                largest_sample_size = length;
            }
        }
        let mult = if (result & I_PACKED_FORMAT_BIT) != 0 { 4 } else { 1 };
        *word_bytes = mult * bit_ceil(largest_sample_size) / 8;
    } else if model == KHR_DF_MODEL_RGBSDA {
        if dfdval(bdfdb, TRANSFER) == KHR_DF_TRANSFER_SRGB {
            result |= I_SRGB_FORMAT_BIT;
        }
        // We only support samples at coordinate 0,0,0,0.
        for s in 0..num_samples {
            if dfdsval(bdfdb, s, SAMPLEPOSITION_ALL) != 0 {
                return I_UNSUPPORTED_MULTIPLE_SAMPLE_LOCATIONS;
            }
        }
    }

    if model == KHR_DF_MODEL_RGBSDA || model == KHR_DF_MODEL_YUVSDA {
        // DEPTH and STENCIL tokens are the same for RGBSDA and YUVSDA.
        for s in 0..num_samples {
            let c = dfdsval(bdfdb, s, CHANNELID);
            if c == KHR_DF_CHANNEL_RGBSDA_DEPTH || c == KHR_DF_CHANNEL_RGBSDA_STENCIL {
                is_depth_stencil = true;
            }
        }

        // Check for mixed channels
        if !is_depth_stencil {
            for i in 0..num_samples {
                let quals = dfdsval(bdfdb, i, QUALIFIERS);
                let is_signed = (quals & KHR_DF_SAMPLE_DATATYPE_SIGNED) != 0;
                let is_float = (quals & KHR_DF_SAMPLE_DATATYPE_FLOAT) != 0;
                if is_signed != has_signed || is_float != has_float {
                    return I_UNSUPPORTED_MIXED_CHANNELS;
                }
            }
        }

        if (result & I_PACKED_FORMAT_BIT) != 0 {
            // A packed format.
            let mut current_channel = !0u32;
            let mut current_bit_offset = 0u32;
            let mut current_byte_offset = 0u32;
            let mut current_bit_length = 0u32;
            *word_bytes = bp0 & 0xFF;
            for s in 0..num_samples {
                let sample_bit_offset = dfdsval(bdfdb, s, BITOFFSET);
                let sample_byte_offset = sample_bit_offset >> 3;
                let sample_bit_length = dfdsval(bdfdb, s, BITLENGTH) + 1;
                let sample_channel = dfdsval(bdfdb, s, CHANNELID);
                let mut idx = match channel_index(sample_channel) {
                    Some(i) => i,
                    None => return I_UNSUPPORTED_CHANNEL_TYPES,
                };
                if sample_channel == current_channel {
                    // Continuation of the same channel.
                    if sample_byte_offset == current_byte_offset.wrapping_sub(1)
                        && ((current_bit_offset + current_bit_length) & 7) == 0
                        && (sample_bit_offset & 7) == 0
                    {
                        result |= I_BIG_ENDIAN_FORMAT_BIT;
                        determined_endianness = true;
                    } else {
                        return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                    }
                    current_bit_offset = sample_bit_offset;
                    current_byte_offset = sample_byte_offset;
                    current_bit_length = sample_bit_length;
                    ch[idx].size += sample_bit_length;
                } else {
                    current_channel = sample_channel;
                    current_bit_offset = sample_bit_offset;
                    current_byte_offset = sample_byte_offset;
                    current_bit_length = sample_bit_length;
                    if ch[idx].size != 0 {
                        if model == KHR_DF_MODEL_YUVSDA && sample_channel == KHR_DF_CHANNEL_YUVSDA_Y {
                            if idx == R_IDX {
                                // Another Y channel. Record details in A.
                                if ch[A_IDX].size == 0 {
                                    idx = A_IDX;
                                } else {
                                    return I_UNSUPPORTED_CHANNEL_TYPES;
                                }
                            }
                        } else {
                            return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                        }
                    }
                    ch[idx].offset = sample_bit_offset;
                    ch[idx].size = sample_bit_length;
                }
            }
            let _ = determined_endianness;
            if (result & I_BIG_ENDIAN_FORMAT_BIT) != 0 {
                // Bit offsets are little-endian; byte swap them.
                let offset_mask = word_bytes.wrapping_sub(1) << 3;
                for c in ch.iter_mut() {
                    c.offset ^= offset_mask;
                }
            }
        } else {
            // Not a packed format. Everything is byte-aligned.
            let mut current_channel = !0u32;
            let mut current_byte_offset = 0u32;
            let mut current_byte_length = 0u32;
            for s in 0..num_samples {
                let sample_byte_offset = dfdsval(bdfdb, s, BITOFFSET) >> 3;
                let sample_byte_length = (dfdsval(bdfdb, s, BITLENGTH) + 1) >> 3;
                let sample_channel = dfdsval(bdfdb, s, CHANNELID);
                let mut idx = match channel_index(sample_channel) {
                    Some(i) => i,
                    None => return I_UNSUPPORTED_CHANNEL_TYPES,
                };
                if sample_channel == current_channel {
                    // Either big-endian, or little-endian with a very large channel.
                    if sample_byte_offset == current_byte_offset.wrapping_sub(1) {
                        if determined_endianness && (result & I_BIG_ENDIAN_FORMAT_BIT) == 0 {
                            return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                        }
                        result |= I_BIG_ENDIAN_FORMAT_BIT;
                        determined_endianness = true;
                        ch[idx].offset = sample_byte_offset;
                    } else if sample_byte_offset == current_byte_offset.wrapping_add(current_byte_length) {
                        if determined_endianness && (result & I_BIG_ENDIAN_FORMAT_BIT) != 0 {
                            return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                        }
                        determined_endianness = true;
                    } else {
                        return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                    }
                    current_byte_offset = sample_byte_offset;
                    current_byte_length = sample_byte_length;
                    ch[idx].size += sample_byte_length;
                    *word_bytes = ch[idx].size;
                } else {
                    current_channel = sample_channel;
                    current_byte_offset = sample_byte_offset;
                    current_byte_length = sample_byte_length;
                    if ch[idx].size != 0 {
                        if model == KHR_DF_MODEL_YUVSDA && sample_channel == KHR_DF_CHANNEL_YUVSDA_Y {
                            if idx == R_IDX {
                                if ch[A_IDX].size == 0 {
                                    idx = A_IDX;
                                } else {
                                    return I_UNSUPPORTED_CHANNEL_TYPES;
                                }
                            }
                        } else {
                            return I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS;
                        }
                    }
                    ch[idx].offset = sample_byte_offset;
                    ch[idx].size = sample_byte_length;
                    *word_bytes = sample_byte_length;
                }
            }
        }
    } else {
        return I_UNSUPPORTED_CHANNEL_TYPES;
    }

    if is_depth_stencil {
        // wordBytes is determined by the required alignment of the larger channel.
        let larger_size = ch[R_IDX].size.max(ch[G_IDX].size);
        *word_bytes = bit_ceil(larger_size);
    }
    result
}
