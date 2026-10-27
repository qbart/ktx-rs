// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
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

/// `FormatDescriptor::sample` (bit fields: `bit_offset` 16, `bit_length` 8,
/// `channel_id` 4, sample positions 8 bits).
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

impl Sample {
    /// `sample(chanId, bLength, offset, dataType, tf, m)`: default
    /// sampleLower / sampleUpper (normalized integer or [0|-1, 1] float).
    pub fn new(
        chan_id: u32,
        b_length: u32,
        offset: u32,
        data_type: u32,
        tf: u32,
        m: u32,
    ) -> Sample {
        let mut s = Sample {
            bit_offset: offset & 0xffff,
            bit_length: b_length.wrapping_sub(1) & 0xff,
            channel_id: chan_id & 0xf,
            ..Sample::default()
        };
        if s.channel_id == 3 && m != KHR_DF_MODEL_XYZW {
            // XYZW does not have an alpha channel. *_ALPHA has the same value
            // for all other 4-channel-capable uncompressed models.
            s.channel_id = KHR_DF_CHANNEL_RGBSDA_ALPHA;
        }
        s.qualifier_float = data_type & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0;
        s.qualifier_signed = data_type & KHR_DF_SAMPLE_DATATYPE_SIGNED != 0;
        s.qualifier_exponent = data_type & KHR_DF_SAMPLE_DATATYPE_EXPONENT != 0;
        s.qualifier_linear = data_type & KHR_DF_SAMPLE_DATATYPE_LINEAR != 0;
        if tf > KHR_DF_TRANSFER_LINEAR && s.channel_id == KHR_DF_CHANNEL_RGBSDA_ALPHA {
            s.qualifier_linear = true;
        }
        // The C++ tests the 8 bit field and shifts like x86 (count mod 32).
        let (lower, upper) = if s.qualifier_float {
            if s.qualifier_signed {
                ((-1.0f32).to_bits(), 1.0f32.to_bits())
            } else {
                (0.0f32.to_bits(), 1.0f32.to_bits())
            }
        } else if s.qualifier_signed {
            // signed normalized
            let upper = if s.bit_length > 32 {
                0x7FFF_FFFF
            } else {
                1u32.wrapping_shl(b_length.wrapping_sub(1)).wrapping_sub(1)
            };
            ((!upper).wrapping_add(1), upper)
        } else {
            // unsigned normalized
            let upper = if s.bit_length > 32 {
                0xFFFF_FFFF
            } else {
                1u32.wrapping_shl(b_length).wrapping_sub(1)
            };
            (0, upper)
        };
        s.lower = lower;
        s.upper = upper;
        s
    }

    /// `sample(chanType, bitLength, offset, sampleLower, sampleUpper,
    /// dataType, tf, m)`: custom bounds, an error for float data.
    #[allow(clippy::too_many_arguments)]
    pub fn with_bounds(
        chan_type: u32,
        bit_length: u32,
        offset: u32,
        sample_lower: u32,
        sample_upper: u32,
        data_type: u32,
        tf: u32,
        m: u32,
    ) -> Result<Sample, String> {
        let mut s = Sample::new(chan_type, bit_length, offset, data_type, tf, m);
        if s.qualifier_float {
            return Err("Invalid use of constructor for float data".to_string());
        }
        s.lower = sample_lower;
        s.upper = sample_upper;
        Ok(s)
    }
}

/// `FormatDescriptor`: the basic descriptor block fields (each 8 bits), the
/// `extended` descriptor (`channel_count`, `same_unit_all_channels`,
/// `oe_gamma`, ICC profile) and the samples.
#[derive(Debug, Clone)]
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
    /// `extended.sameUnitAllChannels`.
    pub same_unit_all_channels: bool,
    /// `extended.oeGamma`: encoding power function exponent or -1.
    pub oe_gamma: f32,
    /// `extended.iccProfile.name`.
    pub icc_profile_name: Vec<u8>,
    /// `extended.iccProfile.profile`.
    pub icc_profile: Vec<u8>,
}

impl Default for FormatDescriptor {
    /// Zero samples: format unknown.
    fn default() -> FormatDescriptor {
        FormatDescriptor {
            model: 0,
            primaries: 0,
            transfer: 0,
            flags: 0,
            texel_block_dimension: [0; 4],
            bytes_plane: [0; 8],
            samples: Vec::new(),
            channel_count: 0,
            same_unit_all_channels: false,
            oe_gamma: -1.0,
            icc_profile_name: Vec::new(),
            icc_profile: Vec::new(),
        }
    }
}

impl PartialEq for FormatDescriptor {
    /// `operator==`: basic block, extended descriptor and samples.
    fn eq(&self, rhs: &FormatDescriptor) -> bool {
        self.model == rhs.model
            && self.primaries == rhs.primaries
            && self.transfer == rhs.transfer
            && self.flags == rhs.flags
            && self.texel_block_dimension == rhs.texel_block_dimension
            && self.bytes_plane == rhs.bytes_plane
            && self.channel_count == rhs.channel_count
            && self.same_unit_all_channels == rhs.same_unit_all_channels
            && self.oe_gamma == rhs.oe_gamma
            && self.icc_profile_name == rhs.icc_profile_name
            && self.icc_profile == rhs.icc_profile
            && self.samples == rhs.samples
    }
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
        FormatDescriptor::from_basic_and_samples(FormatDescriptor {
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
            samples,
            ..FormatDescriptor::default()
        })
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

    /// Basic block for unpacked, non-compressed data (`basicDescriptor`).
    fn basic(pixel_byte_count: u32, t: u32, p: u32, m: u32, f: u32) -> FormatDescriptor {
        let mut bytes_plane = [0; 8];
        bytes_plane[0] = pixel_byte_count & 0xff;
        FormatDescriptor {
            model: m & 0xff,
            primaries: p & 0xff,
            transfer: t & 0xff,
            flags: f & 0xff,
            bytes_plane,
            ..FormatDescriptor::default()
        }
    }

    /// `FormatDescriptor(basicDescriptor, samples)`: `channel_count` is the
    /// sample count, `same_unit_all_channels` compares the samples' sizes and
    /// types. `basic` provides the basic block fields.
    pub fn from_basic_and_samples(basic: FormatDescriptor) -> FormatDescriptor {
        let mut fd = basic;
        fd.channel_count = fd.samples.len() as u32;
        fd.same_unit_all_channels = match fd.samples.first() {
            Some(s0) => fd.samples.iter().skip(1).all(|s| {
                s0.bit_length == s.bit_length
                    && s0.qualifier_exponent == s.qualifier_exponent
                    && s0.qualifier_signed == s.qualifier_signed
                    && s0.qualifier_float == s.qualifier_float
            }),
            None => true,
        };
        fd
    }

    /// `FormatDescriptor(channelCount, channelBitLength, dt, t, p, m, f)`:
    /// unpacked data, normalized samples of the same size and type.
    pub fn new(
        channel_count: u32,
        channel_bit_length: u32,
        dt: u32,
        t: u32,
        p: u32,
        m: u32,
        f: u32,
    ) -> FormatDescriptor {
        let mut fd = FormatDescriptor::basic(
            channel_bit_length.wrapping_mul(channel_count) / 8,
            t,
            p,
            m,
            f,
        );
        fd.channel_count = channel_count;
        for s in 0..channel_count {
            fd.samples.push(Sample::new(
                s,
                channel_bit_length,
                s.wrapping_mul(channel_bit_length),
                dt,
                t,
                m,
            ));
        }
        if m == KHR_DF_MODEL_YUVSDA && channel_count == 2 {
            fd.samples[1].channel_id = KHR_DF_CHANNEL_YUVSDA_ALPHA;
        }
        fd.same_unit_all_channels = true;
        fd
    }

    /// `FormatDescriptor(channelCount, channelBitLength, sampleLower,
    /// sampleUpper, dt, t, p, m, f)`: custom sample bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn with_bounds(
        channel_count: u32,
        channel_bit_length: u32,
        sample_lower: u32,
        sample_upper: u32,
        dt: u32,
        t: u32,
        p: u32,
        m: u32,
        f: u32,
    ) -> Result<FormatDescriptor, String> {
        let mut fd = FormatDescriptor::basic(
            channel_bit_length.wrapping_mul(channel_count) / 8,
            t,
            p,
            m,
            f,
        );
        fd.channel_count = channel_count;
        for s in 0..channel_count {
            fd.samples.push(Sample::with_bounds(
                s,
                channel_bit_length,
                s.wrapping_mul(channel_bit_length),
                sample_lower,
                sample_upper,
                dt,
                t,
                m,
            )?);
        }
        if m == KHR_DF_MODEL_YUVSDA && channel_count == 2 {
            fd.samples[1].channel_id = KHR_DF_CHANNEL_YUVSDA_ALPHA;
        }
        fd.same_unit_all_channels = true;
        Ok(fd)
    }

    /// `isUnknown`.
    pub fn is_unknown(&self) -> bool {
        self.samples.is_empty()
    }

    /// `sameUnitAllChannels`.
    pub fn same_unit_all_channels(&self) -> bool {
        self.same_unit_all_channels
    }

    /// `setTransfer`: also flips the alpha samples' linear qualifier when the
    /// transfer function changes between linear and non-linear.
    pub fn set_transfer(&mut self, t: u32) {
        let old_oetf = self.transfer;
        self.transfer = t & 0xff;
        if (old_oetf <= KHR_DF_TRANSFER_LINEAR) != (t <= KHR_DF_TRANSFER_LINEAR) {
            for s in &mut self.samples {
                if s.channel_id == KHR_DF_CHANNEL_RGBSDA_ALPHA {
                    s.qualifier_linear = t > KHR_DF_TRANSFER_LINEAR;
                }
            }
        }
    }

    /// `setPrimaries`.
    pub fn set_primaries(&mut self, p: u32) {
        self.primaries = p & 0xff;
    }

    /// `setModel`.
    pub fn set_model(&mut self, m: u32) {
        self.model = m & 0xff;
    }

    /// `sampleCount`.
    pub fn sample_count(&self) -> u32 {
        self.samples.len() as u32
    }

    /// `sampleBitLength`.
    pub fn sample_bit_length(&self, s: u32) -> u32 {
        self.samples.get(s as usize).map_or(0, |s| s.bit_length) + 1
    }

    /// `sampleUpper`.
    pub fn sample_upper(&self, s: u32) -> u32 {
        self.samples.get(s as usize).map_or(0, |s| s.upper)
    }

    /// `channelBitLength(c)`: total bits of channel `c`.
    pub fn channel_bit_length_of(&self, c: u32) -> Result<u32, String> {
        let bit_length: u32 = self
            .samples
            .iter()
            .filter(|s| s.channel_id == c)
            .map(|s| s.bit_length + 1)
            .sum();
        if bit_length == 0 {
            return Err("No such channel.".to_string());
        }
        Ok(bit_length)
    }

    /// `channelBitLength()`: the bits of every channel; an error for
    /// differing channel sizes.
    pub fn channel_bit_length(&self) -> Result<u32, String> {
        if self.channel_count == 1 {
            return Ok(self.sample_bit_length(0));
        }
        if !self.same_unit_all_channels {
            return Err("Differing size channels. Specify channel to query.".to_string());
        }
        self.channel_bit_length_of(KHR_DF_CHANNEL_RGBSDA_R)
    }

    /// `largestChannelBitLength`.
    pub fn largest_channel_bit_length(&self) -> u32 {
        (0..16)
            .map(|i| {
                self.samples
                    .iter()
                    .filter(|s| s.channel_id == i)
                    .map(|s| s.bit_length + 1)
                    .sum::<u32>()
            })
            .max()
            .unwrap_or(0)
    }

    /// `anyChannelBitLengthNotEqual`.
    pub fn any_channel_bit_length_not_equal(&self, bit_length: u32) -> bool {
        (0..16).any(|i| {
            let channel_bit_length: u32 = self
                .samples
                .iter()
                .filter(|s| s.channel_id == i)
                .map(|s| s.bit_length + 1)
                .sum();
            bit_length != channel_bit_length
        })
    }

    /// `channelUpper`.
    pub fn channel_upper(&self) -> Result<u32, String> {
        if self.channel_count as usize != self.samples.len() {
            return Err("Multiple samples per channel. Call sampleUpper(uint32_t s).".to_string());
        }
        if !self.same_unit_all_channels {
            return Err("Differing size channels. Call sampleUpper(uint32_t s).".to_string());
        }
        Ok(self.sample_upper(KHR_DF_CHANNEL_RGBSDA_R))
    }

    /// `channelCount`.
    pub fn channel_count(&self) -> u32 {
        self.channel_count
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
