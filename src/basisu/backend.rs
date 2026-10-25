//! Port of `basisu_backend.cpp`: ETC1S endpoint / selector codebook ordering, endpoint
//! prediction and selector history RDO, Huffman coding of the slice data and codebooks.

use super::BasisTexFormat;
use super::bitreader::ApproxMoveToFront;
use super::enc::{ColorRgba, color_distance, crc16};
use super::etc_enc::{EtcBlock, unpack_etc1};
use super::etc1s::{
    COLOR5_PAL0_PREV_HI, COLOR5_PAL1_PREV_HI, CR_ENDPOINT_PRED_INDEX, ENDPOINT_PRED_COUNT_VLC_BITS,
    ENDPOINT_PRED_MIN_REPEAT_COUNT, ENDPOINT_PRED_REPEAT_LAST_SYMBOL, ENDPOINT_PRED_TOTAL_SYMBOLS,
    NUM_ENDPOINT_PREDS, SELECTOR_HISTORY_BUF_RLE_COUNT_BITS, SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH,
    SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL,
};
use super::frontend::{BasisuFrontend, C_BASIS_TEX_TYPE_VIDEO_FRAMES};
use super::huffman_enc::{BitwiseCoder, Histogram, HuffmanEncodingTable, PaletteIndexReorderer};

/// `basist::NO_ENDPOINT_PRED_INDEX`.
pub const NO_ENDPOINT_PRED_INDEX: u32 = 3;
/// `basist::MAX_SELECTOR_HISTORY_BUF_SIZE`.
pub const MAX_SELECTOR_HISTORY_BUF_SIZE: u32 = 64;

/// `etc1_selector_palette_entry`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Etc1SelectorPaletteEntry {
    pub selectors: [u8; 16],
}

impl Etc1SelectorPaletteEntry {
    pub fn clear(&mut self) {
        self.selectors = [0; 16];
    }

    pub fn set_uint32(&mut self, v: u32) {
        for byte_index in 0..4 {
            let b = (v >> (byte_index * 8)) & 0xFF;
            for k in 0..4 {
                self.selectors[byte_index * 4 + k] = ((b >> (2 * k)) & 3) as u8;
            }
        }
    }

    pub fn get_uint32(&self) -> u32 {
        self.get_byte(0)
            | (self.get_byte(1) << 8)
            | (self.get_byte(2) << 16)
            | (self.get_byte(3) << 24)
    }

    pub fn get_byte(&self, byte_index: u32) -> u32 {
        let s = &self.selectors[(byte_index as usize & 3) * 4..];
        s[0] as u32 | ((s[1] as u32) << 2) | ((s[2] as u32) << 4) | ((s[3] as u32) << 6)
    }

    /// `operator()(x, y)`.
    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.selectors[((x & 3) + (y & 3) * 4) as usize]
    }
}

/// `encoder_block`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncoderBlock {
    pub endpoint_predictor: u32,
    pub endpoint_index: u32,
    pub selector_index: u32,
    pub selector_history_buf_index: i32,
    pub is_cr_target: bool,
}

/// `etc1_endpoint_palette_entry`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Etc1EndpointPaletteEntry {
    pub color5: ColorRgba,
    pub inten5: u32,
    pub color5_valid: bool,
}

/// `basisu_backend_params`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasisuBackendParams {
    pub etc1s: bool,
    pub debug: bool,
    pub debug_images: bool,
    pub endpoint_rdo_quality_thresh: f32,
    pub selector_rdo_quality_thresh: f32,
    pub compression_level: u32,
    pub used_global_codebooks: bool,
    pub validate: bool,
}

impl Default for BasisuBackendParams {
    fn default() -> Self {
        BasisuBackendParams {
            etc1s: false,
            debug: false,
            debug_images: false,
            endpoint_rdo_quality_thresh: 0.0,
            selector_rdo_quality_thresh: 0.0,
            compression_level: 0,
            used_global_codebooks: false,
            validate: true,
        }
    }
}

impl BasisuBackendParams {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// `basisu_backend_slice_desc`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasisuBackendSliceDesc {
    pub first_block_index: u32,
    pub orig_width: u32,
    pub orig_height: u32,
    pub width: u32,
    pub height: u32,
    pub num_blocks_x: u32,
    pub num_blocks_y: u32,
    pub num_macroblocks_x: u32,
    pub num_macroblocks_y: u32,
    /// Also the basis image index.
    pub source_file_index: u32,
    pub mip_index: u32,
    pub alpha: bool,
    pub iframe: bool,
}

/// `basisu_backend_output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasisuBackendOutput {
    pub tex_format: BasisTexFormat,
    pub etc1s: bool,
    pub uses_global_codebooks: bool,
    pub srgb: bool,
    pub num_endpoints: u32,
    pub num_selectors: u32,
    pub endpoint_palette: Vec<u8>,
    pub selector_palette: Vec<u8>,
    pub slice_desc: Vec<BasisuBackendSliceDesc>,
    pub slice_image_tables: Vec<u8>,
    pub slice_image_data: Vec<Vec<u8>>,
    pub slice_image_crcs: Vec<u16>,
}

impl Default for BasisuBackendOutput {
    fn default() -> Self {
        BasisuBackendOutput {
            tex_format: BasisTexFormat::Etc1s,
            etc1s: false,
            uses_global_codebooks: false,
            srgb: true,
            num_endpoints: 0,
            num_selectors: 0,
            endpoint_palette: Vec::new(),
            selector_palette: Vec::new(),
            slice_desc: Vec::new(),
            slice_image_tables: Vec::new(),
            slice_image_data: Vec::new(),
            slice_image_crcs: Vec::new(),
        }
    }
}

impl BasisuBackendOutput {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn get_output_size_estimate(&self) -> u32 {
        let mut total = (self.slice_image_tables.len()
            + self.endpoint_palette.len()
            + self.selector_palette.len()) as u32;
        for d in &self.slice_image_data {
            total += d.len() as u32;
        }
        total
    }
}

/// `g_endpoint_preds` (dx, dy).
const ENDPOINT_PREDS: [(i32, i32); 3] = [(-1, 0), (0, -1), (-1, -1)];

/// Per slice `vector2D<encoder_block>`.
#[derive(Debug, Clone, Default)]
struct EncoderBlocks {
    w: u32,
    blocks: Vec<EncoderBlock>,
}

impl EncoderBlocks {
    fn new(w: u32, h: u32) -> Self {
        EncoderBlocks {
            w,
            blocks: vec![EncoderBlock::default(); (w * h) as usize],
        }
    }

    #[inline]
    fn at(&self, x: u32, y: u32) -> &EncoderBlock {
        &self.blocks[(x + y * self.w) as usize]
    }

    #[inline]
    fn at_mut(&mut self, x: u32, y: u32) -> &mut EncoderBlock {
        &mut self.blocks[(x + y * self.w) as usize]
    }
}

/// `basisu_backend`. The frontend is passed to [`BasisuBackend::encode`] instead of being
/// stored as `m_pFront_end`. `BASISU_BACKEND_VERIFY` aborts in C++; here `encode` returns 0.
#[derive(Debug, Clone, Default)]
pub struct BasisuBackend {
    params: BasisuBackendParams,
    slices: Vec<BasisuBackendSliceDesc>,
    output: BasisuBackendOutput,
    endpoint_palette: Vec<Etc1EndpointPaletteEntry>,
    selector_palette: Vec<Etc1SelectorPaletteEntry>,
    slice_encoder_blocks: Vec<EncoderBlocks>,
    endpoint_remap_table_old_to_new: Vec<u32>,
    endpoint_remap_table_new_to_old: Vec<u32>,
    old_endpoint_was_used: Vec<bool>,
    new_endpoint_was_used: Vec<bool>,
    selector_remap_table_old_to_new: Vec<u32>,
    selector_remap_table_new_to_old: Vec<u32>,
    verify_failed: bool,
}

impl BasisuBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.params.clear();
        self.output.clear();
    }

    pub fn init(&mut self, params: &BasisuBackendParams, slice_descs: &[BasisuBackendSliceDesc]) {
        self.params = *params;
        self.slices = slice_descs.to_vec();
    }

    pub fn get_output(&self) -> &BasisuBackendOutput {
        &self.output
    }

    pub fn get_params(&self) -> &BasisuBackendParams {
        &self.params
    }

    #[inline]
    fn verify(&mut self, c: bool) {
        if !c {
            self.verify_failed = true;
        }
    }

    fn get_total_blocks(&self) -> u32 {
        self.slices
            .iter()
            .map(|s| s.num_blocks_x * s.num_blocks_y)
            .sum()
    }

    fn create_endpoint_palette(&mut self, r: &BasisuFrontend) {
        let n = r.get_total_endpoint_clusters();
        self.output.num_endpoints = n;
        self.endpoint_palette.clear();
        self.endpoint_palette
            .resize(n as usize, Etc1EndpointPaletteEntry::default());
        let mut ok = true;
        for i in 0..n {
            let e = &mut self.endpoint_palette[i as usize];
            e.color5_valid = r.get_endpoint_cluster_color_is_used(i, false);
            e.color5 = *r.get_endpoint_cluster_unscaled_color(i, false);
            e.inten5 = r.get_endpoint_cluster_inten_table(i, false);
            ok &= e.color5_valid;
        }
        self.verify(ok);
    }

    fn create_selector_palette(&mut self, r: &BasisuFrontend) {
        let n = r.get_total_selector_clusters();
        self.output.num_selectors = n;
        self.selector_palette.clear();
        self.selector_palette
            .resize(n as usize, Etc1SelectorPaletteEntry::default());
        for i in 0..n {
            let bits = r.get_selector_cluster_selector_bits(i);
            let s = &mut self.selector_palette[i as usize];
            for y in 0..4 {
                for x in 0..4 {
                    s.selectors[(y * 4 + x) as usize] = bits.get_selector(x, y) as u8;
                }
            }
        }
    }

    fn reoptimize_and_sort_endpoints_codebook(
        &mut self,
        r: &mut BasisuFrontend,
        total_block_endpoints_remapped: u32,
        all_endpoint_indices: &mut [u32],
    ) {
        let n_endpoints = r.get_total_endpoint_clusters();
        if self.params.used_global_codebooks {
            self.endpoint_remap_table_old_to_new = (0..n_endpoints).collect();
        } else {
            if total_block_endpoints_remapped != 0 && self.params.compression_level > 1 {
                let mut new_block_endpoints = vec![0u32; self.get_total_blocks() as usize];
                for (si, s) in self.slices.iter().enumerate() {
                    for by in 0..s.num_blocks_y {
                        for bx in 0..s.num_blocks_x {
                            if let Some(e) = new_block_endpoints
                                .get_mut((s.first_block_index + bx + by * s.num_blocks_x) as usize)
                            {
                                *e = self.slice_encoder_blocks[si].at(bx, by).endpoint_index;
                            }
                        }
                    }
                }
                let mut old_to_new = Vec::new();
                r.reoptimize_remapped_endpoints(&new_block_endpoints, &mut old_to_new, true, None);
                self.create_endpoint_palette(r);
                let map = |i: u32| old_to_new.get(i as usize).map_or(0, |&v| v as u32);
                for blocks in self.slice_encoder_blocks.iter_mut() {
                    for m in blocks.blocks.iter_mut() {
                        m.endpoint_index = map(m.endpoint_index);
                    }
                }
                for e in all_endpoint_indices.iter_mut() {
                    *e = map(*e);
                }
            }

            // Sort endpoint codebook
            let mut reorderer = PaletteIndexReorderer::new();
            reorderer.init(
                all_endpoint_indices,
                r.get_total_endpoint_clusters(),
                None,
                0.0,
            );
            self.endpoint_remap_table_old_to_new = reorderer.get_remap_table().to_vec();
        }

        // For endpoints, old_to_new[] may not be bijective!
        let n_endpoints = r.get_total_endpoint_clusters() as usize;
        self.old_endpoint_was_used = vec![false; n_endpoints];
        let mut first_old_entry_index = u32::MAX;
        for blocks in &self.slice_encoder_blocks {
            for m in &blocks.blocks {
                if let Some(u) = self
                    .old_endpoint_was_used
                    .get_mut(m.endpoint_index as usize)
                {
                    *u = true;
                }
                first_old_entry_index = first_old_entry_index.min(m.endpoint_index);
            }
        }
        self.new_endpoint_was_used = vec![false; n_endpoints];
        // Set unused entries in the new array to point to the first used entry in the old array.
        self.endpoint_remap_table_new_to_old = vec![first_old_entry_index; n_endpoints];
        for old_index in 0..self.endpoint_remap_table_old_to_new.len() {
            if self
                .old_endpoint_was_used
                .get(old_index)
                .copied()
                .unwrap_or(false)
            {
                let new_index = self.endpoint_remap_table_old_to_new[old_index] as usize;
                if new_index < n_endpoints {
                    self.new_endpoint_was_used[new_index] = true;
                    self.endpoint_remap_table_new_to_old[new_index] = old_index as u32;
                }
            }
        }
    }

    fn sort_selector_codebook(&mut self, r: &BasisuFrontend) {
        let n = r.get_total_selector_clusters();
        self.selector_remap_table_new_to_old = vec![0; n as usize];
        if self.params.compression_level == 0 || self.params.used_global_codebooks {
            for i in 0..n {
                self.selector_remap_table_new_to_old[i as usize] = i;
            }
        } else if n > 0 {
            let mut prev_selector_index = 0u32;
            let mut remaining: Vec<u32> = (1..n).collect();
            let bytes: Vec<u32> = self
                .selector_palette
                .iter()
                .map(|s| s.get_uint32())
                .collect();
            // This is the traveling salesman problem.
            for i in 1..n {
                let mut best_hamming_dist = 100u32;
                let mut best_index = 0usize;
                for (j, &selector_index) in remaining.iter().enumerate() {
                    let k = bytes[prev_selector_index as usize] ^ bytes[selector_index as usize];
                    let hamming_dist = k.count_ones();
                    if hamming_dist < best_hamming_dist {
                        best_hamming_dist = hamming_dist;
                        best_index = j;
                        if best_hamming_dist <= 1 {
                            break;
                        }
                    }
                }
                prev_selector_index = remaining[best_index];
                self.selector_remap_table_new_to_old[i as usize] = prev_selector_index;
                remaining.swap_remove(best_index);
            }
        }
        self.selector_remap_table_old_to_new = vec![0; n as usize];
        for (i, &o) in self.selector_remap_table_new_to_old.iter().enumerate() {
            if let Some(e) = self.selector_remap_table_old_to_new.get_mut(o as usize) {
                *e = i as u32;
            }
        }
    }

    fn find_video_frame(&self, slice_index: usize, delta: i32) -> i32 {
        let cur = &self.slices[slice_index];
        for (s, sl) in self.slices.iter().enumerate() {
            if sl.source_file_index as i32 != cur.source_file_index as i32 + delta {
                continue;
            }
            if sl.mip_index != cur.mip_index {
                continue;
            }
            // Being super paranoid here.
            if sl.num_blocks_x != cur.num_blocks_x
                || sl.num_blocks_y != cur.num_blocks_y
                || sl.alpha != cur.alpha
            {
                continue;
            }
            return s as i32;
        }
        -1
    }

    fn check_for_valid_cr_blocks(&mut self, r: &BasisuFrontend) {
        if r.get_params().tex_type != C_BASIS_TEX_TYPE_VIDEO_FRAMES {
            return;
        }
        let mut total_invalid_crs = 0u32;
        let mut ok = true;
        for si in 0..self.slices.len() {
            let is_iframe = self.slices[si].iframe;
            let prev = self.find_video_frame(si, -1);
            // If we don't have a previous frame, and we're not an i-frame, something is wrong.
            if prev < 0 && !is_iframe {
                ok = false;
            }
            let blocks = &self.slice_encoder_blocks[si];
            if is_iframe || prev < 0 {
                // Ensure no blocks use CR's
                ok &= blocks
                    .blocks
                    .iter()
                    .all(|m| m.endpoint_predictor != CR_ENDPOINT_PRED_INDEX);
            } else {
                // For blocks that use CR's, make sure the endpoints/selectors haven't really changed.
                let prev_blocks = &self.slice_encoder_blocks[prev as usize];
                for (m, pm) in blocks.blocks.iter().zip(prev_blocks.blocks.iter()) {
                    if m.endpoint_predictor == CR_ENDPOINT_PRED_INDEX
                        && (m.endpoint_index != pm.endpoint_index
                            || m.selector_index != pm.selector_index)
                    {
                        total_invalid_crs += 1;
                    }
                }
            }
        }
        self.verify(ok);
        self.verify(total_invalid_crs == 0);
    }

    fn create_encoder_blocks(&mut self, r: &mut BasisuFrontend) {
        let is_video = r.get_params().tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES;
        let perceptual = r.get_params().perceptual;
        self.slice_encoder_blocks = vec![EncoderBlocks::default(); self.slices.len()];
        let mut total_block_endpoints_remapped = 0u32;
        let mut all_endpoint_indices: Vec<u32> =
            Vec::with_capacity(self.get_total_blocks() as usize);
        let mut ok = true;

        for si in 0..self.slices.len() {
            let prev_frame_slice_index = if is_video {
                self.find_video_frame(si, -1)
            } else {
                -1
            };
            let s = self.slices[si];
            let (nbx, nby) = (s.num_blocks_x, s.num_blocks_y);
            self.slice_encoder_blocks[si] = EncoderBlocks::new(nbx, nby);

            for block_y in 0..nby {
                for block_x in 0..nbx {
                    let block_index = s.first_block_index + block_x + block_y * nbx;
                    let mut m = EncoderBlock {
                        endpoint_index: r.get_subblock_endpoint_cluster_index(block_index, 0),
                        selector_index: r.get_block_selector_cluster_index(block_index),
                        endpoint_predictor: NO_ENDPOINT_PRED_INDEX,
                        ..Default::default()
                    };
                    ok &= r.get_subblock_endpoint_cluster_index(block_index, 0)
                        == r.get_subblock_endpoint_cluster_index(block_index, 1);
                    let block_endpoint = m.endpoint_index;
                    let mut best_endpoint_pred = u32::MAX;

                    for endpoint_pred in 0..NUM_ENDPOINT_PREDS {
                        if is_video && endpoint_pred == CR_ENDPOINT_PRED_INDEX {
                            if prev_frame_slice_index != -1 && !s.iframe {
                                let p = self.slice_encoder_blocks[prev_frame_slice_index as usize]
                                    .at(block_x, block_y);
                                if m.endpoint_index == p.endpoint_index
                                    && m.selector_index == p.selector_index
                                {
                                    best_endpoint_pred = CR_ENDPOINT_PRED_INDEX;
                                    self.slice_encoder_blocks[prev_frame_slice_index as usize]
                                        .at_mut(block_x, block_y)
                                        .is_cr_target = true;
                                }
                            }
                        } else {
                            let (dx, dy) = ENDPOINT_PREDS[endpoint_pred as usize];
                            let pbx = block_x as i32 + dx;
                            let pby = block_y as i32 + dy;
                            if pbx < 0 || pbx >= nbx as i32 || pby < 0 || pby >= nby as i32 {
                                continue;
                            }
                            let pred_endpoint = self.slice_encoder_blocks[si]
                                .at(pbx as u32, pby as u32)
                                .endpoint_index;
                            if pred_endpoint == block_endpoint && endpoint_pred < best_endpoint_pred
                            {
                                best_endpoint_pred = endpoint_pred;
                            }
                        }
                    }

                    if best_endpoint_pred != u32::MAX {
                        m.endpoint_predictor = best_endpoint_pred;
                    } else if self.params.endpoint_rdo_quality_thresh > 0.0 {
                        let src_pixels = r.get_source_pixel_block(block_index);
                        let etc_blk = *r.get_output_block(block_index);
                        let cur_err = etc_blk.evaluate_etc1_error(src_pixels, perceptual, -1);
                        if cur_err != 0 {
                            let thresh_err = (cur_err as f32
                                * self.params.endpoint_rdo_quality_thresh.max(1.0))
                                as u64;
                            let mut trial_etc_block = etc_blk;
                            let mut best_err = u64::MAX;
                            let mut best_endpoint_index = 0u32;
                            best_endpoint_pred = u32::MAX;
                            for endpoint_pred in 0..NUM_ENDPOINT_PREDS {
                                if is_video && endpoint_pred == CR_ENDPOINT_PRED_INDEX {
                                    continue;
                                }
                                let (dx, dy) = ENDPOINT_PREDS[endpoint_pred as usize];
                                let pbx = block_x as i32 + dx;
                                let pby = block_y as i32 + dy;
                                if pbx < 0 || pbx >= nbx as i32 || pby < 0 || pby >= nby as i32 {
                                    continue;
                                }
                                let pred_endpoint_index = self.slice_encoder_blocks[si]
                                    .at(pbx as u32, pby as u32)
                                    .endpoint_index;
                                let pred_inten =
                                    r.get_endpoint_cluster_inten_table(pred_endpoint_index, false);
                                let pred_color = *r.get_endpoint_cluster_unscaled_color(
                                    pred_endpoint_index,
                                    false,
                                );
                                trial_etc_block.set_block_color5(&pred_color, &pred_color);
                                trial_etc_block.set_inten_table(0, pred_inten);
                                trial_etc_block.set_inten_table(1, pred_inten);
                                let mut trial_colors = [[0u8; 4]; 16];
                                unpack_etc1(&trial_etc_block, &mut trial_colors, false);
                                let mut trial_err = 0u64;
                                for p in 0..16 {
                                    trial_err += color_distance(
                                        perceptual,
                                        &src_pixels[p],
                                        &trial_colors[p],
                                        false,
                                    ) as u64;
                                    if trial_err > thresh_err {
                                        break;
                                    }
                                }
                                if trial_err <= thresh_err
                                    && (trial_err < best_err
                                        || (trial_err == best_err
                                            && endpoint_pred < best_endpoint_pred))
                                {
                                    best_endpoint_pred = endpoint_pred;
                                    best_err = trial_err;
                                    best_endpoint_index = pred_endpoint_index;
                                }
                            }
                            if best_endpoint_pred != u32::MAX {
                                m.endpoint_index = best_endpoint_index;
                                m.endpoint_predictor = best_endpoint_pred;
                                total_block_endpoints_remapped += 1;
                            }
                        }
                    }

                    if m.endpoint_predictor == NO_ENDPOINT_PRED_INDEX {
                        all_endpoint_indices.push(m.endpoint_index);
                    }
                    *self.slice_encoder_blocks[si].at_mut(block_x, block_y) = m;
                }
            }
        }
        self.verify(ok);

        self.reoptimize_and_sort_endpoints_codebook(
            r,
            total_block_endpoints_remapped,
            &mut all_endpoint_indices,
        );
        self.sort_selector_codebook(r);
        self.check_for_valid_cr_blocks(r);
    }

    fn compute_slice_crcs(&mut self) {
        for si in 0..self.slices.len() {
            let s = self.slices[si];
            let (gbx, gby) = (s.width.div_ceil(4), s.height.div_ceil(4));
            let mut gi = vec![0u8; (gbx * gby * 8) as usize];
            for block_y in 0..s.num_blocks_y {
                for block_x in 0..s.num_blocks_x {
                    let m = self.slice_encoder_blocks[si].at(block_x, block_y);
                    let mut output_block = EtcBlock::default();
                    output_block.set_diff_bit(true);
                    // Setting the flip bit to false to be compatible with the Khronos KDFS.
                    output_block.set_flip_bit(false);
                    let e = &self.endpoint_palette[m.endpoint_index as usize];
                    output_block.set_block_color5_etc1s(&e.color5);
                    output_block.set_inten_tables_etc1s(e.inten5);
                    let selectors = &self.selector_palette[m.selector_index as usize];
                    for sy in 0..4 {
                        for sx in 0..4 {
                            output_block.set_selector(sx, sy, selectors.get(sx, sy) as u32);
                        }
                    }
                    let ofs = ((block_x + block_y * gbx) * 8) as usize;
                    if let Some(d) = gi.get_mut(ofs..ofs + 8) {
                        d.copy_from_slice(&output_block.bytes);
                    }
                }
            }
            self.output.slice_image_crcs[si] = crc16(&gi, 0);
        }
    }

    fn encode_image(&mut self, r: &mut BasisuFrontend) -> bool {
        let is_video = r.get_params().tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES;
        let perceptual = r.get_params().perceptual;
        let n_selectors = r.get_total_selector_clusters();
        let n_endpoints = r.get_total_endpoint_clusters();

        let mut selector_history_buf = ApproxMoveToFront::new(MAX_SELECTOR_HISTORY_BUF_SIZE);
        let mut selector_history_buf_histogram = Histogram::new(MAX_SELECTOR_HISTORY_BUF_SIZE);
        let mut selector_histogram =
            Histogram::new(n_selectors + MAX_SELECTOR_HISTORY_BUF_SIZE + 1);
        let mut selector_history_buf_rle_histogram =
            Histogram::new(1 << SELECTOR_HISTORY_BUF_RLE_COUNT_BITS);
        let mut selector_syms: Vec<Vec<u32>> = vec![Vec::new(); self.slices.len()];

        let first_hist_sym = n_selectors;
        let rle_sym = first_hist_sym + MAX_SELECTOR_HISTORY_BUF_SIZE;

        self.output.slice_image_crcs = vec![0; self.slices.len()];

        let mut delta_endpoint_histogram = Histogram::new(n_endpoints);
        let mut endpoint_pred_histogram = Histogram::new(ENDPOINT_PRED_TOTAL_SYMBOLS);
        let mut endpoint_pred_syms: Vec<Vec<u32>> = vec![Vec::new(); self.slices.len()];

        let mut total_endpoint_indices_remapped = 0u32;
        let mut block_endpoint_indices: Vec<u32> = Vec::new();
        let mut block_selector_indices: Vec<u32> = Vec::new();

        const COLOR_DELTA_THRESH: i32 = 8;
        const SEL_DIFF_THRESHOLD: i32 = 11;

        fn flush_pred_run(
            hist: &mut Histogram,
            syms: &mut Vec<u32>,
            repeat_count: &mut i32,
            prev_bits: i32,
        ) {
            if *repeat_count > 0 {
                if *repeat_count > ENDPOINT_PRED_MIN_REPEAT_COUNT as i32 {
                    hist.inc(ENDPOINT_PRED_REPEAT_LAST_SYMBOL);
                    syms.push(ENDPOINT_PRED_REPEAT_LAST_SYMBOL);
                    syms.push(*repeat_count as u32);
                } else {
                    for _ in 0..*repeat_count {
                        hist.inc(prev_bits as u32);
                        syms.push(prev_bits as u32);
                    }
                }
                *repeat_count = 0;
            }
        }

        fn flush_rle(
            syms: &mut Vec<u32>,
            rle_hist: &mut Histogram,
            sel_hist: &mut Histogram,
            rle_count: &mut i32,
            rle_sym: u32,
            first_hist_sym: u32,
        ) {
            if *rle_count >= SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH as i32 {
                syms.push(rle_sym);
                syms.push(*rle_count as u32);
                let run_sym = *rle_count - SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH as i32;
                if run_sym >= SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL as i32 - 1 {
                    rle_hist.inc(SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL - 1);
                } else {
                    rle_hist.inc(run_sym as u32);
                }
                sel_hist.inc(rle_sym);
            } else {
                for _ in 0..*rle_count {
                    syms.push(first_hist_sym);
                    sel_hist.inc(first_hist_sym);
                }
            }
            *rle_count = 0;
        }

        for si in 0..self.slices.len() {
            let s = self.slices[si];
            let (nbx, nby) = (s.num_blocks_x, s.num_blocks_y);

            selector_history_buf.reset();
            let mut selector_history_buf_rle_count = 0i32;
            let (mut prev_endpoint_pred_sym_bits, mut endpoint_pred_repeat_count) = (-1i32, 0i32);
            let mut prev_endpoint_index = 0u32;

            let mut referenced = vec![false; (nbx * nby) as usize];
            {
                let mut set_ref = |x: u32, y: u32| {
                    if x < nbx
                        && y < nby
                        && let Some(v) = referenced.get_mut((x + y * nbx) as usize)
                    {
                        *v = true;
                    }
                };
                for block_y in 0..nby {
                    for block_x in 0..nbx {
                        let m = self.slice_encoder_blocks[si].at(block_x, block_y);
                        match m.endpoint_predictor {
                            0 => set_ref(block_x.wrapping_sub(1), block_y),
                            1 => set_ref(block_x, block_y.wrapping_sub(1)),
                            2 if !is_video => {
                                set_ref(block_x.wrapping_sub(1), block_y.wrapping_sub(1))
                            }
                            _ => {}
                        }
                        if is_video && m.is_cr_target {
                            set_ref(block_x, block_y);
                        }
                    }
                }
            }

            for block_y in 0..nby {
                for block_x in 0..nbx {
                    let block_index = s.first_block_index + block_x + block_y * nbx;
                    let mut m = *self.slice_encoder_blocks[si].at(block_x, block_y);

                    if block_x & 1 == 0 && block_y & 1 == 0 {
                        let mut cur_bits = 0u32;
                        for y in 0..2 {
                            for x in 0..2 {
                                let (bx, by) = (block_x + x, block_y + y);
                                let mut pred = NO_ENDPOINT_PRED_INDEX;
                                if bx < nbx && by < nby {
                                    pred =
                                        self.slice_encoder_blocks[si].at(bx, by).endpoint_predictor;
                                }
                                cur_bits |= pred << (x * 2 + y * 4);
                            }
                        }
                        if cur_bits as i32 == prev_endpoint_pred_sym_bits {
                            endpoint_pred_repeat_count += 1;
                        } else {
                            flush_pred_run(
                                &mut endpoint_pred_histogram,
                                &mut endpoint_pred_syms[si],
                                &mut endpoint_pred_repeat_count,
                                prev_endpoint_pred_sym_bits,
                            );
                            endpoint_pred_histogram.inc(cur_bits);
                            endpoint_pred_syms[si].push(cur_bits);
                            prev_endpoint_pred_sym_bits = cur_bits as i32;
                        }
                    }

                    let mut new_endpoint_index =
                        self.endpoint_remap_table_old_to_new[m.endpoint_index as usize];

                    if m.endpoint_predictor == NO_ENDPOINT_PRED_INDEX {
                        let mut endpoint_delta =
                            new_endpoint_index as i32 - prev_endpoint_index as i32;

                        if self.params.endpoint_rdo_quality_thresh > 1.0
                            && endpoint_delta.abs() > 1
                            && !referenced[(block_x + block_y * nbx) as usize]
                        {
                            let src_pixels = r.get_source_pixel_block(block_index);
                            let etc_blk = *r.get_output_block(block_index);
                            let cur_err = etc_blk.evaluate_etc1_error(src_pixels, perceptual, -1);
                            let cur_inten5 = etc_blk.get_inten_table(0);
                            let cur_endpoints = self.endpoint_palette[m.endpoint_index as usize];
                            if cur_err != 0 {
                                let endpoint_remap_thresh =
                                    self.params.endpoint_rdo_quality_thresh.max(1.0);
                                let thresh_err = (cur_err as f32 * endpoint_remap_thresh) as u64;
                                let max_endpoint_search_dist = if self.params.compression_level >= 2
                                {
                                    64
                                } else {
                                    16
                                };

                                // The SSE 4.1 path of the reference build: the block's own
                                // selectors with the trial endpoints.
                                let mut block_selectors = [0u8; 16];
                                for (i, b) in block_selectors.iter_mut().enumerate() {
                                    *b = etc_blk.get_selector(i as u32 & 3, i as u32 >> 2) as u8;
                                }
                                let mut best_trial_err = i64::MAX;
                                let mut best_trial_idx = 0i32;
                                let search_dist =
                                    (endpoint_delta.abs() - 1).min(max_endpoint_search_dist);
                                for d in -search_dist..search_dist {
                                    let mut trial_idx = prev_endpoint_index as i32 + d;
                                    if trial_idx < 0 {
                                        trial_idx += n_endpoints as i32;
                                    } else if trial_idx >= n_endpoints as i32 {
                                        trial_idx -= n_endpoints as i32;
                                    }
                                    if trial_idx == new_endpoint_index as i32 {
                                        continue;
                                    }
                                    // Skip it if this new endpoint palette entry is actually never used.
                                    if !self.new_endpoint_was_used[trial_idx as usize] {
                                        continue;
                                    }
                                    let p = self.endpoint_palette[self
                                        .endpoint_remap_table_new_to_old
                                        [trial_idx as usize]
                                        as usize];
                                    if self.params.compression_level <= 1 {
                                        if p.inten5 > cur_inten5 {
                                            continue;
                                        }
                                        let color_delta = (0..3)
                                            .map(|c| {
                                                (cur_endpoints.color5[c] as i32
                                                    - p.color5[c] as i32)
                                                    .abs()
                                            })
                                            .sum::<i32>();
                                        if color_delta > COLOR_DELTA_THRESH {
                                            continue;
                                        }
                                    }
                                    let mut block_colors = [[0u8; 4]; 4];
                                    EtcBlock::get_block_colors_etc1s_from(
                                        &mut block_colors,
                                        &p.color5,
                                        p.inten5,
                                    );
                                    // The kernel early outs once the partial sum reaches the best
                                    // error, which can never be accepted below.
                                    let mut trial_err = 0i64;
                                    for (px, &sel) in src_pixels.iter().zip(block_selectors.iter())
                                    {
                                        trial_err += color_distance(
                                            perceptual,
                                            &block_colors[sel as usize],
                                            px,
                                            false,
                                        )
                                            as i64;
                                    }
                                    if trial_err < best_trial_err && trial_err <= thresh_err as i64
                                    {
                                        best_trial_err = trial_err;
                                        best_trial_idx = trial_idx;
                                    }
                                }
                                if best_trial_err != i64::MAX {
                                    m.endpoint_index = self.endpoint_remap_table_new_to_old
                                        [best_trial_idx as usize];
                                    new_endpoint_index = best_trial_idx as u32;
                                    endpoint_delta =
                                        new_endpoint_index as i32 - prev_endpoint_index as i32;
                                    total_endpoint_indices_remapped += 1;
                                }
                            }
                        }

                        if endpoint_delta < 0 {
                            endpoint_delta += n_endpoints as i32;
                        }
                        delta_endpoint_histogram.inc(endpoint_delta as u32);
                    }

                    block_endpoint_indices
                        .push(self.endpoint_remap_table_new_to_old[new_endpoint_index as usize]);
                    prev_endpoint_index = new_endpoint_index;

                    if !is_video || m.endpoint_predictor != CR_ENDPOINT_PRED_INDEX {
                        let mut new_selector_index =
                            self.selector_remap_table_old_to_new[m.selector_index as usize] as i32;
                        let selector_remap_thresh =
                            self.params.selector_rdo_quality_thresh.max(1.0);
                        let mut selector_history_buf_index = -1i32;

                        // At low comp levels this hurts compression a tiny amount, but is
                        // significantly faster so it's a good tradeoff.
                        if m.is_cr_target || self.params.compression_level <= 1 {
                            for j in 0..selector_history_buf.size() {
                                if selector_history_buf.get(j) == new_selector_index {
                                    selector_history_buf_index = j as i32;
                                    selector_history_buf_histogram.inc(j as u32);
                                    break;
                                }
                            }
                        }

                        // If the block is a CR target we can't override its selectors.
                        if !m.is_cr_target && selector_history_buf_index == -1 {
                            let src_pixels = r.get_source_pixel_block(block_index);
                            let mut etc_blk = *r.get_output_block(block_index);
                            let q = self.endpoint_palette[self.endpoint_remap_table_new_to_old
                                [new_endpoint_index as usize]
                                as usize];
                            etc_blk.set_block_color5_etc1s(&q.color5);
                            etc_blk.set_inten_tables_etc1s(q.inten5);
                            let mut block_colors = [[0u8; 4]; 4];
                            etc_blk.get_block_colors(&mut block_colors, 0);

                            let cur_selectors =
                                self.selector_palette[m.selector_index as usize].selectors;
                            let mut cur_err = 0u64;
                            for p in 0..16 {
                                cur_err += color_distance(
                                    perceptual,
                                    &src_pixels[p],
                                    &block_colors[cur_selectors[p] as usize],
                                    false,
                                ) as u64;
                            }
                            let limit_err = (cur_err as f32 * selector_remap_thresh).ceil() as u64;

                            let mut best_trial_err = u64::MAX;
                            let mut best_trial_idx = 0i32;
                            let mut best_trial_history_buf_idx = 0u32;

                            for j in 0..selector_history_buf.size() {
                                let trial_idx = selector_history_buf.get(j);
                                let sel = &self.selector_palette[self
                                    .selector_remap_table_new_to_old
                                    [trial_idx as usize]
                                    as usize]
                                    .selectors;
                                if self.params.compression_level <= 1 {
                                    // Predict if evaluating the full color error would cause an
                                    // early out, by summing the abs err of the selector indices.
                                    let mut sel_diff = 0i32;
                                    for p in (0..16).step_by(4) {
                                        for k in 0..4 {
                                            sel_diff += (cur_selectors[p + k] as i32
                                                - sel[p + k] as i32)
                                                .abs();
                                        }
                                        if sel_diff >= SEL_DIFF_THRESHOLD {
                                            break;
                                        }
                                    }
                                    if sel_diff >= SEL_DIFF_THRESHOLD {
                                        continue;
                                    }
                                }
                                let thresh_err = limit_err.min(best_trial_err);
                                let mut trial_err = 0u64;
                                for p in 0..16 {
                                    trial_err += color_distance(
                                        perceptual,
                                        &src_pixels[p],
                                        &block_colors[sel[p] as usize],
                                        false,
                                    ) as u64;
                                    if trial_err > thresh_err {
                                        break;
                                    }
                                }
                                if trial_err < best_trial_err && trial_err <= thresh_err {
                                    best_trial_err = trial_err;
                                    best_trial_idx = trial_idx;
                                    best_trial_history_buf_idx = j as u32;
                                }
                            }

                            if best_trial_err != u64::MAX {
                                new_selector_index = best_trial_idx;
                                selector_history_buf_index = best_trial_history_buf_idx as i32;
                                selector_history_buf_histogram.inc(best_trial_history_buf_idx);
                            }
                        }

                        m.selector_index =
                            self.selector_remap_table_new_to_old[new_selector_index as usize];

                        if selector_history_buf_rle_count != 0 && selector_history_buf_index != 0 {
                            flush_rle(
                                &mut selector_syms[si],
                                &mut selector_history_buf_rle_histogram,
                                &mut selector_histogram,
                                &mut selector_history_buf_rle_count,
                                rle_sym,
                                first_hist_sym,
                            );
                        }

                        if selector_history_buf_index >= 0 {
                            if selector_history_buf_index == 0 {
                                selector_history_buf_rle_count += 1;
                            } else {
                                let sym = first_hist_sym + selector_history_buf_index as u32;
                                selector_syms[si].push(sym);
                                selector_histogram.inc(sym);
                            }
                        } else {
                            selector_syms[si].push(new_selector_index as u32);
                            selector_histogram.inc(new_selector_index as u32);
                        }

                        m.selector_history_buf_index = selector_history_buf_index;

                        if selector_history_buf_index < 0 {
                            selector_history_buf.add(new_selector_index);
                        } else if selector_history_buf.size() != 0 {
                            selector_history_buf.use_index(selector_history_buf_index as usize);
                        }
                    }
                    block_selector_indices.push(m.selector_index);
                    *self.slice_encoder_blocks[si].at_mut(block_x, block_y) = m;
                }
            }

            flush_pred_run(
                &mut endpoint_pred_histogram,
                &mut endpoint_pred_syms[si],
                &mut endpoint_pred_repeat_count,
                prev_endpoint_pred_sym_bits,
            );
            if selector_history_buf_rle_count != 0 {
                flush_rle(
                    &mut selector_syms[si],
                    &mut selector_history_buf_rle_histogram,
                    &mut selector_histogram,
                    &mut selector_history_buf_rle_count,
                    rle_sym,
                    first_hist_sym,
                );
            }
        }

        if total_endpoint_indices_remapped != 0
            && self.params.compression_level > 1
            && !self.params.used_global_codebooks
        {
            let mut unused = Vec::new();
            r.reoptimize_remapped_endpoints(
                &block_endpoint_indices,
                &mut unused,
                false,
                Some(&block_selector_indices),
            );
            self.create_endpoint_palette(r);
        }

        self.check_for_valid_cr_blocks(r);
        self.compute_slice_crcs();

        let model = |h: &mut Histogram| -> Option<HuffmanEncodingTable> {
            if h.get_total() == 0 {
                h.inc(0);
            }
            let mut t = HuffmanEncodingTable::new();
            t.init(h, 16).then_some(t)
        };
        let Some(endpoint_pred_model) = model(&mut endpoint_pred_histogram) else {
            return false;
        };
        let Some(delta_endpoint_model) = model(&mut delta_endpoint_histogram) else {
            return false;
        };
        let Some(selector_model) = model(&mut selector_histogram) else {
            return false;
        };
        let Some(selector_history_buf_rle_model) = model(&mut selector_history_buf_rle_histogram)
        else {
            return false;
        };

        let mut coder = BitwiseCoder::new();
        coder.init(1024 * 1024 * 4);
        coder.emit_huffman_table(&endpoint_pred_model);
        coder.emit_huffman_table(&delta_endpoint_model);
        coder.emit_huffman_table(&selector_model);
        coder.emit_huffman_table(&selector_history_buf_rle_model);
        coder.put_bits(MAX_SELECTOR_HISTORY_BUF_SIZE, 13);
        coder.flush();
        self.output.slice_image_tables = coder.get_bytes().clone();

        self.output.slice_image_data = vec![Vec::new(); self.slices.len()];
        let mut ok = true;
        for si in 0..self.slices.len() {
            let s = self.slices[si];
            coder.init(1024 * 1024 * 4);
            let mut cur_selector_sym_ofs = 0usize;
            let mut selector_rle_count = 0u32;
            let mut endpoint_pred_repeat_count = 0i32;
            let mut cur_endpoint_pred_sym_ofs = 0usize;
            let mut prev_endpoint_index = 0u32;
            let pred_syms = &endpoint_pred_syms[si];
            let sel_syms = &selector_syms[si];
            let sym_at = |v: &[u32], i: usize| v.get(i).copied().unwrap_or(0);

            for block_y in 0..s.num_blocks_y {
                for block_x in 0..s.num_blocks_x {
                    let m = *self.slice_encoder_blocks[si].at(block_x, block_y);

                    if block_x & 1 == 0 && block_y & 1 == 0 {
                        if endpoint_pred_repeat_count > 0 {
                            endpoint_pred_repeat_count -= 1;
                        } else {
                            let sym = sym_at(pred_syms, cur_endpoint_pred_sym_ofs);
                            cur_endpoint_pred_sym_ofs += 1;
                            coder.put_code(sym, &endpoint_pred_model);
                            if sym == ENDPOINT_PRED_REPEAT_LAST_SYMBOL {
                                endpoint_pred_repeat_count =
                                    sym_at(pred_syms, cur_endpoint_pred_sym_ofs) as i32;
                                cur_endpoint_pred_sym_ofs += 1;
                                coder.put_vlc(
                                    (endpoint_pred_repeat_count
                                        - ENDPOINT_PRED_MIN_REPEAT_COUNT as i32)
                                        as u32,
                                    ENDPOINT_PRED_COUNT_VLC_BITS,
                                );
                                endpoint_pred_repeat_count -= 1;
                            }
                        }
                    }

                    let new_endpoint_index =
                        self.endpoint_remap_table_old_to_new[m.endpoint_index as usize];
                    if m.endpoint_predictor == NO_ENDPOINT_PRED_INDEX {
                        let mut endpoint_delta =
                            new_endpoint_index as i32 - prev_endpoint_index as i32;
                        if endpoint_delta < 0 {
                            endpoint_delta += n_endpoints as i32;
                        }
                        coder.put_code(endpoint_delta as u32, &delta_endpoint_model);
                    }
                    prev_endpoint_index = new_endpoint_index;

                    if !is_video || m.endpoint_predictor != CR_ENDPOINT_PRED_INDEX {
                        if selector_rle_count == 0 {
                            let selector_sym_index = sym_at(sel_syms, cur_selector_sym_ofs);
                            cur_selector_sym_ofs += 1;
                            if selector_sym_index == rle_sym {
                                selector_rle_count = sym_at(sel_syms, cur_selector_sym_ofs);
                                cur_selector_sym_ofs += 1;
                            }
                            coder.put_code(selector_sym_index, &selector_model);
                            if selector_sym_index == rle_sym {
                                let run_sym = selector_rle_count as i32
                                    - SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH as i32;
                                if run_sym >= SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL as i32 - 1 {
                                    coder.put_code(
                                        SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL - 1,
                                        &selector_history_buf_rle_model,
                                    );
                                    let n =
                                        selector_rle_count - SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH;
                                    coder.put_vlc(n, 7);
                                } else {
                                    coder.put_code(run_sym as u32, &selector_history_buf_rle_model);
                                }
                            }
                        }
                        selector_rle_count = selector_rle_count.saturating_sub(1);
                    }
                }
            }

            ok &= cur_endpoint_pred_sym_ofs == pred_syms.len();
            ok &= cur_selector_sym_ofs == sel_syms.len();
            coder.flush();
            self.output.slice_image_data[si] = coder.get_bytes().clone();
        }
        self.verify(ok);
        true
    }

    fn encode_endpoint_palette(&mut self, r: &BasisuFrontend) -> bool {
        let n = r.get_total_endpoint_clusters() as usize;
        // The endpoint indices may have been changed by the backend's RDO step, so go and figure
        // out which ones are actually used again.
        let mut old_endpoint_was_used = vec![false; n];
        let mut first_old_entry_index = u32::MAX;
        for blocks in &self.slice_encoder_blocks {
            for m in &blocks.blocks {
                if let Some(u) = old_endpoint_was_used.get_mut(m.endpoint_index as usize) {
                    *u = true;
                }
                first_old_entry_index = first_old_entry_index.min(m.endpoint_index);
            }
        }
        // Maps NEW to OLD endpoints
        let mut new_to_old = vec![first_old_entry_index; n];
        for (old, &new) in self.endpoint_remap_table_old_to_new.iter().enumerate() {
            if old_endpoint_was_used.get(old).copied().unwrap_or(false)
                && let Some(e) = new_to_old.get_mut(new as usize)
            {
                *e = old as u32;
            }
        }

        let is_grayscale = self
            .endpoint_palette
            .iter()
            .all(|e| e.color5[0] == e.color5[1] && e.color5[0] == e.color5[2]);
        let num_comps = if is_grayscale { 1 } else { 3 };

        let mut color5_delta_hist = [Histogram::new(32), Histogram::new(32), Histogram::new(32)];
        let mut inten_delta_hist = Histogram::new(8);
        let pal_index = |prev: i32| {
            if prev <= COLOR5_PAL0_PREV_HI {
                0
            } else if prev <= COLOR5_PAL1_PREV_HI {
                1
            } else {
                2
            }
        };

        let mut prev_color5 = [16i32, 16, 16, 0];
        let mut prev_inten = 0u32;
        for &old in &new_to_old {
            let e = &self.endpoint_palette[old as usize];
            inten_delta_hist.inc(e.inten5.wrapping_sub(prev_inten) & 7);
            prev_inten = e.inten5;
            for i in 0..num_comps {
                let delta = (e.color5[i] as i32 - prev_color5[i]) & 31;
                color5_delta_hist[pal_index(prev_color5[i])].inc(delta as u32);
                prev_color5[i] = e.color5[i] as i32;
            }
        }
        for h in color5_delta_hist.iter_mut() {
            if h.get_total() == 0 {
                h.inc(0);
            }
        }
        let mut color5_delta_model = [
            HuffmanEncodingTable::new(),
            HuffmanEncodingTable::new(),
            HuffmanEncodingTable::new(),
        ];
        for (m, h) in color5_delta_model.iter_mut().zip(color5_delta_hist.iter()) {
            if !m.init(h, 16) {
                return false;
            }
        }
        let mut inten_delta_model = HuffmanEncodingTable::new();
        if !inten_delta_model.init(&inten_delta_hist, 16) {
            return false;
        }

        let mut coder = BitwiseCoder::new();
        coder.init(8192);
        for m in &color5_delta_model {
            coder.emit_huffman_table(m);
        }
        coder.emit_huffman_table(&inten_delta_model);
        coder.put_bits(is_grayscale as u32, 1);

        prev_color5 = [16, 16, 16, 0];
        prev_inten = 0;
        for &old in &new_to_old {
            let e = &self.endpoint_palette[old as usize];
            coder.put_code(e.inten5.wrapping_sub(prev_inten) & 7, &inten_delta_model);
            prev_inten = e.inten5;
            for i in 0..num_comps {
                let delta = (e.color5[i] as i32 - prev_color5[i]) & 31;
                coder.put_code(delta as u32, &color5_delta_model[pal_index(prev_color5[i])]);
                prev_color5[i] = e.color5[i] as i32;
            }
        }
        coder.flush();
        self.output.endpoint_palette = coder.get_bytes().clone();
        true
    }

    fn encode_selector_palette(&mut self, r: &BasisuFrontend) -> bool {
        let n = r.get_total_selector_clusters() as usize;
        let entry =
            |q: usize| &self.selector_palette[self.selector_remap_table_new_to_old[q] as usize];

        let mut delta_selector_pal_histogram = Histogram::new(256);
        for q in 1..n {
            let (cur, predictor) = (entry(q), entry(q - 1));
            for j in 0..4 {
                delta_selector_pal_histogram.inc(cur.get_byte(j) ^ predictor.get_byte(j));
            }
        }
        if delta_selector_pal_histogram.get_total() == 0 {
            delta_selector_pal_histogram.inc(0);
        }
        let mut delta_selector_pal_model = HuffmanEncodingTable::new();
        if !delta_selector_pal_model.init(&delta_selector_pal_histogram, 16) {
            return false;
        }

        let mut coder = BitwiseCoder::new();
        coder.init(1024 * 1024);
        coder.put_bits(0, 1); // use global codebook
        coder.put_bits(0, 1); // uses hybrid codebooks
        coder.put_bits(0, 1); // raw bytes
        coder.emit_huffman_table(&delta_selector_pal_model);
        for q in 0..n {
            if q == 0 {
                for j in 0..4 {
                    coder.put_bits(entry(q).get_byte(j), 8);
                }
                continue;
            }
            let (cur, predictor) = (entry(q), entry(q - 1));
            for j in 0..4 {
                coder.put_code(
                    cur.get_byte(j) ^ predictor.get_byte(j),
                    &delta_selector_pal_model,
                );
            }
        }
        coder.flush();
        let mut out = coder.get_bytes().clone();

        if out.len() >= n * 4 {
            coder.init(1024 * 1024);
            coder.put_bits(0, 1); // use global codebook
            coder.put_bits(0, 1); // uses hybrid codebooks
            coder.put_bits(1, 1); // raw bytes
            for q in 0..n {
                for j in 0..4 {
                    coder.put_bits(entry(q).get_byte(j), 8);
                }
            }
            coder.flush();
            out = coder.get_bytes().clone();
        }
        self.output.selector_palette = out;
        true
    }

    /// `encode()`: returns the total compressed size, 0 on failure.
    pub fn encode(&mut self, front_end: &mut BasisuFrontend) -> u32 {
        self.verify_failed = false;
        self.output.slice_desc = self.slices.clone();
        self.output.etc1s = self.params.etc1s;
        self.output.uses_global_codebooks = self.params.used_global_codebooks;
        self.output.srgb = front_end.get_params().perceptual;

        self.create_endpoint_palette(front_end);
        self.create_selector_palette(front_end);
        self.create_encoder_blocks(front_end);
        if self.verify_failed || !self.encode_image(front_end) {
            return 0;
        }
        if self.verify_failed
            || !self.encode_endpoint_palette(front_end)
            || !self.encode_selector_palette(front_end)
        {
            return 0;
        }
        self.output.get_output_size_estimate()
    }
}
