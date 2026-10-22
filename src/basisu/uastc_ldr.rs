//! Port of `basisu_lowlevel_uastc_ldr_4x4_transcoder` (basisu_transcoder.cpp).

use super::etc::{Color32, mul_8};
use super::etc1s::TranscodeStatus;
use super::uastc::{transcode_uastc_to_astc, unpack_uastc_pixels};
use super::uastc_bc7::transcode_uastc_to_bc7;
use super::{
    BlockFormat, TranscoderState, TranscoderTextureFormat, basis_block_format_is_uncompressed,
    basis_get_bytes_per_block_or_pixel, basis_validate_output_buffer_size, decode_flags,
};

/// Port of `basisu_lowlevel_uastc_ldr_4x4_transcoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowlevelUastcLdr4x4Transcoder;

impl LowlevelUastcLdr4x4Transcoder {
    pub fn new() -> Self {
        Self
    }

    /// Port of `transcode_slice`.
    #[allow(clippy::too_many_arguments)]
    pub fn transcode_slice(
        &self,
        dst: &mut [u8],
        num_blocks_x: u32,
        num_blocks_y: u32,
        image_data: &[u8],
        fmt: BlockFormat,
        output_block_or_pixel_stride_in_bytes: u32,
        _bc1_allow_threecolor_blocks: bool,
        has_alpha: bool,
        orig_width: u32,
        orig_height: u32,
        mut output_row_pitch_in_blocks_or_pixels: u32,
        _state: &mut TranscoderState,
        mut output_rows_in_pixels: u32,
        _channel0: i32,
        _channel1: i32,
        decode_flags_in: u32,
    ) -> TranscodeStatus {
        let total_blocks = num_blocks_x as usize * num_blocks_y as usize;
        if output_row_pitch_in_blocks_or_pixels == 0 {
            output_row_pitch_in_blocks_or_pixels = if basis_block_format_is_uncompressed(fmt) {
                orig_width
            } else if fmt == BlockFormat::Fxt1Rgb {
                orig_width.div_ceil(8)
            } else {
                num_blocks_x
            };
        }
        if basis_block_format_is_uncompressed(fmt) && output_rows_in_pixels == 0 {
            output_rows_in_pixels = orig_height;
        }
        if image_data.len() < 16 * total_blocks {
            return TranscodeStatus::Failed;
        }
        let _high_quality = decode_flags_in & decode_flags::HIGH_QUALITY != 0;
        let _from_alpha = has_alpha
            && decode_flags_in & decode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS != 0;

        let pitch = output_row_pitch_in_blocks_or_pixels as usize;
        let stride = output_block_or_pixel_stride_in_bytes as usize;

        let mut blocks = image_data.as_chunks::<16>().0.iter();
        for block_y in 0..num_blocks_y as usize {
            for block_x in 0..num_blocks_x as usize {
                let Some(src) = blocks.next() else {
                    return TranscodeStatus::Failed;
                };
                let sb = *src;
                let block_ofs = (block_y * pitch + block_x) * stride;

                let status = match fmt {
                    BlockFormat::Bc7 | BlockFormat::Bc7M5Color => {
                        let mut out = [0u8; 16];
                        transcode_uastc_to_bc7(&sb, &mut out) && write(dst, block_ofs, &out)
                    }
                    BlockFormat::AstcLdr4x4 => {
                        let mut out = [0u8; 16];
                        transcode_uastc_to_astc(&sb, &mut out) && write(dst, block_ofs, &out)
                    }
                    BlockFormat::Rgba32
                    | BlockFormat::Rgb565
                    | BlockFormat::Bgr565
                    | BlockFormat::Rgba4444 => {
                        let mut px = [Color32::default(); 16];
                        let ok = unpack_uastc_pixels(&sb, &mut px, false);
                        let bpp = if fmt == BlockFormat::Rgba32 { 4 } else { 2 };
                        let max_x = (pitch as i64 - block_x as i64 * 4).clamp(0, 4) as usize;
                        let max_y = (output_rows_in_pixels as i64 - block_y as i64 * 4).clamp(0, 4)
                            as usize;
                        let base = (block_x * 4 + block_y * 4 * pitch) * bpp;
                        let mut wok = true;
                        for y in 0..max_y {
                            let row = base + y * pitch * bpp;
                            for x in 0..max_x {
                                let c = &px[y * 4 + x];
                                let (r, g, b, a) =
                                    (c.r() as u32, c.g() as u32, c.b() as u32, c.a() as u32);
                                let at = row + x * bpp;
                                wok &= match fmt {
                                    BlockFormat::Rgba32 => write(dst, at, &c.c),
                                    BlockFormat::Rgb565 => {
                                        let p = (mul_8(r, 31) << 11)
                                            | (mul_8(g, 63) << 5)
                                            | mul_8(b, 31);
                                        write(dst, at, &(p as u16).to_le_bytes())
                                    }
                                    BlockFormat::Bgr565 => {
                                        let p = (mul_8(b, 31) << 11)
                                            | (mul_8(g, 63) << 5)
                                            | mul_8(r, 31);
                                        write(dst, at, &(p as u16).to_le_bytes())
                                    }
                                    _ => {
                                        let p = (mul_8(r, 15) << 12)
                                            | (mul_8(g, 15) << 8)
                                            | (mul_8(b, 15) << 4)
                                            | mul_8(a, 15);
                                        write(dst, at, &(p as u16).to_le_bytes())
                                    }
                                };
                            }
                        }
                        ok && wok
                    }
                    _ => return TranscodeStatus::Unsupported,
                };
                if !status {
                    return TranscodeStatus::Failed;
                }
            }
        }
        TranscodeStatus::Ok
    }

    /// Port of `transcode_image`.
    #[allow(clippy::too_many_arguments)]
    pub fn transcode_image(
        &self,
        mut target_format: TranscoderTextureFormat,
        output: &mut [u8],
        output_blocks_buf_size_in_blocks_or_pixels: u32,
        compressed_data: &[u8],
        num_blocks_x: u32,
        num_blocks_y: u32,
        orig_width: u32,
        orig_height: u32,
        _level_index: u32,
        slice_offset: u64,
        slice_length: u32,
        decode_flags_in: u32,
        has_alpha: bool,
        _is_video: bool,
        output_row_pitch_in_blocks_or_pixels: u32,
        state: &mut TranscoderState,
        output_rows_in_pixels: u32,
        channel0: i32,
        channel1: i32,
    ) -> TranscodeStatus {
        use TranscoderTextureFormat as F;
        let Some(src) = usize::try_from(slice_offset)
            .ok()
            .and_then(|s| compressed_data.get(s..s.checked_add(slice_length as usize)?))
        else {
            return TranscodeStatus::Failed;
        };
        if (target_format == F::PVRTC1_4_RGB || target_format == F::PVRTC1_4_RGBA)
            && (!(num_blocks_x * 4).is_power_of_two() || !(num_blocks_y * 4).is_power_of_two())
        {
            return TranscodeStatus::Failed;
        }
        if target_format == F::PVRTC1_4_RGBA && !has_alpha {
            target_format = F::PVRTC1_4_RGB;
        }
        let alpha_to_opaque =
            decode_flags_in & decode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS != 0;
        let bpb = basis_get_bytes_per_block_or_pixel(target_format);
        if !basis_validate_output_buffer_size(
            target_format,
            output_blocks_buf_size_in_blocks_or_pixels,
            orig_width,
            orig_height,
            output_row_pitch_in_blocks_or_pixels,
            output_rows_in_pixels,
        ) {
            return TranscodeStatus::Failed;
        }

        let alpha_chan = if has_alpha && alpha_to_opaque { 3 } else { 0 };
        let (fmt, bc1_3c, c0, c1) = match target_format {
            F::ETC1_RGB => (BlockFormat::Etc1, false, channel0, channel1),
            F::ETC2_RGBA => (BlockFormat::Etc2Rgba, false, channel0, channel1),
            F::BC1_RGB => (BlockFormat::Bc1, true, channel0, channel1),
            F::BC3_RGBA => (BlockFormat::Bc3, false, channel0, channel1),
            F::BC4_R => (BlockFormat::Bc4, false, alpha_chan, -1),
            F::BC5_RG => (BlockFormat::Bc5, false, 0, 3),
            F::BC7_RGBA | F::BC7_ALT => (BlockFormat::Bc7, false, -1, -1),
            F::PVRTC1_4_RGB => (BlockFormat::Pvrtc1_4Rgb, false, -1, -1),
            F::PVRTC1_4_RGBA => (BlockFormat::Pvrtc1_4Rgba, false, -1, -1),
            F::ASTC_LDR_4x4_RGBA => (BlockFormat::AstcLdr4x4, false, -1, -1),
            F::ETC2_EAC_R11 => (BlockFormat::Etc2EacR11, false, alpha_chan, -1),
            F::ETC2_EAC_RG11 => (BlockFormat::Etc2EacRg11, false, 0, 3),
            F::RGBA32 => (BlockFormat::Rgba32, false, -1, -1),
            F::RGB565 => (BlockFormat::Rgb565, false, -1, -1),
            F::BGR565 => (BlockFormat::Bgr565, false, -1, -1),
            F::RGBA4444 => (BlockFormat::Rgba4444, false, -1, -1),
            // ATC, FXT1 and PVRTC2 are unsupported from UASTC LDR 4x4 in C too.
            _ => return TranscodeStatus::Failed,
        };
        self.transcode_slice(
            output,
            num_blocks_x,
            num_blocks_y,
            src,
            fmt,
            bpb,
            bc1_3c,
            has_alpha,
            orig_width,
            orig_height,
            output_row_pitch_in_blocks_or_pixels,
            state,
            output_rows_in_pixels,
            c0,
            c1,
            decode_flags_in,
        )
    }
}

#[inline]
fn write(dst: &mut [u8], at: usize, data: &[u8]) -> bool {
    match dst.get_mut(at..at + data.len()) {
        Some(d) => {
            d.copy_from_slice(data);
            true
        }
        None => false,
    }
}
