// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `basisu_lowlevel_uastc_hdr_4x4_transcoder` (basisu_transcoder.cpp).

use super::astc_helpers::{self as ah, DecodePixels, LogAstcBlock};
use super::bc6h::astc_hdr_transcode_to_bc6h;
use super::etc1s::TranscodeStatus;
use super::{
    BlockFormat, TranscoderState, TranscoderTextureFormat, basis_block_format_is_uncompressed,
    basis_get_bytes_per_block_or_pixel, basis_validate_output_buffer_size,
};

/// Port of `basisu_lowlevel_uastc_hdr_4x4_transcoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowlevelUastcHdr4x4Transcoder;

/// Copies the decoded texels (`bpp` bytes each, row major `dim`x`dim`) into the
/// pixel output at block (`block_x`, `block_y`), clipped like C's max_x/max_y.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_pixels(
    dst: &mut [u8],
    texels: &[u8],
    bpp: usize,
    dim: u32,
    block_x: u32,
    block_y: u32,
    pitch: u32,
    rows: u32,
) -> bool {
    let d = i64::from(dim);
    let max_x = (pitch as i64 - block_x as i64 * d).min(d);
    let max_y = (rows as i64 - block_y as i64 * d).min(d);
    let dim = dim as usize;
    let base = (block_x as usize * dim + block_y as usize * dim * pitch as usize) * bpp;
    for y in 0..max_y.max(0) as usize {
        let row = base + y * pitch as usize * bpp;
        let len = max_x.max(0) as usize * bpp;
        let (Some(d), Some(s)) = (
            dst.get_mut(row..row + len),
            texels.get(y * dim * bpp..y * dim * bpp + len),
        ) else {
            return false;
        };
        d.copy_from_slice(s);
    }
    true
}

impl LowlevelUastcHdr4x4Transcoder {
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
        _has_alpha: bool,
        orig_width: u32,
        orig_height: u32,
        mut output_row_pitch_in_blocks_or_pixels: u32,
        _state: &mut TranscoderState,
        mut output_rows_in_pixels: u32,
        _channel0: i32,
        _channel1: i32,
        _decode_flags: u32,
    ) -> TranscodeStatus {
        let total_blocks = num_blocks_x as usize * num_blocks_y as usize;
        let uncompressed = basis_block_format_is_uncompressed(fmt);
        if output_row_pitch_in_blocks_or_pixels == 0 {
            output_row_pitch_in_blocks_or_pixels = if uncompressed {
                orig_width
            } else {
                num_blocks_x
            };
        }
        if uncompressed && output_rows_in_pixels == 0 {
            output_rows_in_pixels = orig_height;
        }
        if image_data.len() < 16 * total_blocks {
            return TranscodeStatus::Failed;
        }
        let pitch = output_row_pitch_in_blocks_or_pixels;
        let stride = output_block_or_pixel_stride_in_bytes as usize;

        let mut blocks = image_data.as_chunks::<16>().0.iter();
        for block_y in 0..num_blocks_y {
            for block_x in 0..num_blocks_x {
                let Some(src) = blocks.next() else {
                    return TranscodeStatus::Failed;
                };
                let at = (block_y as usize * pitch as usize + block_x as usize) * stride;
                let status = match fmt {
                    BlockFormat::UastcHdr4x4 | BlockFormat::AstcHdr4x4 => {
                        match dst.get_mut(at..at + 16) {
                            Some(d) => {
                                d.copy_from_slice(src);
                                true
                            }
                            None => false,
                        }
                    }
                    BlockFormat::Bc6h => {
                        let mut out = [0u8; 16];
                        astc_hdr_transcode_to_bc6h(src, &mut out)
                            && match dst.get_mut(at..at + 16) {
                                Some(d) => {
                                    d.copy_from_slice(&out);
                                    true
                                }
                                None => false,
                            }
                    }
                    BlockFormat::Rgb9e5 => {
                        let mut log_blk = LogAstcBlock::default();
                        let mut texels = [0u32; 16];
                        let mut bytes = [0u8; 64];
                        ah::unpack_block(src, &mut log_blk, 4, 4)
                            && ah::decode_block(
                                &log_blk,
                                &mut DecodePixels::Rgb9e5(&mut texels),
                                4,
                                4,
                            )
                            && {
                                for (b, t) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(texels) {
                                    *b = t.to_le_bytes();
                                }
                                true
                            }
                            && write_pixels(
                                dst,
                                &bytes,
                                4,
                                4,
                                block_x,
                                block_y,
                                pitch,
                                output_rows_in_pixels,
                            )
                    }
                    BlockFormat::RgbaHalf | BlockFormat::RgbHalf => {
                        let mut log_blk = LogAstcBlock::default();
                        let mut texels = [[0u16; 4]; 16];
                        if !(ah::unpack_block(src, &mut log_blk, 4, 4)
                            && ah::decode_block(
                                &log_blk,
                                &mut DecodePixels::Hdr16(&mut texels),
                                4,
                                4,
                            ))
                        {
                            false
                        } else {
                            let comps = if fmt == BlockFormat::RgbaHalf { 4 } else { 3 };
                            let mut bytes = [0u8; 16 * 8];
                            for (i, t) in texels.iter().enumerate() {
                                for (c, v) in t.iter().take(comps).enumerate() {
                                    let o = (i * comps + c) * 2;
                                    bytes[o..o + 2].copy_from_slice(&v.to_le_bytes());
                                }
                            }
                            write_pixels(
                                dst,
                                &bytes,
                                comps * 2,
                                4,
                                block_x,
                                block_y,
                                pitch,
                                output_rows_in_pixels,
                            )
                        }
                    }
                    _ => false,
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
        target_format: TranscoderTextureFormat,
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
        decode_flags: u32,
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
        let fmt = match target_format {
            F::ASTC_HDR_4x4_RGBA => BlockFormat::AstcHdr4x4,
            F::BC6H => BlockFormat::Bc6h,
            F::RGB_HALF => BlockFormat::RgbHalf,
            F::RGBA_HALF => BlockFormat::RgbaHalf,
            F::RGB_9E5 => BlockFormat::Rgb9e5,
            _ => return TranscodeStatus::Failed,
        };
        self.transcode_slice(
            output,
            num_blocks_x,
            num_blocks_y,
            src,
            fmt,
            bpb,
            false,
            has_alpha,
            orig_width,
            orig_height,
            output_row_pitch_in_blocks_or_pixels,
            state,
            output_rows_in_pixels,
            channel0,
            channel1,
            decode_flags,
        )
    }
}
