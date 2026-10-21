//! Port of lib/src/formatsize.h and `ktxFormatSize_initFromDfd` (texture2.c).

use crate::dfd::khr_df::*;
use crate::dfd::{
    BYTESPLANE0, BYTESPLANE1, CHANNELID, I_COMPRESSED_FORMAT_BIT, I_PACKED_FORMAT_BIT,
    I_UNSUPPORTED_ERROR_BIT, I_YUVSDA_FORMAT_BIT, InterpretedDfdChannel, MODEL, TEXELBLOCKDIMENSION0,
    TEXELBLOCKDIMENSION1, TEXELBLOCKDIMENSION2, VERSIONNUMBER, DESCRIPTORBLOCKSIZE, bdb, dfdsamplecount,
    dfdsizewords, dfdsval, dfdval, interpret_dfd, reconstruct_dfd_bytes_planes_from_samples,
};
use crate::error::{ErrorCode, KtxResult};

pub const KTX_FORMAT_SIZE_PACKED_BIT: u32 = 0x0000_0001;
pub const KTX_FORMAT_SIZE_COMPRESSED_BIT: u32 = 0x0000_0002;
pub const KTX_FORMAT_SIZE_PALETTIZED_BIT: u32 = 0x0000_0004;
pub const KTX_FORMAT_SIZE_DEPTH_BIT: u32 = 0x0000_0008;
pub const KTX_FORMAT_SIZE_STENCIL_BIT: u32 = 0x0000_0010;
pub const KTX_FORMAT_SIZE_YUVSDA_BIT: u32 = 0x0000_0020;

/// Port of `ktxFormatSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormatSize {
    pub flags: u32,
    /// For KTX1.
    pub palette_size_in_bits: u32,
    pub block_size_in_bits: u32,
    /// in texels
    pub block_width: u32,
    pub block_height: u32,
    pub block_depth: u32,
    /// Minimum required number of blocks
    pub min_blocks_x: u32,
    pub min_blocks_y: u32,
}

/// `e5b9g9r9_ufloat_comparator` (little-endian bitfield order), whole BDB words.
fn e5b9g9r9_ufloat_comparator() -> [u32; 30] {
    let shift = |s: u32, v: u32| v << s;
    let ss = |s: u32, v: u32| v << s;
    let s = |ch: u32, len: u32, off: u32| {
        ss(KHR_DF_SAMPLESHIFT_CHANNELID, ch) | ss(KHR_DF_SAMPLESHIFT_BITLENGTH, len) | ss(KHR_DF_SAMPLESHIFT_BITOFFSET, off)
    };
    [
        0,
        shift(KHR_DF_SHIFT_DESCRIPTORBLOCKSIZE, 30 * 4) | shift(KHR_DF_SHIFT_VERSIONNUMBER, 2),
        shift(KHR_DF_SHIFT_FLAGS, KHR_DF_FLAG_ALPHA_STRAIGHT)
            | shift(KHR_DF_SHIFT_TRANSFER, KHR_DF_TRANSFER_LINEAR)
            | shift(KHR_DF_SHIFT_PRIMARIES, KHR_DF_PRIMARIES_BT709)
            | shift(KHR_DF_SHIFT_MODEL, KHR_DF_MODEL_RGBSDA),
        0,
        shift(KHR_DF_SHIFT_BYTESPLANE0, 4),
        0,
        s(KHR_DF_CHANNEL_RGBSDA_RED, 8, 0),
        0,
        0,
        8448,
        s(KHR_DF_CHANNEL_RGBSDA_RED | KHR_DF_SAMPLE_DATATYPE_EXPONENT, 4, 27),
        0,
        15,
        31,
        s(KHR_DF_CHANNEL_RGBSDA_GREEN, 8, 9),
        0,
        0,
        8448,
        s(KHR_DF_CHANNEL_RGBSDA_GREEN | KHR_DF_SAMPLE_DATATYPE_EXPONENT, 4, 27),
        0,
        15,
        31,
        s(KHR_DF_CHANNEL_RGBSDA_BLUE, 8, 18),
        0,
        0,
        8448,
        s(KHR_DF_CHANNEL_RGBSDA_BLUE | KHR_DF_SAMPLE_DATATYPE_EXPONENT, 4, 27),
        0,
        15,
        31,
    ]
}

impl FormatSize {
    /// Port of `ktxFormatSize_initFromDfd`. May rewrite bytesPlane fields of a
    /// DFD that uses the deprecated (zeroed) supercompression convention.
    pub fn init_from_dfd(dfd: &mut [u32]) -> KtxResult<FormatSize> {
        const SIZEOF_DFD_TOTAL_SIZE: u32 = 4;
        const SIZEOF_BDFD_HEADER: u32 = (KHR_DF_WORD_DESCRIPTORBLOCKSIZE + 1) * 4;
        const SIZEOF_BDFD: u32 = 4 * KHR_DF_WORD_SAMPLESTART;
        const SIZEOF_BDFD_SAMPLE: u32 = 4 * KHR_DF_WORD_SAMPLEWORDS;

        let total = *dfd.first().ok_or(ErrorCode::FileDataError)?;
        // dfd[0] must match the buffer we actually have.
        if (total as usize) > dfd.len() * 4 {
            return Err(ErrorCode::FileDataError);
        }
        let first_word = dfd.get(1).copied().unwrap_or(1);
        if total < SIZEOF_DFD_TOTAL_SIZE + SIZEOF_BDFD_HEADER || first_word != 0 {
            // Either descriptorType or vendorId is not 0
            return Err(ErrorCode::FileDataError);
        }

        // Iterate all descriptor blocks; their sizes must sum to totalSize - 4.
        let mut descriptor_size = total - 4;
        let mut word = 1usize;
        while descriptor_size > SIZEOF_BDFD_HEADER {
            let block = dfd.get(word..).unwrap_or(&[]);
            let descriptor_block_size = dfdval(block, DESCRIPTORBLOCKSIZE);
            if descriptor_block_size != 0 && descriptor_block_size <= descriptor_size {
                descriptor_size -= descriptor_block_size;
                word += (descriptor_block_size / 4) as usize;
            } else {
                break;
            }
        }
        if descriptor_size != 0 {
            return Err(ErrorCode::FileDataError);
        }

        if dfdval(bdb(dfd), VERSIONNUMBER) != KHR_DF_VERSIONNUMBER_1_3 {
            return Err(ErrorCode::FileDataError);
        }
        if total < SIZEOF_DFD_TOTAL_SIZE + SIZEOF_BDFD {
            return Err(ErrorCode::FileDataError); // Missing or partial basic DFD block
        }
        if total < SIZEOF_DFD_TOTAL_SIZE + SIZEOF_BDFD + SIZEOF_BDFD_SAMPLE {
            return Err(ErrorCode::FileDataError); // Missing or partial basic DFD sample
        }

        let mut this = FormatSize::default();
        let b = bdb(dfd);
        this.block_width = dfdval(b, TEXELBLOCKDIMENSION0) + 1;
        this.block_height = dfdval(b, TEXELBLOCKDIMENSION1) + 1;
        this.block_depth = dfdval(b, TEXELBLOCKDIMENSION2) + 1;
        if dfdval(b, BYTESPLANE0) == 0 {
            // Deprecated way of indicating a supercompressed texture.
            reconstruct_dfd_bytes_planes_from_samples(dfd);
        }
        let b = bdb(dfd);
        this.block_size_in_bits = dfdval(b, BYTESPLANE0) * 8;
        // Account for ETC1S with possible second slice.
        this.block_size_in_bits += dfdval(b, BYTESPLANE1) * 8;
        this.palette_size_in_bits = 0;
        this.flags = 0;
        this.min_blocks_x = 1;
        this.min_blocks_y = 1;
        let model = dfdval(b, MODEL);
        if model >= KHR_DF_MODEL_DXT1A {
            // A block compressed format. Entire block is a single sample.
            this.flags |= KTX_FORMAT_SIZE_COMPRESSED_BIT;
            if model == KHR_DF_MODEL_ETC1S {
                // Special case the only multi-plane format we handle (as in C).
                this.block_size_in_bits += dfdval(b, BYTESPLANE1) * 8;
            }
            if model == KHR_DF_MODEL_PVRTC {
                this.min_blocks_x = 2;
                this.min_blocks_y = 2;
            }
        } else {
            let sample_count = dfdsamplecount(b);
            // Samples must be present in the buffer before inspecting them.
            if b.len() < dfdsizewords(sample_count) as usize {
                return Err(ErrorCode::FileDataError);
            }
            if dfdsval(b, 0, CHANNELID) == KHR_DF_CHANNEL_RGBSDA_DEPTH {
                if sample_count == 1 {
                    this.flags |= KTX_FORMAT_SIZE_DEPTH_BIT;
                } else if sample_count == 2 {
                    this.flags |= KTX_FORMAT_SIZE_STENCIL_BIT | KTX_FORMAT_SIZE_DEPTH_BIT | KTX_FORMAT_SIZE_PACKED_BIT;
                } else {
                    return Err(ErrorCode::FileDataError);
                }
            } else if dfdsval(b, 0, CHANNELID) == KHR_DF_CHANNEL_RGBSDA_STENCIL {
                this.flags |= KTX_FORMAT_SIZE_STENCIL_BIT;
            } else if sample_count == 6 && {
                let cmp = e5b9g9r9_ufloat_comparator();
                let start = KHR_DF_WORD_TEXELBLOCKDIMENSION0 as usize;
                b.get(start..30) == cmp.get(start..30)
            } {
                // Special case VK_FORMAT_E5B9G9R9_UFLOAT_PACK32.
                this.flags |= KTX_FORMAT_SIZE_PACKED_BIT;
            } else {
                let mut rgba = [InterpretedDfdChannel::default(); 4];
                let mut word_bytes = 0u32;
                let [r, g, bl, a] = &mut rgba;
                let result = interpret_dfd(dfd, r, g, bl, a, &mut word_bytes);
                if result >= I_UNSUPPORTED_ERROR_BIT {
                    return Err(ErrorCode::UnsupportedTextureType);
                }
                if result & I_PACKED_FORMAT_BIT != 0 {
                    this.flags |= KTX_FORMAT_SIZE_PACKED_BIT;
                }
                if result & I_COMPRESSED_FORMAT_BIT != 0 {
                    this.flags |= KTX_FORMAT_SIZE_COMPRESSED_BIT;
                }
                if result & I_YUVSDA_FORMAT_BIT != 0 {
                    this.flags |= KTX_FORMAT_SIZE_YUVSDA_BIT;
                }
            }
        }
        Ok(this)
    }
}
