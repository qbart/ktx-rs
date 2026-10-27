// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `basisu_lowlevel_uastc_hdr_6x6_intermediate_transcoder`
//! (basisu_transcoder.cpp).

use super::astc_6x6_hdr::{FastBc6hParams, decode_6x6_hdr, fast_encode_bc6h};
use super::astc_helpers::{self as ah, DecodePixels, HalfFloat, LogAstcBlock};
use super::etc1s::TranscodeStatus;
use super::uastc_hdr::write_pixels;
use super::{
    BlockFormat, TranscoderState, TranscoderTextureFormat, basis_block_format_is_uncompressed,
    basis_get_bytes_per_block_or_pixel, basis_validate_output_buffer_size,
};
use crate::defs::transcode_flags::HIGH_QUALITY;

/// Port of `basisu_lowlevel_uastc_hdr_6x6_intermediate_transcoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowlevelUastcHdr6x6IntermediateTranscoder;

/// Unpacks and decodes one ASTC 6x6 block to half floats (`cDecodeModeHDR16`).
fn decode_hdr16(blk: &[u8; 16], texels: &mut [[HalfFloat; 4]; 36]) -> bool {
    let mut log_blk = LogAstcBlock::default();
    ah::unpack_block(blk, &mut log_blk, 6, 6)
        && ah::decode_block(&log_blk, &mut DecodePixels::Hdr16(texels), 6, 6)
}

impl LowlevelUastcHdr6x6IntermediateTranscoder {
    pub fn new() -> Self {
        Self
    }

    /// Port of `transcode_slice` (`num_blocks_x/y` are source 6x6 blocks).
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
        decode_flags: u32,
    ) -> TranscodeStatus {
        let Some(decoded) = decode_6x6_hdr(image_data) else {
            return TranscodeStatus::Failed;
        };
        if decoded.width != orig_width
            || decoded.height != orig_height
            || decoded.num_blocks_x != num_blocks_x
            || decoded.num_blocks_y != num_blocks_y
        {
            return TranscodeStatus::Failed;
        }

        let output_block_width = if fmt == BlockFormat::AstcHdr6x6 { 6 } else { 4 };
        let uncompressed = basis_block_format_is_uncompressed(fmt);
        if output_row_pitch_in_blocks_or_pixels == 0 {
            output_row_pitch_in_blocks_or_pixels = if uncompressed {
                orig_width
            } else {
                orig_width.div_ceil(output_block_width)
            };
        }
        if uncompressed && output_rows_in_pixels == 0 {
            output_rows_in_pixels = orig_height;
        }
        let pitch = output_row_pitch_in_blocks_or_pixels;
        let src_blocks = &decoded.blocks;
        let src_block = |x: u32, y: u32| src_blocks.get((x + y * num_blocks_x) as usize);

        if fmt == BlockFormat::Bc6h {
            let num_dst_blocks_x = orig_width.div_ceil(4);
            let num_dst_blocks_y = orig_height.div_ceil(4);
            if pitch < num_dst_blocks_x || output_block_or_pixel_stride_in_bytes != 16 {
                return TranscodeStatus::Failed;
            }
            let params = FastBc6hParams {
                max_2subset_pats_to_try: u32::from(decode_flags & HIGH_QUALITY != 0),
                ..FastBc6hParams::default()
            };
            // [y][x][c]
            let mut unpacked_blocks = [[[0 as HalfFloat; 3]; 12]; 12];
            for src_block_y in (0..num_blocks_y).step_by(2) {
                let num_inner_blocks_y = 2.min(num_blocks_y - src_block_y);
                for src_block_x in (0..num_blocks_x).step_by(2) {
                    let num_inner_blocks_x = 2.min(num_blocks_x - src_block_x);
                    for iy in 0..num_inner_blocks_y {
                        for ix in 0..num_inner_blocks_x {
                            let mut blk_texels = [[0 as HalfFloat; 4]; 36];
                            let Some(s) = src_block(src_block_x + ix, src_block_y + iy) else {
                                return TranscodeStatus::Failed;
                            };
                            if !decode_hdr16(s, &mut blk_texels) {
                                return TranscodeStatus::Failed;
                            }
                            for y in 0..6 {
                                for x in 0..6 {
                                    let t = blk_texels[y * 6 + x];
                                    unpacked_blocks[iy as usize * 6 + y][ix as usize * 6 + x] =
                                        [t[0], t[1], t[2]];
                                }
                            }
                        }
                    }

                    let dst_block_x = (src_block_x * 6) >> 2;
                    let dst_block_y = (src_block_y * 6) >> 2;
                    let num_inner_dst_blocks_x = 3.min(num_dst_blocks_x - dst_block_x);
                    let num_inner_dst_blocks_y = 3.min(num_dst_blocks_y - dst_block_y);
                    for dy in 0..num_inner_dst_blocks_y {
                        for dx in 0..num_inner_dst_blocks_x {
                            let mut src_pixels = [0 as HalfFloat; 48]; // [y][x][c]
                            for y in 0..4 {
                                let spy = (dy * 4 + y).min(num_inner_blocks_y * 6 - 1) as usize;
                                for x in 0..4 {
                                    let spx = (dx * 4 + x).min(num_inner_blocks_x * 6 - 1) as usize;
                                    let o = ((y * 4 + x) * 3) as usize;
                                    src_pixels[o..o + 3]
                                        .copy_from_slice(&unpacked_blocks[spy][spx]);
                                }
                            }
                            let mut out = [0u8; 16];
                            fast_encode_bc6h(&src_pixels, &mut out, &params);
                            let at =
                                ((dst_block_x + dx) + (dst_block_y + dy) * pitch) as usize * 16;
                            let Some(d) = dst.get_mut(at..at + 16) else {
                                return TranscodeStatus::Failed;
                            };
                            d.copy_from_slice(&out);
                        }
                    }
                }
            }
            return TranscodeStatus::Ok;
        }

        let stride = output_block_or_pixel_stride_in_bytes as usize;
        for block_y in 0..num_blocks_y {
            for block_x in 0..num_blocks_x {
                let Some(src) = src_block(block_x, block_y) else {
                    return TranscodeStatus::Failed;
                };
                let status = match fmt {
                    BlockFormat::AstcHdr6x6 => {
                        // Nothing to do, ASTC HDR 6x6 is just ASTC.
                        let at = (block_y as usize * pitch as usize + block_x as usize) * stride;
                        match dst.get_mut(at..at + 16) {
                            Some(d) => {
                                d.copy_from_slice(src);
                                true
                            }
                            None => false,
                        }
                    }
                    BlockFormat::Rgb9e5 => {
                        let mut log_blk = LogAstcBlock::default();
                        let mut texels = [0u32; 36];
                        let mut bytes = [0u8; 36 * 4];
                        ah::unpack_block(src, &mut log_blk, 6, 6)
                            && ah::decode_block(
                                &log_blk,
                                &mut DecodePixels::Rgb9e5(&mut texels),
                                6,
                                6,
                            )
                            && {
                                for (b, t) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(texels) {
                                    *b = t.to_le_bytes();
                                }
                                write_pixels(
                                    dst,
                                    &bytes,
                                    4,
                                    6,
                                    block_x,
                                    block_y,
                                    pitch,
                                    output_rows_in_pixels,
                                )
                            }
                    }
                    BlockFormat::RgbaHalf | BlockFormat::RgbHalf => {
                        let mut texels = [[0 as HalfFloat; 4]; 36];
                        decode_hdr16(src, &mut texels) && {
                            let comps = if fmt == BlockFormat::RgbaHalf { 4 } else { 3 };
                            let mut bytes = [0u8; 36 * 8];
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
                                6,
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
            F::ASTC_HDR_6x6_RGBA => BlockFormat::AstcHdr6x6,
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
