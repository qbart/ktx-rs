//! Port of `basisu_lowlevel_etc1s_transcoder` (basisu_transcoder.cpp).
//!
//! Implemented targets: ETC1, RGBA32, RGB565, BGR565, RGBA4444 (and the internal
//! indices/alpha helpers). Other targets report [`TranscodeStatus::Unsupported`]
//! until their block encoders are ported.

use super::astc::convert_etc1s_to_astc_4x4;
use super::atc::{AtcBlock, convert_etc1s_to_atc};
use super::bc7m5::{
    Bc7Mode5, chroma_filter_bc7_mode5, convert_etc1s_to_bc7_m5_alpha, convert_etc1s_to_bc7_m5_color,
};
use super::bitreader::{ApproxMoveToFront, BitwiseDecoder, HuffmanDecodingTable};
use super::dxt::{Dxt1Block, Dxt5aBlock, convert_etc1s_to_dxt1, convert_etc1s_to_dxt5a};
use super::eac::{EacBlock, convert_etc1s_to_etc2_eac_a8, convert_etc1s_to_etc2_eac_r11};
use super::etc::{Color32, DecoderEtcBlock, Endpoint, Selector, mul_8};
use super::pvrtc;
use super::pvrtc2::{Pvrtc2Block, convert_etc1s_to_pvrtc2_rgb, convert_etc1s_to_pvrtc2_rgba};
use super::{
    BlockFormat, TranscoderState, TranscoderTextureFormat, basis_block_format_is_uncompressed,
    basis_get_bytes_per_block_or_pixel, basis_validate_output_buffer_size, decode_flags,
};

pub const COLOR5_PAL0_PREV_HI: i32 = 9;
pub const COLOR5_PAL1_PREV_HI: i32 = 21;
pub const ENDPOINT_PRED_TOTAL_SYMBOLS: u32 = (4 * 4 * 4 * 4) + 1;
pub const ENDPOINT_PRED_REPEAT_LAST_SYMBOL: u32 = ENDPOINT_PRED_TOTAL_SYMBOLS - 1;
pub const ENDPOINT_PRED_MIN_REPEAT_COUNT: u32 = 3;
pub const ENDPOINT_PRED_COUNT_VLC_BITS: u32 = 4;
pub const NUM_ENDPOINT_PREDS: u32 = 3;
pub const CR_ENDPOINT_PRED_INDEX: u32 = NUM_ENDPOINT_PREDS - 1;
pub const SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH: u32 = 3;
pub const SELECTOR_HISTORY_BUF_RLE_COUNT_BITS: u32 = 6;
pub const SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL: u32 = 1 << SELECTOR_HISTORY_BUF_RLE_COUNT_BITS;

/// Outcome of a transcode call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeStatus {
    Ok,
    /// Corrupt or inconsistent input (C returns false).
    Failed,
    /// Target format not yet ported.
    Unsupported,
}

/// Destination of a slice transcode: a byte buffer plus a byte offset applied to
/// every block/pixel write (C passes `pOutput_blocks + 8` for the second half of
/// 16-byte blocks).
pub struct SliceOutput<'a> {
    pub buf: &'a mut [u8],
    pub offset: usize,
}

impl SliceOutput<'_> {
    #[inline]
    fn write(&mut self, at: usize, data: &[u8]) -> bool {
        let start = self.offset + at;
        match self.buf.get_mut(start..start + data.len()) {
            Some(d) => {
                d.copy_from_slice(data);
                true
            }
            None => false,
        }
    }

    #[inline]
    fn read_u16(&self, at: usize) -> Option<u16> {
        let s = self.offset + at;
        self.buf
            .get(s..s + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
}

/// Port of `basisu_lowlevel_etc1s_transcoder`.
#[derive(Debug, Clone, Default)]
pub struct LowlevelEtc1sTranscoder {
    pub local_endpoints: Vec<Endpoint>,
    pub local_selectors: Vec<Selector>,
    endpoint_pred_model: HuffmanDecodingTable,
    delta_endpoint_model: HuffmanDecodingTable,
    selector_model: HuffmanDecodingTable,
    selector_history_buf_rle_model: HuffmanDecodingTable,
    selector_history_buf_size: u32,
    def_state: Option<Box<TranscoderState>>,
}

impl LowlevelEtc1sTranscoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `decode_palettes`.
    #[allow(clippy::needless_range_loop)]
    pub fn decode_palettes(
        &mut self,
        num_endpoints: u32,
        endpoints_data: &[u8],
        num_selectors: u32,
        selectors_data: &[u8],
    ) -> bool {
        let mut sym_codec = BitwiseDecoder::new(endpoints_data);
        let mut color5_delta_model0 = HuffmanDecodingTable::new();
        let mut color5_delta_model1 = HuffmanDecodingTable::new();
        let mut color5_delta_model2 = HuffmanDecodingTable::new();
        let mut inten_delta_model = HuffmanDecodingTable::new();
        if !sym_codec.read_huffman_table(&mut color5_delta_model0)
            || !sym_codec.read_huffman_table(&mut color5_delta_model1)
            || !sym_codec.read_huffman_table(&mut color5_delta_model2)
            || !sym_codec.read_huffman_table(&mut inten_delta_model)
        {
            return false;
        }
        if !color5_delta_model0.is_valid()
            || !color5_delta_model1.is_valid()
            || !color5_delta_model2.is_valid()
            || !inten_delta_model.is_valid()
        {
            return false;
        }
        let endpoints_are_grayscale = sym_codec.get_bits(1) != 0;

        self.local_endpoints = vec![Endpoint::default(); num_endpoints as usize];
        let mut prev_color5 = [16i32, 16, 16];
        let mut prev_inten = 0u32;
        for e in self.local_endpoints.iter_mut() {
            let inten_delta = sym_codec.decode_huffman(&inten_delta_model);
            e.inten5 = (inten_delta.wrapping_add(prev_inten) & 7) as u8;
            prev_inten = e.inten5 as u32;
            let nc = if endpoints_are_grayscale { 1 } else { 3 };
            for c in 0..nc {
                let delta = if prev_color5[c] <= COLOR5_PAL0_PREV_HI {
                    sym_codec.decode_huffman(&color5_delta_model0)
                } else if prev_color5[c] <= COLOR5_PAL1_PREV_HI {
                    sym_codec.decode_huffman(&color5_delta_model1)
                } else {
                    sym_codec.decode_huffman(&color5_delta_model2)
                } as i32;
                let v = (prev_color5[c] + delta) & 31;
                e.color5.c[c] = v as u8;
                prev_color5[c] = v;
            }
            if endpoints_are_grayscale {
                e.color5.c[1] = e.color5.c[0];
                e.color5.c[2] = e.color5.c[0];
            }
        }

        self.local_selectors = vec![Selector::default(); num_selectors as usize];
        let mut sym_codec = BitwiseDecoder::new(selectors_data);
        let used_global_selector_cb = sym_codec.get_bits(1) == 1;
        if used_global_selector_cb {
            return false; // global selector codebooks are unsupported
        }
        let used_hybrid_selector_cb = sym_codec.get_bits(1) == 1;
        if used_hybrid_selector_cb {
            return false; // hybrid global selector codebooks are unsupported
        }
        let used_raw_encoding = sym_codec.get_bits(1) == 1;
        let set_row = |s: &mut Selector, j: u32, cur_byte: u32| {
            for k in 0..4 {
                s.set_selector(k, j, (cur_byte >> (k * 2)) & 3);
            }
        };
        if used_raw_encoding {
            for s in self.local_selectors.iter_mut() {
                for j in 0..4 {
                    let cur_byte = sym_codec.get_bits(8);
                    set_row(s, j, cur_byte);
                }
                s.init_flags();
            }
        } else {
            let mut delta_selector_pal_model = HuffmanDecodingTable::new();
            if !sym_codec.read_huffman_table(&mut delta_selector_pal_model) {
                return false;
            }
            if num_selectors > 1 && !delta_selector_pal_model.is_valid() {
                return false;
            }
            let mut prev_bytes = [0u8; 4];
            for (i, s) in self.local_selectors.iter_mut().enumerate() {
                if i == 0 {
                    for j in 0..4 {
                        let cur_byte = sym_codec.get_bits(8);
                        prev_bytes[j as usize] = cur_byte as u8;
                        set_row(s, j, cur_byte);
                    }
                    s.init_flags();
                    continue;
                }
                for j in 0..4 {
                    let delta_byte = sym_codec.decode_huffman(&delta_selector_pal_model);
                    let cur_byte = delta_byte ^ prev_bytes[j as usize] as u32;
                    prev_bytes[j as usize] = cur_byte as u8;
                    set_row(s, j, cur_byte);
                }
                s.init_flags();
            }
        }
        true
    }

    /// Port of `decode_tables`.
    pub fn decode_tables(&mut self, table_data: &[u8]) -> bool {
        let mut sym_codec = BitwiseDecoder::new(table_data);
        if !sym_codec.read_huffman_table(&mut self.endpoint_pred_model)
            || self.endpoint_pred_model.code_sizes().is_empty()
        {
            return false;
        }
        if !sym_codec.read_huffman_table(&mut self.delta_endpoint_model)
            || self.delta_endpoint_model.code_sizes().is_empty()
        {
            return false;
        }
        if !sym_codec.read_huffman_table(&mut self.selector_model)
            || self.selector_model.code_sizes().is_empty()
        {
            return false;
        }
        if !sym_codec.read_huffman_table(&mut self.selector_history_buf_rle_model)
            || self.selector_history_buf_rle_model.code_sizes().is_empty()
        {
            return false;
        }
        self.selector_history_buf_size = sym_codec.get_bits(13);
        self.selector_history_buf_size != 0
    }

    /// Port of `transcode_slice`.
    #[allow(clippy::too_many_arguments)]
    pub fn transcode_slice(
        &self,
        dst: &mut SliceOutput<'_>,
        num_blocks_x: u32,
        num_blocks_y: u32,
        image_data: &[u8],
        fmt: BlockFormat,
        output_block_or_pixel_stride_in_bytes: u32,
        bc1_allow_threecolor_blocks: bool,
        is_video: bool,
        is_alpha_slice: bool,
        level_index: u32,
        orig_width: u32,
        orig_height: u32,
        mut output_row_pitch_in_blocks_or_pixels: u32,
        state: &mut TranscoderState,
        transcode_alpha: bool,
        alpha_blocks: Option<&[u8]>,
        mut output_rows_in_pixels: u32,
        decode_flags_in: u32,
    ) -> TranscodeStatus {
        if !Self::block_format_supported(fmt) {
            return TranscodeStatus::Unsupported;
        }
        let total_blocks = num_blocks_x * num_blocks_y;
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
        if is_video && level_index as usize >= TranscoderState::MAX_PREV_FRAME_LEVELS {
            return TranscodeStatus::Failed;
        }

        let endpoints = &self.local_endpoints;
        let selectors = &self.local_selectors;
        if endpoints.is_empty() || selectors.is_empty() {
            return TranscodeStatus::Failed;
        }

        let mut sym_codec = BitwiseDecoder::new(image_data);
        let mut selector_history_buf = ApproxMoveToFront::new(self.selector_history_buf_size);
        let mut cur_selector_rle_count: u32 = 0;

        let mut block = DecoderEtcBlock::default();
        // Flip bit false to be compatible with the Khronos KDFS.
        block.set_flip_bit(false);
        block.set_diff_bit(true);

        if state.block_endpoint_preds[0].len() < num_blocks_x as usize {
            state.block_endpoint_preds[0].resize(num_blocks_x as usize, Default::default());
            state.block_endpoint_preds[1].resize(num_blocks_x as usize, Default::default());
        }
        if is_video {
            let v = &mut state.prev_frame_indices[is_alpha_slice as usize][level_index as usize];
            if v.len() < total_blocks as usize {
                v.resize(total_blocks as usize, 0);
            }
        }

        // PVRTC1 work memory: ETC1 blocks + packed PVRTC endpoints per block.
        let is_pvrtc1 = matches!(fmt, BlockFormat::Pvrtc1_4Rgb | BlockFormat::Pvrtc1_4Rgba);
        let mut pvrtc_etc_blocks: Vec<DecoderEtcBlock> = Vec::new();
        let mut pvrtc_endpoints: Vec<u32> = Vec::new();
        if is_pvrtc1 {
            if pvrtc_etc_blocks
                .try_reserve_exact(total_blocks as usize)
                .is_err()
                || pvrtc_endpoints
                    .try_reserve_exact(total_blocks as usize)
                    .is_err()
            {
                return TranscodeStatus::Failed;
            }
            pvrtc_etc_blocks.resize(total_blocks as usize, DecoderEtcBlock::default());
            pvrtc_endpoints.resize(total_blocks as usize, 0);
        }

        let bc7_chroma_filtering = decode_flags_in & decode_flags::NO_ETC1S_CHROMA_FILTERING == 0
            && (fmt == BlockFormat::Bc7M5Color || fmt == BlockFormat::Bc7);
        let mut decoded_endpoints: Vec<u16> = Vec::new();
        if bc7_chroma_filtering {
            if decoded_endpoints
                .try_reserve_exact(total_blocks as usize)
                .is_err()
            {
                return TranscodeStatus::Failed;
            }
            decoded_endpoints.resize(total_blocks as usize, 0);
        }

        let mut cur_pred_bits: u32 = 0;
        let mut prev_endpoint_pred_sym: u32 = 0;
        let mut endpoint_pred_repeat_count: u32 = 0;
        let mut prev_endpoint_index: u32 = 0;
        let num_selectors = selectors.len() as u32;
        let selector_history_buf_rle_symbol_index = self.selector_history_buf_size + num_selectors;
        let stride = output_block_or_pixel_stride_in_bytes as usize;
        let pitch = output_row_pitch_in_blocks_or_pixels as usize;

        for block_y in 0..num_blocks_y {
            let cur = (block_y & 1) as usize;
            for block_x in 0..num_blocks_x {
                let bx = block_x as usize;
                // Decode endpoint index predictor symbols
                if block_x & 1 == 0 {
                    if block_y & 1 == 0 {
                        if endpoint_pred_repeat_count != 0 {
                            endpoint_pred_repeat_count -= 1;
                            cur_pred_bits = prev_endpoint_pred_sym;
                        } else {
                            cur_pred_bits = sym_codec.decode_huffman(&self.endpoint_pred_model);
                            if cur_pred_bits == ENDPOINT_PRED_REPEAT_LAST_SYMBOL {
                                endpoint_pred_repeat_count = (sym_codec
                                    .decode_vlc(ENDPOINT_PRED_COUNT_VLC_BITS)
                                    + ENDPOINT_PRED_MIN_REPEAT_COUNT)
                                    .wrapping_sub(1);
                                cur_pred_bits = prev_endpoint_pred_sym;
                            } else {
                                prev_endpoint_pred_sym = cur_pred_bits;
                            }
                        }
                        if let Some(p) = state.block_endpoint_preds[cur ^ 1].get_mut(bx) {
                            p.pred_bits = (cur_pred_bits >> 4) as u8;
                        }
                    } else {
                        cur_pred_bits = state.block_endpoint_preds[cur]
                            .get(bx)
                            .map_or(0, |p| p.pred_bits as u32);
                    }
                }

                // Decode endpoint index
                let mut endpoint_index: u32;
                let mut selector_index: u32 = 0;
                let pred = cur_pred_bits & 3;
                cur_pred_bits >>= 2;

                if pred == 0 {
                    // Left
                    if block_x == 0 {
                        return TranscodeStatus::Failed;
                    }
                    endpoint_index = prev_endpoint_index;
                } else if pred == 1 {
                    // Upper
                    if block_y == 0 {
                        return TranscodeStatus::Failed;
                    }
                    endpoint_index = state.block_endpoint_preds[cur ^ 1]
                        .get(bx)
                        .map_or(0, |p| p.endpoint_index as u32);
                } else if pred == 2 {
                    if is_video {
                        let v = &state.prev_frame_indices[is_alpha_slice as usize]
                            [level_index as usize];
                        endpoint_index = v
                            .get((block_x + block_y * num_blocks_x) as usize)
                            .copied()
                            .unwrap_or(0);
                        selector_index = endpoint_index >> 16;
                        endpoint_index &= 0xFFFF;
                    } else {
                        // Upper left
                        if block_x == 0 || block_y == 0 {
                            return TranscodeStatus::Failed;
                        }
                        endpoint_index = state.block_endpoint_preds[cur ^ 1]
                            .get(bx - 1)
                            .map_or(0, |p| p.endpoint_index as u32);
                    }
                } else {
                    // Decode and apply delta
                    let delta_sym = sym_codec.decode_huffman(&self.delta_endpoint_model);
                    endpoint_index = delta_sym.wrapping_add(prev_endpoint_index);
                    if endpoint_index as usize >= endpoints.len() {
                        endpoint_index = endpoint_index.wrapping_sub(endpoints.len() as u32);
                    }
                }

                if let Some(p) = state.block_endpoint_preds[cur].get_mut(bx) {
                    p.endpoint_index = endpoint_index as u16;
                }
                prev_endpoint_index = endpoint_index;

                // Decode selector index
                if !is_video || pred != CR_ENDPOINT_PRED_INDEX {
                    let mut selector_sym: u32;
                    if cur_selector_rle_count > 0 {
                        cur_selector_rle_count -= 1;
                        selector_sym = num_selectors;
                    } else {
                        selector_sym = sym_codec.decode_huffman(&self.selector_model);
                        if selector_sym == selector_history_buf_rle_symbol_index {
                            let run_sym =
                                sym_codec.decode_huffman(&self.selector_history_buf_rle_model);
                            cur_selector_rle_count =
                                if run_sym == SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL - 1 {
                                    sym_codec
                                        .decode_vlc(7)
                                        .wrapping_add(SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH)
                                } else {
                                    run_sym + SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH
                                };
                            if cur_selector_rle_count > total_blocks {
                                return TranscodeStatus::Failed;
                            }
                            selector_sym = num_selectors;
                            cur_selector_rle_count -= 1;
                        }
                    }
                    if selector_sym >= num_selectors {
                        let history_buf_index = (selector_sym - num_selectors) as usize;
                        if history_buf_index >= selector_history_buf.size() {
                            return TranscodeStatus::Failed;
                        }
                        selector_index = selector_history_buf.get(history_buf_index) as u32;
                        if history_buf_index != 0 {
                            selector_history_buf.use_index(history_buf_index);
                        }
                    } else {
                        selector_index = selector_sym;
                        if self.selector_history_buf_size != 0 {
                            selector_history_buf.add(selector_index as i32);
                        }
                    }
                }

                if endpoint_index as usize >= endpoints.len()
                    || selector_index as usize >= selectors.len()
                {
                    return TranscodeStatus::Failed;
                }
                if is_video {
                    let v = &mut state.prev_frame_indices[is_alpha_slice as usize]
                        [level_index as usize];
                    if let Some(slot) = v.get_mut((block_x + block_y * num_blocks_x) as usize) {
                        *slot = endpoint_index | (selector_index << 16);
                    }
                }

                let ep = &endpoints[endpoint_index as usize];
                let sel = &selectors[selector_index as usize];
                let block_ofs = (bx + block_y as usize * pitch) * stride;
                let pix_ofs = |bpp: usize| (bx * 4 + block_y as usize * 4 * pitch) * bpp;
                let max_x = (pitch as i64 - bx as i64 * 4).clamp(0, 4) as usize;
                let max_y =
                    (output_rows_in_pixels as i64 - block_y as i64 * 4).clamp(0, 4) as usize;

                let ok = match fmt {
                    BlockFormat::Etc1 => {
                        block.set_base5_color(DecoderEtcBlock::pack_color5(&ep.color5, false));
                        block.set_inten_table(0, ep.inten5 as u32);
                        block.set_inten_table(1, ep.inten5 as u32);
                        let mut out = [0u8; 8];
                        out[..4].copy_from_slice(&block.bytes[..4]);
                        out[4..].copy_from_slice(&sel.bytes);
                        dst.write(block_ofs, &out)
                    }
                    BlockFormat::Bc1 => {
                        let mut b = Dxt1Block::default();
                        convert_etc1s_to_dxt1(&mut b, ep, sel, bc1_allow_threecolor_blocks);
                        dst.write(block_ofs, &b.bytes)
                    }
                    BlockFormat::Bc4 => {
                        let mut b = Dxt5aBlock::default();
                        convert_etc1s_to_dxt5a(&mut b, ep, sel);
                        dst.write(block_ofs, &b.bytes)
                    }
                    BlockFormat::Etc2EacA8 | BlockFormat::Etc2EacR11 => {
                        let mut b = EacBlock::default();
                        if fmt == BlockFormat::Etc2EacA8 {
                            convert_etc1s_to_etc2_eac_a8(&mut b, ep, sel);
                        } else {
                            convert_etc1s_to_etc2_eac_r11(&mut b, ep, sel);
                        }
                        dst.write(block_ofs, &b.bytes)
                    }
                    BlockFormat::Bc7 | BlockFormat::Bc7M5Color => {
                        if bc7_chroma_filtering
                            && let Some(d) = decoded_endpoints
                                .get_mut((block_x + block_y * num_blocks_x) as usize)
                        {
                            *d = endpoint_index as u16;
                        }
                        let mut b = Bc7Mode5::default();
                        convert_etc1s_to_bc7_m5_color(&mut b, ep, sel);
                        dst.write(block_ofs, &b.to_bytes())
                    }
                    BlockFormat::Bc7M5Alpha => {
                        let start = dst.offset + block_ofs;
                        match dst.buf.get(start..start + 16) {
                            Some(existing) => {
                                let mut b = Bc7Mode5::from_bytes(existing);
                                convert_etc1s_to_bc7_m5_alpha(&mut b, ep, sel);
                                dst.write(block_ofs, &b.to_bytes())
                            }
                            None => false,
                        }
                    }
                    BlockFormat::Pvrtc1_4Rgb | BlockFormat::Pvrtc1_4Rgba => {
                        let bi = (block_x + block_y * num_blocks_x) as usize;
                        block.set_base5_color(DecoderEtcBlock::pack_color5(&ep.color5, false));
                        block.set_inten_table(0, ep.inten5 as u32);
                        block.set_inten_table(1, ep.inten5 as u32);
                        let rgba = fmt == BlockFormat::Pvrtc1_4Rgba;
                        // RGB keeps ETC-packed selectors, RGBA linear ones.
                        block.set_raw_selector_bits(if rgba { sel.selectors } else { sel.bytes });
                        let bounds = DecoderEtcBlock::get_block_colors5_bounds(
                            &ep.color5,
                            ep.inten5 as u32,
                            sel.lo_selector as u32,
                            sel.hi_selector as u32,
                        );
                        let (mut c0, mut c1) = (bounds[0].c, bounds[1].c);
                        let mut temp = 0u32;
                        if rgba {
                            let Some(ai) = alpha_blocks.and_then(|a| a.get(bi * 4..bi * 4 + 4))
                            else {
                                return TranscodeStatus::Failed;
                            };
                            let aei = u16::from_le_bytes([ai[0], ai[1]]) as usize;
                            let asi = u16::from_le_bytes([ai[2], ai[3]]) as usize;
                            let (Some(aep), Some(asel)) = (endpoints.get(aei), selectors.get(asi))
                            else {
                                return TranscodeStatus::Failed;
                            };
                            let ab = DecoderEtcBlock::get_block_colors5_bounds_g(
                                &aep.color5,
                                aep.inten5 as u32,
                                asel.lo_selector as u32,
                                asel.hi_selector as u32,
                            );
                            c0[3] = ab[0] as u8;
                            c1[3] = ab[1] as u8;
                            pvrtc::set_endpoint_floor(&mut temp, 0, c0);
                            pvrtc::set_endpoint_ceil(&mut temp, 1, c1);
                        } else {
                            pvrtc::set_opaque_endpoint_floor(&mut temp, 0, c0);
                            pvrtc::set_opaque_endpoint_ceil(&mut temp, 1, c1);
                        }
                        match (pvrtc_etc_blocks.get_mut(bi), pvrtc_endpoints.get_mut(bi)) {
                            (Some(b), Some(e)) => {
                                *b = block;
                                *e = temp;
                                true
                            }
                            _ => false,
                        }
                    }
                    BlockFormat::Pvrtc2_4Rgb => {
                        let mut b = Pvrtc2Block::default();
                        convert_etc1s_to_pvrtc2_rgb(&mut b, ep, sel);
                        dst.write(block_ofs, &b.to_bytes())
                    }
                    BlockFormat::AtcRgb => {
                        let mut b = AtcBlock::default();
                        convert_etc1s_to_atc(&mut b, ep, sel);
                        dst.write(block_ofs, &b.to_bytes())
                    }
                    BlockFormat::Pvrtc2_4Rgba => {
                        let start = dst.offset + block_ofs;
                        let mut raw = [0u8; 8];
                        match dst.buf.get(start..start + 8) {
                            Some(existing) => raw.copy_from_slice(existing),
                            None => return TranscodeStatus::Failed,
                        }
                        // The alpha pass stored endpoint/selector indices here.
                        let ei = u16::from_le_bytes([raw[0], raw[1]]) as usize;
                        let si = u16::from_le_bytes([raw[2], raw[3]]) as usize;
                        let (Some(aep), Some(asel)) = (endpoints.get(ei), selectors.get(si)) else {
                            return TranscodeStatus::Failed;
                        };
                        let mut b = Pvrtc2Block::from_bytes(&raw);
                        convert_etc1s_to_pvrtc2_rgba(&mut b, ep, sel, aep, asel);
                        dst.write(block_ofs, &b.to_bytes())
                    }
                    BlockFormat::AstcLdr4x4 => {
                        let start = dst.offset + block_ofs;
                        let mut b = [0u8; 16];
                        match dst.buf.get(start..start + 16) {
                            Some(existing) => b.copy_from_slice(existing),
                            None => return TranscodeStatus::Failed,
                        }
                        // The alpha pass stored endpoint/selector indices here.
                        let alpha = if transcode_alpha {
                            let ei = u16::from_le_bytes([b[0], b[1]]) as usize;
                            let si = u16::from_le_bytes([b[2], b[3]]) as usize;
                            match (endpoints.get(ei), selectors.get(si)) {
                                (Some(e), Some(s)) => Some((e, s)),
                                _ => return TranscodeStatus::Failed,
                            }
                        } else {
                            None
                        };
                        convert_etc1s_to_astc_4x4(&mut b, ep, sel, alpha);
                        dst.write(block_ofs, &b)
                    }
                    BlockFormat::Indices => {
                        let mut out = [0u8; 4];
                        out[..2].copy_from_slice(&(endpoint_index as u16).to_le_bytes());
                        out[2..].copy_from_slice(&(selector_index as u16).to_le_bytes());
                        dst.write(block_ofs, &out)
                    }
                    BlockFormat::A32 => {
                        let colors =
                            DecoderEtcBlock::get_block_colors5_g(&ep.color5, ep.inten5 as u32);
                        let mut ok = true;
                        for y in 0..max_y {
                            let s = sel.selectors[y] as u32;
                            let row = pix_ofs(4) + y * pitch * 4;
                            for x in 0..max_x {
                                ok &= dst.write(
                                    row + 3 + 4 * x,
                                    &[colors[((s >> (x * 2)) & 3) as usize] as u8],
                                );
                            }
                        }
                        ok
                    }
                    BlockFormat::Rgb32 | BlockFormat::Rgba32 => {
                        let colors =
                            DecoderEtcBlock::get_block_colors5(&ep.color5, ep.inten5 as u32);
                        let mut ok = true;
                        for y in 0..max_y {
                            let s = sel.selectors[y] as u32;
                            let row = pix_ofs(4) + y * pitch * 4;
                            for x in 0..max_x {
                                let c = &colors[((s >> (x * 2)) & 3) as usize];
                                if fmt == BlockFormat::Rgba32 {
                                    ok &= dst.write(row + 4 * x, &[c.r(), c.g(), c.b(), 255]);
                                } else {
                                    ok &= dst.write(row + 4 * x, &[c.r(), c.g(), c.b()]);
                                }
                            }
                        }
                        ok
                    }
                    BlockFormat::Rgb565
                    | BlockFormat::Bgr565
                    | BlockFormat::Rgba4444ColorOpaque
                    | BlockFormat::Rgba4444Alpha => {
                        let colors =
                            DecoderEtcBlock::get_block_colors5(&ep.color5, ep.inten5 as u32);
                        let packed: [u16; 4] = core::array::from_fn(|i| pack16(fmt, &colors[i]));
                        let mut ok = true;
                        for y in 0..max_y {
                            let s = sel.selectors[y] as u32;
                            let row = pix_ofs(2) + y * pitch * 2;
                            for x in 0..max_x {
                                ok &= dst.write(
                                    row + 2 * x,
                                    &packed[((s >> (x * 2)) & 3) as usize].to_le_bytes(),
                                );
                            }
                        }
                        ok
                    }
                    BlockFormat::Rgba4444Color => {
                        let colors =
                            DecoderEtcBlock::get_block_colors5(&ep.color5, ep.inten5 as u32);
                        let packed: [u16; 4] = core::array::from_fn(|i| pack16(fmt, &colors[i]));
                        let mut ok = true;
                        for y in 0..max_y {
                            let s = sel.selectors[y] as u32;
                            let row = pix_ofs(2) + y * pitch * 2;
                            for x in 0..max_x {
                                let at = row + 2 * x;
                                let cur = dst.read_u16(at).unwrap_or(0);
                                let v = (cur & 0xF) | packed[((s >> (x * 2)) & 3) as usize];
                                ok &= dst.write(at, &v.to_le_bytes());
                            }
                        }
                        ok
                    }
                    _ => return TranscodeStatus::Unsupported,
                };
                if !ok {
                    return TranscodeStatus::Failed;
                }
            }
        }

        if endpoint_pred_repeat_count != 0 {
            return TranscodeStatus::Failed;
        }
        if fmt == BlockFormat::Pvrtc1_4Rgb {
            let off = dst.offset;
            let Some(out) = dst.buf.get_mut(off..) else {
                return TranscodeStatus::Failed;
            };
            if !pvrtc::fixup_pvrtc1_4_modulation_rgb(
                &pvrtc_etc_blocks,
                &pvrtc_endpoints,
                out,
                num_blocks_x,
                num_blocks_y,
            ) {
                return TranscodeStatus::Failed;
            }
        } else if fmt == BlockFormat::Pvrtc1_4Rgba {
            let off = dst.offset;
            let (Some(out), Some(ab)) = (dst.buf.get_mut(off..), alpha_blocks) else {
                return TranscodeStatus::Failed;
            };
            if !pvrtc::fixup_pvrtc1_4_modulation_rgba(
                &pvrtc_etc_blocks,
                &pvrtc_endpoints,
                out,
                num_blocks_x,
                num_blocks_y,
                ab,
                endpoints,
                selectors,
            ) {
                return TranscodeStatus::Failed;
            }
        }
        if bc7_chroma_filtering {
            let off = dst.offset;
            chroma_filter_bc7_mode5(
                &decoded_endpoints,
                dst.buf,
                off,
                num_blocks_x,
                num_blocks_y,
                output_row_pitch_in_blocks_or_pixels,
                endpoints,
            );
        }
        TranscodeStatus::Ok
    }

    fn block_format_supported(fmt: BlockFormat) -> bool {
        use BlockFormat::*;
        matches!(
            fmt,
            Etc1 | Bc1
                | Bc4
                | Pvrtc1_4Rgb
                | Pvrtc1_4Rgba
                | Pvrtc2_4Rgb
                | Pvrtc2_4Rgba
                | AtcRgb
                | AstcLdr4x4
                | Bc7
                | Bc7M5Color
                | Bc7M5Alpha
                | Etc2EacA8
                | Etc2EacR11
                | Indices
                | A32
                | Rgb32
                | Rgba32
                | Rgb565
                | Bgr565
                | Rgba4444Color
                | Rgba4444Alpha
                | Rgba4444ColorOpaque
        )
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
        level_index: u32,
        rgb_offset: u64,
        rgb_length: u32,
        alpha_offset: u64,
        alpha_length: u32,
        decode_flags_in: u32,
        basis_file_has_alpha_slices: bool,
        is_video: bool,
        output_row_pitch_in_blocks_or_pixels: u32,
        state: &mut TranscoderState,
        output_rows_in_pixels: u32,
    ) -> TranscodeStatus {
        use TranscoderTextureFormat as F;
        let slice = |off: u64, len: u32| -> Option<&[u8]> {
            let start = usize::try_from(off).ok()?;
            compressed_data.get(start..start.checked_add(len as usize)?)
        };
        let Some(rgb_data) = slice(rgb_offset, rgb_length) else {
            return TranscodeStatus::Failed;
        };
        let alpha_data: &[u8] = if alpha_length != 0 {
            match slice(alpha_offset, alpha_length) {
                Some(a) => a,
                None => return TranscodeStatus::Failed,
            }
        } else {
            &[]
        };
        if (target_format == F::PVRTC1_4_RGB || target_format == F::PVRTC1_4_RGBA)
            && (!(num_blocks_x * 4).is_power_of_two() || !(num_blocks_y * 4).is_power_of_two())
        {
            return TranscodeStatus::Failed;
        }
        if target_format == F::PVRTC1_4_RGBA && !basis_file_has_alpha_slices {
            target_format = F::PVRTC1_4_RGB;
        }
        let transcode_alpha_data_to_opaque_formats =
            decode_flags_in & decode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS != 0;
        let bytes_per_block_or_pixel = basis_get_bytes_per_block_or_pixel(target_format);
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

        let (data, is_alpha_slice) =
            if basis_file_has_alpha_slices && transcode_alpha_data_to_opaque_formats {
                (alpha_data, true)
            } else {
                (rgb_data, false)
            };

        let run_ta = |out: &mut [u8],
                      offset: usize,
                      src: &[u8],
                      fmt: BlockFormat,
                      stride: u32,
                      alpha: bool,
                      st: &mut TranscoderState,
                      transcode_alpha: bool| {
            let mut so = SliceOutput { buf: out, offset };
            self.transcode_slice(
                &mut so,
                num_blocks_x,
                num_blocks_y,
                src,
                fmt,
                stride,
                fmt == BlockFormat::Bc1 && target_format == F::BC1_RGB,
                is_video,
                alpha,
                level_index,
                orig_width,
                orig_height,
                output_row_pitch_in_blocks_or_pixels,
                st,
                transcode_alpha,
                None,
                output_rows_in_pixels,
                decode_flags_in,
            )
        };
        let run = |out: &mut [u8],
                   offset: usize,
                   src: &[u8],
                   fmt: BlockFormat,
                   stride: u32,
                   alpha: bool,
                   st: &mut TranscoderState| {
            run_ta(out, offset, src, fmt, stride, alpha, st, false)
        };

        match target_format {
            F::ETC1_RGB => run(
                output,
                0,
                data,
                BlockFormat::Etc1,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::BC1_RGB => run(
                output,
                0,
                data,
                BlockFormat::Bc1,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::BC4_R => run(
                output,
                0,
                data,
                BlockFormat::Bc4,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::BC3_RGBA => {
                // First decode the alpha data
                if basis_file_has_alpha_slices {
                    let s = run(output, 0, alpha_data, BlockFormat::Bc4, 16, true, state);
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                } else if !write_opaque_alpha_blocks(
                    num_blocks_x,
                    num_blocks_y,
                    output,
                    0,
                    BlockFormat::Bc4,
                    16,
                    output_row_pitch_in_blocks_or_pixels,
                ) {
                    return TranscodeStatus::Failed;
                }
                // Now decode the color data. 3 color blocks are not allowed in BC3.
                run(output, 8, rgb_data, BlockFormat::Bc1, 16, false, state)
            }
            F::BC5_RG => {
                let s = run(output, 0, rgb_data, BlockFormat::Bc4, 16, false, state);
                if s != TranscodeStatus::Ok {
                    return s;
                }
                if basis_file_has_alpha_slices {
                    run(output, 8, alpha_data, BlockFormat::Bc4, 16, true, state)
                } else if write_opaque_alpha_blocks(
                    num_blocks_x,
                    num_blocks_y,
                    output,
                    8,
                    BlockFormat::Bc4,
                    16,
                    output_row_pitch_in_blocks_or_pixels,
                ) {
                    TranscodeStatus::Ok
                } else {
                    TranscodeStatus::Failed
                }
            }
            F::BC7_RGBA | F::BC7_ALT => {
                // Color slice: opaque mode 5 blocks.
                let s = run(
                    output,
                    0,
                    rgb_data,
                    BlockFormat::Bc7M5Color,
                    bytes_per_block_or_pixel,
                    false,
                    state,
                );
                if s != TranscodeStatus::Ok || !basis_file_has_alpha_slices {
                    return s;
                }
                // Alpha slice updates the mode 5 blocks with alpha.
                run(
                    output,
                    0,
                    alpha_data,
                    BlockFormat::Bc7M5Alpha,
                    bytes_per_block_or_pixel,
                    true,
                    state,
                )
            }
            F::ETC2_RGBA => {
                if basis_file_has_alpha_slices {
                    let s = run(
                        output,
                        0,
                        alpha_data,
                        BlockFormat::Etc2EacA8,
                        16,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                } else if !write_opaque_alpha_blocks(
                    num_blocks_x,
                    num_blocks_y,
                    output,
                    0,
                    BlockFormat::Etc2EacA8,
                    16,
                    output_row_pitch_in_blocks_or_pixels,
                ) {
                    return TranscodeStatus::Failed;
                }
                run(output, 8, rgb_data, BlockFormat::Etc1, 16, false, state)
            }
            F::ETC2_EAC_R11 => run(
                output,
                0,
                data,
                BlockFormat::Etc2EacR11,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::ETC2_EAC_RG11 => {
                if basis_file_has_alpha_slices {
                    let s = run(
                        output,
                        8,
                        alpha_data,
                        BlockFormat::Etc2EacR11,
                        16,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                } else if !write_opaque_alpha_blocks(
                    num_blocks_x,
                    num_blocks_y,
                    output,
                    8,
                    BlockFormat::Etc2EacR11,
                    16,
                    output_row_pitch_in_blocks_or_pixels,
                ) {
                    return TranscodeStatus::Failed;
                }
                run(
                    output,
                    0,
                    rgb_data,
                    BlockFormat::Etc2EacR11,
                    16,
                    false,
                    state,
                )
            }
            F::RGBA32 => {
                if basis_file_has_alpha_slices {
                    let s = run(output, 0, alpha_data, BlockFormat::A32, 4, true, state);
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                }
                let fmt = if basis_file_has_alpha_slices {
                    BlockFormat::Rgb32
                } else {
                    BlockFormat::Rgba32
                };
                run(output, 0, rgb_data, fmt, 4, false, state)
            }
            F::PVRTC1_4_RGB => run(
                output,
                0,
                data,
                BlockFormat::Pvrtc1_4Rgb,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::PVRTC1_4_RGBA => {
                // Alpha endpoint/selector indices go to a temp buffer first.
                let total = (num_blocks_x * num_blocks_y) as usize * 4;
                let mut temp: Vec<u8> = Vec::new();
                if temp.try_reserve_exact(total).is_err() {
                    return TranscodeStatus::Failed;
                }
                temp.resize(total, 0);
                let s = self.transcode_slice(
                    &mut SliceOutput {
                        buf: &mut temp,
                        offset: 0,
                    },
                    num_blocks_x,
                    num_blocks_y,
                    alpha_data,
                    BlockFormat::Indices,
                    4,
                    false,
                    is_video,
                    true,
                    level_index,
                    orig_width,
                    orig_height,
                    num_blocks_x,
                    state,
                    false,
                    None,
                    0,
                    decode_flags_in,
                );
                if s != TranscodeStatus::Ok {
                    return s;
                }
                self.transcode_slice(
                    &mut SliceOutput {
                        buf: output,
                        offset: 0,
                    },
                    num_blocks_x,
                    num_blocks_y,
                    rgb_data,
                    BlockFormat::Pvrtc1_4Rgba,
                    bytes_per_block_or_pixel,
                    false,
                    is_video,
                    false,
                    level_index,
                    orig_width,
                    orig_height,
                    output_row_pitch_in_blocks_or_pixels,
                    state,
                    false,
                    Some(&temp),
                    0,
                    decode_flags_in,
                )
            }
            F::ATC_RGB => run(
                output,
                0,
                data,
                BlockFormat::AtcRgb,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::ATC_RGBA => {
                // First decode the alpha data
                if basis_file_has_alpha_slices {
                    let s = run(
                        output,
                        0,
                        alpha_data,
                        BlockFormat::Bc4,
                        bytes_per_block_or_pixel,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                } else if !write_opaque_alpha_blocks(
                    num_blocks_x,
                    num_blocks_y,
                    output,
                    0,
                    BlockFormat::Bc4,
                    16,
                    output_row_pitch_in_blocks_or_pixels,
                ) {
                    return TranscodeStatus::Failed;
                }
                run(
                    output,
                    8,
                    rgb_data,
                    BlockFormat::AtcRgb,
                    bytes_per_block_or_pixel,
                    false,
                    state,
                )
            }
            F::PVRTC2_4_RGB => run(
                output,
                0,
                data,
                BlockFormat::Pvrtc2_4Rgb,
                bytes_per_block_or_pixel,
                is_alpha_slice,
                state,
            ),
            F::PVRTC2_4_RGBA => {
                if basis_file_has_alpha_slices {
                    // Alpha indices go to the output first (used as a temp buffer).
                    let s = run(
                        output,
                        0,
                        alpha_data,
                        BlockFormat::Indices,
                        bytes_per_block_or_pixel,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                    run_ta(
                        output,
                        0,
                        rgb_data,
                        BlockFormat::Pvrtc2_4Rgba,
                        bytes_per_block_or_pixel,
                        false,
                        state,
                        true,
                    )
                } else {
                    run(
                        output,
                        0,
                        rgb_data,
                        BlockFormat::Pvrtc2_4Rgb,
                        bytes_per_block_or_pixel,
                        false,
                        state,
                    )
                }
            }
            F::ASTC_LDR_4x4_RGBA => {
                if basis_file_has_alpha_slices {
                    // Alpha indices go to the output first (used as a temp buffer).
                    let s = run(
                        output,
                        0,
                        alpha_data,
                        BlockFormat::Indices,
                        bytes_per_block_or_pixel,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                    run_ta(
                        output,
                        0,
                        rgb_data,
                        BlockFormat::AstcLdr4x4,
                        bytes_per_block_or_pixel,
                        false,
                        state,
                        true,
                    )
                } else {
                    run(
                        output,
                        0,
                        rgb_data,
                        BlockFormat::AstcLdr4x4,
                        bytes_per_block_or_pixel,
                        false,
                        state,
                    )
                }
            }
            F::RGB565 | F::BGR565 => {
                let fmt = if target_format == F::RGB565 {
                    BlockFormat::Rgb565
                } else {
                    BlockFormat::Bgr565
                };
                run(output, 0, data, fmt, 2, is_alpha_slice, state)
            }
            F::RGBA4444 => {
                if basis_file_has_alpha_slices {
                    let s = run(
                        output,
                        0,
                        alpha_data,
                        BlockFormat::Rgba4444Alpha,
                        2,
                        true,
                        state,
                    );
                    if s != TranscodeStatus::Ok {
                        return s;
                    }
                }
                let fmt = if basis_file_has_alpha_slices {
                    BlockFormat::Rgba4444Color
                } else {
                    BlockFormat::Rgba4444ColorOpaque
                };
                run(output, 0, rgb_data, fmt, 2, false, state)
            }
            _ => TranscodeStatus::Unsupported,
        }
    }

    /// Default transcoder state (`m_def_state`).
    pub fn default_state(&mut self) -> &mut TranscoderState {
        self.def_state.get_or_insert_with(Default::default)
    }
}

/// Port of `basisu_transcoder::write_opaque_alpha_blocks`.
pub fn write_opaque_alpha_blocks(
    num_blocks_x: u32,
    num_blocks_y: u32,
    output: &mut [u8],
    offset: usize,
    fmt: BlockFormat,
    block_stride_in_bytes: u32,
    mut output_row_pitch_in_blocks_or_pixels: u32,
) -> bool {
    if output_row_pitch_in_blocks_or_pixels == 0 {
        output_row_pitch_in_blocks_or_pixels = num_blocks_x;
    }
    let blk: [u8; 8] = match fmt {
        BlockFormat::Bc4 => Dxt5aBlock::opaque().bytes,
        BlockFormat::Etc2EacA8 | BlockFormat::Etc2EacR11 => EacBlock::opaque().bytes,
        _ => return true,
    };
    for y in 0..num_blocks_y as usize {
        let mut dst_ofs = offset
            + y * output_row_pitch_in_blocks_or_pixels as usize * block_stride_in_bytes as usize;
        for _ in 0..num_blocks_x {
            match output.get_mut(dst_ofs..dst_ofs + blk.len()) {
                Some(d) => d.copy_from_slice(&blk),
                None => return false,
            }
            dst_ofs += block_stride_in_bytes as usize;
        }
    }
    true
}

/// Packs a color for the 16-bit uncompressed formats.
fn pack16(fmt: BlockFormat, c: &Color32) -> u16 {
    let (r, g, b) = (c.r() as u32, c.g() as u32, c.b() as u32);
    let v = match fmt {
        BlockFormat::Rgb565 => (mul_8(r, 31) << 11) | (mul_8(g, 63) << 5) | mul_8(b, 31),
        BlockFormat::Bgr565 => (mul_8(b, 31) << 11) | (mul_8(g, 63) << 5) | mul_8(r, 31),
        BlockFormat::Rgba4444Color => {
            (mul_8(r, 15) << 12) | (mul_8(g, 15) << 8) | (mul_8(b, 15) << 4)
        }
        BlockFormat::Rgba4444ColorOpaque => {
            (mul_8(r, 15) << 12) | (mul_8(g, 15) << 8) | (mul_8(b, 15) << 4) | 0xF
        }
        BlockFormat::Rgba4444Alpha => mul_8(g, 15),
        _ => 0,
    };
    v as u16
}
