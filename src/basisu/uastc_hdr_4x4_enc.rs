// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_uastc_hdr_4x4_enc.cpp`: UASTC HDR 4x4 block encoding (solid, CEM 11
//! and CEM 7 with one or two subsets restricted to the ASTC / BC6H common partitions), the BC6H
//! transcode of every candidate and the BC6H aware weight refinement.
//!
//! `astc_hdr_4x4_block_stats` (debug statistics) is not ported.

#![allow(clippy::needless_range_loop, clippy::field_reassign_with_default)]

use super::astc_hdr_enc::{
    AstcHdrCodecBaseOptions, BIG_FLOAT_VAL, MAX_HALF_FLOAT, OptMode, Q_LOG_BIAS_4X4,
    Q_LOG_BIAS_6X6, Vec3, compute_block_error, encode_astc_hdr_block_mode_7,
    encode_astc_hdr_block_mode_11, float_to_half_non_neg_no_nan_inf, half_to_qlog16, q,
};
use super::astc_helpers::{
    BISE_3_LEVELS, BISE_4_LEVELS, BISE_5_LEVELS, BISE_6_LEVELS, BISE_8_LEVELS, BISE_12_LEVELS,
    BISE_16_LEVELS, BISE_40_LEVELS, BISE_64_LEVELS, BISE_80_LEVELS, BISE_128_LEVELS,
    BISE_192_LEVELS, BISE_256_LEVELS, DecodePixels, HalfFloat, LogAstcBlock, decode_block,
    get_ise_levels, pack_astc_block,
};
use super::bc6h::{
    NUM_MODE7_ENDPOINTS, NUM_MODE11_ENDPOINTS, TOTAL_ASTC_BC6H_COMMON_PARTITIONS2,
    astc_hdr_transcode_log_to_bc6h, bc6h_enc_block_solid_color, decode_mode7_to_qlog12,
    decode_mode11_to_qlog12, qlog_to_half, transcode_bc6h_1subset, transcode_bc6h_2subsets,
};
use super::gpu_texture::unpack_bc6h;
use super::uastc_tables::{ASTC_BC7_COMMON_PARTITIONS2, BC7_PARTITION2};

const UHDR_MODE11_FIRST_ISE_RANGE: u32 = BISE_3_LEVELS;
const UHDR_MODE11_LAST_ISE_RANGE: u32 = BISE_16_LEVELS;
const UHDR_MODE7_PART1_FIRST_ISE_RANGE: u32 = BISE_3_LEVELS;
const UHDR_MODE7_PART1_LAST_ISE_RANGE: u32 = BISE_16_LEVELS;
const UHDR_MODE7_PART2_FIRST_ISE_RANGE: u32 = BISE_3_LEVELS;
const UHDR_MODE7_PART2_LAST_ISE_RANGE: u32 = BISE_8_LEVELS;
const UHDR_MODE11_PART2_FIRST_ISE_RANGE: u32 = BISE_3_LEVELS;
const UHDR_MODE11_PART2_LAST_ISE_RANGE: u32 = BISE_4_LEVELS;

/// `uastc_hdr_4x4_codec_options`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UastcHdr4x4CodecOptions {
    pub base: AstcHdrCodecBaseOptions,
    pub bc6h_err_weight: f32,
    pub use_solid: bool,
    pub use_mode11_part1: bool,
    pub mode11_uber_mode: bool,
    pub first_mode11_weight_ise_range: u32,
    pub last_mode11_weight_ise_range: u32,
    pub mode11_direct_only: bool,
    pub first_mode11_submode: i32,
    pub last_mode11_submode: i32,
    pub use_mode7_part1: bool,
    pub first_mode7_part1_weight_ise_range: u32,
    pub last_mode7_part1_weight_ise_range: u32,
    pub use_mode7_part2: bool,
    pub mode7_part2_part_masks: u32,
    pub first_mode7_part2_weight_ise_range: u32,
    pub last_mode7_part2_weight_ise_range: u32,
    pub use_mode11_part2: bool,
    pub mode11_part2_part_masks: u32,
    pub first_mode11_part2_weight_ise_range: u32,
    pub last_mode11_part2_weight_ise_range: u32,
    pub refine_weights: bool,
    pub level: u32,
    pub use_estimated_partitions: bool,
    pub max_estimated_partitions: u32,
}

impl Default for UastcHdr4x4CodecOptions {
    fn default() -> Self {
        let mut o = UastcHdr4x4CodecOptions {
            base: AstcHdrCodecBaseOptions::default(),
            bc6h_err_weight: 0.0,
            use_solid: false,
            use_mode11_part1: false,
            mode11_uber_mode: false,
            first_mode11_weight_ise_range: 0,
            last_mode11_weight_ise_range: 0,
            mode11_direct_only: false,
            first_mode11_submode: 0,
            last_mode11_submode: 0,
            use_mode7_part1: false,
            first_mode7_part1_weight_ise_range: 0,
            last_mode7_part1_weight_ise_range: 0,
            use_mode7_part2: false,
            mode7_part2_part_masks: 0,
            first_mode7_part2_weight_ise_range: 0,
            last_mode7_part2_weight_ise_range: 0,
            use_mode11_part2: false,
            mode11_part2_part_masks: 0,
            first_mode11_part2_weight_ise_range: 0,
            last_mode11_part2_weight_ise_range: 0,
            refine_weights: false,
            level: 0,
            use_estimated_partitions: false,
            max_estimated_partitions: 0,
        };
        o.init();
        o
    }
}

impl UastcHdr4x4CodecOptions {
    pub const MIN_LEVEL: i32 = 0;
    pub const MAX_LEVEL: i32 = 4;
    pub const DEFAULT_LEVEL: i32 = 1;

    /// `init`.
    pub fn init(&mut self) {
        self.base.init();
        // The initial release used Q_LOG_BIAS_4x4, which is too low.
        self.base.q_log_bias = Q_LOG_BIAS_6X6;
        self.bc6h_err_weight = 0.85;
        // Must set the quality level at least once to reset this struct.
        self.set_quality_level(Self::DEFAULT_LEVEL);
    }

    fn set_quality_best(&mut self) {
        // highest achievable quality
        self.mode11_direct_only = false;
        self.use_solid = true;
        self.use_mode11_part1 = true;
        self.mode11_uber_mode = true;
        self.first_mode11_weight_ise_range = UHDR_MODE11_FIRST_ISE_RANGE;
        self.last_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE;
        self.first_mode11_submode = -1;
        self.last_mode11_submode = 7;
        self.use_mode7_part1 = true;
        self.first_mode7_part1_weight_ise_range = UHDR_MODE7_PART1_FIRST_ISE_RANGE;
        self.last_mode7_part1_weight_ise_range = UHDR_MODE7_PART1_LAST_ISE_RANGE;
        self.base.mode7_full_s_optimization = true;
        self.use_mode7_part2 = true;
        self.mode7_part2_part_masks = u32::MAX;
        self.first_mode7_part2_weight_ise_range = UHDR_MODE7_PART2_FIRST_ISE_RANGE;
        self.last_mode7_part2_weight_ise_range = UHDR_MODE7_PART2_LAST_ISE_RANGE;
        self.use_mode11_part2 = true;
        self.mode11_part2_part_masks = u32::MAX;
        self.first_mode11_part2_weight_ise_range = UHDR_MODE11_PART2_FIRST_ISE_RANGE;
        self.last_mode11_part2_weight_ise_range = UHDR_MODE11_PART2_LAST_ISE_RANGE;
        self.refine_weights = true;
        self.use_estimated_partitions = false;
        self.max_estimated_partitions = 0;
    }

    fn set_quality_normal(&mut self) {
        self.use_solid = true;
        // We'll allow uber mode in normal if the user allows it.
        self.use_mode11_part1 = true;
        self.mode11_uber_mode = true;
        self.first_mode11_weight_ise_range = 6;
        self.last_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE;
        self.use_mode7_part1 = true;
        self.first_mode7_part1_weight_ise_range = UHDR_MODE7_PART1_LAST_ISE_RANGE;
        self.last_mode7_part1_weight_ise_range = UHDR_MODE7_PART1_LAST_ISE_RANGE;
        self.use_mode7_part2 = true;
        self.mode7_part2_part_masks = u32::MAX;
        self.first_mode7_part2_weight_ise_range = UHDR_MODE7_PART2_LAST_ISE_RANGE;
        self.last_mode7_part2_weight_ise_range = UHDR_MODE7_PART2_LAST_ISE_RANGE;
        self.use_mode11_part2 = true;
        self.mode11_part2_part_masks = u32::MAX;
        self.first_mode11_part2_weight_ise_range = UHDR_MODE11_PART2_LAST_ISE_RANGE;
        self.last_mode11_part2_weight_ise_range = UHDR_MODE11_PART2_LAST_ISE_RANGE;
        self.refine_weights = true;
    }

    fn set_quality_fastest(&mut self) {
        self.use_solid = true;
        self.use_mode11_part1 = true;
        self.mode11_uber_mode = false;
        self.first_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE;
        self.last_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE;
        self.use_mode7_part1 = false;
        self.base.mode7_full_s_optimization = false;
        self.use_mode7_part2 = false;
        self.use_mode11_part2 = false;
        self.refine_weights = false;
    }

    /// `set_quality_level` (0-4).
    pub fn set_quality_level(&mut self, level: i32) {
        let level = level.clamp(Self::MIN_LEVEL, Self::MAX_LEVEL);
        self.level = level as u32;
        // First ensure all options are set to best.
        self.set_quality_best();
        match level {
            0 => self.set_quality_fastest(),
            1 => {
                self.set_quality_normal();
                self.first_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE - 1;
                self.last_mode11_weight_ise_range = UHDR_MODE11_LAST_ISE_RANGE;
                self.use_mode7_part1 = false;
                self.base.mode7_full_s_optimization = false;
                self.use_mode7_part2 = false;
                self.use_estimated_partitions = true;
                self.max_estimated_partitions = 1;
                self.mode11_part2_part_masks = 1 | 2;
                self.mode7_part2_part_masks = 1 | 2;
            }
            2 => {
                self.set_quality_normal();
                self.use_estimated_partitions = true;
                self.max_estimated_partitions = 2;
                self.mode11_part2_part_masks = 1 | 2;
                self.mode7_part2_part_masks = 1 | 2;
            }
            3 => {
                self.use_estimated_partitions = true;
                self.max_estimated_partitions = 2;
                self.mode11_part2_part_masks = 1 | 2 | 4 | 8;
                self.mode7_part2_part_masks = 1 | 2 | 4 | 8;
            }
            _ => {} // best options already set
        }
    }
}

/// `astc_hdr_4x4_pack_results`.
#[derive(Debug, Clone, Copy)]
pub struct AstcHdr4x4PackResults {
    pub best_block_error: f64,
    /// Not used/set by the encoder, here for convenience.
    pub bc6h_block_error: f64,
    pub best_blk: LogAstcBlock,
    pub best_submodes: [u32; 2],
    pub best_pat_index: u32,
    pub constrained_weights: bool,
    pub improved_via_refinement_flag: bool,
    pub solid_blk: [u8; 16],
    pub bc6h_block: [u8; 16],
    pub is_solid: bool,
}

impl Default for AstcHdr4x4PackResults {
    fn default() -> Self {
        let mut r = AstcHdr4x4PackResults {
            best_block_error: 0.0,
            bc6h_block_error: 0.0,
            best_blk: LogAstcBlock::default(),
            best_submodes: [0; 2],
            best_pat_index: 0,
            constrained_weights: false,
            improved_via_refinement_flag: false,
            solid_blk: [0; 16],
            bc6h_block: [0; 16],
            is_solid: false,
        };
        r.clear();
        r
    }
}

impl AstcHdr4x4PackResults {
    pub fn clear(&mut self) {
        self.best_block_error = BIG_FLOAT_VAL as f64;
        self.bc6h_block_error = BIG_FLOAT_VAL as f64;
        self.best_blk.clear();
        self.best_blk.grid_width = 4;
        self.best_blk.grid_height = 4;
        self.best_blk.endpoint_ise_range = 20; // 0-255
        self.best_submodes = [0; 2];
        self.best_pat_index = 0;
        self.constrained_weights = false;
        self.bc6h_block = [0; 16];
        self.is_solid = false;
        self.improved_via_refinement_flag = false;
    }
}

fn pack_solid(
    block_linear_colors: &[[f32; 4]; 16],
    all_results: &mut Vec<AstcHdr4x4PackResults>,
    coptions: &UastcHdr4x4CodecOptions,
) -> bool {
    let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
    const LOG_BIAS: f32 = 0.125;
    let mut solid_block = true;
    for c in block_linear_colors.iter() {
        if block_linear_colors[0][0] != c[0]
            || block_linear_colors[0][1] != c[1]
            || block_linear_colors[0][2] != c[2]
        {
            solid_block = false;
        }
        r += (c[0] + LOG_BIAS).log2();
        g += (c[1] + LOG_BIAS).log2();
        b += (c[2] + LOG_BIAS).log2();
    }
    if solid_block {
        r = block_linear_colors[0][0];
        g = block_linear_colors[0][1];
        b = block_linear_colors[0][2];
    } else {
        let maxf = |a: f32, b: f32| if a > b { a } else { b };
        let minf = |a: f32, b: f32| if a < b { a } else { b };
        r = maxf(0.0, 2.0f32.powf(r * (1.0 / 16.0)) - LOG_BIAS);
        g = maxf(0.0, 2.0f32.powf(g * (1.0 / 16.0)) - LOG_BIAS);
        b = maxf(0.0, 2.0f32.powf(b * (1.0 / 16.0)) - LOG_BIAS);
        // for safety
        r = minf(r, MAX_HALF_FLOAT as f32);
        g = minf(g, MAX_HALF_FLOAT as f32);
        b = minf(b, MAX_HALF_FLOAT as f32);
    }
    let rh = float_to_half_non_neg_no_nan_inf(r);
    let gh = float_to_half_non_neg_no_nan_inf(g);
    let bh = float_to_half_non_neg_no_nan_inf(b);
    let ah = float_to_half_non_neg_no_nan_inf(1.0);

    let mut results = AstcHdr4x4PackResults::default();
    results.is_solid = true;
    let p = &mut results.solid_blk;
    p[0] = 0b1111_1100;
    for v in p.iter_mut().take(8).skip(1) {
        *v = 255;
    }
    for (i, h) in [rh, gh, bh, ah].iter().enumerate() {
        p[8 + i * 2..10 + i * 2].copy_from_slice(&h.to_le_bytes());
    }
    results.best_block_error = 0.0;
    if !solid_block {
        let r_weight = coptions.base.r_err_scale as f64;
        let g_weight = coptions.base.g_err_scale as f64;
        // This MUST match how errors are computed in eval_selectors().
        for c in block_linear_colors.iter() {
            let dr = float_to_half_non_neg_no_nan_inf(c[0]);
            let dg = float_to_half_non_neg_no_nan_inf(c[1]);
            let db = float_to_half_non_neg_no_nan_inf(c[2]);
            let rd = q(rh, Q_LOG_BIAS_4X4) - q(dr, Q_LOG_BIAS_4X4);
            let gd = q(gh, Q_LOG_BIAS_4X4) - q(dg, Q_LOG_BIAS_4X4);
            let bd = q(bh, Q_LOG_BIAS_4X4) - q(db, Q_LOG_BIAS_4X4);
            results.best_block_error += r_weight * (rd * rd) + g_weight * (gd * gd) + bd * bd;
        }
    }
    bc6h_enc_block_solid_color(&mut results.bc6h_block, [rh, gh, bh]);
    all_results.push(results);
    solid_block
}

fn half_endpoints(e: &[[i32; 3]; 2]) -> [[HalfFloat; 2]; 3] {
    let h = |v: i32| qlog_to_half(v as u32, 12);
    [
        [h(e[0][0]), h(e[1][0])],
        [h(e[0][1]), h(e[1][1])],
        [h(e[0][2]), h(e[1][2])],
    ]
}

#[allow(clippy::too_many_arguments)]
fn pack_mode11(
    block_pixels_half: &[[HalfFloat; 3]; 16],
    block_pixels_q16: &[[f32; 4]; 16],
    all_results: &mut Vec<AstcHdr4x4PackResults>,
    coptions: &UastcHdr4x4CodecOptions,
    first_weight_ise_range: u32,
    last_weight_ise_range: u32,
    constrain_ise_weight_selectors: bool,
) {
    let mut trial_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
    let mut trial_weights = [0u8; 16];
    let mut trial_submode11 = 0u32;
    for weight_ise_range in first_weight_ise_range..=last_weight_ise_range {
        let direct_only = coptions.mode11_direct_only;
        let endpoint_ise_range = if weight_ise_range == BISE_16_LEVELS {
            BISE_192_LEVELS
        } else {
            BISE_256_LEVELS
        };
        let trial_error = encode_astc_hdr_block_mode_11(
            16,
            block_pixels_half,
            block_pixels_q16,
            weight_ise_range,
            &mut trial_submode11,
            BIG_FLOAT_VAL as f64,
            &mut trial_endpoints,
            &mut trial_weights,
            &coptions.base,
            direct_only,
            endpoint_ise_range,
            coptions.mode11_uber_mode
                && weight_ise_range >= BISE_4_LEVELS
                && coptions.base.allow_uber_mode,
            constrain_ise_weight_selectors,
            coptions.first_mode11_submode,
            coptions.last_mode11_submode,
            false,
            OptMode::OrdinaryLeastSquares,
            None,
        );
        if trial_error < BIG_FLOAT_VAL as f64 {
            let mut results = AstcHdr4x4PackResults::default();
            results.best_block_error = trial_error;
            results.best_submodes[0] = trial_submode11;
            results.constrained_weights = constrain_ise_weight_selectors;
            results.best_blk.num_partitions = 1;
            results.best_blk.color_endpoint_modes[0] = 11;
            results.best_blk.weight_ise_range = weight_ise_range as u8;
            results.best_blk.endpoint_ise_range = endpoint_ise_range as u8;
            results.best_blk.endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_endpoints);
            results.best_blk.weights[..16].copy_from_slice(&trial_weights);

            // transcode to BC6H: get qlog12 endpoints
            let mut e = [[0i32; 3]; 2];
            decode_mode11_to_qlog12(&results.best_blk.endpoints, &mut e, endpoint_ise_range);
            let h_e = half_endpoints(&e);
            let blk = results.best_blk;
            transcode_bc6h_1subset(&h_e, &blk, &mut results.bc6h_block);
            all_results.push(results);
        }
    }
}

fn pack_mode7_single_part(
    block_pixels_half: &[[HalfFloat; 3]; 16],
    block_pixels_q16: &[[f32; 4]; 16],
    all_results: &mut Vec<AstcHdr4x4PackResults>,
    coptions: &UastcHdr4x4CodecOptions,
    first_weight_ise_range: u32,
    last_weight_ise_range: u32,
) {
    let mut trial_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
    let mut trial_weights = [0u8; 16];
    let mut trial_submode7 = 0u32;
    for weight_ise_range in first_weight_ise_range..=last_weight_ise_range {
        let ise_endpoint_range = BISE_256_LEVELS;
        let trial_error = encode_astc_hdr_block_mode_7(
            16,
            block_pixels_half,
            block_pixels_q16,
            weight_ise_range,
            &mut trial_submode7,
            BIG_FLOAT_VAL as f64,
            &mut trial_endpoints,
            &mut trial_weights,
            &coptions.base,
            ise_endpoint_range,
            0,
            5,
            None,
        );
        if trial_error < BIG_FLOAT_VAL as f64 {
            let mut results = AstcHdr4x4PackResults::default();
            results.best_block_error = trial_error;
            results.best_submodes[0] = trial_submode7;
            results.best_blk.num_partitions = 1;
            results.best_blk.color_endpoint_modes[0] = 7;
            results.best_blk.weight_ise_range = weight_ise_range as u8;
            results.best_blk.endpoint_ise_range = ise_endpoint_range as u8;
            results.best_blk.endpoints[..NUM_MODE7_ENDPOINTS].copy_from_slice(&trial_endpoints);
            results.best_blk.weights[..16].copy_from_slice(&trial_weights);

            let mut e = [[0i32; 3]; 2];
            if !decode_mode7_to_qlog12(&results.best_blk.endpoints, &mut e, ise_endpoint_range) {
                continue;
            }
            let h_e = half_endpoints(&e);
            let blk = results.best_blk;
            transcode_bc6h_1subset(&h_e, &blk, &mut results.bc6h_block);
            all_results.push(results);
        }
    }
}

fn estimate_partition(
    block_pixels_half: &[[HalfFloat; 3]; 16],
    best_parts: &mut [i32],
    num_best_parts: usize,
) -> bool {
    let sqd = |a: Vec3, b: Vec3| {
        let mut d2 = 0f32;
        for i in 0..3 {
            let d = a[i] - b[i];
            d2 += d * d;
        }
        d2
    };
    let mut training_vecs = [[0f32; 3]; 16];
    let mut mean = [0f32; 3];
    for (v, p) in training_vecs.iter_mut().zip(block_pixels_half.iter()) {
        *v = [p[0] as f32, p[1] as f32, p[2] as f32];
        for c in 0..3 {
            mean[c] += v[c];
        }
    }
    for c in mean.iter_mut() {
        *c *= 1.0 / 16.0;
    }
    let mut cluster_centroids = [
        [mean[0] - 0.1, mean[1] - 0.1, mean[2] - 0.1],
        [mean[0] + 0.1, mean[1] + 0.1, mean[2] + 0.1],
    ];
    let mut cluster_pixels = [[0usize; 16]; 2];
    let mut num_cluster_pixels = [0usize; 2];
    for _ in 0..4 {
        num_cluster_pixels = [0; 2];
        let mut new_cluster_means = [[0f32; 3]; 2];
        for (i, v) in training_vecs.iter().enumerate() {
            let d0 = sqd(*v, cluster_centroids[0]);
            let d1 = sqd(*v, cluster_centroids[1]);
            let k = if d0 < d1 { 0 } else { 1 };
            cluster_pixels[k][num_cluster_pixels[k]] = i;
            for c in 0..3 {
                new_cluster_means[k][c] += v[c];
            }
            num_cluster_pixels[k] += 1;
        }
        if num_cluster_pixels[0] == 0 || num_cluster_pixels[1] == 0 {
            return false;
        }
        for k in 0..2 {
            let n = num_cluster_pixels[k] as f32;
            cluster_centroids[k] = [
                new_cluster_means[k][0] / n,
                new_cluster_means[k][1] / n,
                new_cluster_means[k][2] / n,
            ];
        }
    }
    let mut desired_parts = [0i32; 16]; // [y * 4 + x]
    for p in 0..2 {
        for &pix_index in cluster_pixels[p].iter().take(num_cluster_pixels[p]) {
            desired_parts[pix_index] = p as i32;
        }
    }
    let mut part_similarity = [0u32; TOTAL_ASTC_BC6H_COMMON_PARTITIONS2];
    for (part_index, sim) in part_similarity.iter_mut().enumerate() {
        let bc7_pattern = ASTC_BC7_COMMON_PARTITIONS2[part_index].0 as usize;
        let (mut total_sim_non_inv, mut total_sim_inv) = (0i32, 0i32);
        for (i, &d) in desired_parts.iter().enumerate() {
            let part = BC7_PARTITION2[16 * bc7_pattern + i] as i32;
            if part == d {
                total_sim_non_inv += 1;
            }
            if (part ^ 1) == d {
                total_sim_inv += 1;
            }
        }
        let total_sim = total_sim_non_inv.max(total_sim_inv);
        *sim = ((total_sim << 8) as u32) | part_index as u32;
    }
    part_similarity.sort_unstable();
    for (i, b) in best_parts.iter_mut().enumerate().take(num_best_parts) {
        *b = (part_similarity[(TOTAL_ASTC_BC6H_COMMON_PARTITIONS2 - 1) - i] & 0xFF) as i32;
    }
    true
}

/// Extracts each subset's texels of a common partition: (half, q16, counts, pixel subset).
#[allow(clippy::type_complexity)]
fn split_partition(
    part_index: usize,
    block_pixels_half: &[[HalfFloat; 3]; 16],
    block_pixels_q16: &[[f32; 4]; 16],
) -> (
    [[[HalfFloat; 3]; 16]; 2],
    [[[f32; 4]; 16]; 2],
    [usize; 2],
    [usize; 16],
) {
    let (bc7_pattern, _, invert_flag) = ASTC_BC7_COMMON_PARTITIONS2[part_index];
    let mut part_pixels_half = [[[0 as HalfFloat; 3]; 16]; 2];
    let mut part_pixels_q16 = [[[0f32; 4]; 16]; 2];
    let mut num_part_pixels = [0usize; 2];
    let mut pixel_part_index = [0usize; 16];
    for i in 0..16 {
        let mut part = BC7_PARTITION2[16 * bc7_pattern as usize + i] as usize;
        if invert_flag {
            part = 1 - part;
        }
        pixel_part_index[i] = part;
        let n = num_part_pixels[part];
        part_pixels_half[part][n] = block_pixels_half[i];
        part_pixels_q16[part][n] = block_pixels_q16[i];
        num_part_pixels[part] = n + 1;
    }
    (
        part_pixels_half,
        part_pixels_q16,
        num_part_pixels,
        pixel_part_index,
    )
}

/// The 2 subset candidate partitions: estimated ones, or all masked common partitions.
fn partitions_to_try(num_estimated_partitions: usize, estimated: &[i32], masks: u32) -> Vec<usize> {
    if num_estimated_partitions != 0 {
        estimated
            .iter()
            .take(num_estimated_partitions)
            .map(|&p| p as usize)
            .collect()
    } else {
        (0..TOTAL_ASTC_BC6H_COMMON_PARTITIONS2)
            .filter(|&p| (1u32 << p) & masks != 0)
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn pack_2part(
    cem: u8,
    block_pixels_half: &[[HalfFloat; 3]; 16],
    block_pixels_q16: &[[f32; 4]; 16],
    all_results: &mut Vec<AstcHdr4x4PackResults>,
    coptions: &UastcHdr4x4CodecOptions,
    num_estimated_partitions: usize,
    estimated_partitions: &[i32],
    first_weight_ise_range: u32,
    last_weight_ise_range: u32,
) {
    let num_endpoints = if cem == 7 {
        NUM_MODE7_ENDPOINTS
    } else {
        NUM_MODE11_ENDPOINTS
    };
    let masks = if cem == 7 {
        coptions.mode7_part2_part_masks
    } else {
        coptions.mode11_part2_part_masks
    };
    let mut trial_blk = LogAstcBlock {
        grid_width: 4,
        grid_height: 4,
        num_partitions: 2,
        ..Default::default()
    };
    trial_blk.color_endpoint_modes[0] = cem;
    trial_blk.color_endpoint_modes[1] = cem;

    for part_index in partitions_to_try(num_estimated_partitions, estimated_partitions, masks) {
        let astc_pattern = ASTC_BC7_COMMON_PARTITIONS2[part_index].1;
        let (part_pixels_half, part_pixels_q16, num_part_pixels, pixel_part_index) =
            split_partition(part_index, block_pixels_half, block_pixels_q16);
        trial_blk.partition_id = astc_pattern;

        for weight_ise_range in first_weight_ise_range..=last_weight_ise_range {
            let ise_endpoint_range = if cem == 7 {
                match weight_ise_range {
                    BISE_5_LEVELS => BISE_192_LEVELS,
                    BISE_6_LEVELS => BISE_128_LEVELS,
                    BISE_8_LEVELS => BISE_80_LEVELS,
                    _ => BISE_256_LEVELS,
                }
            } else if weight_ise_range == BISE_4_LEVELS {
                BISE_40_LEVELS
            } else {
                BISE_64_LEVELS
            };
            let mut trial_endpoints = [[0u8; NUM_MODE11_ENDPOINTS]; 2];
            let mut trial_weights = [[0u8; 16]; 2];
            let mut trial_submode = [0u32; 2];
            let mut total_trial_err = 0f64;
            for pp in 0..2 {
                total_trial_err += if cem == 7 {
                    encode_astc_hdr_block_mode_7(
                        num_part_pixels[pp],
                        &part_pixels_half[pp],
                        &part_pixels_q16[pp],
                        weight_ise_range,
                        &mut trial_submode[pp],
                        BIG_FLOAT_VAL as f64,
                        &mut trial_endpoints[pp],
                        &mut trial_weights[pp],
                        &coptions.base,
                        ise_endpoint_range,
                        0,
                        5,
                        None,
                    )
                } else {
                    encode_astc_hdr_block_mode_11(
                        num_part_pixels[pp],
                        &part_pixels_half[pp],
                        &part_pixels_q16[pp],
                        weight_ise_range,
                        &mut trial_submode[pp],
                        BIG_FLOAT_VAL as f64,
                        &mut trial_endpoints[pp],
                        &mut trial_weights[pp],
                        &coptions.base,
                        false,
                        ise_endpoint_range,
                        coptions.mode11_uber_mode
                            && weight_ise_range >= BISE_4_LEVELS
                            && coptions.base.allow_uber_mode,
                        false,
                        coptions.first_mode11_submode,
                        coptions.last_mode11_submode,
                        false,
                        OptMode::OrdinaryLeastSquares,
                        None,
                    )
                };
            }
            if total_trial_err < BIG_FLOAT_VAL as f64 {
                trial_blk.weight_ise_range = weight_ise_range as u8;
                trial_blk.endpoint_ise_range = ise_endpoint_range as u8;
                for pp in 0..2 {
                    trial_blk.endpoints[pp * num_endpoints..(pp + 1) * num_endpoints]
                        .copy_from_slice(&trial_endpoints[pp][..num_endpoints]);
                }
                let mut src_pixel_index = [0usize; 2];
                for (i, &p) in pixel_part_index.iter().enumerate() {
                    trial_blk.weights[i] = trial_weights[p][src_pixel_index[p]];
                    src_pixel_index[p] += 1;
                }
                let mut results = AstcHdr4x4PackResults::default();
                results.best_block_error = total_trial_err;
                results.best_submodes = trial_submode;
                results.best_pat_index = part_index as u32;
                results.best_blk = trial_blk;
                transcode_bc6h_2subsets(part_index as u32, &trial_blk, &mut results.bc6h_block);
                all_results.push(results);
            }
        }
    }
}

/// `astc_hdr_4x4_enc_block`: encodes one 4x4 block (RGB floats and their halves) into every
/// candidate configuration allowed by `coptions`.
pub fn astc_hdr_4x4_enc_block(
    rgb_pixels: &[f32; 48],
    rgb_pixels_half: &[HalfFloat; 48],
    coptions: &UastcHdr4x4CodecOptions,
    all_results: &mut Vec<AstcHdr4x4PackResults>,
) -> bool {
    all_results.clear();
    let mut block_pixels_half = [[0 as HalfFloat; 3]; 16];
    let mut block_linear_colors = [[0f32; 4]; 16];
    let mut block_pixels_q16 = [[0f32; 4]; 16];
    let mut is_greyscale = true;
    for i in 0..16 {
        let (fr, fg, fb) = (
            rgb_pixels[i * 3],
            rgb_pixels[i * 3 + 1],
            rgb_pixels[i * 3 + 2],
        );
        block_linear_colors[i] = [fr, fg, fb, 1.0];
        let (hr, hg, hb) = (
            rgb_pixels_half[i * 3],
            rgb_pixels_half[i * 3 + 1],
            rgb_pixels_half[i * 3 + 2],
        );
        block_pixels_half[i] = [hr, hg, hb];
        block_pixels_q16[i] = [
            half_to_qlog16(hr as i32) as f32,
            half_to_qlog16(hg as i32) as f32,
            half_to_qlog16(hb as i32) as f32,
            0.0,
        ];
        if hr != hg || hr != hb {
            is_greyscale = false;
        }
    }

    let mut is_solid = false;
    if coptions.use_solid {
        is_solid = pack_solid(&block_linear_colors, all_results, coptions);
    }
    if !is_solid {
        let bph = &block_pixels_half;
        let bpq = &block_pixels_q16;
        if is_greyscale && coptions.level == 0 {
            // Special case if it's a pure grayscale block - just try mode 7.
            pack_mode7_single_part(bph, bpq, all_results, coptions, 1, 1);
            pack_mode7_single_part(
                bph,
                bpq,
                all_results,
                coptions,
                UHDR_MODE7_PART1_LAST_ISE_RANGE,
                UHDR_MODE7_PART1_LAST_ISE_RANGE,
            );
        } else {
            if coptions.use_mode11_part1 {
                let cur_num_results = all_results.len();
                pack_mode11(
                    bph,
                    bpq,
                    all_results,
                    coptions,
                    coptions.first_mode11_weight_ise_range,
                    coptions.last_mode11_weight_ise_range,
                    false,
                );
                if coptions.last_mode11_weight_ise_range >= BISE_12_LEVELS {
                    // Try constrained weights if we're allowed to use 12/16 level ISE weight modes
                    pack_mode11(
                        bph,
                        bpq,
                        all_results,
                        coptions,
                        coptions.first_mode11_weight_ise_range.max(BISE_12_LEVELS),
                        coptions.last_mode11_weight_ise_range,
                        true,
                    );
                }
                // Guarantee at least 1 non-solid result: fall back to weight ISE range 7.
                if all_results.len() == cur_num_results
                    && coptions.first_mode11_weight_ise_range == BISE_16_LEVELS
                {
                    pack_mode11(
                        bph,
                        bpq,
                        all_results,
                        coptions,
                        BISE_12_LEVELS,
                        BISE_12_LEVELS,
                        false,
                    );
                }
            }
            if coptions.use_mode7_part1 {
                // Mode 7 1-subset never requires endpoint quantization.
                pack_mode7_single_part(
                    bph,
                    bpq,
                    all_results,
                    coptions,
                    coptions.first_mode7_part1_weight_ise_range,
                    coptions.last_mode7_part1_weight_ise_range,
                );
            } else if is_greyscale {
                // Mode 11 has worse B channel quantization, so try mode 7 anyway.
                pack_mode7_single_part(bph, bpq, all_results, coptions, 1, 1);
                pack_mode7_single_part(
                    bph,
                    bpq,
                    all_results,
                    coptions,
                    UHDR_MODE7_PART1_LAST_ISE_RANGE,
                    UHDR_MODE7_PART1_LAST_ISE_RANGE,
                );
            }
        }

        let mut have_est = false;
        let mut best_parts = [0i32; TOTAL_ASTC_BC6H_COMMON_PARTITIONS2];
        if (coptions.use_mode7_part2 || coptions.use_mode11_part2)
            && coptions.use_estimated_partitions
        {
            have_est = estimate_partition(
                bph,
                &mut best_parts,
                coptions.max_estimated_partitions as usize,
            );
        }
        let num_est = if have_est {
            coptions.max_estimated_partitions as usize
        } else {
            0
        };

        if coptions.use_mode7_part2 {
            let cur_num_results = all_results.len();
            pack_2part(
                7,
                bph,
                bpq,
                all_results,
                coptions,
                num_est,
                &best_parts,
                coptions.first_mode7_part2_weight_ise_range,
                coptions.last_mode7_part2_weight_ise_range,
            );
            // Fall back to 4 levels, which doesn't require endpoint quantization.
            if all_results.len() == cur_num_results
                && coptions.first_mode7_part2_weight_ise_range >= BISE_5_LEVELS
            {
                pack_2part(
                    7,
                    bph,
                    bpq,
                    all_results,
                    coptions,
                    num_est,
                    &best_parts,
                    BISE_4_LEVELS,
                    BISE_4_LEVELS,
                );
            }
        }
        if coptions.use_mode11_part2 {
            // This always requires endpoint quant, so it could fail to find any usable solutions.
            pack_2part(
                11,
                bph,
                bpq,
                all_results,
                coptions,
                num_est,
                &best_parts,
                coptions.first_mode11_part2_weight_ise_range,
                coptions.last_mode11_part2_weight_ise_range,
            );
        }
        if coptions.refine_weights {
            for r in all_results.iter_mut() {
                let mut improved = false;
                astc_hdr_4x4_refine_weights(
                    rgb_pixels_half,
                    r,
                    coptions,
                    coptions.bc6h_err_weight,
                    &mut improved,
                );
                r.improved_via_refinement_flag = improved;
            }
        }
    }
    true
}

/// `astc_hdr_4x4_pack_results_to_block`.
pub fn astc_hdr_4x4_pack_results_to_block(
    dst_blk: &mut [u8; 16],
    results: &AstcHdr4x4PackResults,
) -> bool {
    if results.is_solid {
        *dst_blk = results.solid_blk;
        true
    } else {
        pack_astc_block(dst_blk, &results.best_blk)
    }
}

fn decode_hdr16(blk: &LogAstcBlock, out: &mut [[HalfFloat; 4]; 16]) -> bool {
    decode_block(blk, &mut DecodePixels::Hdr16(out), 4, 4)
}

/// `astc_hdr_4x4_refine_weights`: refines a block's chosen weight indices, balancing BC6H and
/// ASTC HDR error.
pub fn astc_hdr_4x4_refine_weights(
    source_block: &[HalfFloat; 48],
    cur_results: &mut AstcHdr4x4PackResults,
    coptions: &UastcHdr4x4CodecOptions,
    bc6h_weight: f32,
    improved_flag: &mut bool,
) -> bool {
    *improved_flag = false;
    if cur_results.is_solid {
        return true;
    }
    let total_weights = get_ise_levels(cur_results.best_blk.weight_ise_range as u32);
    let mut best_err = [BIG_FLOAT_VAL as f64; 16];
    let mut best_weight = [0u8; 16];
    let c_weights = [coptions.base.r_err_scale, coptions.base.g_err_scale, 1.0f32];

    for weight_index in 0..total_weights {
        let mut temp_blk = cur_results.best_blk;
        for w in temp_blk.weights.iter_mut().take(16) {
            *w = weight_index as u8;
        }
        let mut unpacked_astc = [[0 as HalfFloat; 4]; 16];
        decode_hdr16(&temp_blk, &mut unpacked_astc);
        let mut trial_bc6h_blk = [0u8; 16];
        astc_hdr_transcode_log_to_bc6h(&temp_blk, &mut trial_bc6h_blk);
        let mut unpacked_bc6h = [[0 as HalfFloat; 3]; 16];
        unpack_bc6h(&trial_bc6h_blk, &mut unpacked_bc6h, false);

        for i in 0..16 {
            let mut total_err = 0f64;
            for c in 0..3 {
                let orig_c_q = q(source_block[i * 3 + c], Q_LOG_BIAS_4X4);
                let astc_c_q = q(unpacked_astc[i][c], Q_LOG_BIAS_4X4);
                let d = astc_c_q - orig_c_q;
                let astc_e = (d * d) * c_weights[c] as f64;
                let bc6h_c_q = q(unpacked_bc6h[i][c], Q_LOG_BIAS_4X4);
                let d = bc6h_c_q - orig_c_q;
                let bc6h_e = (d * d) * c_weights[c] as f64;
                total_err += astc_e * (1.0 - bc6h_weight) as f64 + bc6h_e * bc6h_weight as f64;
            }
            if total_err < best_err[i] {
                best_err[i] = total_err;
                best_weight[i] = weight_index as u8;
            }
        }
    }

    let any_changed = cur_results.best_blk.weights[..16] != best_weight;
    if any_changed {
        cur_results.best_blk.weights[..16].copy_from_slice(&best_weight);
        let blk = cur_results.best_blk;
        astc_hdr_transcode_log_to_bc6h(&blk, &mut cur_results.bc6h_block);
        let mut unpacked_astc = [[0 as HalfFloat; 4]; 16];
        decode_hdr16(&blk, &mut unpacked_astc);
        let mut rgb = [[0 as HalfFloat; 3]; 16];
        for (d, s) in rgb.iter_mut().zip(unpacked_astc.iter()) {
            d.copy_from_slice(&s[..3]);
        }
        let mut src = [[0 as HalfFloat; 3]; 16];
        for (i, d) in src.iter_mut().enumerate() {
            d.copy_from_slice(&source_block[i * 3..i * 3 + 3]);
        }
        cur_results.best_block_error = compute_block_error(16, &src, &rgb, &coptions.base);
        *improved_flag = true;
    }
    true
}
