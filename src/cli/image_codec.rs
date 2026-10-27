// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/imageio/imagecodec.hpp (`ImageCodec`) and the parts of
//! tools/imageio/imagespan.hpp (`ImageSpan`, `TexelBlockPtr`) used by
//! `ktx compare`. Function pointers become enums; element reads past the end of
//! the data give 0 instead of reading out of bounds.

use super::format_descriptor::{
    half_to_float, packed10bit_to_float, packed11bit_to_float, unpack_f3x9_e1x5,
};
use crate::VkFormat;
use crate::dfd::khr_df::*;
use crate::dfd::{
    BITLENGTH, BITOFFSET, BYTESPLANE0, BYTESPLANE1, MODEL, QUALIFIERS, SAMPLEUPPER,
    TEXELBLOCKDIMENSION0, TEXELBLOCKDIMENSION1, TEXELBLOCKDIMENSION2, TEXELBLOCKDIMENSION3, bdb,
    dfdsamplecount, dfdsval, dfdval,
};

/// `ImageCodec::SampleInfo`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SampleInfo {
    element_index: u32,
    bit_offset: u32,
    bit_length: u32,
}

/// `codec.decodeUINT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeUint {
    D16S8,
    D24,
    D24S8,
    /// `decodeUINT_UINTVec<TYPE, COMPONENTS>` (element bytes, components).
    UintVec(u32, u32),
    /// `decodeUINT_UINTPacked<TYPE>`.
    UintPacked,
}

/// `codec.decodeSINT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeSint {
    /// `decodeSINT_SINTVec<TYPE, COMPONENTS>` (element bytes, components).
    SintVec(u32, u32),
    /// `decodeSINT_SINTPacked<TYPE>`.
    SintPacked,
}

/// `codec.decodeFLOAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeFloat {
    E5B9G9R9,
    B10G11R11,
    Sfixed5Nv(u32),
    D16S8,
    D24,
    D24S8,
    D32S8,
    Fp32Vec(u32),
    Fp16Vec(u32),
    UintVec(u32, u32),
    SintVec(u32, u32),
    UintPacked,
    SintPacked,
}

/// Port of `ImageCodec`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageCodec {
    valid: bool,
    is_block_compressed: bool,
    is_packed: bool,
    is_float: bool,
    is_float_half: bool,
    is_signed: bool,
    is_normalized: bool,
    texel_block_dimensions: [u32; 4],
    packed_element_byte_size: u32,
    packed_element_count: u32,
    texel_block_byte_size: u32,
    channels: u32,
    packed_sample_info: Vec<SampleInfo>,
    decode_uint: Option<DecodeUint>,
    decode_sint: Option<DecodeSint>,
    decode_float: Option<DecodeFloat>,
}

/// Little-endian unsigned element `index` of `size` bytes (0 past the end).
fn element(data: &[u8], size: u32, index: u32) -> u32 {
    let start = index as usize * size as usize;
    match data.get(start..start + size as usize) {
        Some(b) => b
            .iter()
            .rev()
            .fold(0u32, |acc, &x| (acc << 8) | u32::from(x)),
        None => 0,
    }
}

/// Element sign extended from `size` bytes.
fn element_signed(data: &[u8], size: u32, index: u32) -> i32 {
    sign_extend(element(data, size, index), size * 8)
}

/// imageio `sign_extend`.
fn sign_extend(value: u32, num_bits: u32) -> i32 {
    let sign_bit_mask = 1u32.wrapping_shl(num_bits.wrapping_sub(1));
    if value & sign_bit_mask != 0 {
        (value | !(sign_bit_mask.wrapping_sub(1))) as i32
    } else {
        value as i32
    }
}

/// imageio `convertUNORMToFloat(rawBits, numBits)`.
fn convert_unorm_to_float(raw_bits: u32, num_bits: u32) -> f32 {
    raw_bits as f32 / (1u32.wrapping_shl(num_bits).wrapping_sub(1)) as f32
}

/// `(1u << bitLength) - 1u` with x86 shift-count masking.
fn bit_mask(bit_length: u32) -> u32 {
    1u32.wrapping_shl(bit_length).wrapping_sub(1)
}

impl ImageCodec {
    /// Port of `ImageCodec(VkFormat, uint32_t typeSize, const uint32_t* dfd)`.
    pub fn new(vk_format: VkFormat, type_size: u32, dfd: &[u32]) -> ImageCodec {
        let mut c = ImageCodec {
            valid: true,
            ..ImageCodec::default()
        };
        let b = bdb(dfd);
        let model = dfdval(b, MODEL);
        c.texel_block_dimensions = [
            dfdval(b, TEXELBLOCKDIMENSION0) + 1,
            dfdval(b, TEXELBLOCKDIMENSION1) + 1,
            dfdval(b, TEXELBLOCKDIMENSION2) + 1,
            dfdval(b, TEXELBLOCKDIMENSION3) + 1,
        ];

        c.packed_element_byte_size = type_size;
        // Packed element size must always be 1, 2, or 4 currently
        if !matches!(type_size, 1 | 2 | 4) {
            c.valid = false;
            return c;
        }

        // We do not currently support multiple planes
        c.packed_element_count = dfdval(b, BYTESPLANE0) / type_size;
        if dfdval(b, BYTESPLANE1) != 0 || c.packed_element_count == 0 {
            c.valid = false;
            return c;
        }

        c.channels = 0;
        match model {
            KHR_DF_MODEL_RGBSDA
            | KHR_DF_MODEL_YUVSDA
            | KHR_DF_MODEL_YIQSDA
            | KHR_DF_MODEL_LABSDA
            | KHR_DF_MODEL_CMYKA
            | KHR_DF_MODEL_XYZW
            | KHR_DF_MODEL_HSVA_ANG
            | KHR_DF_MODEL_HSLA_ANG
            | KHR_DF_MODEL_HSVA_HEX
            | KHR_DF_MODEL_HSLA_HEX
            | KHR_DF_MODEL_YCGCOA
            | KHR_DF_MODEL_YCCBCCRC
            | KHR_DF_MODEL_ICTCP
            | KHR_DF_MODEL_CIEXYZ
            | KHR_DF_MODEL_CIEXYY => {
                if !c.init_per_channel(vk_format, b) {
                    c.valid = false;
                    return c;
                }
            }
            KHR_DF_MODEL_BC1A | KHR_DF_MODEL_BC2 | KHR_DF_MODEL_BC3 | KHR_DF_MODEL_BC4
            | KHR_DF_MODEL_BC5 | KHR_DF_MODEL_BC6H | KHR_DF_MODEL_BC7 | KHR_DF_MODEL_ETC1
            | KHR_DF_MODEL_ETC2 | KHR_DF_MODEL_ASTC | KHR_DF_MODEL_PVRTC | KHR_DF_MODEL_PVRTC2 => {
                c.is_block_compressed = true;
            }
            KHR_DF_MODEL_UASTC => {
                c.packed_element_byte_size = 1;
                c.packed_element_count = 16;
                c.is_block_compressed = true;
            }
            // ETC1S (as used by BasisLZ) is not supported directly
            _ => {
                c.valid = false;
                return c;
            }
        }

        c.texel_block_byte_size = c.packed_element_byte_size * c.packed_element_count;
        if c.texel_block_byte_size == 0 {
            c.valid = false;
        }
        c
    }

    /// The simple per-channel color models. Returns false where the C++ sets
    /// `flags.valid = false` and returns.
    fn init_per_channel(&mut self, vk_format: VkFormat, b: &[u32]) -> bool {
        match vk_format {
            VkFormat::E5B9G9R9_UFLOAT_PACK32 => {
                self.is_float = true;
                self.channels = 3;
                self.decode_float = Some(DecodeFloat::E5B9G9R9);
                return true;
            }
            VkFormat::B10G11R11_UFLOAT_PACK32 => {
                self.channels = 3;
                self.decode_float = Some(DecodeFloat::B10G11R11);
                return true;
            }
            VkFormat::R16G16_SFIXED5_NV => {
                self.channels = 2;
                self.decode_float = Some(DecodeFloat::Sfixed5Nv(2));
                return true;
            }
            VkFormat::D16_UNORM_S8_UINT => {
                self.is_normalized = true;
                self.channels = 2;
                self.decode_uint = Some(DecodeUint::D16S8);
                self.decode_float = Some(DecodeFloat::D16S8);
                return true;
            }
            VkFormat::X8_D24_UNORM_PACK32 => {
                self.is_normalized = true;
                self.channels = 1;
                self.decode_uint = Some(DecodeUint::D24);
                self.decode_float = Some(DecodeFloat::D24);
                return true;
            }
            VkFormat::D24_UNORM_S8_UINT => {
                self.is_normalized = true;
                self.channels = 2;
                self.decode_uint = Some(DecodeUint::D24S8);
                self.decode_float = Some(DecodeFloat::D24S8);
                return true;
            }
            VkFormat::D32_SFLOAT_S8_UINT => {
                self.channels = 2;
                self.decode_float = Some(DecodeFloat::D32S8);
                return true;
            }
            _ => {}
        }

        // For other formats we only support cases where the number formats match across the samples
        let sample_count = dfdsamplecount(b);
        let first_data_type = dfdsval(b, 0, QUALIFIERS) & !KHR_DF_SAMPLE_DATATYPE_LINEAR;
        let first_bit_length = dfdsval(b, 0, BITLENGTH) + 1;
        let sample_upper = dfdsval(b, 0, SAMPLEUPPER);
        self.is_float = first_data_type & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0;
        self.is_float_half = self.is_float && first_bit_length == 16;
        self.is_signed = first_data_type & KHR_DF_SAMPLE_DATATYPE_SIGNED != 0;
        self.is_normalized = sample_upper != if self.is_float { 1.0f32.to_bits() } else { 1 };

        // Channel count matches sample count for these cases
        self.channels = sample_count;

        if first_data_type & KHR_DF_SAMPLE_DATATYPE_EXPONENT != 0 {
            // No other shared exponent formats are supported
            return false;
        }
        let element_bits = self.packed_element_byte_size * 8;
        for i in 0..sample_count {
            let data_type = dfdsval(b, i, QUALIFIERS) & !KHR_DF_SAMPLE_DATATYPE_LINEAR;
            let bit_length = dfdsval(b, i, BITLENGTH) + 1;
            // If not all elements match the packed element byte size then this is a packed format
            if bit_length != first_bit_length || bit_length != element_bits {
                self.is_packed = true;
            }
            // We do not support mixed component types
            if data_type != first_data_type {
                return false;
            }
            // For floats we also require either 32-bit or 16-bit float
            if self.is_float && bit_length != 16 && bit_length != 32 {
                return false;
            }
        }

        let size = self.packed_element_byte_size;
        if self.is_float_half || self.is_float {
            if !(1..=4).contains(&sample_count) {
                return false;
            }
            self.decode_float = Some(if self.is_float_half {
                DecodeFloat::Fp16Vec(sample_count)
            } else {
                DecodeFloat::Fp32Vec(sample_count)
            });
        } else if self.is_packed {
            // Data is packed so use the more general decoders
            for i in 0..sample_count {
                let bit_offset = dfdsval(b, i, BITOFFSET);
                let bit_length = dfdsval(b, i, BITLENGTH) + 1;
                let info = SampleInfo {
                    element_index: (bit_offset >> 3) / size,
                    bit_offset: bit_offset % (size << 3),
                    bit_length,
                };
                // If any of the samples straddle the packed elements then there's something wrong
                if info.bit_offset + info.bit_length > (size << 3) {
                    return false;
                }
                self.packed_sample_info.push(info);
            }
            if self.is_signed {
                self.decode_sint = Some(DecodeSint::SintPacked);
                if self.is_normalized {
                    self.decode_float = Some(DecodeFloat::SintPacked);
                }
            } else {
                self.decode_uint = Some(DecodeUint::UintPacked);
                if self.is_normalized {
                    self.decode_float = Some(DecodeFloat::UintPacked);
                }
            }
        } else {
            // Data is not packed so we can use the optimized decoders
            if !(1..=4).contains(&sample_count) {
                return false;
            }
            if self.is_signed {
                self.decode_sint = Some(DecodeSint::SintVec(size, sample_count));
                if self.is_normalized {
                    self.decode_float = Some(DecodeFloat::SintVec(size, sample_count));
                }
            } else {
                self.decode_uint = Some(DecodeUint::UintVec(size, sample_count));
                if self.is_normalized {
                    self.decode_float = Some(DecodeFloat::UintVec(size, sample_count));
                }
            }
        }
        true
    }

    /// `operator bool`.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn is_block_compressed(&self) -> bool {
        self.is_block_compressed
    }

    pub fn is_packed(&self) -> bool {
        self.is_packed
    }

    pub fn is_float(&self) -> bool {
        self.is_float
    }

    pub fn is_float_half(&self) -> bool {
        self.is_float_half
    }

    pub fn is_signed(&self) -> bool {
        self.is_signed
    }

    pub fn is_normalized(&self) -> bool {
        self.is_normalized
    }

    pub fn can_decode_uint(&self) -> bool {
        self.decode_uint.is_some()
    }

    pub fn can_decode_sint(&self) -> bool {
        self.decode_sint.is_some()
    }

    pub fn can_decode_float(&self) -> bool {
        self.decode_float.is_some()
    }

    pub fn get_texel_block_dimensions(&self) -> [u32; 4] {
        self.texel_block_dimensions
    }

    pub fn get_packed_element_byte_size(&self) -> u32 {
        self.packed_element_byte_size
    }

    pub fn get_packed_element_count(&self) -> u32 {
        self.packed_element_count
    }

    pub fn get_texel_block_byte_size(&self) -> u32 {
        self.texel_block_byte_size
    }

    pub fn get_channel_count(&self) -> u32 {
        self.channels
    }

    /// Port of `pixelToTexelBlockSize` (0 for a zero block dimension).
    pub fn pixel_to_texel_block_size(&self, pixel_size: [u32; 4]) -> [u32; 4] {
        let d = self.texel_block_dimensions;
        std::array::from_fn(|i| {
            pixel_size[i]
                .wrapping_add(d[i])
                .wrapping_sub(1)
                .checked_div(d[i])
                .unwrap_or(0)
        })
    }

    /// Port of `getPackedElement` (0 when the codec has no element reader).
    pub fn get_packed_element(&self, data: &[u8], index: u32) -> u32 {
        match self.packed_element_byte_size {
            1 | 2 | 4 => element(data, self.packed_element_byte_size, index),
            _ => 0,
        }
    }

    fn packed_raw(&self, data: &[u8], info: &SampleInfo) -> u32 {
        let e = element(data, self.packed_element_byte_size, info.element_index);
        e.wrapping_shr(info.bit_offset) & bit_mask(info.bit_length)
    }

    /// Port of `decodeUINT` (zeros when not decodable).
    pub fn decode_uint(&self, data: &[u8]) -> [u32; 4] {
        let mut r = [0u32; 4];
        match self.decode_uint {
            Some(DecodeUint::D16S8) => {
                r[0] = element(data, 2, 0);
                r[1] = element(data, 2, 1) & 0xFF;
            }
            Some(DecodeUint::D24) => r[0] = element(data, 4, 0) & 0xFF_FFFF,
            Some(DecodeUint::D24S8) => {
                let v = element(data, 4, 0);
                r[0] = v >> 8;
                r[1] = v & 0xFF;
            }
            Some(DecodeUint::UintVec(size, n)) => {
                for i in 0..n.min(4) {
                    r[i as usize] = element(data, size, i);
                }
            }
            Some(DecodeUint::UintPacked) => {
                for (i, info) in self.packed_sample_info.iter().take(4).enumerate() {
                    r[i] = self.packed_raw(data, info);
                }
            }
            None => {}
        }
        r
    }

    /// Port of `decodeSINT` (zeros when not decodable).
    pub fn decode_sint(&self, data: &[u8]) -> [i32; 4] {
        let mut r = [0i32; 4];
        match self.decode_sint {
            Some(DecodeSint::SintVec(size, n)) => {
                for i in 0..n.min(4) {
                    r[i as usize] = element_signed(data, size, i);
                }
            }
            Some(DecodeSint::SintPacked) => {
                for (i, info) in self.packed_sample_info.iter().take(4).enumerate() {
                    r[i] = sign_extend(self.packed_raw(data, info), info.bit_length);
                }
            }
            None => {}
        }
        r
    }

    /// Port of `decodeFLOAT` (`(0, 0, 0, 0)` when not decodable).
    pub fn decode_float(&self, data: &[u8]) -> [f32; 4] {
        let Some(kind) = self.decode_float else {
            return [0.0; 4];
        };
        let mut r = [0.0, 0.0, 0.0, 1.0];
        match kind {
            DecodeFloat::E5B9G9R9 => {
                let [x, y, z] = unpack_f3x9_e1x5(element(data, 4, 0));
                r = [x, y, z, 1.0];
            }
            DecodeFloat::B10G11R11 => {
                let v = element(data, 4, 0);
                r[0] = packed11bit_to_float(v & 0x7FF);
                r[1] = packed11bit_to_float((v >> 11) & 0x7FF);
                r[2] = packed10bit_to_float((v >> 22) & 0x3FF);
                // Need to handle NaN and infinity as special cases, because GLM "swallows" them
                const EXPONENT_SHIFTS: [u32; 3] = [6, 11 + 6, 22 + 5];
                const MANTISSA_SHIFTS: [u32; 3] = [0, 11, 22];
                const MANTISSA_MASKS: [u32; 3] = [0x3F, 0x3F, 0x1F];
                for ch in 0..3 {
                    let exponent = (v >> EXPONENT_SHIFTS[ch]) & 0x1F;
                    let mantissa = (v >> MANTISSA_SHIFTS[ch]) & MANTISSA_MASKS[ch];
                    if exponent == 31 {
                        r[ch] = if mantissa == 0 {
                            f32::INFINITY
                        } else {
                            f32::NAN
                        };
                    }
                }
            }
            DecodeFloat::Sfixed5Nv(n) => {
                for i in 0..n.min(4) {
                    r[i as usize] = element_signed(data, 2, i) as f32 / 32.0;
                }
            }
            DecodeFloat::D16S8 => {
                r = [
                    convert_unorm_to_float(element(data, 2, 0), 16),
                    (element(data, 2, 1) & 0xFF) as f32,
                    0.0,
                    1.0,
                ];
            }
            DecodeFloat::D24 => {
                r = [
                    convert_unorm_to_float(element(data, 4, 0) & 0xFF_FFFF, 24),
                    0.0,
                    0.0,
                    1.0,
                ];
            }
            DecodeFloat::D24S8 => {
                let v = element(data, 4, 0);
                r = [
                    convert_unorm_to_float(v >> 8, 24),
                    (v & 0xFF) as f32,
                    0.0,
                    1.0,
                ];
            }
            DecodeFloat::D32S8 => {
                r = [
                    f32::from_bits(element(data, 4, 0)),
                    (element(data, 4, 1) & 0xFF) as f32,
                    0.0,
                    1.0,
                ];
            }
            DecodeFloat::Fp32Vec(n) => {
                for i in 0..n.min(4) {
                    r[i as usize] = f32::from_bits(element(data, 4, i));
                }
            }
            DecodeFloat::Fp16Vec(n) => {
                for i in 0..n.min(4) {
                    r[i as usize] = half_to_float(element(data, 2, i) as u16);
                }
            }
            DecodeFloat::UintVec(size, n) => {
                // The C++ uses (256u << sizeof(TYPE)) - 1u as the upper value.
                let upper = ((256u32 << size) - 1) as f32;
                for i in 0..n.min(4) {
                    r[i as usize] = element(data, size, i) as f32 / upper;
                }
            }
            DecodeFloat::SintVec(size, n) => {
                // The C++ uses (128u << sizeof(TYPE)) - 1u as the upper value.
                let upper = ((128u32 << size) - 1) as f32;
                for i in 0..n.min(4) {
                    r[i as usize] = (element_signed(data, size, i) as f32 / upper).max(-1.0);
                }
            }
            DecodeFloat::UintPacked => {
                for (i, info) in self.packed_sample_info.iter().take(4).enumerate() {
                    r[i] = self.packed_raw(data, info) as f32 / bit_mask(info.bit_length) as f32;
                }
            }
            DecodeFloat::SintPacked => {
                for (i, info) in self.packed_sample_info.iter().take(4).enumerate() {
                    let upper = bit_mask(info.bit_length.wrapping_sub(1)) as f32;
                    let raw = self.packed_raw(data, info);
                    r[i] = sign_extend(raw, info.bit_length) as f32 / upper;
                }
            }
        }
        r
    }
}

/// Port of the `ImageSpan` geometry: texel block counts of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSpan {
    pub texel_block_width: u32,
    pub texel_block_height: u32,
    pub texel_block_depth: u32,
    pub texel_block_byte_size: u32,
}

impl ImageSpan {
    /// Port of `ImageSpan(width, height, depth, pixels, imageCodec)`.
    pub fn new(width: u32, height: u32, depth: u32, codec: &ImageCodec) -> ImageSpan {
        let d = codec.texel_block_dimensions;
        let blocks = |n: u32, b: u32| {
            n.wrapping_add(b)
                .wrapping_sub(1)
                .checked_div(b)
                .unwrap_or(0)
        };
        ImageSpan {
            texel_block_width: blocks(width, d[0]),
            texel_block_height: blocks(height, d[1]),
            texel_block_depth: blocks(depth, d[2]),
            texel_block_byte_size: codec.texel_block_byte_size,
        }
    }

    /// `texelBlockByteOffset` of `at(blockX, blockY, blockZ)`.
    pub fn texel_block_byte_offset(&self, x: u32, y: u32, z: u32) -> usize {
        let w = self.texel_block_width as usize;
        let h = self.texel_block_height as usize;
        (x as usize + y as usize * w + z as usize * w * h) * self.texel_block_byte_size as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dfd::vk2dfd;

    fn codec(f: VkFormat, type_size: u32) -> ImageCodec {
        ImageCodec::new(f, type_size, &vk2dfd(f).unwrap_or_default())
    }

    #[test]
    fn per_channel() {
        let c = codec(VkFormat::R8G8B8A8_UNORM, 1);
        assert!(c.is_valid() && c.can_decode_uint() && c.can_decode_float());
        assert_eq!(c.get_texel_block_byte_size(), 4);
        assert_eq!(c.decode_uint(&[1, 2, 3, 4]), [1, 2, 3, 4]);
        // The C++ divides by (256 << 1) - 1.
        assert_eq!(c.decode_float(&[255, 0, 0, 0])[0], 255.0 / 511.0);

        let c = codec(VkFormat::R16G16_SINT, 2);
        assert_eq!(c.decode_sint(&[0xFF, 0xFF, 2, 0]), [-1, 2, 0, 0]);

        let c = codec(VkFormat::R5G6B5_UNORM_PACK16, 2);
        assert!(c.is_packed());
        assert_eq!(c.decode_uint(&[0xFF, 0xFF]), [31, 63, 31, 0]);

        let c = codec(VkFormat::A2B10G10R10_SINT_PACK32, 4);
        assert_eq!(c.decode_sint(&[0xFF, 0x03, 0, 0xC0]), [-1, 0, 0, -1]);

        let c = codec(VkFormat::B10G11R11_UFLOAT_PACK32, 4);
        assert!(c.can_decode_float() && !c.can_decode_uint());
        assert_eq!(
            c.decode_float(&0x84a1_2424u32.to_le_bytes()),
            [3.125, 3.125, 3.125, 1.0]
        );
        assert!(c.decode_float(&0x7C0u32.to_le_bytes())[0].is_infinite());
    }

    #[test]
    fn block_compressed() {
        let c = codec(VkFormat::BC1_RGB_UNORM_BLOCK, 1);
        assert!(c.is_valid() && c.is_block_compressed());
        assert_eq!(c.get_packed_element_count(), 8);
        assert_eq!(c.get_texel_block_dimensions(), [4, 4, 1, 1]);
        assert_eq!(c.pixel_to_texel_block_size([5, 4, 1, 1]), [2, 1, 1, 1]);
        assert!(!codec(VkFormat::R64_UINT, 8).is_valid());
    }
}
