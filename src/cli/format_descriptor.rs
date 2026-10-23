//! Port of tools/ktx/format_descriptor.h (`createFormatDescriptor`) and the
//! parts of tools/imageio/formatdesc.h (`FormatDescriptor`) used by the
//! commands, plus imageio_utility.h bit helpers.

use super::command::*;
use super::formats::to_string_vk_format;
use crate::VkFormat;
use crate::dfd::khr_df::*;
use crate::dfd::{
    BITLENGTH, BITOFFSET, BYTESPLANE0, BYTESPLANE1, BYTESPLANE2, BYTESPLANE3, BYTESPLANE4,
    BYTESPLANE5, BYTESPLANE6, BYTESPLANE7, CHANNELID, FLAGS, MODEL, PRIMARIES, QUALIFIERS,
    SAMPLELOWER, SAMPLEPOSITION0, SAMPLEPOSITION1, SAMPLEPOSITION2, SAMPLEPOSITION3, SAMPLEUPPER,
    TEXELBLOCKDIMENSION0, TEXELBLOCKDIMENSION1, TEXELBLOCKDIMENSION2, TEXELBLOCKDIMENSION3,
    TRANSFER, bdb, dfdsamplecount, dfdsval, dfdval, vk2dfd,
};

/// `FormatDescriptor::sample`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sample {
    pub bit_offset: u32,
    /// Bit length minus one, as stored in the DFD.
    pub bit_length: u32,
    pub channel_id: u32,
    pub qualifier_float: bool,
    pub qualifier_signed: bool,
    pub qualifier_exponent: bool,
    pub qualifier_linear: bool,
    pub sample_position: [u32; 4],
    pub lower: u32,
    pub upper: u32,
}

/// `FormatDescriptor` (basic descriptor block + samples).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatDescriptor {
    pub model: u32,
    pub primaries: u32,
    pub transfer: u32,
    pub flags: u32,
    /// `texelBlockDimension0..3` (dimension minus one).
    pub texel_block_dimension: [u32; 4],
    pub bytes_plane: [u32; 8],
    pub samples: Vec<Sample>,
    /// `extended.channelCount`.
    pub channel_count: u32,
}

impl FormatDescriptor {
    /// `createFormatDescriptor(const uint32_t* dfd)`.
    pub fn from_dfd(dfd: &[u32]) -> FormatDescriptor {
        let b = bdb(dfd);
        let samples: Vec<Sample> = (0..dfdsamplecount(b))
            .map(|i| {
                let data_type = dfdsval(b, i, QUALIFIERS);
                Sample {
                    bit_offset: dfdsval(b, i, BITOFFSET),
                    bit_length: dfdsval(b, i, BITLENGTH),
                    channel_id: dfdsval(b, i, CHANNELID),
                    qualifier_float: data_type & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0,
                    qualifier_signed: data_type & KHR_DF_SAMPLE_DATATYPE_SIGNED != 0,
                    qualifier_exponent: data_type & KHR_DF_SAMPLE_DATATYPE_EXPONENT != 0,
                    qualifier_linear: data_type & KHR_DF_SAMPLE_DATATYPE_LINEAR != 0,
                    sample_position: [
                        dfdsval(b, i, SAMPLEPOSITION0),
                        dfdsval(b, i, SAMPLEPOSITION1),
                        dfdsval(b, i, SAMPLEPOSITION2),
                        dfdsval(b, i, SAMPLEPOSITION3),
                    ],
                    lower: dfdsval(b, i, SAMPLELOWER),
                    upper: dfdsval(b, i, SAMPLEUPPER),
                }
            })
            .collect();
        FormatDescriptor {
            model: dfdval(b, MODEL),
            primaries: dfdval(b, PRIMARIES),
            transfer: dfdval(b, TRANSFER),
            flags: dfdval(b, FLAGS),
            texel_block_dimension: [
                dfdval(b, TEXELBLOCKDIMENSION0),
                dfdval(b, TEXELBLOCKDIMENSION1),
                dfdval(b, TEXELBLOCKDIMENSION2),
                dfdval(b, TEXELBLOCKDIMENSION3),
            ],
            bytes_plane: [
                BYTESPLANE0,
                BYTESPLANE1,
                BYTESPLANE2,
                BYTESPLANE3,
                BYTESPLANE4,
                BYTESPLANE5,
                BYTESPLANE6,
                BYTESPLANE7,
            ]
            .map(|f| dfdval(b, f)),
            channel_count: samples.len() as u32,
            samples,
        }
    }

    /// `createFormatDescriptor(VkFormat, Reporter&)`.
    pub fn from_vk_format(vk_format: VkFormat, report: &Reporter) -> CmdResult<FormatDescriptor> {
        match vk2dfd(vk_format) {
            Some(dfd) => Ok(FormatDescriptor::from_dfd(&dfd)),
            None => Err(report.fatal(
                ReturnCode::RuntimeError, // DFD_FAILURE
                &format!(
                    "Failed to create format descriptor for: {}",
                    to_string_vk_format(vk_format)
                ),
            )),
        }
    }

    /// `find`: the first sample of `channel`.
    pub fn find(&self, channel: u32) -> Option<&Sample> {
        self.samples.iter().find(|s| s.channel_id == channel)
    }

    /// `pixelByteCount`.
    pub fn pixel_byte_count(&self) -> u32 {
        self.bytes_plane[0]
    }

    /// `removeLastChannel`.
    pub fn remove_last_channel(&mut self) {
        let num_channels = self.samples.len() as u32;
        self.samples.pop();
        if let Some(per_channel) = self.bytes_plane[0].checked_div(num_channels) {
            self.bytes_plane[0] = per_channel * (num_channels - 1);
        }
        if self.channel_count != 0 {
            self.channel_count -= 1;
        }
    }
}

/// `extract_bits<uint32_t>`: `num_bits` bits at bit `offset` (LSB first).
pub fn extract_bits(data: &[u8], offset: u32, num_bits: u32) -> u32 {
    let mut target = 0u32;
    for i in 0..num_bits.min(32) {
        let bit = offset + i;
        let byte = data.get((bit / 8) as usize).copied().unwrap_or(0);
        if byte & (1 << (bit % 8)) != 0 {
            target |= 1 << i;
        }
    }
    target
}

/// `convertUNORM`: left bit replication upscale, rounding downscale.
pub fn convert_unorm(mut raw_bits: u32, source_bits: u32, target_bits: u32) -> u32 {
    let mask = |bits: u32| ((1u64 << bits) - 1) as u32;
    raw_bits &= mask(source_bits);
    if target_bits == source_bits {
        raw_bits
    } else if target_bits >= source_bits {
        // Upscale with "left bit replication" to fill in the least significant bits
        let mut result: u64 = 0;
        let mut i = 0;
        while i < target_bits {
            result |= (u64::from(raw_bits) << (target_bits - i)) >> source_bits;
            i += source_bits;
        }
        result as u32
    } else {
        // Downscale with rounding: Check the most significant bit that was dropped: 1 -> up, 0 -> down
        let ms_dropped_bit_index = source_bits - target_bits - 1;
        if raw_bits & (1 << ms_dropped_bit_index) != 0 {
            // Min stops the 'overflow' if every targetBit is saturated and we would round up
            ((raw_bits >> (source_bits - target_bits)) + 1).min(mask(target_bits))
        } else {
            raw_bits >> (source_bits - target_bits)
        }
    }
}

/// imageio `half_to_float` (same algorithm as tinyexr's).
pub fn half_to_float(value: u16) -> f32 {
    crate::tinyexr::half_to_float(value)
}

/// glm `packed11bitToFloat`; NaN / Inf encodings give `~0` converted to float (-1).
pub fn packed11bit_to_float(x: u32) -> f32 {
    if x == 0 {
        0.0
    } else if x == (1 << 11) - 1 || x == 0x1f << 6 {
        -1.0
    } else {
        f32::from_bits(
            ((((x & 0x07c0) << 17).wrapping_add(0x3800_0000)) & 0x7f80_0000) | ((x & 0x003f) << 17),
        )
    }
}

/// glm `packed10bitToFloat`; NaN / Inf encodings give `~0` converted to float (-1).
pub fn packed10bit_to_float(x: u32) -> f32 {
    if x == 0 {
        0.0
    } else if x == (1 << 10) - 1 || x == 0x1f << 5 {
        -1.0
    } else {
        f32::from_bits(
            ((((x & 0x03e0) << 18).wrapping_add(0x3800_0000)) & 0x7f80_0000) | ((x & 0x001f) << 18),
        )
    }
}

/// glm `unpackF3x9_E1x5`: (r, g, b).
pub fn unpack_f3x9_e1x5(v: u32) -> [f32; 3] {
    let scale = 2.0f32.powi((v >> 27) as i32 - 15 - 9);
    [v & 0x1ff, (v >> 9) & 0x1ff, (v >> 18) & 0x1ff].map(|c| c as f32 * scale)
}

/// `convertSFloatToFloat` (16 or 32 bits).
pub fn convert_sfloat_to_float(raw_bits: u32, num_bits: u32, scale: f32, offset: f32) -> f32 {
    match num_bits {
        16 => half_to_float(raw_bits as u16) * scale + offset,
        32 => f32::from_bits(raw_bits),
        _ => 0.0,
    }
}

/// `convertUFloatToFloat` (10 or 11 bits).
pub fn convert_ufloat_to_float(raw_bits: u32, num_bits: u32, scale: f32, offset: f32) -> f32 {
    let base = match num_bits {
        10 => packed10bit_to_float(raw_bits),
        11 => packed11bit_to_float(raw_bits),
        _ => 0.0,
    };
    base * scale + offset
}

/// `convertSIntToFloat` (sign and magnitude, like the C++).
pub fn convert_sint_to_float(raw_bits: u32, num_bits: u32) -> f32 {
    let sign_mask = 1u32 << (num_bits - 1);
    let value = (raw_bits & !sign_mask) as i32;
    (if raw_bits & sign_mask != 0 {
        value.wrapping_neg()
    } else {
        value
    }) as f32
}

/// `convertUIntToFloat`.
pub fn convert_uint_to_float(raw_bits: u32) -> f32 {
    raw_bits as f32
}

/// `convertSNORMToFloat` (the raw bits are not sign extended, like the C++).
pub fn convert_snorm_to_float(raw_bits: u32, num_bits: u32, scale: f32, offset: f32) -> f32 {
    let upper = ((1u64 << (num_bits - 1)) - 1) as u32 as f32;
    let v = raw_bits as f32 / upper;
    (if v < -1.0 { -1.0 } else { v }) * scale + offset
}

/// `convertUNORMToFloat`.
pub fn convert_unorm_to_float(raw_bits: u32, num_bits: u32, scale: f32, offset: f32) -> f32 {
    let upper = ((1u64 << num_bits) - 1) as u32 as f32;
    (raw_bits as f32 / upper) * scale + offset
}

/// `convertSFloatToUInt`.
pub fn convert_sfloat_to_uint(raw_bits: u32, num_bits: u32) -> u32 {
    match num_bits {
        16 => half_to_float(raw_bits as u16) as u32,
        32 => f32::from_bits(raw_bits) as u32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits() {
        assert_eq!(extract_bits(&[0xab, 0xcd], 4, 8), 0xda);
        assert_eq!(convert_unorm(0xf, 4, 8), 0xff);
        assert_eq!(convert_unorm(0x5, 4, 8), 0x55);
        assert_eq!(convert_unorm(0x1, 1, 8), 0xff);
        assert_eq!(convert_unorm(0x3ff, 10, 16), 0xffff);
        assert_eq!(convert_unorm(0x80, 8, 4), 0x8);
        assert_eq!(convert_unorm(0xff, 8, 4), 0xf);
        assert_eq!(convert_unorm(0x7, 8, 4), 0x0);
        assert_eq!(packed11bit_to_float(0x3c0), 1.0);
        assert_eq!(packed10bit_to_float(0x1e0), 1.0);
        assert_eq!(
            unpack_f3x9_e1x5((16 << 27) | (256 << 18) | (128 << 9) | 256),
            [1.0, 0.5, 1.0]
        );
        assert_eq!(convert_sint_to_float(0x81, 8), -1.0);
    }
}
