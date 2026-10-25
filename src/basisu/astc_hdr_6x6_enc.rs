//! Port of `encoder/basisu_astc_hdr_6x6_enc.cpp`: the ASTC HDR 6x6 / UASTC HDR 6x6 intermediate
//! encoder (`astc_6x6_hdr::compress_photo`).
//!
//! Float evaluation follows the SSE build (no FMA): f32 vector math in `vec<N, float>` order and
//! f64 where the C++ uses `double`. `powf` calls whose exponent LLVM would fold into `sqrt` are
//! routed through `std::hint::black_box` so the libm call matches GCC's.

// The float literals keep the C++ spelling (they round to the same f32).
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::excessive_precision
)]

use std::sync::OnceLock;

use super::astc_hdr_enc::{
    AstcHdrCodecBaseOptions, BIG_FLOAT_VAL, OptMode, Q_LOG_BIAS_6X6, compute_block_error,
    downsample_weight_grid, encode_astc_hdr_block_mode_7, encode_astc_hdr_block_mode_11,
    get_6x6_downsample_matrix, refine_endpoints,
};
use super::astc_helpers::{
    self as ah, DecodePixels, HalfFloat, LogAstcBlock, compute_texel_partition, decode_block,
    dequant_tables, float_to_half, get_num_cem_values, half_to_float, pack_astc_block,
};
use super::bc6h::{NUM_MODE7_ENDPOINTS, NUM_MODE11_ENDPOINTS};
use super::comp::{Image, ImageF};
use super::hdr_tables::{PART2_UNIQUE_INDEX_TO_SEED, PART3_UNIQUE_INDEX_TO_SEED};
use super::huffman_enc::BitwiseCoder;
use super::resample_filters::{BASISU_BELL_FILTER_SUPPORT, gaussian_filter};
use super::resampler::{BoundaryOp, ContribList, Resampler};

pub use super::astc_6x6_hdr::{NUM_UNIQUE_PARTITIONS2, NUM_UNIQUE_PARTITIONS3};

pub const ASTC_HDR_6X6_DEF_USER_COMP_LEVEL: u32 = 2;
pub const ASTC_HDR_6X6_MAX_USER_COMP_LEVEL: u32 = 12;
pub const ASTC_HDR_6X6_MAX_COMP_LEVEL: u32 = 4;
pub const NUM_REUSE_XY_DELTAS: u32 = 32;
pub const ASTC_HDR_MAX_VAL: f32 = 65216.0;
const SMALL_FLOAT_VAL: f32 = 0.0000125;

/// `basisu_astc_hdr_core.h` 6x6 block mode flags.
pub const BASIST_HDR_6X6_LEVEL0: u32 = 1;
pub const BASIST_HDR_6X6_LEVEL1: u32 = 2;
pub const BASIST_HDR_6X6_LEVEL2: u32 = 4;

/// `endpoint_mode`.
pub const EM_RAW: u32 = 0;
pub const EM_USE_LEFT: u32 = 1;
pub const EM_USE_UPPER: u32 = 2;
pub const EM_USE_LEFT_DELTA: u32 = 3;
pub const EM_USE_UPPER_DELTA: u32 = 4;
pub const EM_TOTAL: u32 = 5;
/// `block_mode::cBMTotalModes`.
pub const BM_TOTAL_MODES: u32 = 75;

pub const RUN_CODE: u32 = 0b000;
pub const RUN_CODE_LEN: u32 = 3;
pub const SOLID_CODE: u32 = 0b100;
pub const SOLID_CODE_LEN: u32 = 3;
pub const REUSE_CODE: u32 = 0b10;
pub const REUSE_CODE_LEN: u32 = 2;
pub const BLOCK_CODE: u32 = 0b1;
pub const BLOCK_CODE_LEN: u32 = 1;

pub type Vec3 = [f32; 3];

/// libm `powf` with an opaque exponent (GCC keeps `powf(x, .5f)`, LLVM would use `sqrt`).
#[inline]
fn powf_libm(x: f32, y: f32) -> f32 {
    x.powf(std::hint::black_box(y))
}

#[inline]
fn lerpf(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

#[inline]
fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline]
fn dot3(a: Vec3, b: Vec3) -> f32 {
    let mut r = a[0] * b[0];
    r += a[1] * b[1];
    r += a[2] * b[2];
    r
}

// ---------------------------------------------------------------------------------------------
// Global config

/// `astc_hdr_6x6_global_config` (debug image / output image options are not ported).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AstcHdr6x6GlobalConfig {
    pub rec2020_bt2100_color_gamut: bool,
    pub master_comp_level: u32,
    pub highest_comp_level: u32,
    pub lambda: f32,
    pub extra_patterns_flag: bool,
    pub brute_force_partition_matching: bool,
    pub jnd_optimization: bool,
    pub jnd_delta_itp_thresh: f32,
    pub force_one_strip: bool,
    pub gaussian1_fallback: bool,
    pub gaussian1_strength: f32,
    pub gaussian2_fallback: bool,
    pub gaussian2_strength: f32,
    pub disable_delta_endpoint_usage: bool,
    pub delta_itp_dark_adjustment: bool,
    pub debug_images: bool,
    pub output_images: bool,
    pub debug_output: bool,
    pub image_stats: bool,
    pub status_output: bool,
    pub deblocking_flag: bool,
    pub deblock_penalty_weight: f32,
    pub disable_twothree_subsets: bool,
    pub use_solid_blocks: bool,
    pub use_runs: bool,
    pub block_stat_optimizations_flag: bool,
    pub rdo_candidate_diversity_boost: bool,
    pub rdo_candidate_diversity_boost_bit_window_weight: f32,
    pub favor_higher_compression: bool,
    pub num_reuse_xy_deltas: u32,
    pub write_basisu_1_6_compatible_files: bool,
}

impl Default for AstcHdr6x6GlobalConfig {
    fn default() -> Self {
        AstcHdr6x6GlobalConfig {
            rec2020_bt2100_color_gamut: false,
            master_comp_level: 0,
            highest_comp_level: 1,
            lambda: 0.0,
            extra_patterns_flag: false,
            brute_force_partition_matching: false,
            jnd_optimization: false,
            jnd_delta_itp_thresh: 0.75,
            force_one_strip: false,
            gaussian1_fallback: true,
            gaussian1_strength: 1.45,
            gaussian2_fallback: true,
            gaussian2_strength: 1.83,
            disable_delta_endpoint_usage: false,
            delta_itp_dark_adjustment: true,
            debug_images: false,
            output_images: false,
            debug_output: false,
            image_stats: false,
            status_output: false,
            deblocking_flag: true,
            deblock_penalty_weight: 0.03,
            disable_twothree_subsets: false,
            use_solid_blocks: true,
            use_runs: true,
            block_stat_optimizations_flag: true,
            rdo_candidate_diversity_boost: true,
            rdo_candidate_diversity_boost_bit_window_weight: 1.2,
            favor_higher_compression: true,
            num_reuse_xy_deltas: NUM_REUSE_XY_DELTAS,
            write_basisu_1_6_compatible_files: true,
        }
    }
}

impl AstcHdr6x6GlobalConfig {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// `set_user_level` (0-12).
    pub fn set_user_level(&mut self, level: i32) {
        let level = level.clamp(0, ASTC_HDR_6X6_MAX_USER_COMP_LEVEL as i32);
        self.master_comp_level = 0;
        self.highest_comp_level = 0;
        self.num_reuse_xy_deltas = NUM_REUSE_XY_DELTAS;
        self.extra_patterns_flag = false;
        self.brute_force_partition_matching = false;
        let (m, h) = match level {
            0 => {
                // Both reduce compression a lot when lambda>0
                self.favor_higher_compression = false;
                self.num_reuse_xy_deltas = NUM_REUSE_XY_DELTAS / 2;
                (0, 0)
            }
            1 => (0, 0),
            2 => (0, 1),
            3 => (1, 1),
            4 => (1, 2),
            5 => (1, 3),
            6 => (1, 4),
            7 => (2, 2),
            8 => (2, 3),
            9 => (2, 4),
            10 => (3, 3),
            11 => (3, 4),
            _ => {
                self.extra_patterns_flag = true;
                self.brute_force_partition_matching = true;
                (4, 4)
            }
        };
        self.master_comp_level = m;
        self.highest_comp_level = h;
    }
}

// ---------------------------------------------------------------------------------------------
// PQ / ITP

const M1: f32 = 0.1593017578125;
const M2: f32 = 78.84375;
const C1: f32 = 0.8359375;
const C2: f32 = 18.8515625;
const C3: f32 = 18.6875;

/// `forwardPQ`.
fn forward_pq(y: f32) -> f32 {
    // 10,000 here is an absolute scale - it's in nits
    let l = y * (1.0 / 10000.0);
    let num = powf_libm(l, M1);
    powf_libm((C1 + C2 * num) / (1.0 + C3 * num), M2)
}

const PQ_APPROX_MIN_EXP: i32 = -16;
const PQ_APPROX_MAX_EXP: i32 = 16;
const PQ_APPROX_EXP_RANGE: usize = (PQ_APPROX_MAX_EXP - PQ_APPROX_MIN_EXP + 1) as usize;
const SMALLEST_PQ_VAL_IN: f32 = 0.000015258829080;
const SMALLEST_PQ_VAL: f32 = 0.000551903737;
const LARGEST_PQ_VAL: f32 = 1.251312;

#[inline]
fn bfloat16_to_float(b: u16) -> f32 {
    f32::from_bits((b as u32) << 16)
}

/// `float_to_bfloat16` with `round_flag = false` (truncation; denormals to 0).
#[inline]
fn float_to_bfloat16_trunc(input: f32) -> u16 {
    let u = input.to_bits();
    if (u >> 23) & 0xFF == 0 {
        return 0;
    }
    (u >> 16) as u16
}

#[inline]
fn bfloat16_init(sign: i32, exp: i32, mant: i32) -> u16 {
    let mut res: u16 = if sign < 0 { 0x8000 } else { 0 };
    res |= ((exp + 127) << 7) as u16;
    res |= mant as u16;
    res
}

/// `g_pq_approx_tabs` (`init_pq_tables`).
fn pq_tables() -> &'static [[f32; 128]; PQ_APPROX_EXP_RANGE] {
    static T: OnceLock<[[f32; 128]; PQ_APPROX_EXP_RANGE]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0f32; 128]; PQ_APPROX_EXP_RANGE];
        for exp in PQ_APPROX_MIN_EXP..=PQ_APPROX_MAX_EXP {
            for mant in 0..128 {
                let bf = bfloat16_to_float(bfloat16_init(1, exp, mant));
                t[(exp - PQ_APPROX_MIN_EXP) as usize][mant as usize] = forward_pq(bf);
            }
        }
        t
    })
}

/// `forwardPQTab`: bfloat16 table lookup with linear interpolation.
fn forward_pq_tab(v: f32) -> f32 {
    if v == 0.0 {
        return 0.0;
    }
    let t = pq_tables();
    let bf = float_to_bfloat16_trunc(v);
    let exp = ((bf >> 7) & 0xFF) as i32 - 127;
    if exp < PQ_APPROX_MIN_EXP {
        // not accurate but should be good enough for our uses
        let r = v / SMALLEST_PQ_VAL_IN;
        return lerpf(0.0, SMALLEST_PQ_VAL, if 1.0 < r { 1.0 } else { r });
    } else if exp > PQ_APPROX_MAX_EXP {
        return LARGEST_PQ_VAL;
    }
    let mant = (bf & 0x7F) as i32;
    let a = t[(exp - PQ_APPROX_MIN_EXP) as usize][mant as usize];
    let bf_f32 = bfloat16_to_float(bf);
    let mut next_mant = mant + 1;
    let mut next_exp = exp;
    if next_mant == 128 {
        next_mant = 0;
        next_exp += 1;
        if next_exp > PQ_APPROX_MAX_EXP {
            return a;
        }
    }
    let b = t[(next_exp - PQ_APPROX_MIN_EXP) as usize][next_mant as usize];
    let next_bf_f32 = bfloat16_to_float(bfloat16_init(1, next_exp, next_mant));
    let lerp_factor = (v - bf_f32) / (next_bf_f32 - bf_f32);
    lerpf(a, b, lerp_factor)
}

/// `forwardPQTab` (public for the tests).
pub fn forward_pq_tab_pub(v: f32) -> f32 {
    forward_pq_tab(v)
}

/// `linear_rgb_to_ictcp`: absolute linear REC 709 (or REC 2020) RGB to ICtCp / ITP.
pub fn linear_rgb_to_ictcp(rgb_in: Vec3, itp_flag: bool, rec2020_bt2100_color_gamut: bool) -> Vec3 {
    let c = rgb_in;
    let (l, m, s) = if !rec2020_bt2100_color_gamut {
        (
            c[0] * 0.2958097 + c[1] * 0.6230863 + c[2] * 0.0811040,
            c[0] * 0.1562512 + c[1] * 0.7272980 + c[2] * 0.1164508,
            c[0] * 0.0351435 + c[1] * 0.1565601 + c[2] * 0.8082964,
        )
    } else {
        (
            0.412109375 * c[0] + 0.52392578125 * c[1] + 0.06396484375 * c[2],
            0.166748046875 * c[0] + 0.720458984375 * c[1] + 0.11279296875 * c[2],
            0.024169921875 * c[0] + 0.075439453125 * c[1] + 0.900390625 * c[2],
        )
    };
    let ld = forward_pq_tab(l);
    let md = forward_pq_tab(m);
    let sd = forward_pq_tab(s);
    let i = 0.5 * ld + 0.5 * md;
    let t = if itp_flag {
        0.806884765625 * ld + -1.6617431640625 * md + 0.8548583984375 * sd
    } else {
        1.61376953125 * ld + -3.323486328125 * md + 1.709716796875 * sd
    };
    let p = 4.378173828125 * ld + -4.24560546875 * md + -0.132568359375 * sd;
    [i, t, p]
}

/// `linear_rgb_to_itp`.
#[inline]
pub fn linear_rgb_to_itp(rgb_in: Vec3, cfg: &AstcHdr6x6GlobalConfig) -> Vec3 {
    linear_rgb_to_ictcp(rgb_in, true, cfg.rec2020_bt2100_color_gamut)
}

// ---------------------------------------------------------------------------------------------
// Partition patterns and the vantage point tree

const NUM_PART3_MAPPINGS: usize = 6;
const PART3_MAPPING: [[u8; 3]; NUM_PART3_MAPPINGS] = [
    [0, 1, 2],
    [1, 2, 0],
    [2, 0, 1],
    [0, 2, 1],
    [1, 0, 2],
    [2, 1, 0],
];

/// `partition_pattern_vec`: a 6x6 subset index pattern.
pub type PartitionPatternVec = [u8; 36];

/// `get_squared_distance` (actually the L1 distance).
#[inline]
fn pat_squared_distance(a: &PartitionPatternVec, b: &PartitionPatternVec) -> i32 {
    let mut total = 0;
    for i in 0..36 {
        total += (a[i] as i32 - b[i] as i32).abs();
    }
    total
}

#[inline]
fn pat_distance(a: &PartitionPatternVec, b: &PartitionPatternVec) -> f32 {
    (pat_squared_distance(a, b) as f32).sqrt()
}

fn pat_permuted2(p: &PartitionPatternVec, permute_index: u8) -> PartitionPatternVec {
    p.map(|v| v ^ permute_index)
}

fn pat_permuted3(p: &PartitionPatternVec, permute_index: usize) -> PartitionPatternVec {
    p.map(|v| PART3_MAPPING[permute_index][v as usize % 3])
}

#[derive(Debug, Clone, Copy)]
struct VpTreeNode {
    vantage_point: PartitionPatternVec,
    point_index: u32,
    dist: f32,
    inner_node: i32,
    outer_node: i32,
}

/// `vp_tree::result`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VpResult {
    pub pat_index: u32,
    pub mapping_index: u32,
    pub dist: f32,
}

/// `vp_tree::result_queue`: a bounded max-heap (1-based array) of the nearest results.
#[derive(Debug, Clone)]
pub struct ResultQueue {
    elements: [VpResult; 259],
    cur_size: usize,
}

impl Default for ResultQueue {
    fn default() -> Self {
        ResultQueue {
            elements: [VpResult::default(); 259],
            cur_size: 0,
        }
    }
}

impl ResultQueue {
    const MAX_SUPPORTED_SIZE: usize = 256 + 1;

    pub fn get_size(&self) -> usize {
        self.cur_size
    }

    /// The heap array (`elements[1..=size]` hold the results, in heap order).
    pub fn get_elements(&self) -> &[VpResult] {
        &self.elements
    }

    pub fn clear(&mut self) {
        self.cur_size = 0;
    }

    fn top(&self) -> &VpResult {
        &self.elements[1]
    }

    pub fn insert(&mut self, val: VpResult, max_size: usize) -> bool {
        if self.cur_size >= Self::MAX_SUPPORTED_SIZE {
            return false;
        }
        self.cur_size += 1;
        self.elements[self.cur_size] = val;
        self.up_heap(self.cur_size);
        if self.cur_size > max_size {
            self.pop();
        }
        true
    }

    pub fn pop(&mut self) -> bool {
        if self.cur_size == 0 {
            return false;
        }
        self.elements[1] = self.elements[self.cur_size];
        self.cur_size -= 1;
        self.down_heap(1);
        true
    }

    pub fn get_highest_dist(&self) -> f32 {
        if self.cur_size == 0 {
            0.0
        } else {
            self.top().dist
        }
    }

    fn up_heap(&mut self, mut index: usize) {
        while index > 1 && self.elements[index].dist > self.elements[index >> 1].dist {
            self.elements.swap(index, index >> 1);
            index >>= 1;
        }
    }

    fn down_heap(&mut self, mut index: usize) {
        loop {
            let mut largest = index;
            let (left_child, right_child) = (2 * index, 2 * index + 1);
            if left_child <= self.cur_size
                && self.elements[left_child].dist > self.elements[largest].dist
            {
                largest = left_child;
            }
            if right_child <= self.cur_size
                && self.elements[right_child].dist > self.elements[largest].dist
            {
                largest = right_child;
            }
            if largest == index {
                break;
            }
            self.elements.swap(index, largest);
            index = largest;
        }
    }
}

/// `vp_tree` over unique partition patterns.
#[derive(Debug, Clone, Default)]
pub struct VpTree {
    nodes: Vec<VpTreeNode>,
}

impl VpTree {
    /// `init`: requires all patterns to be unique.
    pub fn init(&mut self, pats: &[PartitionPatternVec]) -> bool {
        self.nodes.clear();
        let n = pats.len();
        let pat_indices: Vec<u32> = (0..n as u32).collect();
        let (root, root_dist) = Self::find_best_vantage_point(pats, &pat_indices);
        if root < 0 {
            return false;
        }
        self.nodes.push(VpTreeNode {
            vantage_point: pats[root as usize],
            point_index: root as u32,
            dist: root_dist,
            inner_node: -1,
            outer_node: -1,
        });
        let (mut inner_list, mut outer_list) = (Vec::new(), Vec::new());
        for pat_index in 0..n as u32 {
            if pat_index as i32 == root {
                continue;
            }
            let dist = pat_distance(&self.nodes[0].vantage_point, &pats[pat_index as usize]);
            if dist <= root_dist {
                inner_list.push(pat_index);
            } else {
                outer_list.push(pat_index);
            }
        }
        if !inner_list.is_empty() {
            let r = self.create_node(pats, &inner_list);
            self.nodes[0].inner_node = r;
            if r < 0 {
                return false;
            }
        }
        if !outer_list.is_empty() {
            let r = self.create_node(pats, &outer_list);
            self.nodes[0].outer_node = r;
            if r < 0 {
                return false;
            }
        }
        true
    }

    /// `find_nearest`: the `max_results` nearest patterns over all subset permutations.
    pub fn find_nearest(
        &self,
        num_subsets: u32,
        desired_pat: &PartitionPatternVec,
        results: &mut ResultQueue,
        max_results: usize,
    ) {
        results.clear();
        if self.nodes.is_empty() {
            return;
        }
        let mut desired_pats = [[0u8; 36]; NUM_PART3_MAPPINGS];
        let num_desired_pats = if num_subsets == 2 {
            for i in 0..2 {
                desired_pats[i] = pat_permuted2(desired_pat, i as u8);
            }
            2
        } else {
            for i in 0..NUM_PART3_MAPPINGS {
                desired_pats[i] = pat_permuted3(desired_pat, i);
            }
            NUM_PART3_MAPPINGS
        };
        self.find_nearest_at_node_non_recursive(
            0,
            &desired_pats[..num_desired_pats],
            results,
            max_results,
        );
    }

    fn find_nearest_at_node_non_recursive(
        &self,
        init_node_index: i32,
        desired_pats: &[PartitionPatternVec],
        results: &mut ResultQueue,
        max_results: usize,
    ) {
        let mut node_stack: Vec<i32> = Vec::with_capacity(16);
        node_stack.push(init_node_index);
        while let Some(node_index) = node_stack.pop() {
            let node = &self.nodes[node_index as usize];
            let mut best_dist_to_vantage = BIG_FLOAT_VAL;
            let mut best_mapping = 0;
            for (i, p) in desired_pats.iter().enumerate() {
                let dist = pat_distance(p, &node.vantage_point);
                if dist < best_dist_to_vantage {
                    best_dist_to_vantage = dist;
                    best_mapping = i as u32;
                }
            }
            results.insert(
                VpResult {
                    dist: best_dist_to_vantage,
                    mapping_index: best_mapping,
                    pat_index: node.point_index,
                },
                max_results,
            );
            if best_dist_to_vantage <= node.dist {
                if node.outer_node >= 0
                    && (results.get_size() < max_results
                        || (node.dist - best_dist_to_vantage) <= results.get_highest_dist())
                {
                    node_stack.push(node.outer_node);
                }
                // inner first
                if node.inner_node >= 0 {
                    node_stack.push(node.inner_node);
                }
            } else {
                if node.inner_node >= 0
                    && (results.get_size() < max_results
                        || (best_dist_to_vantage - node.dist) <= results.get_highest_dist())
                {
                    node_stack.push(node.inner_node);
                }
                // outer first
                if node.outer_node >= 0 {
                    node_stack.push(node.outer_node);
                }
            }
        }
    }

    /// `create_node`: returns the new node index, or -1 on error.
    fn create_node(&mut self, pats: &[PartitionPatternVec], pat_indices: &[u32]) -> i32 {
        let (root, root_dist) = Self::find_best_vantage_point(pats, pat_indices);
        if root < 0 {
            return -1;
        }
        self.nodes.push(VpTreeNode {
            vantage_point: pats[root as usize],
            point_index: root as u32,
            dist: root_dist,
            inner_node: -1,
            outer_node: -1,
        });
        let new_node_index = self.nodes.len() - 1;
        let (mut inner_list, mut outer_list) = (Vec::new(), Vec::new());
        for &pat_index in pat_indices {
            if pat_index as i32 == root {
                continue;
            }
            let dist = pat_distance(
                &self.nodes[new_node_index].vantage_point,
                &pats[pat_index as usize],
            );
            if dist <= root_dist {
                inner_list.push(pat_index);
            } else {
                outer_list.push(pat_index);
            }
        }
        if !inner_list.is_empty() {
            let r = self.create_node(pats, &inner_list);
            self.nodes[new_node_index].inner_node = r;
        }
        if !outer_list.is_empty() {
            let r = self.create_node(pats, &outer_list);
            self.nodes[new_node_index].outer_node = r;
        }
        new_node_index as i32
    }

    /// `find_best_vantage_point`: (pattern index or -1, split distance).
    fn find_best_vantage_point(pats: &[PartitionPatternVec], pat_indices: &[u32]) -> (i32, f32) {
        let n = pat_indices.len();
        if n == 1 {
            return (pat_indices[0] as i32, 0.0);
        }
        let mut best_split_metric = -1.0f32;
        let mut best_split_pat = -1i32;
        let mut best_split_dist = 0.0f32;
        let mut best_split_var = 0.0f32;
        let mut dists: Vec<f32> = Vec::with_capacity(n);
        for &split_pat_index in pat_indices {
            let trial_vantage = &pats[split_pat_index as usize];
            dists.clear();
            for &pat_index in pat_indices {
                if pat_index == split_pat_index {
                    continue;
                }
                dists.push(pat_distance(trial_vantage, &pats[pat_index as usize]));
            }
            // stats<double>::calc variance
            let nd = dists.len() as f64;
            let mut total = 0f64;
            for &d in &dists {
                total += d as f64;
            }
            let avg = total / nd;
            let mut var = 0f64;
            for &d in &dists {
                let dd = d as f64 - avg;
                var += dd * dd;
            }
            var /= nd;
            // Only the median value matters, so any sort order of equal keys is fine.
            let mut sorted = dists.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let num_dists = sorted.len();
            let mut split_dist = sorted[num_dists / 2];
            if num_dists & 1 == 0 {
                split_dist = (split_dist + sorted[num_dists / 2 - 1]) * 0.5;
            }
            let (mut total_inner, mut total_outer) = (0u32, 0u32);
            for &d in &dists {
                if d <= split_dist {
                    total_inner += 1;
                } else {
                    total_outer += 1;
                }
            }
            let split_metric =
                total_inner.min(total_outer) as f32 / total_inner.max(total_outer) as f32;
            if split_metric > best_split_metric
                || (split_metric == best_split_metric && var > best_split_var as f64)
            {
                best_split_metric = split_metric;
                best_split_dist = split_dist;
                best_split_pat = split_pat_index as i32;
                best_split_var = var as f32;
            }
        }
        (best_split_pat, best_split_dist)
    }
}

/// Global 6x6 encoder tables (`global_init`).
pub struct Hdr6x6EncTables {
    pub partitions2: Vec<PartitionPatternVec>,
    /// `g_part2_seed_to_unique_index` (0 for unused seeds, as in C++).
    pub part2_seed_to_unique_index: [i32; 1024],
    pub part2_vp_tree: VpTree,
    pub partitions3: Vec<PartitionPatternVec>,
    /// `g_part3_seed_to_unique_index` (-1 for unused seeds).
    pub part3_seed_to_unique_index: [i32; 1024],
    pub part3_vp_tree: VpTree,
    /// `g_contrib_lists[1..=6]` (index 0 unused).
    pub contrib_lists: Vec<Vec<ContribList>>,
}

/// `astc_6x6_hdr::global_init` tables (built once).
pub fn hdr6x6_tables() -> &'static Hdr6x6EncTables {
    static T: OnceLock<Hdr6x6EncTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut partitions2 = vec![[0u8; 36]; NUM_UNIQUE_PARTITIONS2 as usize];
        let mut part2_seed_to_unique_index = [0i32; 1024];
        for (unique_index, pat) in partitions2.iter_mut().enumerate() {
            let seed_index = PART2_UNIQUE_INDEX_TO_SEED[unique_index] as u32;
            part2_seed_to_unique_index[seed_index as usize & 1023] = unique_index as i32;
            for y in 0..6 {
                for x in 0..6 {
                    pat[(x + y * 6) as usize] =
                        compute_texel_partition(seed_index, x, y, 0, 2, false) as u8;
                }
            }
        }
        let mut part2_vp_tree = VpTree::default();
        part2_vp_tree.init(&partitions2);

        // The unique 3 subset patterns (up to permutation) in seed order, which is what
        // `g_part3_unique_index_to_seed` lists.
        let mut partitions3 = vec![[0u8; 36]; NUM_UNIQUE_PARTITIONS3 as usize];
        let mut part3_seed_to_unique_index = [-1i32; 1024];
        for (unique_index, pat) in partitions3.iter_mut().enumerate() {
            let seed_index = PART3_UNIQUE_INDEX_TO_SEED[unique_index] as u32;
            part3_seed_to_unique_index[seed_index as usize & 1023] = unique_index as i32;
            for y in 0..6 {
                for x in 0..6 {
                    pat[(x + y * 6) as usize] =
                        compute_texel_partition(seed_index, x, y, 0, 3, false) as u8;
                }
            }
        }
        let mut part3_vp_tree = VpTree::default();
        part3_vp_tree.init(&partitions3);

        let mut contrib_lists = vec![Vec::new()];
        for dst_width in 1..=6u32 {
            contrib_lists.push(
                Resampler::make_clist(
                    6,
                    6,
                    BoundaryOp::Clamp,
                    gaussian_filter,
                    BASISU_BELL_FILTER_SUPPORT,
                    6.0 / dst_width as f32,
                    0.0,
                )
                .unwrap_or_default(),
            );
        }
        Hdr6x6EncTables {
            partitions2,
            part2_seed_to_unique_index,
            part2_vp_tree,
            partitions3,
            part3_seed_to_unique_index,
            part3_vp_tree,
            contrib_lists,
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Partition estimation

/// `bu_math::inv_sqrt` (fast approximation).
#[inline]
fn inv_sqrt(v: f32) -> f32 {
    let f = f32::from_bits(0x5F1F_FFF9u32.wrapping_sub(v.to_bits() >> 1));
    0.703952253 * f * (2.38924456 - v * (f * f))
}

#[inline]
fn vec3_norm_approx(axis: Vec3) -> Vec3 {
    let l = dot3(axis, axis);
    if l.abs() >= SMALL_FLOAT_VAL {
        let s = inv_sqrt(l);
        [axis[0] * s, axis[1] * s, axis[2] * s]
    } else {
        [0.577350269; 3]
    }
}

#[inline]
fn maxf(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn half_as_float3(p: &[HalfFloat; 3]) -> Vec3 {
    [p[0] as f32, p[1] as f32, p[2] as f32]
}

/// `estimate_partition2_6x6`: the `num_best_parts` closest 2 subset patterns (vp tree heap order).
pub fn estimate_partition2_6x6(
    block_pixels_half: &[[HalfFloat; 3]; 36],
    best_parts: &mut [i32],
    num_best_parts: usize,
) -> bool {
    let t = hdr6x6_tables();
    let mut training_vecs = [[0f32; 3]; 36];
    let mut mean = [0f32; 3];
    for (v, p) in training_vecs.iter_mut().zip(block_pixels_half.iter()) {
        *v = half_as_float3(p);
        for c in 0..3 {
            mean[c] += v[c];
        }
    }
    for c in mean.iter_mut() {
        *c *= 1.0 / 36.0;
    }
    let mut max_vals = [-BIG_FLOAT_VAL; 3];
    for v in &training_vecs {
        for c in 0..3 {
            max_vals[c] = maxf(max_vals[c], v[c]);
        }
    }
    // Initialize principle axis approximation
    let mut axis = [
        max_vals[0] - mean[0],
        max_vals[1] - mean[1],
        max_vals[2] - mean[2],
    ];
    // Incremental approx. PCA
    for v in &training_vecs {
        axis = vec3_norm_approx(axis);
        let color = [v[0] - mean[0], v[1] - mean[1], v[2] - mean[2]];
        let d = dot3(color, axis);
        for c in 0..3 {
            axis[c] += color[c] * d;
        }
    }
    if dot3(axis, axis) < SMALL_FLOAT_VAL {
        axis = [0.57735027; 3];
    } else {
        let len = dot3(axis, axis).sqrt();
        if len != 0.0 {
            let s = 1.0 / len;
            axis = [axis[0] * s, axis[1] * s, axis[2] * s];
        }
    }
    let mut desired_part = [0u8; 36];
    for (i, v) in training_vecs.iter().enumerate() {
        let proj = dot3([v[0] - mean[0], v[1] - mean[1], v[2] - mean[2]], axis);
        desired_part[i] = u8::from(proj < 0.0);
    }
    let mut results = ResultQueue::default();
    t.part2_vp_tree
        .find_nearest(2, &desired_part, &mut results, num_best_parts);
    let elements = results.get_elements();
    for i in 0..results.get_size() {
        best_parts[i] = elements[1 + i].pat_index as i32;
    }
    true
}

#[inline]
fn sqdist3(a: Vec3, b: Vec3) -> f32 {
    let mut d2 = 0f32;
    for i in 0..3 {
        let d = a[i] - b[i];
        d2 += d * d;
    }
    d2
}

/// `estimate_partition3_6x6`: k-means seeded 3 subset estimate, nearest patterns via the vp tree.
pub fn estimate_partition3_6x6(
    block_pixels_half: &[[HalfFloat; 3]; 36],
    best_parts: &mut [i32],
    num_best_parts: usize,
) -> bool {
    let t = hdr6x6_tables();
    let mut training_vecs = [[0f32; 3]; 36];
    let mut brightest_inten = 0f32;
    let mut darkest_inten = BIG_FLOAT_VAL;
    let mut cluster_centroids = [[0f32; 3]; 3];
    for (v, p) in training_vecs.iter_mut().zip(block_pixels_half.iter()) {
        *v = half_as_float3(p);
        let inten = dot3(*v, [1.0; 3]);
        if inten < darkest_inten {
            darkest_inten = inten;
            cluster_centroids[0] = *v;
        }
        if inten > brightest_inten {
            brightest_inten = inten;
            cluster_centroids[1] = *v;
        }
    }
    if cluster_centroids[0] == cluster_centroids[1] {
        return false;
    }
    let mut furthest_dist2 = 0f32;
    for v in &training_vecs {
        let dist_a = sqdist3(*v, cluster_centroids[0]);
        if dist_a == 0.0 {
            continue;
        }
        let dist_b = sqdist3(*v, cluster_centroids[1]);
        if dist_b == 0.0 {
            continue;
        }
        let dist2 = dist_a + dist_b;
        if dist2 > furthest_dist2 {
            furthest_dist2 = dist2;
            cluster_centroids[2] = *v;
        }
    }
    if cluster_centroids[0] == cluster_centroids[2] || cluster_centroids[1] == cluster_centroids[2]
    {
        return false;
    }
    let mut cluster_pixels = [[0usize; 36]; 3];
    let mut num_cluster_pixels = [0usize; 3];
    for _ in 0..4 {
        num_cluster_pixels = [0; 3];
        let mut new_cluster_means = [[0f32; 3]; 3];
        for (i, v) in training_vecs.iter().enumerate() {
            let d = [
                sqdist3(*v, cluster_centroids[0]),
                sqdist3(*v, cluster_centroids[1]),
                sqdist3(*v, cluster_centroids[2]),
            ];
            let mut min_d = d[0];
            let mut min_idx = 0;
            for j in 1..3 {
                if d[j] < min_d {
                    min_d = d[j];
                    min_idx = j;
                }
            }
            cluster_pixels[min_idx][num_cluster_pixels[min_idx]] = i;
            for c in 0..3 {
                new_cluster_means[min_idx][c] += v[c];
            }
            num_cluster_pixels[min_idx] += 1;
        }
        for j in 0..3 {
            if num_cluster_pixels[j] == 0 {
                return false;
            }
            let n = num_cluster_pixels[j] as f32;
            cluster_centroids[j] = [
                new_cluster_means[j][0] / n,
                new_cluster_means[j][1] / n,
                new_cluster_means[j][2] / n,
            ];
        }
    }
    let mut desired_part = [0u8; 36];
    for p in 0..3 {
        for &pix_index in cluster_pixels[p].iter().take(num_cluster_pixels[p]) {
            desired_part[pix_index] = p as u8;
        }
    }
    let mut results = ResultQueue::default();
    t.part3_vp_tree
        .find_nearest(3, &desired_part, &mut results, num_best_parts);
    let elements = results.get_elements();
    for i in 0..results.get_size() {
        best_parts[i] = elements[1 + i].pat_index as i32;
    }
    true
}

// ---------------------------------------------------------------------------------------------
// 2 / 3 subset block encoders

/// `MIN_REFINE_LEVEL` is 0, so the C++ `comp_level >= MIN_REFINE_LEVEL` checks always pass.
const _MIN_REFINE_LEVEL: u32 = 0;

/// `trial_result`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrialResult {
    pub log_blk: LogAstcBlock,
    pub err: f64,
    pub valid: bool,
}

/// Subset texels of a 6x6 partition pattern: (q16, half, pixel offsets, counts).
#[allow(clippy::type_complexity)]
fn split_subsets<const N: usize>(
    pat: &PartitionPatternVec,
    block_pixels_half: &[[HalfFloat; 3]; 36],
    block_pixels_q16: &[[f32; 4]; 36],
) -> (
    [[[f32; 4]; 36]; N],
    [[[HalfFloat; 3]; 36]; N],
    [[u8; 36]; N],
    [usize; N],
) {
    let mut part_pixels_q16 = [[[0f32; 4]; 36]; N];
    let mut part_half_pixels = [[[0 as HalfFloat; 3]; 36]; N];
    let mut part_pixel_index = [[0u8; 36]; N];
    let mut part_total_pixels = [0usize; N];
    for i in 0..36 {
        let p = pat[i] as usize % N;
        let l = part_total_pixels[p];
        part_pixels_q16[p][l] = block_pixels_q16[i];
        part_half_pixels[p][l] = block_pixels_half[i];
        part_pixel_index[p][l] = i as u8;
        part_total_pixels[p] = l + 1;
    }
    (
        part_pixels_q16,
        part_half_pixels,
        part_pixel_index,
        part_total_pixels,
    )
}

/// Downsamples [6][6] ISE weights to a grid via the 6x6 downsample matrix.
fn downsample_to_grid(
    ise_weights: &[u8; 36],
    weights_ise_range: u32,
    grid_w: u32,
    grid_h: u32,
    dst: &mut [u8],
) -> bool {
    let tabs = dequant_tables().weight_tab(weights_ise_range);
    let mut desired_weights = [0u8; 36];
    for i in 0..36 {
        desired_weights[i] = tabs
            .ise_to_val
            .get(ise_weights[i] as usize)
            .copied()
            .unwrap_or(0);
    }
    let Some(matrix) = get_6x6_downsample_matrix(grid_w, grid_h) else {
        return false;
    };
    let mut downsampled_weights = [0u8; 36];
    downsample_weight_grid(
        matrix,
        6,
        6,
        grid_w,
        grid_h,
        &desired_weights,
        &mut downsampled_weights,
    );
    for i in 0..(grid_w * grid_h) as usize {
        dst[i] = tabs
            .val_to_ise
            .get(downsampled_weights[i] as usize)
            .copied()
            .unwrap_or(0);
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn encode_subset(
    cem: u32,
    num_pixels: usize,
    half: &[[HalfFloat; 3]],
    q16: &[[f32; 4]],
    weight_ise_range: u32,
    endpoint_ise_range: u32,
    submode: &mut u32,
    endpoints: &mut [u8],
    weights: &mut [u8],
    coptions: &AstcHdrCodecBaseOptions,
    uber_mode_flag: bool,
    mode11_opt_mode: OptMode,
) -> f64 {
    if cem == 7 {
        encode_astc_hdr_block_mode_7(
            num_pixels,
            half,
            q16,
            weight_ise_range,
            submode,
            BIG_FLOAT_VAL as f64,
            endpoints,
            weights,
            coptions,
            endpoint_ise_range,
            0,
            5,
            None,
        )
    } else {
        encode_astc_hdr_block_mode_11(
            num_pixels,
            half,
            q16,
            weight_ise_range,
            submode,
            BIG_FLOAT_VAL as f64,
            endpoints,
            weights,
            coptions,
            false,
            endpoint_ise_range,
            uber_mode_flag,
            false,
            -1,
            7,
            false,
            mode11_opt_mode,
            None,
        )
    }
}

/// `encode_block_2_subsets`: res[0] is the plain encode, res[1] (if valid) the refined one.
#[allow(clippy::too_many_arguments)]
pub fn encode_block_2_subsets(
    res: &mut [TrialResult; 2],
    grid_w: u32,
    grid_h: u32,
    cem: u32,
    weights_ise_range: u32,
    endpoints_ise_range: u32,
    block_pixels_half: &[[HalfFloat; 3]; 36],
    block_pixels_q16: &[[f32; 4]; 36],
    coptions: &AstcHdrCodecBaseOptions,
    uber_mode_flag: bool,
    unique_pat_index: i32,
    _comp_level: u32,
    mode11_opt_mode: OptMode,
    refine_endpoints_flag: bool,
) -> bool {
    let t = hdr6x6_tables();
    let num_endpoint_vals = if cem == 11 {
        NUM_MODE11_ENDPOINTS
    } else {
        NUM_MODE7_ENDPOINTS
    };
    res[0].valid = false;
    res[1].valid = false;

    let mut best_log_blk = LogAstcBlock {
        num_partitions: 2,
        grid_width: grid_w as u8,
        grid_height: grid_h as u8,
        weight_ise_range: weights_ise_range as u8,
        endpoint_ise_range: endpoints_ise_range as u8,
        ..Default::default()
    };
    best_log_blk.color_endpoint_modes[0] = cem as u8;
    best_log_blk.color_endpoint_modes[1] = cem as u8;

    let Some(pat) = t.partitions2.get(unique_pat_index as usize) else {
        return false;
    };
    let p_seed = PART2_UNIQUE_INDEX_TO_SEED[unique_pat_index as usize];
    let (part_pixels_q16, part_half_pixels, part_pixel_index, part_total_pixels) =
        split_subsets::<2>(pat, block_pixels_half, block_pixels_q16);

    let mut blk_endpoints = [[0u8; NUM_MODE11_ENDPOINTS]; 2];
    let mut blk_weights = [[0u8; 36]; 2];
    let mut best_submode = [0u32; 2];
    for p in 0..2 {
        let e = encode_subset(
            cem,
            part_total_pixels[p],
            &part_half_pixels[p],
            &part_pixels_q16[p],
            weights_ise_range,
            endpoints_ise_range,
            &mut best_submode[p],
            &mut blk_endpoints[p],
            &mut blk_weights[p],
            coptions,
            uber_mode_flag,
            mode11_opt_mode,
        );
        if e == BIG_FLOAT_VAL as f64 {
            return false;
        }
    }

    let mut ise_weights = [0u8; 36];
    let mut src_pixel_index = [0usize; 2];
    for i in 0..36 {
        let p = pat[i] as usize & 1;
        ise_weights[i] = blk_weights[p][src_pixel_index[p]];
        src_pixel_index[p] += 1;
    }

    best_log_blk.partition_id = p_seed;
    best_log_blk.endpoints[..num_endpoint_vals]
        .copy_from_slice(&blk_endpoints[0][..num_endpoint_vals]);
    best_log_blk.endpoints[num_endpoint_vals..2 * num_endpoint_vals]
        .copy_from_slice(&blk_endpoints[1][..num_endpoint_vals]);
    if grid_w == 6 && grid_h == 6 {
        best_log_blk.weights[..36].copy_from_slice(&ise_weights);
        res[0].valid = true;
        res[0].log_blk = best_log_blk;
    } else {
        if !downsample_to_grid(
            &ise_weights,
            weights_ise_range,
            grid_w,
            grid_h,
            &mut best_log_blk.weights,
        ) {
            return false;
        }
        res[0].valid = true;
        res[0].log_blk = best_log_blk;
        if refine_endpoints_flag && (grid_w < 6 || grid_h < 6) {
            let mut any_refined = false;
            for p in 0..2 {
                let weights = best_log_blk.weights;
                let refine_status = refine_endpoints(
                    cem,
                    endpoints_ise_range,
                    &mut best_log_blk.endpoints[p * num_endpoint_vals..],
                    6,
                    6,
                    grid_w,
                    grid_h,
                    &weights,
                    weights_ise_range,
                    part_total_pixels[p],
                    &part_half_pixels[p],
                    &part_pixels_q16[p],
                    Some(&part_pixel_index[p]),
                    coptions,
                    mode11_opt_mode,
                );
                if refine_status {
                    any_refined = true;
                }
            }
            if any_refined {
                res[1].valid = true;
                res[1].log_blk = best_log_blk;
            }
        }
    }
    true
}

/// Decodes a log block to 6x6 RGB halves.
fn decode_block_half3(blk: &LogAstcBlock, out: &mut [[HalfFloat; 3]; 36]) -> bool {
    let mut h4 = [[0 as HalfFloat; 4]; 36];
    if !decode_block(blk, &mut DecodePixels::Hdr16(&mut h4), 6, 6) {
        return false;
    }
    for (d, s) in out.iter_mut().zip(h4.iter()) {
        d.copy_from_slice(&s[..3]);
    }
    true
}

/// `encode_block_3_subsets`: tries the estimated (or all) 3 subset patterns, keeping the best.
#[allow(clippy::too_many_arguments)]
pub fn encode_block_3_subsets(
    res: &mut TrialResult,
    cem: u32,
    grid_w: u32,
    grid_h: u32,
    weights_ise_range: u32,
    endpoints_ise_range: u32,
    block_pixels_half: &[[HalfFloat; 3]; 36],
    block_pixels_q16: &[[f32; 4]; 36],
    coptions: &AstcHdrCodecBaseOptions,
    uber_mode_flag: bool,
    est_patterns: &[i32],
    num_est_patterns: usize,
    _comp_level: u32,
    mode11_opt_mode: OptMode,
) -> bool {
    let t = hdr6x6_tables();
    let num_endpoint_vals = get_num_cem_values(cem) as usize;
    res.valid = false;
    let mut best_e = BIG_FLOAT_VAL as f64;
    let mut best_log_blk = LogAstcBlock {
        num_partitions: 3,
        grid_width: grid_w as u8,
        grid_height: grid_h as u8,
        weight_ise_range: weights_ise_range as u8,
        endpoint_ise_range: endpoints_ise_range as u8,
        ..Default::default()
    };
    best_log_blk.color_endpoint_modes[..3].copy_from_slice(&[cem as u8; 3]);

    let n = if num_est_patterns != 0 {
        num_est_patterns
    } else {
        NUM_UNIQUE_PARTITIONS3 as usize
    };
    for unique_p_iter in 0..n {
        let unique_part_index = if num_est_patterns != 0 {
            est_patterns[unique_p_iter] as usize
        } else {
            unique_p_iter
        };
        let Some(pat) = t.partitions3.get(unique_part_index) else {
            continue;
        };
        let (part_pixels_q16, part_half_pixels, part_pixel_index, part_total_pixels) =
            split_subsets::<3>(pat, block_pixels_half, block_pixels_q16);
        let mut blk_endpoints = [[0u8; NUM_MODE11_ENDPOINTS]; 3];
        let mut blk_weights = [[0u8; 36]; 3];
        let mut best_submode = [0u32; 3];
        let mut failed_flag = false;
        let mut e = 0f64;
        for p in 0..3 {
            let part_e = encode_subset(
                cem,
                part_total_pixels[p],
                &part_half_pixels[p],
                &part_pixels_q16[p],
                weights_ise_range,
                endpoints_ise_range,
                &mut best_submode[p],
                &mut blk_endpoints[p],
                &mut blk_weights[p],
                coptions,
                uber_mode_flag,
                mode11_opt_mode,
            );
            if part_e == BIG_FLOAT_VAL as f64 {
                failed_flag = true;
                break;
            }
            e += part_e;
        }
        if failed_flag {
            continue;
        }
        let mut ise_weights = [0u8; 36];
        let mut src_pixel_index = [0usize; 3];
        for i in 0..36 {
            let p = pat[i] as usize % 3;
            ise_weights[i] = blk_weights[p][src_pixel_index[p]];
            src_pixel_index[p] += 1;
        }
        let seed = PART3_UNIQUE_INDEX_TO_SEED[unique_part_index];
        if grid_w == 6 && grid_h == 6 {
            if e < best_e {
                best_e = e;
                best_log_blk.partition_id = seed;
                for p in 0..3 {
                    best_log_blk.endpoints[num_endpoint_vals * p..num_endpoint_vals * (p + 1)]
                        .copy_from_slice(&blk_endpoints[p][..num_endpoint_vals]);
                }
                best_log_blk.weights[..36].copy_from_slice(&ise_weights);
            }
        } else {
            let mut trial_blk = best_log_blk;
            trial_blk.partition_id = seed;
            for p in 0..3 {
                trial_blk.endpoints[num_endpoint_vals * p..num_endpoint_vals * (p + 1)]
                    .copy_from_slice(&blk_endpoints[p][..num_endpoint_vals]);
            }
            if !downsample_to_grid(
                &ise_weights,
                weights_ise_range,
                grid_w,
                grid_h,
                &mut trial_blk.weights,
            ) {
                return false;
            }
            if grid_w < 6 || grid_h < 6 {
                for p in 0..3 {
                    let weights = trial_blk.weights;
                    refine_endpoints(
                        cem,
                        endpoints_ise_range,
                        &mut trial_blk.endpoints[p * num_endpoint_vals..],
                        6,
                        6,
                        grid_w,
                        grid_h,
                        &weights,
                        weights_ise_range,
                        part_total_pixels[p],
                        &part_half_pixels[p],
                        &part_pixels_q16[p],
                        Some(&part_pixel_index[p]),
                        coptions,
                        mode11_opt_mode,
                    );
                }
            }
            let mut decoded = [[0 as HalfFloat; 3]; 36];
            if !decode_block_half3(&trial_blk, &mut decoded) {
                return false;
            }
            let trial_err = compute_block_error(36, block_pixels_half, &decoded, coptions);
            if trial_err < best_e {
                best_e = trial_err;
                best_log_blk = trial_blk;
            }
        }
    }
    if best_e < BIG_FLOAT_VAL as f64 {
        res.log_blk = best_log_blk;
        res.valid = true;
        res.err = best_e;
    } else {
        res.valid = false;
    }
    res.valid
}

// ---------------------------------------------------------------------------------------------
// Block coding

/// `encode_values`: BISE style value coding (trits / quints packed in 8 / 7 bit groups first).
pub fn encode_values(
    coder: &mut BitwiseCoder,
    total_values: usize,
    vals: &[u8],
    endpoint_range: u32,
) -> u32 {
    let r = ah::ISE_RANGE_TABLE
        .get(endpoint_range as usize)
        .copied()
        .unwrap_or([8, 0, 0]);
    let (ep_bits, ep_trits, ep_quints) = (r[0] as u32, r[1] != 0, r[2] != 0);
    let mut bit_values = [0u32; 64];
    let mut tq_values = [0u32; 22];
    let (mut total_tq_values, mut tq_accum, mut tq_mul) = (0usize, 0u32, 1u32);
    for i in 0..total_values {
        let val = vals[i] as u32;
        bit_values[i] = val & ((1 << ep_bits) - 1);
        let tq = val >> ep_bits;
        if ep_trits {
            tq_accum += tq * tq_mul;
            tq_mul *= 3;
            if tq_mul == 243 {
                tq_values[total_tq_values] = tq_accum;
                total_tq_values += 1;
                tq_accum = 0;
                tq_mul = 1;
            }
        } else if ep_quints {
            tq_accum += tq * tq_mul;
            tq_mul *= 5;
            if tq_mul == 125 {
                tq_values[total_tq_values] = tq_accum;
                total_tq_values += 1;
                tq_accum = 0;
                tq_mul = 1;
            }
        }
    }
    let mut total_bits_output = 0;
    for &v in tq_values.iter().take(total_tq_values) {
        let num_bits = if ep_trits { 8 } else { 7 };
        coder.put_bits(v, num_bits);
        total_bits_output += num_bits;
    }
    if tq_mul > 1 {
        let num_bits = if ep_trits {
            match tq_mul {
                3 => 2,
                9 => 4,
                27 => 5,
                _ => 7,
            }
        } else if tq_mul == 5 {
            3
        } else {
            5
        };
        coder.put_bits(tq_accum, num_bits);
        total_bits_output += num_bits;
    }
    for &b in bit_values.iter().take(total_values) {
        coder.put_bits(b, ep_bits);
        total_bits_output += ep_bits;
    }
    total_bits_output
}

/// `get_num_endpoint_vals`.
#[inline]
pub fn get_num_endpoint_vals(cem: u32) -> usize {
    if cem == 11 {
        NUM_MODE11_ENDPOINTS
    } else {
        NUM_MODE7_ENDPOINTS
    }
}

/// `code_block`: block mode, endpoint mode, endpoints (raw / delta) and weights.
pub fn code_block(
    coder: &mut BitwiseCoder,
    log_blk: &LogAstcBlock,
    block_mode_index: u32,
    em: u32,
    ep_deltas: Option<&[u8]>,
) {
    let t = hdr6x6_tables();
    coder.put_truncated_binary(block_mode_index, BM_TOTAL_MODES);
    coder.put_truncated_binary(em, EM_TOTAL);
    let num_endpoint_vals = get_num_endpoint_vals(log_blk.color_endpoint_modes[0] as u32);
    if em == EM_USE_LEFT_DELTA || em == EM_USE_UPPER_DELTA {
        if let Some(d) = ep_deltas {
            for &v in d.iter().take(num_endpoint_vals) {
                coder.put_bits(v as u32, super::astc_6x6_hdr::NUM_ENDPOINT_DELTA_BITS);
            }
        }
    } else if em == EM_RAW {
        if log_blk.num_partitions == 2 {
            let u = t.part2_seed_to_unique_index[log_blk.partition_id as usize & 1023];
            coder.put_truncated_binary(u as u32, NUM_UNIQUE_PARTITIONS2);
        } else if log_blk.num_partitions == 3 {
            let u = t.part3_seed_to_unique_index[log_blk.partition_id as usize & 1023];
            coder.put_truncated_binary(u as u32, NUM_UNIQUE_PARTITIONS3);
        }
        encode_values(
            coder,
            num_endpoint_vals * log_blk.num_partitions as usize,
            &log_blk.endpoints,
            log_blk.endpoint_ise_range as u32,
        );
    }
    let nw = log_blk.grid_width as usize
        * log_blk.grid_height as usize
        * if log_blk.dual_plane { 2 } else { 1 };
    encode_values(coder, nw, &log_blk.weights, log_blk.weight_ise_range as u32);
}

// ---------------------------------------------------------------------------------------------
// Block filtering and statistics

/// `filter_block` (vec4 version): separable Gaussian low pass to a `grid_x` x `grid_y` like
/// resolution, clamped to the ASTC HDR range (xyz only).
pub fn filter_block_v4(grid_x: u32, grid_y: u32, src: &[[f32; 4]; 36], dst: &mut [[f32; 4]; 36]) {
    let t = hdr6x6_tables();
    let mut temp = [[0f32; 4]; 36];
    if grid_x == 6 {
        temp = *src;
    } else {
        let row_lists = &t.contrib_lists[grid_x as usize];
        for y in 0..6 {
            for x in 0..6 {
                let mut p = [0f32; 3];
                for c in row_lists[x].p.iter().take(row_lists[x].n as usize) {
                    let s = &src[y * 6 + c.pixel as usize];
                    for k in 0..3 {
                        p[k] += s[k] * c.weight;
                    }
                }
                for v in p.iter_mut() {
                    *v = v.clamp(0.0, ASTC_HDR_MAX_VAL);
                }
                temp[y * 6 + x] = [p[0], p[1], p[2], 0.0];
            }
        }
    }
    if grid_y == 6 {
        for i in 0..36 {
            dst[i][..3].copy_from_slice(&temp[i][..3]);
        }
    } else {
        let col_lists = &t.contrib_lists[grid_y as usize];
        for x in 0..6 {
            for y in 0..6 {
                let mut p = [0f32; 3];
                for c in col_lists[y].p.iter().take(col_lists[y].n as usize) {
                    let s = &temp[c.pixel as usize * 6 + x];
                    for k in 0..3 {
                        p[k] += s[k] * c.weight;
                    }
                }
                for v in p.iter_mut() {
                    *v = v.clamp(0.0, ASTC_HDR_MAX_VAL);
                }
                dst[x + y * 6] = [p[0], p[1], p[2], 0.0];
            }
        }
    }
}

/// `filter_block` (vec3 version, no clamping).
pub fn filter_block_v3(grid_x: u32, grid_y: u32, src: &[Vec3; 36], dst: &mut [Vec3; 36]) {
    let t = hdr6x6_tables();
    let mut temp = [[0f32; 3]; 36];
    if grid_x == 6 {
        temp = *src;
    } else {
        let row_lists = &t.contrib_lists[grid_x as usize];
        for y in 0..6 {
            for x in 0..6 {
                let mut p = [0f32; 3];
                for c in row_lists[x].p.iter().take(row_lists[x].n as usize) {
                    let s = &src[y * 6 + c.pixel as usize];
                    for k in 0..3 {
                        p[k] += s[k] * c.weight;
                    }
                }
                temp[y * 6 + x] = p;
            }
        }
    }
    if grid_y == 6 {
        *dst = temp;
    } else {
        let col_lists = &t.contrib_lists[grid_y as usize];
        for x in 0..6 {
            for y in 0..6 {
                let mut p = [0f32; 3];
                for c in col_lists[y].p.iter().take(col_lists[y].n as usize) {
                    let s = &temp[c.pixel as usize * 6 + x];
                    for k in 0..3 {
                        p[k] += s[k] * c.weight;
                    }
                }
                dst[x + y * 6] = p;
            }
        }
    }
}

/// `diff_blocks`: mean squared RGB difference.
pub fn diff_blocks(a: &[[f32; 4]; 36], b: &[[f32; 4]; 36]) -> f32 {
    let mut diff = 0f32;
    for i in 0..36 {
        let (d0, d1, d2) = (a[i][0] - b[i][0], a[i][1] - b[i][1], a[i][2] - b[i][2]);
        diff += d0 * d0 + d1 * d1 + d2 * d2;
    }
    diff * (1.0 / 36.0)
}

/// `sub_and_compute_std_dev`: the largest per channel standard deviation of `a - b`.
pub fn sub_and_compute_std_dev(a: &[Vec3; 36], b: &[Vec3; 36]) -> f32 {
    let mut mean = [0f32; 3];
    for i in 0..36 {
        for c in 0..3 {
            mean[c] += a[i][c] - b[i][c];
        }
    }
    for c in mean.iter_mut() {
        *c *= 1.0 / 36.0;
    }
    let mut diff_sum = [0f32; 3];
    for i in 0..36 {
        for c in 0..3 {
            let mut d = a[i][c] - b[i][c];
            d -= mean[c];
            diff_sum[c] += d * d;
        }
    }
    let var = diff_sum.map(|v| v * (1.0 / 36.0));
    let sd = var.map(|v| v.sqrt());
    maxf(maxf(sd[0], sd[1]), sd[2])
}

/// `stats<float>` (the fields the encoder uses), from `calc`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StatsF {
    pub n: u32,
    pub total: f32,
    pub avg: f32,
    pub var: f32,
    pub std_dev: f32,
    pub min: f32,
    pub max: f32,
    pub range: f32,
}

impl StatsF {
    /// `stats<float>::calc` over strided values.
    pub fn calc(vals: impl Iterator<Item = f32> + Clone) -> Self {
        let mut s = StatsF {
            min: BIG_FLOAT_VAL,
            max: -BIG_FLOAT_VAL,
            ..Default::default()
        };
        let mut total_sq = 0f32;
        for (i, v) in vals.clone().enumerate() {
            s.n += 1;
            s.total += v;
            total_sq += v * v;
            if i == 0 {
                s.min = v;
                s.max = v;
            } else {
                s.min = if s.min < v { s.min } else { v };
                s.max = if s.max > v { s.max } else { v };
            }
        }
        let _ = total_sq;
        if s.n == 0 {
            return StatsF {
                min: BIG_FLOAT_VAL,
                max: -BIG_FLOAT_VAL,
                ..Default::default()
            };
        }
        s.range = s.max - s.min;
        let nd = s.n as f32;
        s.avg = s.total / nd;
        for v in vals {
            let d = v - s.avg;
            s.var += d * d;
        }
        s.var /= nd;
        s.std_dev = s.var.sqrt();
        s
    }

    /// `stats<float>::calc_simplified` (average, variance, standard deviation).
    pub fn calc_simplified(vals: impl Iterator<Item = f32> + Clone) -> Self {
        let mut s = StatsF {
            min: BIG_FLOAT_VAL,
            max: -BIG_FLOAT_VAL,
            ..Default::default()
        };
        for v in vals.clone() {
            s.n += 1;
            s.total += v;
        }
        if s.n == 0 {
            return s;
        }
        let nd = s.n as f32;
        s.avg = s.total / nd;
        for v in vals {
            let d = v - s.avg;
            s.var += d * d;
        }
        s.var /= nd;
        s.std_dev = s.var.sqrt();
        s
    }
}

/// `comparative_stats<float>::calc_cov`.
pub fn calc_cov(a: &[f32], b: &[f32], sa: &StatsF, sb: &StatsF) -> f32 {
    let mut cov = 0f32;
    for (&fa, &fb) in a.iter().zip(b) {
        cov += (fa - sa.avg) * (fb - sb.avg);
    }
    cov / a.len() as f32
}

/// `comparative_stats<float>::calc_pearson` (only `m_pearson`).
pub fn calc_pearson(a: &[f32], b: &[f32], sa: &StatsF, sb: &StatsF) -> f32 {
    let cov = calc_cov(a, b, sa, sb);
    let dv = sa.std_dev * sb.std_dev;
    if dv != 0.0 { cov / dv } else { 0.0 }
}

/// `tracked_stat_dbl::get_std_dev` over a set of values.
fn tracked_std_dev(vals: impl Iterator<Item = f64>) -> f64 {
    let (mut num, mut total, mut total2) = (0u64, 0f64, 0f64);
    for v in vals {
        num += 1;
        total += v;
        total2 += v * v;
    }
    if num != 0 {
        (num as f64 * total2 - total * total).sqrt() / num as f64
    } else {
        0.0
    }
}

/// `smooth_map_params` (debug images are not ported).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmoothMapParams {
    pub no_mse_scaling: bool,
    pub max_smooth_std_dev: f32,
    pub smooth_max_mse_scale: f32,
    pub max_med_smooth_std_dev: f32,
    pub med_smooth_max_mse_scale: f32,
    pub max_ultra_smooth_std_dev: f32,
    pub ultra_smooth_max_mse_scale: f32,
}

impl Default for SmoothMapParams {
    fn default() -> Self {
        SmoothMapParams {
            no_mse_scaling: false,
            max_smooth_std_dev: 100.0,
            smooth_max_mse_scale: 13000.0,
            max_med_smooth_std_dev: 9.0,
            med_smooth_max_mse_scale: 15000.0,
            max_ultra_smooth_std_dev: 4.0,
            ultra_smooth_max_mse_scale: 2_000_000.0,
        }
    }
}

/// `create_smooth_maps2`: per pixel MSE scales (row major `width * height`) from local
/// activity of the tone mapped image.
pub fn create_smooth_maps2(orig_img: &Image, params: &SmoothMapParams) -> Vec<f32> {
    let (width, height) = (orig_img.get_width(), orig_img.get_height());
    if params.no_mse_scaling {
        return vec![1.0; (width * height) as usize];
    }
    let mut scales = vec![0f32; (width * height) as usize];
    let region_std_dev = |x: u32, y: u32, lo: i32, hi: i32| -> f32 {
        let mut max_std_dev = 0f32;
        for c in 0..3 {
            let mut vals = Vec::with_capacity(((hi - lo) * (hi - lo)) as usize);
            for yd in lo..hi {
                for xd in lo..hi {
                    let p = orig_img.get_clamped(x as i32 + xd, y as i32 + yd);
                    vals.push(p[c] as f32 as f64);
                }
            }
            max_std_dev = maxf(max_std_dev, tracked_std_dev(vals.into_iter()) as f32);
        }
        max_std_dev
    };
    for y in 0..height {
        for x in 0..width {
            let i = (x + y * width) as usize;
            {
                let max_std_dev = region_std_dev(x, y, -1, 2);
                let mut yl = (max_std_dev / params.max_smooth_std_dev).clamp(0.0, 1.0);
                yl = powf_libm(yl, 1.0 / 2.0); // substantially less bits
                scales[i] = lerpf(params.smooth_max_mse_scale, 1.0, yl);
            }
            {
                let max_std_dev = region_std_dev(x, y, -3, 3);
                let yl = (max_std_dev / params.max_med_smooth_std_dev).clamp(0.0, 1.0);
                scales[i] = lerpf(params.med_smooth_max_mse_scale, scales[i], yl);
            }
            {
                let max_std_dev = region_std_dev(x, y, -5, 5);
                let mut yl = (max_std_dev / params.max_ultra_smooth_std_dev).clamp(0.0, 1.0);
                yl = yl * yl;
                scales[i] = lerpf(params.ultra_smooth_max_mse_scale, scales[i], yl);
            }
        }
    }
    scales
}

/// `tonemap_image_compressive2`: maps the used half float range linearly to 8 bits.
pub fn tonemap_image_compressive2(hdr_img: &ImageF) -> (Image, bool) {
    let (width, height) = (hdr_img.get_width(), hdr_img.get_height());
    let mut dst = Image::with_size(width, height);
    for p in dst.get_pixels_mut() {
        *p = [0, 0, 0, 255];
    }
    let mut half_img = vec![0u16; (width * 3 * height) as usize];
    let (mut low_h, mut high_h) = (u32::MAX, 0u32);
    for y in 0..height {
        for x in 0..width {
            let p = hdr_img.get(x, y);
            for i in 0..3 {
                let mut f = p[i];
                if f.is_nan() || f.is_infinite() || f < 0.0 {
                    f = 0.0;
                } else if f as f64 > 65504.0 {
                    f = 65504.0;
                }
                let h = float_to_half(f, false) as u32;
                low_h = low_h.min(h);
                high_h = high_h.max(h);
                half_img[((x + y * width) * 3 + i as u32) as usize] = h as u16;
            }
        }
    }
    if low_h == high_h {
        return (dst, false);
    }
    for y in 0..height {
        for x in 0..width {
            for i in 0..3 {
                let h = half_img[((x + y * width) * 3 + i as u32) as usize] as u32;
                let f = (h - low_h) as f32 / (high_h - low_h) as f32;
                dst.get_mut(x, y)[i] = ((f * 255.0).round() as i32).clamp(0, 255) as u8;
            }
        }
    }
    (dst, true)
}

// ---------------------------------------------------------------------------------------------
// ITP error metrics

const REALLY_DARK_I_THRESHOLD: f32 = 0.0625;
const REALLY_DARK_MSE_ERR_SCALE: f32 = 128.0;
const REALLY_DARK_DELTA_ITP_JND_SCALE: f32 = 5.0;

/// `bu_math::smoothstep`.
#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let x = saturate((x - edge0) / (edge1 - edge0));
    x * x * (3.0 - 2.0 * x)
}

/// `compute_pixel_mse_itp`.
pub fn compute_pixel_mse_itp(orig: Vec3, comp: Vec3, delta_itp_dark_adjustment: bool) -> f32 {
    let (di, dt, dp) = (orig[0] - comp[0], orig[1] - comp[1], orig[2] - comp[2]);
    let mut err = (di * di) + (dt * dt) + (dp * dp);
    if delta_itp_dark_adjustment {
        // Artifically amplify MSE on very dark pixels.
        let mut s = smoothstep(0.0, REALLY_DARK_I_THRESHOLD, orig[0]);
        s = lerpf(REALLY_DARK_MSE_ERR_SCALE, 1.0, s);
        err *= s;
    }
    err
}

/// `compute_block_mse_itp`.
pub fn compute_block_mse_itp(
    orig: &[Vec3; 36],
    comp: &[Vec3; 36],
    delta_itp_dark_adjustment: bool,
) -> f32 {
    let mut total_mse = 0f32;
    for i in 0..36 {
        total_mse += compute_pixel_mse_itp(orig[i], comp[i], delta_itp_dark_adjustment);
    }
    total_mse * (1.0 / 36.0)
}

/// `compute_block_ssim_itp`.
pub fn compute_block_ssim_itp(orig: &[Vec3; 36], comp: &[Vec3; 36]) -> f32 {
    let mut ssim = [0f32; 3];
    let d = 1.0f64;
    let (k1, k2) = (0.01f32 as f64, 0.03f32 as f64);
    // weight mean error more highly to reduce blocking
    let (ap, bp, cp) = (1.5f32, 1.0f32, 1.0f32);
    let s_c1 = (k1 * d) * (k1 * d);
    let s_c2 = (k2 * d) * (k2 * d);
    let s_c3 = s_c2 * 0.5f32 as f64;
    for c in 0..3 {
        let xa: Vec<f32> = orig.iter().map(|p| p[c]).collect();
        let ya: Vec<f32> = comp.iter().map(|p| p[c]).collect();
        let xs = StatsF::calc_simplified(xa.iter().copied());
        let ys = StatsF::calc_simplified(ya.iter().copied());
        let cov = calc_cov(&xa, &ya, &xs, &ys);
        // The products and sums of the float stats are float; only the constants are double.
        let lum_n = (2.0f32 * xs.avg * ys.avg) as f64 + s_c1;
        let lum_d = (xs.avg * xs.avg + ys.avg * ys.avg) as f64 + s_c1;
        let mut lum = (lum_n / lum_d) as f32;
        lum = saturate(lum);
        let con_n = (2.0f32 * xs.std_dev * ys.std_dev) as f64 + s_c2;
        let con_d = (xs.var + ys.var) as f64 + s_c2;
        let mut con = (con_n / con_d) as f32;
        con = saturate(con);
        let mut st = ((cov as f64 + s_c3) / ((xs.std_dev * ys.std_dev) as f64 + s_c3)) as f32;
        st = saturate(st);
        ssim[c] = powf_libm(lum, ap) * powf_libm(con, bp) * powf_libm(st, cp);
    }
    ssim[0] * ssim[1] * ssim[2]
}

/// `compute_pixel_delta_itp`: delta ITP (1.0 is a JND), higher error at low light.
pub fn compute_pixel_delta_itp(
    a: Vec3,
    b: Vec3,
    orig: Vec3,
    delta_itp_dark_adjustment: bool,
) -> f32 {
    let (di, dt, dp) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    let mut err = 720.0 * ((di * di) + (dt * dt) + (dp * dp)).sqrt();
    let mut s = smoothstep(0.0, REALLY_DARK_I_THRESHOLD, orig[0]);
    if delta_itp_dark_adjustment {
        s = lerpf(REALLY_DARK_DELTA_ITP_JND_SCALE, 1.0, s);
        err *= s;
    }
    err
}

/// `calc_deblocking_penalty_itp`: relative change of the edge MSE against the neighbors.
pub fn calc_deblocking_penalty_itp(
    bx: u32,
    by: u32,
    width: u32,
    height: u32,
    pass_src_img_itp: &ImageF,
    comp_pixels_itp: &[Vec3; 36],
) -> f32 {
    let mut total_deblock_penalty = 0f32;
    let (mut total_orig_mse, mut total_comp_mse) = (0f32, 0f32);
    let mut total_c = 0u32;
    let (bx, by) = (bx as i32, by as i32);
    for b in 0..4 {
        for i in 0..6i32 {
            let (ox, oy, qx, qy) = match b {
                0 => (bx * 6 + i, (by - 1) * 6 + 5, bx * 6 + i, by * 6),
                1 => (bx * 6 + i, (by + 1) * 6, bx * 6 + i, by * 6 + 5),
                2 => ((bx - 1) * 6 + 5, by * 6 + i, bx * 6, by * 6 + i),
                _ => ((bx + 1) * 6, by * 6 + i, bx * 6 + 5, by * 6 + i),
            };
            if ox < 0 || oy < 0 || ox >= width as i32 || oy >= height as i32 {
                continue;
            }
            let o = pass_src_img_itp.get(ox as u32, oy as u32);
            let q = pass_src_img_itp.get(qx as u32, qy as u32);
            let d = comp_pixels_itp[((qy - by * 6) * 6 + (qx - bx * 6)) as usize];
            let (a0, a1, a2) = (o[0] - q[0], o[1] - q[1], o[2] - q[2]);
            total_orig_mse += a0 * a0 + a1 * a1 + a2 * a2;
            let (b0, b1, b2) = (o[0] - d[0], o[1] - d[1], o[2] - d[2]);
            total_comp_mse += b0 * b0 + b1 * b1 + b2 * b2;
            total_c += 1;
        }
    }
    if total_c != 0 {
        total_orig_mse /= total_c as f32;
        total_comp_mse /= total_c as f32;
        if total_orig_mse != 0.0 {
            total_deblock_penalty = ((total_comp_mse - total_orig_mse) / total_orig_mse).abs();
        }
    }
    total_deblock_penalty
}

/// `calc_strip_size`: (total strips, rows per strip), `None` on a failed check.
pub fn calc_strip_size(
    lambda: f32,
    num_blocks_y: u32,
    total_threads: u32,
    force_one_strip: bool,
) -> Option<(u32, u32)> {
    let mut total_strips = 1u32;
    if lambda == 0.0 {
        if !force_one_strip {
            total_strips = total_threads;
        }
    } else {
        const MIN_DESIRED_STRIPS: u32 = 8;
        const MAX_TARGET_STRIPS: u32 = 32;
        const TARGET_ASTC_6X6_ROWS_PER_STRIP: u32 = 12;
        if !force_one_strip {
            total_strips = (num_blocks_y / TARGET_ASTC_6X6_ROWS_PER_STRIP).max(1);
            if num_blocks_y >= MIN_DESIRED_STRIPS * 2 {
                total_strips = total_strips.max(MIN_DESIRED_STRIPS);
            }
        }
        total_strips = total_strips.min(MAX_TARGET_STRIPS);
    }
    let rows_per_strip = if total_strips <= 1 {
        num_blocks_y
    } else {
        ((num_blocks_y / total_strips) & !1).max(2)
    };
    if rows_per_strip == 0 {
        return None;
    }
    let total_strips = num_blocks_y.div_ceil(rows_per_strip);
    let mut total_rows = 0;
    for strip_index in 0..total_strips {
        let strip_first_by = strip_index * rows_per_strip;
        let mut strip_last_by = (strip_first_by + rows_per_strip - 1).min(num_blocks_y);
        if strip_index == total_strips - 1 {
            strip_last_by = num_blocks_y - 1;
        }
        total_rows += (strip_last_by - strip_first_by) + 1;
    }
    if total_rows != num_blocks_y {
        return None;
    }
    Some((total_strips, rows_per_strip))
}

/// `convet_rgb_image_to_itp`.
pub fn convert_rgb_image_to_itp(src_img: &ImageF, cfg: &AstcHdr6x6GlobalConfig) -> ImageF {
    let (width, height) = (src_img.get_width(), src_img.get_height());
    let mut dst = ImageF::with_size(width, height);
    for y in 0..height {
        for x in 0..width {
            let s = src_img.get(x, y);
            let itp = linear_rgb_to_itp([s[0], s[1], s[2]], cfg);
            *dst.get_mut(x, y) = [itp[0], itp[1], itp[2], 0.0];
        }
    }
    dst
}

// ---------------------------------------------------------------------------------------------
// Partition pattern pre-selection

#[derive(Debug, Clone, Copy, Default)]
struct CandidateRes {
    total_sq_dist: f32,
    index: u32,
}

fn part_means(num_parts: usize, pat: &PartitionPatternVec, pixels: &[Vec3; 36]) -> [Vec3; 3] {
    let mut means = [[0f32; 3]; 3];
    let mut totals = [0u32; 3];
    for i in 0..36 {
        let p = pat[i] as usize % num_parts.max(1);
        for c in 0..3 {
            means[p][c] += pixels[i][c];
        }
        totals[p] += 1;
    }
    for p in 0..num_parts {
        let n = totals[p] as f32;
        for c in 0..3 {
            means[p][c] /= n;
        }
    }
    means
}

/// `estimate_partitions_mode7_and_11`: ranks candidate patterns by the residual variance off the
/// principal axis (mode 11) and off the grayscale axis (mode 7).
#[allow(clippy::too_many_arguments)]
pub fn estimate_partitions_mode7_and_11(
    num_parts: usize,
    unique_pats: &[PartitionPatternVec],
    pats_to_examine: &[u32],
    half_pixels_as_floats: &[Vec3; 36],
    num_desired_pats: usize,
    desired_mode11: &mut [i32],
    desired_mode7: &mut [i32],
) {
    let grayscale_axis = 0.5773502691f32;
    let mut mode11_candidates = Vec::with_capacity(pats_to_examine.len());
    let mut mode7_candidates = Vec::with_capacity(pats_to_examine.len());
    for &unique_part_index in pats_to_examine {
        let pat = &unique_pats[unique_part_index as usize];
        let means = part_means(num_parts, pat, half_pixels_as_floats);
        let mut part_cov = [[0f32; 6]; 3];
        for i in 0..36 {
            let p = pat[i] as usize % num_parts.max(1);
            let v = half_pixels_as_floats[i];
            let (r, g, b) = (v[0] - means[p][0], v[1] - means[p][1], v[2] - means[p][2]);
            let cv = &mut part_cov[p];
            cv[0] += r * r;
            cv[1] += r * g;
            cv[2] += r * b;
            cv[3] += g * g;
            cv[4] += g * b;
            cv[5] += b * b;
        }
        let mut mode11_eig = [0f32; 3];
        let mut mode7_eig = [0f32; 3];
        let mut total_variance = [0f32; 3];
        for p in 0..num_parts {
            let cv = &part_cov[p];
            total_variance[p] = cv[0] + cv[3] + cv[5];
            let (mut xr, mut xg, mut xb) = (0.9f32, 1.0f32, 0.7f32);
            for _ in 0..4 {
                let mut r = xr * cv[0] + xg * cv[1] + xb * cv[2];
                let mut g = xr * cv[1] + xg * cv[3] + xb * cv[4];
                let mut b = xr * cv[2] + xg * cv[4] + xb * cv[5];
                let mut m = maxf(maxf(r.abs(), g.abs()), b.abs());
                if m >= 1e-10 {
                    m = 1.0 / m;
                    r *= m;
                    g *= m;
                    b *= m;
                }
                xr = r;
                xg = g;
                xb = b;
            }
            let mut len_sq = xr * xr + xg * xg + xb * xb;
            if len_sq < 1e-10 {
                xr = grayscale_axis;
                xg = grayscale_axis;
                xb = grayscale_axis;
            } else {
                len_sq = 1.0 / len_sq.sqrt();
                xr *= len_sq;
                xg *= len_sq;
                xb *= len_sq;
            }
            {
                let r = xr * cv[0] + xg * cv[1] + xb * cv[2];
                let g = xr * cv[1] + xg * cv[3] + xb * cv[4];
                let b = xr * cv[2] + xg * cv[4] + xb * cv[5];
                mode11_eig[p] = r * xr + g * xg + b * xb;
            }
            {
                let y = grayscale_axis;
                let r = y * cv[0] + y * cv[1] + y * cv[2];
                let g = y * cv[1] + y * cv[3] + y * cv[4];
                let b = y * cv[2] + y * cv[4] + y * cv[5];
                mode7_eig[p] = r * y + g * y + b * y;
            }
        }
        let (mut d11, mut d7) = (0f32, 0f32);
        for p in 0..num_parts {
            d11 += maxf(0.0, total_variance[p] - mode11_eig[p]);
        }
        for p in 0..num_parts {
            d7 += maxf(0.0, total_variance[p] - mode7_eig[p]);
        }
        mode11_candidates.push(CandidateRes {
            total_sq_dist: d11,
            index: unique_part_index,
        });
        mode7_candidates.push(CandidateRes {
            total_sq_dist: d7,
            index: unique_part_index,
        });
    }
    super::enc::std_sort_by(&mut mode11_candidates, |a, b| {
        a.total_sq_dist < b.total_sq_dist
    });
    super::enc::std_sort_by(&mut mode7_candidates, |a, b| {
        a.total_sq_dist < b.total_sq_dist
    });
    for i in 0..num_desired_pats {
        desired_mode11[i] = mode11_candidates.get(i).map_or(0, |c| c.index as i32);
    }
    for i in 0..num_desired_pats {
        desired_mode7[i] = mode7_candidates.get(i).map_or(0, |c| c.index as i32);
    }
}

/// `dist_to_line_squared`.
#[inline]
fn dist_to_line_squared(p: Vec3, line_org: Vec3, line_dir: Vec3) -> f32 {
    let q = [p[0] - line_org[0], p[1] - line_org[1], p[2] - line_org[2]];
    let d = dot3(q, line_dir);
    let v = [
        q[0] - d * line_dir[0],
        q[1] - d * line_dir[1],
        q[2] - d * line_dir[2],
    ];
    dot3(v, v)
}

/// `estimate_partitions_mode7`: ranks candidate patterns by the distance to the grayscale axis.
pub fn estimate_partitions_mode7(
    num_parts: usize,
    unique_pats: &[PartitionPatternVec],
    pats_to_examine: &[u32],
    half_pixels_as_floats: &[Vec3; 36],
    num_desired_pats: usize,
    desired: &mut [i32],
) {
    let part_axis = [0.5773502691f32; 3];
    let mut candidates = Vec::with_capacity(pats_to_examine.len());
    for &unique_part_index in pats_to_examine {
        let pat = &unique_pats[unique_part_index as usize];
        let means = part_means(num_parts, pat, half_pixels_as_floats);
        let mut total_sq_dist_to_line = 0f32;
        for i in 0..36 {
            let p = pat[i] as usize % num_parts.max(1);
            total_sq_dist_to_line +=
                dist_to_line_squared(half_pixels_as_floats[i], means[p], part_axis);
        }
        candidates.push(CandidateRes {
            total_sq_dist: total_sq_dist_to_line,
            index: unique_part_index,
        });
    }
    super::enc::std_sort_by(&mut candidates, |a, b| a.total_sq_dist < b.total_sq_dist);
    for i in 0..num_desired_pats {
        desired[i] = candidates.get(i).map_or(0, |c| c.index as i32);
    }
}

// ---------------------------------------------------------------------------------------------
// Image helpers

/// `image_resample(const imagef&, imagef&, filter, filter_scale)` (clamped boundary, all four
/// components, no output clamping). `dst` must already have the destination size.
pub fn image_resample_f(src: &ImageF, dst: &mut ImageF, filter: &str, filter_scale: f32) -> bool {
    let (src_w, src_h) = (src.get_width(), src.get_height());
    let (dst_w, dst_h) = (dst.get_width(), dst.get_height());
    if src_w.max(src_h) > 16384
        || src_w == 0
        || src_h == 0
        || dst_w == 0
        || dst_h == 0
        || dst_w.max(dst_h) > 16384
    {
        return false;
    }
    if src_w == dst_w && src_h == dst_h && filter_scale == 1.0 {
        *dst = src.clone();
        return true;
    }
    const NUM_COMPS: usize = 4;
    let mut resamplers: Vec<Resampler> = Vec::with_capacity(NUM_COMPS);
    let first = Resampler::new(
        src_w as i32,
        src_h as i32,
        dst_w as i32,
        dst_h as i32,
        BoundaryOp::Clamp,
        1.0,
        0.0,
        Some(filter),
        None,
        None,
        filter_scale,
        filter_scale,
        0.0,
        0.0,
    );
    let (cx, cy) = (first.get_clist_x(), first.get_clist_y());
    resamplers.push(first);
    for _ in 1..NUM_COMPS {
        resamplers.push(Resampler::new(
            src_w as i32,
            src_h as i32,
            dst_w as i32,
            dst_h as i32,
            BoundaryOp::Clamp,
            1.0,
            0.0,
            Some(filter),
            cx.clone(),
            cy.clone(),
            filter_scale,
            filter_scale,
            0.0,
            0.0,
        ));
    }
    let mut samples = vec![vec![0f32; src_w as usize]; NUM_COMPS];
    let mut dst_y = 0u32;
    for src_y in 0..src_h {
        for x in 0..src_w {
            let p = src.get(x, src_y);
            for c in 0..NUM_COMPS {
                samples[c][x as usize] = p[c];
            }
        }
        for c in 0..NUM_COMPS {
            if !resamplers[c].put_line(&samples[c]) {
                return false;
            }
        }
        loop {
            let mut c = 0;
            while c < NUM_COMPS {
                let Some(out) = resamplers[c].get_line() else {
                    break;
                };
                if dst_y < dst_h {
                    for x in 0..dst_w {
                        dst.get_mut(x, dst_y)[c] = out[x as usize];
                    }
                }
                c += 1;
            }
            if c < NUM_COMPS {
                break;
            }
            dst_y += 1;
        }
    }
    true
}

/// `decode_astc_block`: 6x6 RGB floats of a log block.
pub fn decode_astc_block(log_blk: &LogAstcBlock, pixels: &mut [Vec3; 36]) -> bool {
    let mut h4 = [[0 as HalfFloat; 4]; 36];
    if !decode_block(log_blk, &mut DecodePixels::Hdr16(&mut h4), 6, 6) {
        return false;
    }
    for (d, s) in pixels.iter_mut().zip(h4.iter()) {
        *d = [
            half_to_float(s[0]),
            half_to_float(s[1]),
            half_to_float(s[2]),
        ];
    }
    true
}

/// `validate_log_blk`: whether the block packs to valid ASTC.
pub fn validate_log_blk(decomp_blk: &LogAstcBlock) -> bool {
    let mut phys = [0u8; 16];
    pack_astc_block(&mut phys, decomp_blk)
}

// ---------------------------------------------------------------------------------------------
// Strip encoder

const ET_INVALID: i32 = -1;
const ET_RUN: i32 = 0;
const ET_SOLID: i32 = 1;
const ET_REUSE: i32 = 2;
const ET_BLOCK: i32 = 3;

const SOLID_PENALTY: f32 = 4.0;
const REUSE_PENALTY: f32 = 1.0;
const RUN_PENALTY: f32 = 10.0;
const MSE_WEIGHT: f32 = 300000.0;
const SSIM_WEIGHT: f32 = 200.0;
const TWO_LEVEL_PENALTY: f32 = 1.425;
const SWITCH_TO_GAUSSIAN_FILTERED_THRESH1_D_SSIM: f32 = 0.04;
const SWITCH_TO_GAUSSIAN_FILTERED_THRESH2_D_SSIM: f32 = 0.04;
const COMPLEX_BLOCK_WEIGHT_GRID_2X2_MSE_PENALTY: f32 = 1.5;
const COMPLEX_BLOCK_WEIGHT_GRID_3X3_MSE_PENALTY: f32 = 1.25;
const COMPLEX_BLOCK_WEIGHT_GRID_4X4_MSE_PENALTY: f32 = 1.15;
const REC_709_R: f32 = 0.212656;
const REC_709_G: f32 = 0.715158;
const REC_709_B: f32 = 0.072186;

/// `candidate_encoding`.
#[derive(Debug, Clone)]
struct CandidateEncoding {
    encoding_type: i32,
    solid_color: [HalfFloat; 3],
    run_len: u32,
    comp_pixels: [Vec3; 36],
    comp_pixels_itp: [Vec3; 36],
    endpoint_mode: i32,
    block_mode: i32,
    coder: BitwiseCoder,
    /// The block to code (may not be valid ASTC before transcoding).
    coded_log_blk: LogAstcBlock,
    /// The block the decoder outputs.
    decomp_log_blk: LogAstcBlock,
    reuse_delta_index: i32,
    t: f64,
    d: f64,
    bits: f32,
}

impl Default for CandidateEncoding {
    fn default() -> Self {
        CandidateEncoding {
            encoding_type: ET_INVALID,
            solid_color: [0; 3],
            run_len: 0,
            comp_pixels: [[0.0; 3]; 36],
            comp_pixels_itp: [[0.0; 3]; 36],
            endpoint_mode: -1,
            block_mode: -1,
            coder: BitwiseCoder::new(),
            coded_log_blk: LogAstcBlock::default(),
            decomp_log_blk: LogAstcBlock::default(),
            reuse_delta_index: 0,
            t: 0.0,
            d: 0.0,
            bits: 0.0,
        }
    }
}

/// `uastc_hdr_6x6_encode_state` (`packed_img` is only used for debug output and not kept).
struct EncodeState {
    master_coptions: AstcHdrCodecBaseOptions,
    src_img: ImageF,
    src_img_filtered1: ImageF,
    src_img_filtered2: ImageF,
    src_img_itp: ImageF,
    src_img_filtered1_itp: ImageF,
    src_img_filtered2_itp: ImageF,
    /// Row major `width * height` (empty unless lambda > 0).
    smooth_block_mse_scales: Vec<f32>,
    strip_bits: Vec<BitwiseCoder>,
    final_astc_blocks: Vec<[u8; 16]>,
    coded_blocks: Vec<CandidateEncoding>,
}

/// `fast_float_to_half_no_clamp_neg_nan_or_inf` (round to nearest even).
#[inline]
fn fast_float_to_half_no_clamp(f: f32) -> HalfFloat {
    let fu = (f * f32::from_bits(0x0780_0000)).to_bits();
    let mut h = (fu >> (23 - 10)) & 0x7FFF;
    let mant = fu & 8191;
    let inc = u32::from(mant > 4096) | (u32::from(mant == 4096) & (h & 1));
    h += inc;
    h.min(0x7BFF) as HalfFloat
}

/// Per pass inputs of the current 6x6 block.
struct BlockPixels {
    block_pixels: [[f32; 4]; 36],
    half_pixels: [[HalfFloat; 3]; 36],
    half_pixels_as_floats: [Vec3; 36],
    block_pixels_q16: [[f32; 4]; 36],
    block_pixels_as_itp: [Vec3; 36],
}

fn pack_weights_and_decomp(
    cand: &mut CandidateEncoding,
    md: &super::astc_6x6_hdr::BlockModeDesc,
    transcode_endpoints: &[u8],
    use_orig_behavior: bool,
) -> bool {
    let grid_x = md.grid_x;
    let grid_y = md.grid_y;
    let num_grid_samples = (grid_x * grid_y) as usize;
    let dual_plane = md.dp;
    let num_endpoint_vals = get_num_endpoint_vals(md.cem);
    let mut transcode_weights = [0u8; 72];
    super::astc_6x6_hdr::requantize_astc_weights(
        num_grid_samples * if dual_plane { 2 } else { 1 },
        &cand.coded_log_blk.weights,
        md.weight_ise_range,
        &mut transcode_weights,
        md.transcode_weight_ise_range,
    );
    // Create the block the decoder would transcode into.
    let d = &mut cand.decomp_log_blk;
    d.clear();
    d.color_endpoint_modes[0] = md.cem as u8;
    d.dual_plane = dual_plane;
    d.color_component_selector = md.dp_channel as u8;
    d.num_partitions = 1;
    d.endpoint_ise_range = md.transcode_endpoint_ise_range as u8;
    d.weight_ise_range = md.transcode_weight_ise_range as u8;
    d.endpoints[..num_endpoint_vals].copy_from_slice(&transcode_endpoints[..num_endpoint_vals]);
    super::astc_6x6_hdr::copy_weight_grid(
        dual_plane,
        grid_x,
        grid_y,
        &transcode_weights,
        d,
        use_orig_behavior,
    );
    true
}

/// `compress_strip_task`: encodes the block rows `strip_first_by..=strip_last_by` into
/// `strip_bits[strip_index]`. Returns false on an internal failure.
#[allow(clippy::too_many_arguments)]
fn compress_strip_task(
    strip_index: usize,
    strip_first_by: u32,
    strip_last_by: u32,
    num_blocks_x: u32,
    width: u32,
    height: u32,
    global_cfg: &AstcHdr6x6GlobalConfig,
    enc_state: &mut EncodeState,
) -> bool {
    use super::astc_6x6_hdr::{
        BlockModeDesc, block_mode_desc, requantize_astc_weights, requantize_ise_endpoints,
    };
    use super::astc_hdr_enc::{
        EncodeAstcBlockStats, decode_cem_7_config, decode_cem_11_config, downsample_ise_weights,
        downsample_ise_weights_dual_plane, encode_astc_hdr_block_downsampled_mode_11,
        encode_astc_hdr_block_mode_11_dual_plane, eval_selectors, eval_selectors_dual_plane,
        get_astc_hdr_mode_7_block_colors, get_astc_hdr_mode_11_block_colors,
    };
    use super::hdr_tables::REUSE_XY_DELTAS;

    let t = hdr6x6_tables();
    let nbx = num_blocks_x as usize;
    let mut prev_comp_pixels = [[0f32; 3]; 36];
    let mut prev_run_len = 0u32;
    let mut prev_encoding = BitwiseCoder::new();
    let mut prev_non_run_candidate_encoding = CandidateEncoding::default();
    let mut strip_coded_bits = BitwiseCoder::new();
    let mut candidates: Vec<CandidateEncoding> = Vec::with_capacity(1536);
    let use_orig_behavior = global_cfg.write_basisu_1_6_compatible_files;
    let descs: Vec<BlockModeDesc> = (0..BM_TOTAL_MODES)
        .map(|i| block_mode_desc(i).unwrap_or_default())
        .collect();

    for by in strip_first_by..=strip_last_by {
        let has_upper_neighbor = by > strip_first_by;
        for bx in 0..num_blocks_x {
            let mut outer_pass = 0;
            while outer_pass < 3 {
                let has_left_neighbor = bx > 0;
                let (pass_src_img, pass_src_img_itp) = match outer_pass {
                    2 => (
                        &enc_state.src_img_filtered2,
                        &enc_state.src_img_filtered2_itp,
                    ),
                    1 => (
                        &enc_state.src_img_filtered1,
                        &enc_state.src_img_filtered1_itp,
                    ),
                    _ => (&enc_state.src_img, &enc_state.src_img_itp),
                };
                let mut bp = BlockPixels {
                    block_pixels: [[0.0; 4]; 36],
                    half_pixels: [[0; 3]; 36],
                    half_pixels_as_floats: [[0.0; 3]; 36],
                    block_pixels_q16: [[0.0; 4]; 36],
                    block_pixels_as_itp: [[0.0; 3]; 36],
                };
                pass_src_img.extract_block_clamped(&mut bp.block_pixels, bx * 6, by * 6, 6, 6);
                let mut block_pixels_itp = [[0f32; 4]; 36];
                pass_src_img_itp.extract_block_clamped(&mut block_pixels_itp, bx * 6, by * 6, 6, 6);

                let mut is_grayscale = true;
                candidates.clear();
                let (mut block_ly, mut block_hy, mut block_avg_y) = (BIG_FLOAT_VAL, 0f32, 0f32);
                for i in 0..36 {
                    let mut rgb_input = [0f32; 3];
                    for c in 0..3 {
                        let v = bp.block_pixels[i][c];
                        rgb_input[c] = v;
                        let h = fast_float_to_half_no_clamp(v);
                        bp.half_pixels[i][c] = h;
                        bp.block_pixels_q16[i][c] =
                            super::astc_hdr_enc::half_to_qlog16(h as i32) as f32;
                        bp.half_pixels_as_floats[i][c] = h as f32;
                    }
                    let py = dot3(rgb_input, [REC_709_R, REC_709_G, REC_709_B]);
                    if py < block_ly {
                        block_ly = py;
                    }
                    if py > block_hy {
                        block_hy = py;
                    }
                    block_avg_y += py;
                    bp.block_pixels_as_itp[i] = [
                        block_pixels_itp[i][0],
                        block_pixels_itp[i][1],
                        block_pixels_itp[i][2],
                    ];
                    bp.block_pixels_q16[i][3] = 0.0;
                    let hp = bp.half_pixels[i];
                    if hp[0] != hp[1] || hp[0] != hp[2] {
                        is_grayscale = false;
                    }
                }
                block_avg_y *= 1.0 / 36.0;

                let mut enc_block_stats = EncodeAstcBlockStats::default();
                enc_block_stats.init(36, &bp.block_pixels_q16);

                let mut x_filtered = [[0f32; 4]; 36];
                let mut y_filtered = [[0f32; 4]; 36];
                filter_block_v4(3, 6, &bp.block_pixels, &mut x_filtered);
                filter_block_v4(6, 3, &bp.block_pixels, &mut y_filtered);
                let filtered_x_err = diff_blocks(&bp.block_pixels, &x_filtered);
                let filtered_y_err = diff_blocks(&bp.block_pixels, &y_filtered);
                let filter_horizontally = filtered_x_err < filtered_y_err;

                let mut lowpass_filtered = [[0f32; 3]; 36];
                filter_block_v3(3, 3, &bp.half_pixels_as_floats, &mut lowpass_filtered);
                let lowpass_std_dev =
                    sub_and_compute_std_dev(&lowpass_filtered, &bp.half_pixels_as_floats);
                let very_detailed_block = lowpass_std_dev > 350.0;
                let very_blurry_block = lowpass_std_dev < 30.0;
                let super_blurry_block = lowpass_std_dev < 15.0;

                let chans: [Vec<f32>; 3] =
                    [0, 1, 2].map(|c| bp.half_pixels_as_floats.iter().map(|p| p[c]).collect());
                let half_comp_stats: [StatsF; 3] =
                    [0, 1, 2].map(|c| StatsF::calc(chans[c].iter().copied()));

                const SINGLE_PART_HALF_THRESH: f32 = 256.0;
                const COMPLEX_HALF_THRESH: f32 = 1024.0;
                const VERY_COMPLEX_HALF_THRESH: f32 = 1400.0;
                let max_std_dev = maxf(
                    maxf(half_comp_stats[0].std_dev, half_comp_stats[1].std_dev),
                    half_comp_stats[2].std_dev,
                );
                let very_simple_block = max_std_dev < SINGLE_PART_HALF_THRESH;
                let complex_block = max_std_dev > COMPLEX_HALF_THRESH;
                let very_complex_block = max_std_dev > VERY_COMPLEX_HALF_THRESH;

                // Dynamically choose a comp_level for this block.
                let mut coptions = enc_state.master_coptions;
                let mut comp_level = global_cfg.master_comp_level;
                if very_complex_block {
                    comp_level = global_cfg.highest_comp_level;
                } else if complex_block {
                    comp_level =
                        (global_cfg.master_comp_level + global_cfg.highest_comp_level).div_ceil(2);
                }
                let level_allows = |i: usize| -> bool {
                    let f = descs[i].flags;
                    match comp_level {
                        0 => f & BASIST_HDR_6X6_LEVEL0 != 0,
                        1 => f & BASIST_HDR_6X6_LEVEL1 != 0,
                        2 => f & BASIST_HDR_6X6_LEVEL2 != 0,
                        _ => true,
                    }
                };
                let (
                    mut any_2subset_enabled,
                    mut any_2subset_mode7_enabled,
                    mut any_3subset_enabled,
                ) = (false, false, false);
                for i in 0..BM_TOTAL_MODES as usize {
                    if !level_allows(i) {
                        continue;
                    }
                    if descs[i].num_partitions == 2 {
                        any_2subset_enabled = true;
                        if descs[i].cem == 7 {
                            any_2subset_mode7_enabled = true;
                        }
                    } else if descs[i].num_partitions == 3 {
                        any_3subset_enabled = true;
                    }
                }
                coptions.mode7_full_s_optimization = comp_level >= 2;
                let uber_mode_flag = comp_level >= 3;
                coptions.allow_uber_mode = uber_mode_flag;
                coptions.ultra_quant = comp_level >= 4;
                coptions.take_first_non_clamping_mode11_submode = comp_level <= 2;
                coptions.take_first_non_clamping_mode7_submode = comp_level <= 2;
                coptions.disable_weight_plane_optimization = comp_level >= 2;

                let total_used_block_chans: u32 = half_comp_stats
                    .iter()
                    .map(|s| u32::from(s.range > 0.0))
                    .sum();
                let is_solid_block = total_used_block_chans == 0;

                let pearson = |a: usize, b: usize| {
                    calc_pearson(
                        &chans[a],
                        &chans[b],
                        &half_comp_stats[a],
                        &half_comp_stats[b],
                    )
                };
                let cross = [pearson(0, 1), pearson(0, 2), pearson(1, 2)];
                let rg_corr = cross[0].abs();
                let rb_corr = cross[1].abs();
                let gb_corr = cross[2].abs();
                let (mut min_corr, mut max_corr) = (BIG_FLOAT_VAL, -BIG_FLOAT_VAL);
                const CHAN_PAIRS: [[usize; 2]; 3] = [[0, 1], [0, 2], [1, 2]];
                for i in 0..3 {
                    let [a, b] = CHAN_PAIRS[i];
                    if half_comp_stats[a].range > 0.0 && half_comp_stats[b].range > 0.0 {
                        let c = cross[i].abs();
                        min_corr = minf32(min_corr, c);
                        max_corr = maxf(max_corr, c);
                    }
                }
                let _ = max_corr;

                let mut use_single_subset_mode7 = true;
                if comp_level <= 1 {
                    const MODE7_MIN_CHAN_CORR: f32 = 0.5;
                    const MODE7_PCA_ANGLE_THRESH: f32 = 0.9;
                    use_single_subset_mode7 = is_grayscale
                        || is_solid_block
                        || total_used_block_chans == 1
                        || min_corr >= MODE7_MIN_CHAN_CORR;
                    if use_single_subset_mode7 {
                        let cos_ang = dot3(enc_block_stats.axis_q16, [0.5773502691; 3]).abs();
                        if cos_ang < MODE7_PCA_ANGLE_THRESH {
                            use_single_subset_mode7 = false;
                        }
                    }
                }

                let strong_corr_thresh = if comp_level <= 1 {
                    0.5
                } else if comp_level <= 3 {
                    0.75
                } else {
                    0.9
                };
                let mut desired_dp_chan: i32 = -1;
                if total_used_block_chans > 1 && min_corr < strong_corr_thresh {
                    if total_used_block_chans == 2 {
                        if half_comp_stats[0].range == 0.0 {
                            if gb_corr < strong_corr_thresh {
                                desired_dp_chan = 1;
                            }
                        } else if half_comp_stats[1].range == 0.0 {
                            if rb_corr < strong_corr_thresh {
                                desired_dp_chan = 0;
                            }
                        } else if rg_corr < strong_corr_thresh {
                            desired_dp_chan = 0;
                        }
                    } else if rg_corr < gb_corr && rb_corr < gb_corr {
                        desired_dp_chan = 0;
                    } else if rg_corr < rb_corr && gb_corr < rb_corr {
                        desired_dp_chan = 1;
                    } else {
                        desired_dp_chan = 2;
                    }
                }
                // 2x2 is special for RDO at higher lambdas - always pick a preferred channel.
                let mut desired_dp_chan_2x2: i32 = 0;
                if total_used_block_chans == 2 {
                    if half_comp_stats[0].range == 0.0 {
                        desired_dp_chan_2x2 = 1;
                    }
                } else if total_used_block_chans == 3 {
                    desired_dp_chan_2x2 = if rg_corr < gb_corr && rb_corr < gb_corr {
                        0
                    } else if rg_corr < rb_corr && gb_corr < rb_corr {
                        1
                    } else {
                        2
                    };
                }

                // ---- Run candidate
                if global_cfg.use_runs && (has_left_neighbor || has_upper_neighbor) {
                    let mut candidate = CandidateEncoding {
                        encoding_type: ET_RUN,
                        decomp_log_blk: prev_non_run_candidate_encoding.decomp_log_blk,
                        coded_log_blk: prev_non_run_candidate_encoding.coded_log_blk,
                        comp_pixels: prev_comp_pixels,
                        ..Default::default()
                    };
                    if prev_run_len == 0 {
                        candidate.coder.put_bits(RUN_CODE, RUN_CODE_LEN);
                        candidate.coder.put_vlc(0, 5);
                    } else {
                        // extend current run - compute the # of new bits needed for the extension.
                        let prev_run_bits = prev_encoding.get_total_bits_u32();
                        let mut temp_coder = BitwiseCoder::new();
                        temp_coder.put_bits(RUN_CODE, RUN_CODE_LEN);
                        temp_coder.put_vlc((prev_run_len + 1) - 1, 5);
                        let cur_run_bits = temp_coder.get_total_bits_u32();
                        let total_new_bits = cur_run_bits.saturating_sub(prev_run_bits);
                        if total_new_bits > 0 {
                            candidate.coder.put_bits(0, total_new_bits); // dummy bits
                        }
                    }
                    candidate.run_len = prev_run_len + 1;
                    candidates.push(candidate);
                }

                // ---- Reuse candidate
                if !is_solid_block && global_cfg.lambda > 0.0 {
                    for reuse_delta_index in 0..global_cfg.num_reuse_xy_deltas {
                        let [dx, dy] = REUSE_XY_DELTAS[reuse_delta_index as usize];
                        let reuse_bx = bx as i32 + dx as i32;
                        let reuse_by = by as i32 + dy as i32;
                        if reuse_bx < 0 || reuse_bx >= num_blocks_x as i32 {
                            continue;
                        }
                        if reuse_by < strip_first_by as i32 {
                            break;
                        }
                        let prev_candidate =
                            &enc_state.coded_blocks[reuse_bx as usize + reuse_by as usize * nbx];
                        // TODO - support this.
                        if prev_candidate.encoding_type == ET_SOLID {
                            continue;
                        }
                        let mut candidate = CandidateEncoding::default();
                        let prev_coded = prev_candidate.coded_log_blk;
                        let grid_x = prev_coded.grid_width as u32;
                        let grid_y = prev_coded.grid_height as u32;
                        let dual_plane = prev_coded.dual_plane;
                        let num_grid_samples = (grid_x * grid_y) as usize;
                        let num_endpoint_vals =
                            get_num_endpoint_vals(prev_coded.color_endpoint_modes[0] as u32);
                        candidate.coded_log_blk = prev_coded;
                        candidate.decomp_log_blk = prev_candidate.decomp_log_blk;
                        let prev_block_mode = prev_candidate.block_mode;
                        let prev_endpoint_mode = prev_candidate.endpoint_mode;
                        let cl = &mut candidate.coded_log_blk;
                        let wr = cl.weight_ise_range as u32;
                        let er = cl.endpoint_ise_range as u32;
                        let levels = ah::get_ise_levels(wr) as usize;
                        let cem = prev_coded.color_endpoint_modes[0] as u32;
                        let block_colors = |endpoints: &[u8], decoded: &mut [[HalfFloat; 3]]| {
                            if cem == 7 {
                                get_astc_hdr_mode_7_block_colors(endpoints, decoded, levels, wr, er)
                            } else {
                                get_astc_hdr_mode_11_block_colors(
                                    endpoints, decoded, levels, wr, er,
                                )
                            }
                        };
                        let mut transcode_weights = [0u8; 72];
                        if prev_coded.num_partitions == 1 {
                            let mut decoded_half = [[0 as HalfFloat; 3]; 32];
                            block_colors(&cl.endpoints, &mut decoded_half);
                            let mut trial_weights0 = [0u8; 36];
                            let mut trial_weights1 = [0u8; 36];
                            if dual_plane {
                                eval_selectors_dual_plane(
                                    prev_coded.color_component_selector as usize,
                                    36,
                                    &mut trial_weights0,
                                    &mut trial_weights1,
                                    &bp.half_pixels,
                                    levels,
                                    &decoded_half,
                                    &coptions,
                                    u32::MAX,
                                );
                                downsample_ise_weights_dual_plane(
                                    wr,
                                    wr,
                                    6,
                                    6,
                                    grid_x,
                                    grid_y,
                                    &trial_weights0,
                                    &trial_weights1,
                                    &mut cl.weights,
                                );
                                requantize_astc_weights(
                                    num_grid_samples * 2,
                                    &cl.weights,
                                    wr,
                                    &mut transcode_weights,
                                    candidate.decomp_log_blk.weight_ise_range as u32,
                                );
                            } else {
                                eval_selectors(
                                    36,
                                    &mut trial_weights0,
                                    wr,
                                    &bp.half_pixels,
                                    levels,
                                    &decoded_half,
                                    &coptions,
                                    u32::MAX,
                                );
                                downsample_ise_weights(
                                    wr,
                                    wr,
                                    6,
                                    6,
                                    grid_x,
                                    grid_y,
                                    &trial_weights0,
                                    &mut cl.weights,
                                );
                                requantize_astc_weights(
                                    num_grid_samples,
                                    &cl.weights,
                                    wr,
                                    &mut transcode_weights,
                                    candidate.decomp_log_blk.weight_ise_range as u32,
                                );
                            }
                        } else {
                            let np = prev_coded.num_partitions as usize;
                            let pat = if np == 2 {
                                let u = t.part2_seed_to_unique_index
                                    [prev_coded.partition_id as usize & 1023];
                                &t.partitions2[u as usize]
                            } else {
                                let u = t.part3_seed_to_unique_index
                                    [prev_coded.partition_id as usize & 1023];
                                &t.partitions3[u.max(0) as usize]
                            };
                            let mut part_half_pixels = [[[0 as HalfFloat; 3]; 36]; 3];
                            let mut part_total_pixels = [0usize; 3];
                            for i in 0..36 {
                                let p = pat[i] as usize % np;
                                part_half_pixels[p][part_total_pixels[p]] = bp.half_pixels[i];
                                part_total_pixels[p] += 1;
                            }
                            let mut blk_weights = [[0u8; 36]; 3];
                            for p in 0..np {
                                let mut decoded_half = [[0 as HalfFloat; 3]; 32];
                                if np == 3 {
                                    get_astc_hdr_mode_7_block_colors(
                                        &cl.endpoints[num_endpoint_vals * p..],
                                        &mut decoded_half,
                                        levels,
                                        wr,
                                        er,
                                    );
                                } else {
                                    block_colors(
                                        &cl.endpoints[num_endpoint_vals * p..],
                                        &mut decoded_half,
                                    );
                                }
                                eval_selectors(
                                    part_total_pixels[p],
                                    &mut blk_weights[p],
                                    wr,
                                    &part_half_pixels[p],
                                    levels,
                                    &decoded_half,
                                    &coptions,
                                    u32::MAX,
                                );
                            }
                            let mut ise_weights = [0u8; 36];
                            let mut src_pixel_index = [0usize; 3];
                            for i in 0..36 {
                                let p = pat[i] as usize % np;
                                ise_weights[i] = blk_weights[p][src_pixel_index[p]];
                                src_pixel_index[p] += 1;
                            }
                            downsample_ise_weights(
                                wr,
                                wr,
                                6,
                                6,
                                grid_x,
                                grid_y,
                                &ise_weights,
                                &mut cl.weights,
                            );
                            requantize_astc_weights(
                                num_grid_samples,
                                &cl.weights,
                                wr,
                                &mut transcode_weights,
                                candidate.decomp_log_blk.weight_ise_range as u32,
                            );
                        }
                        // Create the block the decoder would transcode into.
                        super::astc_6x6_hdr::copy_weight_grid(
                            dual_plane,
                            grid_x,
                            grid_y,
                            &transcode_weights,
                            &mut candidate.decomp_log_blk,
                            use_orig_behavior,
                        );
                        if !validate_log_blk(&candidate.decomp_log_blk) {
                            return false;
                        }
                        if !decode_astc_block(&candidate.decomp_log_blk, &mut candidate.comp_pixels)
                        {
                            return false;
                        }
                        candidate.coder.put_bits(REUSE_CODE, REUSE_CODE_LEN);
                        candidate
                            .coder
                            .put_bits(reuse_delta_index, super::astc_6x6_hdr::REUSE_XY_DELTA_BITS);
                        let cl = candidate.coded_log_blk;
                        encode_values(
                            &mut candidate.coder,
                            num_grid_samples * if dual_plane { 2 } else { 1 },
                            &cl.weights,
                            cl.weight_ise_range as u32,
                        );
                        candidate.encoding_type = ET_REUSE;
                        candidate.block_mode = prev_block_mode;
                        candidate.endpoint_mode = prev_endpoint_mode;
                        candidate.reuse_delta_index = reuse_delta_index as i32;
                        candidates.push(candidate);
                    }
                }

                // ---- Solid candidate
                if global_cfg.use_solid_blocks {
                    let mut candidate = CandidateEncoding {
                        encoding_type: ET_SOLID,
                        ..Default::default()
                    };
                    let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
                    const LOG_BIAS: f32 = 0.125;
                    let mut solid_block = true;
                    for i in 0..36 {
                        let p = &bp.block_pixels[i];
                        let p0 = &bp.block_pixels[0];
                        if p0[0] != p[0] || p0[1] != p[1] || p0[2] != p[2] {
                            solid_block = false;
                        }
                        r += (p[0] + LOG_BIAS).log2();
                        g += (p[1] + LOG_BIAS).log2();
                        b += (p[2] + LOG_BIAS).log2();
                    }
                    if solid_block {
                        r = bp.block_pixels[0][0];
                        g = bp.block_pixels[0][1];
                        b = bp.block_pixels[0][2];
                    } else {
                        r = maxf(0.0, 2.0f32.powf(r * (1.0 / 36.0)) - LOG_BIAS);
                        g = maxf(0.0, 2.0f32.powf(g * (1.0 / 36.0)) - LOG_BIAS);
                        b = maxf(0.0, 2.0f32.powf(b * (1.0 / 36.0)) - LOG_BIAS);
                        r = minf32(r, 65504.0);
                        g = minf32(g, 65504.0);
                        b = minf32(b, 65504.0);
                    }
                    let rh = super::astc_hdr_enc::float_to_half_non_neg_no_nan_inf(r);
                    let gh = super::astc_hdr_enc::float_to_half_non_neg_no_nan_inf(g);
                    let bh = super::astc_hdr_enc::float_to_half_non_neg_no_nan_inf(b);
                    candidate.solid_color = [rh, gh, bh];
                    candidate.coder.put_bits(SOLID_CODE, SOLID_CODE_LEN);
                    candidate.coder.put_bits(rh as u32, 15);
                    candidate.coder.put_bits(gh as u32, 15);
                    candidate.coder.put_bits(bh as u32, 15);
                    let cp = [half_to_float(rh), half_to_float(gh), half_to_float(bh)];
                    candidate.comp_pixels = [cp; 36];
                    let lb = &mut candidate.coded_log_blk;
                    lb.clear();
                    lb.solid_color_flag_hdr = true;
                    lb.set_solid_color(0, rh);
                    lb.set_solid_color(1, gh);
                    lb.set_solid_color(2, bh);
                    lb.set_solid_color(3, float_to_half(1.0, false));
                    candidate.decomp_log_blk = candidate.coded_log_blk;
                    candidates.push(candidate);
                }

                if !is_solid_block || !global_cfg.use_solid_blocks {
                    const PARTS2_NORMAL: [u32; 5] = [0, 2, 4, 6, 8];
                    const PARTS3_NORMAL: [u32; 5] = [0, 0, 4, 6, 8];
                    const PARTS2_COMPLEX: [u32; 5] = [0, 4, 8, 10, 16];
                    const PARTS3_COMPLEX: [u32; 5] = [0, 0, 8, 10, 16];
                    const PARTS2_VERY_COMPLEX: [u32; 5] = [0, 8, 12, 14, 20];
                    const PARTS3_VERY_COMPLEX: [u32; 5] = [0, 0, 12, 14, 20];
                    let cl_i = (comp_level as usize).min(4);
                    let (mut total_parts2, mut total_parts3);
                    if very_simple_block && comp_level <= 3 {
                        // 2-3 subsets are unlikely to help much
                        total_parts2 = 0;
                        total_parts3 = 0;
                    } else if very_complex_block {
                        total_parts2 = PARTS2_VERY_COMPLEX[cl_i];
                        total_parts3 = PARTS3_VERY_COMPLEX[cl_i];
                        if global_cfg.extra_patterns_flag {
                            total_parts2 += if comp_level == 4 { 30 } else { 20 };
                            total_parts3 += if comp_level == 4 { 30 } else { 20 };
                        }
                    } else if complex_block {
                        total_parts2 = PARTS2_COMPLEX[cl_i];
                        total_parts3 = PARTS3_COMPLEX[cl_i];
                        if global_cfg.extra_patterns_flag {
                            total_parts2 += if comp_level == 4 { 15 } else { 10 };
                            total_parts3 += if comp_level == 4 { 15 } else { 10 };
                        }
                    } else {
                        total_parts2 = PARTS2_NORMAL[cl_i];
                        total_parts3 = PARTS3_NORMAL[cl_i];
                        if global_cfg.extra_patterns_flag {
                            total_parts2 += 5;
                            total_parts3 += 5;
                        }
                    }
                    if !any_2subset_enabled {
                        total_parts2 = 0;
                    }
                    if !any_3subset_enabled {
                        total_parts3 = 0;
                    }
                    let total_parts2 = total_parts2 as usize;
                    let total_parts3 = total_parts3 as usize;

                    let mut best_parts2_mode11 = vec![0i32; NUM_UNIQUE_PARTITIONS2 as usize];
                    let mut best_parts2_mode7 = vec![0i32; NUM_UNIQUE_PARTITIONS2 as usize];
                    let mut has_estimated_parts2 = false;
                    if total_parts2 != 0 {
                        if global_cfg.brute_force_partition_matching {
                            let all2: Vec<u32> = (0..NUM_UNIQUE_PARTITIONS2).collect();
                            if any_2subset_enabled {
                                estimate_partitions_mode7_and_11(
                                    2,
                                    &t.partitions2,
                                    &all2,
                                    &bp.half_pixels_as_floats,
                                    total_parts2,
                                    &mut best_parts2_mode11,
                                    &mut best_parts2_mode7,
                                );
                            }
                            has_estimated_parts2 = true;
                        } else if comp_level >= 1 {
                            const MAX_CANDIDATES2: usize = 48;
                            let mut candidate_pats2 = [0i32; MAX_CANDIDATES2 * 2];
                            let mut num_candidate_pats2 =
                                ((total_parts2 * 3) / 2).max(if very_complex_block {
                                    MAX_CANDIDATES2
                                } else {
                                    MAX_CANDIDATES2 / 2
                                });
                            num_candidate_pats2 = num_candidate_pats2.min(candidate_pats2.len());
                            has_estimated_parts2 = estimate_partition2_6x6(
                                &bp.half_pixels,
                                &mut candidate_pats2,
                                num_candidate_pats2,
                            );
                            if has_estimated_parts2 {
                                let c: Vec<u32> = candidate_pats2[..num_candidate_pats2]
                                    .iter()
                                    .map(|&v| v as u32)
                                    .collect();
                                estimate_partitions_mode7_and_11(
                                    2,
                                    &t.partitions2,
                                    &c,
                                    &bp.half_pixels_as_floats,
                                    total_parts2,
                                    &mut best_parts2_mode11,
                                    &mut best_parts2_mode7,
                                );
                            }
                        } else {
                            has_estimated_parts2 = estimate_partition2_6x6(
                                &bp.half_pixels,
                                &mut best_parts2_mode11,
                                total_parts2,
                            );
                            if has_estimated_parts2 && any_2subset_mode7_enabled {
                                let src = best_parts2_mode11[..total_parts2].to_vec();
                                best_parts2_mode7[..total_parts2].copy_from_slice(&src);
                            }
                        }
                    }

                    let mut best_parts3 = vec![0i32; NUM_UNIQUE_PARTITIONS3 as usize];
                    let mut has_estimated_parts3 = false;
                    if total_parts3 != 0 {
                        if global_cfg.brute_force_partition_matching {
                            let all3: Vec<u32> = (0..NUM_UNIQUE_PARTITIONS3).collect();
                            estimate_partitions_mode7(
                                3,
                                &t.partitions3,
                                &all3,
                                &bp.half_pixels_as_floats,
                                total_parts3,
                                &mut best_parts3,
                            );
                            has_estimated_parts3 = true;
                        } else {
                            const MAX_CANDIDATES3: usize = 48;
                            let mut candidate_pats3 = [0i32; MAX_CANDIDATES3 * 2];
                            let mut num_candidate_pats3 =
                                ((total_parts3 * 3) / 2).max(if very_complex_block {
                                    MAX_CANDIDATES3
                                } else {
                                    MAX_CANDIDATES3 / 2
                                });
                            num_candidate_pats3 = num_candidate_pats3.min(candidate_pats3.len());
                            has_estimated_parts3 = estimate_partition3_6x6(
                                &bp.half_pixels,
                                &mut candidate_pats3,
                                num_candidate_pats3,
                            );
                            if has_estimated_parts3 {
                                let c: Vec<u32> = candidate_pats3[..num_candidate_pats3]
                                    .iter()
                                    .map(|&v| v as u32)
                                    .collect();
                                estimate_partitions_mode7(
                                    3,
                                    &t.partitions3,
                                    &c,
                                    &bp.half_pixels_as_floats,
                                    total_parts3,
                                    &mut best_parts3,
                                );
                            }
                        }
                    }

                    let mode11_opt_mode = if complex_block {
                        OptMode::WeightedLeastSquares
                    } else {
                        OptMode::OrdinaryLeastSquares
                    };

                    // ---- Encoded block candidates
                    for block_mode_iter in 0..BM_TOTAL_MODES as usize {
                        let md = descs[block_mode_iter];
                        if !level_allows(block_mode_iter) {
                            continue;
                        }
                        if global_cfg.block_stat_optimizations_flag {
                            if comp_level <= 3 && md.dp {
                                let want = if global_cfg.lambda > 0.0
                                    && !complex_block
                                    && md.grid_x == 2
                                    && md.grid_y == 2
                                {
                                    desired_dp_chan_2x2
                                } else {
                                    desired_dp_chan
                                };
                                if md.dp_channel as i32 != want {
                                    continue;
                                }
                            }
                            if comp_level <= 3 {
                                let (grid_x, grid_y) = (md.grid_x, md.grid_y);
                                if !md.dp {
                                    if very_detailed_block && grid_x * grid_y <= 12 {
                                        continue;
                                    }
                                    if very_blurry_block
                                        && (grid_x > 4 || grid_y > 4 || md.num_partitions > 1)
                                    {
                                        continue;
                                    }
                                    if super_blurry_block
                                        && (grid_x > 3 || grid_y > 3 || md.num_partitions > 1)
                                    {
                                        continue;
                                    }
                                }
                                if grid_x != grid_y {
                                    if grid_x < grid_y {
                                        if !filter_horizontally {
                                            continue;
                                        }
                                    } else if filter_horizontally {
                                        continue;
                                    }
                                }
                            }
                            // Rarely useful if lambda=0
                            if global_cfg.lambda == 0.0 && md.grid_x == 2 && md.grid_y == 2 {
                                continue;
                            }
                        }
                        if !use_single_subset_mode7 && md.cem == 7 && md.num_partitions == 1 {
                            continue;
                        }

                        for em in 0..EM_TOTAL {
                            if global_cfg.lambda == 0.0 && em != EM_RAW {
                                continue;
                            }
                            if global_cfg.disable_delta_endpoint_usage
                                && (em == EM_USE_UPPER_DELTA || em == EM_USE_LEFT_DELTA)
                            {
                                continue;
                            }
                            if !global_cfg.favor_higher_compression {
                                if comp_level == 0 && em == EM_USE_UPPER_DELTA {
                                    continue;
                                }
                                if comp_level <= 1 && (em == EM_USE_LEFT || em == EM_USE_UPPER) {
                                    continue;
                                }
                            }
                            let cem = md.cem;
                            match em {
                                EM_USE_LEFT | EM_USE_UPPER => {
                                    if md.num_partitions > 1 {
                                        continue;
                                    }
                                    if (em == EM_USE_LEFT && !has_left_neighbor)
                                        || (em == EM_USE_UPPER && !has_upper_neighbor)
                                    {
                                        continue;
                                    }
                                    let (nx, ny) = if em == EM_USE_LEFT {
                                        (bx - 1, by)
                                    } else {
                                        (bx, by - 1)
                                    };
                                    let neighbor_blk =
                                        &enc_state.coded_blocks[nx as usize + ny as usize * nbx];
                                    if neighbor_blk.encoding_type == ET_SOLID {
                                        continue;
                                    }
                                    let neighbor_md =
                                        descs[neighbor_blk.block_mode.max(0) as usize];
                                    if neighbor_md.cem != cem {
                                        continue;
                                    }
                                    let (grid_x, grid_y) = (md.grid_x, md.grid_y);
                                    let dual_plane = md.dp;
                                    let num_endpoint_vals = get_num_endpoint_vals(md.cem);
                                    let mut candidate = CandidateEncoding::default();
                                    {
                                        let cl = &mut candidate.coded_log_blk;
                                        cl.grid_width = grid_x as u8;
                                        cl.grid_height = grid_y as u8;
                                        cl.dual_plane = dual_plane;
                                        cl.color_component_selector = md.dp_channel as u8;
                                        cl.num_partitions = 1;
                                        cl.color_endpoint_modes[0] = neighbor_md.cem as u8;
                                        cl.weight_ise_range = md.weight_ise_range as u8;
                                        // Reuse the neighbor's endpoints unchanged (so no loss).
                                        cl.endpoint_ise_range =
                                            neighbor_blk.coded_log_blk.endpoint_ise_range;
                                        cl.endpoints[..num_endpoint_vals].copy_from_slice(
                                            &neighbor_blk.coded_log_blk.endpoints
                                                [..num_endpoint_vals],
                                        );
                                    }
                                    let mut transcode_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
                                    requantize_ise_endpoints(
                                        neighbor_md.cem,
                                        neighbor_blk.coded_log_blk.endpoint_ise_range as u32,
                                        &neighbor_blk.coded_log_blk.endpoints,
                                        md.transcode_endpoint_ise_range,
                                        &mut transcode_endpoints,
                                    );
                                    let levels = ah::get_ise_levels(md.weight_ise_range) as usize;
                                    let mut decoded_half = [[0 as HalfFloat; 3]; 32];
                                    let ok = if cem == 7 {
                                        get_astc_hdr_mode_7_block_colors(
                                            &transcode_endpoints,
                                            &mut decoded_half,
                                            levels,
                                            md.weight_ise_range,
                                            md.transcode_endpoint_ise_range,
                                        )
                                    } else {
                                        get_astc_hdr_mode_11_block_colors(
                                            &transcode_endpoints,
                                            &mut decoded_half,
                                            levels,
                                            md.weight_ise_range,
                                            md.transcode_endpoint_ise_range,
                                        )
                                    };
                                    if !ok {
                                        continue;
                                    }
                                    let mut trial_weights0 = [0u8; 36];
                                    let mut trial_weights1 = [0u8; 36];
                                    let wr = md.weight_ise_range;
                                    if dual_plane {
                                        eval_selectors_dual_plane(
                                            md.dp_channel as usize,
                                            36,
                                            &mut trial_weights0,
                                            &mut trial_weights1,
                                            &bp.half_pixels,
                                            levels,
                                            &decoded_half,
                                            &coptions,
                                            u32::MAX,
                                        );
                                        downsample_ise_weights_dual_plane(
                                            wr,
                                            wr,
                                            6,
                                            6,
                                            grid_x,
                                            grid_y,
                                            &trial_weights0,
                                            &trial_weights1,
                                            &mut candidate.coded_log_blk.weights,
                                        );
                                    } else {
                                        eval_selectors(
                                            36,
                                            &mut trial_weights0,
                                            wr,
                                            &bp.half_pixels,
                                            levels,
                                            &decoded_half,
                                            &coptions,
                                            u32::MAX,
                                        );
                                        downsample_ise_weights(
                                            wr,
                                            wr,
                                            6,
                                            6,
                                            grid_x,
                                            grid_y,
                                            &trial_weights0,
                                            &mut candidate.coded_log_blk.weights,
                                        );
                                    }
                                    pack_weights_and_decomp(
                                        &mut candidate,
                                        &md,
                                        &transcode_endpoints,
                                        use_orig_behavior,
                                    );
                                    if !validate_log_blk(&candidate.decomp_log_blk) {
                                        return false;
                                    }
                                    if !decode_astc_block(
                                        &candidate.decomp_log_blk,
                                        &mut candidate.comp_pixels,
                                    ) {
                                        return false;
                                    }
                                    candidate.coder.put_bits(BLOCK_CODE, BLOCK_CODE_LEN);
                                    let cl = candidate.coded_log_blk;
                                    code_block(
                                        &mut candidate.coder,
                                        &cl,
                                        block_mode_iter as u32,
                                        em,
                                        None,
                                    );
                                    candidate.encoding_type = ET_BLOCK;
                                    candidate.endpoint_mode = em as i32;
                                    candidate.block_mode = block_mode_iter as i32;
                                    candidates.push(candidate);
                                }
                                EM_USE_LEFT_DELTA | EM_USE_UPPER_DELTA => {
                                    if md.num_partitions > 1 {
                                        continue;
                                    }
                                    if (em == EM_USE_LEFT_DELTA && !has_left_neighbor)
                                        || (em == EM_USE_UPPER_DELTA && !has_upper_neighbor)
                                    {
                                        continue;
                                    }
                                    let (nx, ny) = if em == EM_USE_LEFT_DELTA {
                                        (bx - 1, by)
                                    } else {
                                        (bx, by - 1)
                                    };
                                    let neighbor_blk =
                                        &enc_state.coded_blocks[nx as usize + ny as usize * nbx];
                                    if neighbor_blk.encoding_type == ET_SOLID {
                                        continue;
                                    }
                                    let neighbor_md =
                                        descs[neighbor_blk.block_mode.max(0) as usize];
                                    if neighbor_md.cem != cem {
                                        continue;
                                    }
                                    let (grid_x, grid_y) = (md.grid_x, md.grid_y);
                                    let dual_plane = md.dp;
                                    let num_endpoint_vals = get_num_endpoint_vals(md.cem);
                                    // Dequantize neighbor's endpoints to ISE 20
                                    let mut neighbor_endpoints_ise20 = [0u8; NUM_MODE11_ENDPOINTS];
                                    requantize_ise_endpoints(
                                        neighbor_md.cem,
                                        neighbor_blk.coded_log_blk.endpoint_ise_range as u32,
                                        &neighbor_blk.coded_log_blk.endpoints,
                                        ah::BISE_256_LEVELS,
                                        &mut neighbor_endpoints_ise20,
                                    );
                                    // Requantize neighbor's endpoints to our local coding ISE range
                                    let mut neighbor_local = [0u8; NUM_MODE11_ENDPOINTS];
                                    requantize_ise_endpoints(
                                        neighbor_md.cem,
                                        ah::BISE_256_LEVELS,
                                        &neighbor_endpoints_ise20,
                                        md.endpoint_ise_range,
                                        &mut neighbor_local,
                                    );
                                    let mut blk_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
                                    let mut blk_weights0 = [0u8; 36];
                                    let mut blk_weights1 = [0u8; 36];
                                    let mut best_submode = 0u32;
                                    let err = if cem == 7 {
                                        let (submode_index, _maj) =
                                            decode_cem_7_config(&neighbor_endpoints_ise20);
                                        encode_astc_hdr_block_mode_7(
                                            36,
                                            &bp.half_pixels,
                                            &bp.block_pixels_q16,
                                            md.weight_ise_range,
                                            &mut best_submode,
                                            BIG_FLOAT_VAL as f64,
                                            &mut blk_endpoints,
                                            &mut blk_weights0,
                                            &coptions,
                                            md.endpoint_ise_range,
                                            submode_index,
                                            submode_index,
                                            Some(&enc_block_stats),
                                        )
                                    } else {
                                        let (submode_index, maj_index) =
                                            decode_cem_11_config(&neighbor_endpoints_ise20);
                                        let (first_submode, last_submode) = if maj_index == 3 {
                                            (-1, -1)
                                        } else {
                                            (submode_index, submode_index)
                                        };
                                        if dual_plane {
                                            encode_astc_hdr_block_mode_11_dual_plane(
                                                36,
                                                &bp.half_pixels,
                                                &bp.block_pixels_q16,
                                                md.dp_channel as usize,
                                                md.weight_ise_range,
                                                &mut best_submode,
                                                BIG_FLOAT_VAL as f64,
                                                &mut blk_endpoints,
                                                &mut blk_weights0,
                                                &mut blk_weights1,
                                                &coptions,
                                                false,
                                                md.endpoint_ise_range,
                                                false,
                                                false,
                                                first_submode,
                                                last_submode,
                                                true,
                                            )
                                        } else {
                                            encode_astc_hdr_block_mode_11(
                                                36,
                                                &bp.half_pixels,
                                                &bp.block_pixels_q16,
                                                md.weight_ise_range,
                                                &mut best_submode,
                                                BIG_FLOAT_VAL as f64,
                                                &mut blk_endpoints,
                                                &mut blk_weights0,
                                                &coptions,
                                                false,
                                                md.endpoint_ise_range,
                                                false,
                                                false,
                                                first_submode,
                                                last_submode,
                                                true,
                                                mode11_opt_mode,
                                                Some(&enc_block_stats),
                                            )
                                        }
                                    };
                                    if err == BIG_FLOAT_VAL as f64 {
                                        continue;
                                    }
                                    // 5 bits per endpoint delta, computed in ISE rank space.
                                    let total_endpoint_delta_vals =
                                        1i32 << super::astc_6x6_hdr::NUM_ENDPOINT_DELTA_BITS;
                                    let low_delta_limit = -(total_endpoint_delta_vals / 2);
                                    let high_delta_limit = (total_endpoint_delta_vals / 2) - 1;
                                    let ise_to_rank = &dequant_tables()
                                        .endpoint_tab(md.endpoint_ise_range)
                                        .ise_to_rank;
                                    let rank = |v: u8| {
                                        ise_to_rank.get(v as usize).copied().unwrap_or(0) as i32
                                    };
                                    let mut all_deltas_in_limits = true;
                                    let mut endpoint_deltas = [0u8; NUM_MODE11_ENDPOINTS];
                                    for i in 0..num_endpoint_vals {
                                        let endpoint_delta =
                                            rank(blk_endpoints[i]) - rank(neighbor_local[i]);
                                        if endpoint_delta < low_delta_limit
                                            || endpoint_delta > high_delta_limit
                                        {
                                            all_deltas_in_limits = false;
                                        }
                                        endpoint_deltas[i] =
                                            (endpoint_delta - low_delta_limit) as u8;
                                    }
                                    if !all_deltas_in_limits {
                                        continue;
                                    }
                                    let mut candidate = CandidateEncoding::default();
                                    {
                                        let cl = &mut candidate.coded_log_blk;
                                        cl.grid_width = grid_x as u8;
                                        cl.grid_height = grid_y as u8;
                                        cl.dual_plane = dual_plane;
                                        cl.color_component_selector = md.dp_channel as u8;
                                        cl.num_partitions = 1;
                                        cl.color_endpoint_modes[0] = md.cem as u8;
                                        cl.weight_ise_range = md.weight_ise_range as u8;
                                        cl.endpoint_ise_range = md.endpoint_ise_range as u8;
                                        cl.endpoints[..num_endpoint_vals]
                                            .copy_from_slice(&blk_endpoints[..num_endpoint_vals]);
                                    }
                                    let mut transcode_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
                                    requantize_ise_endpoints(
                                        md.cem,
                                        md.endpoint_ise_range,
                                        &blk_endpoints,
                                        md.transcode_endpoint_ise_range,
                                        &mut transcode_endpoints,
                                    );
                                    let wr = md.weight_ise_range;
                                    if dual_plane {
                                        downsample_ise_weights_dual_plane(
                                            wr,
                                            wr,
                                            6,
                                            6,
                                            grid_x,
                                            grid_y,
                                            &blk_weights0,
                                            &blk_weights1,
                                            &mut candidate.coded_log_blk.weights,
                                        );
                                    } else {
                                        downsample_ise_weights(
                                            wr,
                                            wr,
                                            6,
                                            6,
                                            grid_x,
                                            grid_y,
                                            &blk_weights0,
                                            &mut candidate.coded_log_blk.weights,
                                        );
                                    }
                                    pack_weights_and_decomp(
                                        &mut candidate,
                                        &md,
                                        &transcode_endpoints,
                                        use_orig_behavior,
                                    );
                                    if !validate_log_blk(&candidate.decomp_log_blk) {
                                        return false;
                                    }
                                    if !decode_astc_block(
                                        &candidate.decomp_log_blk,
                                        &mut candidate.comp_pixels,
                                    ) {
                                        return false;
                                    }
                                    candidate.coder.put_bits(BLOCK_CODE, BLOCK_CODE_LEN);
                                    let cl = candidate.coded_log_blk;
                                    code_block(
                                        &mut candidate.coder,
                                        &cl,
                                        block_mode_iter as u32,
                                        em,
                                        Some(&endpoint_deltas),
                                    );
                                    candidate.encoding_type = ET_BLOCK;
                                    candidate.endpoint_mode = em as i32;
                                    candidate.block_mode = block_mode_iter as i32;
                                    candidates.push(candidate);
                                }
                                _ => {
                                    // EM_RAW
                                    let dual_plane = md.dp;
                                    if global_cfg.disable_twothree_subsets && md.num_partitions >= 2
                                    {
                                        continue;
                                    }
                                    if md.num_partitions == 3 {
                                        if !has_estimated_parts3 {
                                            continue;
                                        }
                                        let mut res = TrialResult::default();
                                        if !encode_block_3_subsets(
                                            &mut res,
                                            cem,
                                            md.grid_x,
                                            md.grid_y,
                                            md.weight_ise_range,
                                            md.endpoint_ise_range,
                                            &bp.half_pixels,
                                            &bp.block_pixels_q16,
                                            &coptions,
                                            uber_mode_flag,
                                            &best_parts3,
                                            total_parts3,
                                            comp_level,
                                            mode11_opt_mode,
                                        ) {
                                            continue;
                                        }
                                        let mut candidate = CandidateEncoding {
                                            coded_log_blk: res.log_blk,
                                            decomp_log_blk: res.log_blk,
                                            ..Default::default()
                                        };
                                        if !validate_log_blk(&candidate.decomp_log_blk) {
                                            return false;
                                        }
                                        if !decode_astc_block(
                                            &candidate.decomp_log_blk,
                                            &mut candidate.comp_pixels,
                                        ) {
                                            return false;
                                        }
                                        candidate.coder.put_bits(BLOCK_CODE, BLOCK_CODE_LEN);
                                        let cl = candidate.coded_log_blk;
                                        code_block(
                                            &mut candidate.coder,
                                            &cl,
                                            block_mode_iter as u32,
                                            em,
                                            None,
                                        );
                                        candidate.encoding_type = ET_BLOCK;
                                        candidate.endpoint_mode = em as i32;
                                        candidate.block_mode = block_mode_iter as i32;
                                        candidates.push(candidate);
                                    } else if md.num_partitions == 2 {
                                        if !has_estimated_parts2 {
                                            continue;
                                        }
                                        for est_part_iter in 0..total_parts2 {
                                            let mut results = [TrialResult::default(); 2];
                                            let pat = if cem == 11 {
                                                best_parts2_mode11[est_part_iter]
                                            } else {
                                                best_parts2_mode7[est_part_iter]
                                            };
                                            if !encode_block_2_subsets(
                                                &mut results,
                                                md.grid_x,
                                                md.grid_y,
                                                md.cem,
                                                md.weight_ise_range,
                                                md.endpoint_ise_range,
                                                &bp.half_pixels,
                                                &bp.block_pixels_q16,
                                                &coptions,
                                                uber_mode_flag,
                                                pat,
                                                comp_level,
                                                mode11_opt_mode,
                                                true,
                                            ) {
                                                continue;
                                            }
                                            for res in results.iter() {
                                                if !res.valid {
                                                    continue;
                                                }
                                                let mut candidate = CandidateEncoding {
                                                    coded_log_blk: res.log_blk,
                                                    decomp_log_blk: res.log_blk,
                                                    ..Default::default()
                                                };
                                                if !validate_log_blk(&candidate.decomp_log_blk) {
                                                    return false;
                                                }
                                                if !decode_astc_block(
                                                    &candidate.decomp_log_blk,
                                                    &mut candidate.comp_pixels,
                                                ) {
                                                    return false;
                                                }
                                                candidate
                                                    .coder
                                                    .put_bits(BLOCK_CODE, BLOCK_CODE_LEN);
                                                let cl = candidate.coded_log_blk;
                                                code_block(
                                                    &mut candidate.coder,
                                                    &cl,
                                                    block_mode_iter as u32,
                                                    em,
                                                    None,
                                                );
                                                candidate.encoding_type = ET_BLOCK;
                                                candidate.endpoint_mode = em as i32;
                                                candidate.block_mode = block_mode_iter as i32;
                                                candidates.push(candidate);
                                            }
                                        }
                                    } else {
                                        // 1 subset
                                        let mut blk_weights0 = [0u8; 36];
                                        let mut blk_weights1 = [0u8; 36];
                                        let mut best_submode = 0u32;
                                        let mut candidate = CandidateEncoding::default();
                                        let (grid_x, grid_y) = (md.grid_x, md.grid_y);
                                        {
                                            let cl = &mut candidate.coded_log_blk;
                                            cl.grid_width = grid_x as u8;
                                            cl.grid_height = grid_y as u8;
                                            cl.dual_plane = dual_plane;
                                            cl.color_component_selector = md.dp_channel as u8;
                                            cl.num_partitions = 1;
                                            cl.color_endpoint_modes[0] = md.cem as u8;
                                            cl.weight_ise_range = md.weight_ise_range as u8;
                                            cl.endpoint_ise_range = md.endpoint_ise_range as u8;
                                        }
                                        if cem == 11 && !dual_plane && (grid_x < 6 || grid_y < 6) {
                                            let cl = &mut candidate.coded_log_blk;
                                            let e = encode_astc_hdr_block_downsampled_mode_11(
                                                6,
                                                6,
                                                grid_x,
                                                grid_y,
                                                md.weight_ise_range,
                                                md.endpoint_ise_range,
                                                36,
                                                &bp.half_pixels,
                                                &bp.block_pixels_q16,
                                                BIG_FLOAT_VAL as f64,
                                                -1,
                                                7,
                                                false,
                                                mode11_opt_mode,
                                                &mut cl.endpoints,
                                                &mut cl.weights,
                                                &mut best_submode,
                                                &coptions,
                                                Some(&enc_block_stats),
                                            );
                                            if e == BIG_FLOAT_VAL as f64 {
                                                continue;
                                            }
                                        } else {
                                            if cem == 7 {
                                                encode_astc_hdr_block_mode_7(
                                                    36,
                                                    &bp.half_pixels,
                                                    &bp.block_pixels_q16,
                                                    md.weight_ise_range,
                                                    &mut best_submode,
                                                    BIG_FLOAT_VAL as f64,
                                                    &mut candidate.coded_log_blk.endpoints,
                                                    &mut blk_weights0,
                                                    &coptions,
                                                    md.endpoint_ise_range,
                                                    0,
                                                    5,
                                                    Some(&enc_block_stats),
                                                );
                                            } else {
                                                let e = if dual_plane {
                                                    encode_astc_hdr_block_mode_11_dual_plane(
                                                        36,
                                                        &bp.half_pixels,
                                                        &bp.block_pixels_q16,
                                                        md.dp_channel as usize,
                                                        md.weight_ise_range,
                                                        &mut best_submode,
                                                        BIG_FLOAT_VAL as f64,
                                                        &mut candidate.coded_log_blk.endpoints,
                                                        &mut blk_weights0,
                                                        &mut blk_weights1,
                                                        &coptions,
                                                        false,
                                                        md.endpoint_ise_range,
                                                        uber_mode_flag,
                                                        false,
                                                        -1,
                                                        7,
                                                        false,
                                                    )
                                                } else {
                                                    encode_astc_hdr_block_mode_11(
                                                        36,
                                                        &bp.half_pixels,
                                                        &bp.block_pixels_q16,
                                                        md.weight_ise_range,
                                                        &mut best_submode,
                                                        BIG_FLOAT_VAL as f64,
                                                        &mut candidate.coded_log_blk.endpoints,
                                                        &mut blk_weights0,
                                                        &coptions,
                                                        false,
                                                        md.endpoint_ise_range,
                                                        uber_mode_flag,
                                                        false,
                                                        -1,
                                                        7,
                                                        false,
                                                        mode11_opt_mode,
                                                        Some(&enc_block_stats),
                                                    )
                                                };
                                                if e == BIG_FLOAT_VAL as f64 {
                                                    continue;
                                                }
                                            }
                                            let wr = md.weight_ise_range;
                                            if dual_plane {
                                                downsample_ise_weights_dual_plane(
                                                    wr,
                                                    wr,
                                                    6,
                                                    6,
                                                    grid_x,
                                                    grid_y,
                                                    &blk_weights0,
                                                    &blk_weights1,
                                                    &mut candidate.coded_log_blk.weights,
                                                );
                                            } else {
                                                downsample_ise_weights(
                                                    wr,
                                                    wr,
                                                    6,
                                                    6,
                                                    grid_x,
                                                    grid_y,
                                                    &blk_weights0,
                                                    &mut candidate.coded_log_blk.weights,
                                                );
                                                if grid_x < 6 || grid_y < 6 {
                                                    let cl = &mut candidate.coded_log_blk;
                                                    let weights = cl.weights;
                                                    refine_endpoints(
                                                        cem,
                                                        md.endpoint_ise_range,
                                                        &mut cl.endpoints,
                                                        6,
                                                        6,
                                                        grid_x,
                                                        grid_y,
                                                        &weights,
                                                        wr,
                                                        36,
                                                        &bp.half_pixels,
                                                        &bp.block_pixels_q16,
                                                        None,
                                                        &coptions,
                                                        mode11_opt_mode,
                                                    );
                                                }
                                            }
                                        }
                                        let mut transcode_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
                                        requantize_ise_endpoints(
                                            md.cem,
                                            md.endpoint_ise_range,
                                            &candidate.coded_log_blk.endpoints,
                                            md.transcode_endpoint_ise_range,
                                            &mut transcode_endpoints,
                                        );
                                        pack_weights_and_decomp(
                                            &mut candidate,
                                            &md,
                                            &transcode_endpoints,
                                            use_orig_behavior,
                                        );
                                        if !validate_log_blk(&candidate.decomp_log_blk) {
                                            return false;
                                        }
                                        if !decode_astc_block(
                                            &candidate.decomp_log_blk,
                                            &mut candidate.comp_pixels,
                                        ) {
                                            return false;
                                        }
                                        candidate.coder.put_bits(BLOCK_CODE, BLOCK_CODE_LEN);
                                        let cl = candidate.coded_log_blk;
                                        code_block(
                                            &mut candidate.coder,
                                            &cl,
                                            block_mode_iter as u32,
                                            em,
                                            None,
                                        );
                                        candidate.encoding_type = ET_BLOCK;
                                        candidate.endpoint_mode = em as i32;
                                        candidate.block_mode = block_mode_iter as i32;
                                        candidates.push(candidate);
                                    }
                                }
                            }
                        }
                    }
                }

                //------------------------------------------------
                for candidate in candidates.iter_mut() {
                    for i in 0..36 {
                        candidate.comp_pixels_itp[i] =
                            linear_rgb_to_itp(candidate.comp_pixels[i], global_cfg);
                    }
                }

                // Find best overall candidate
                let mut best_t = f64::MAX;
                let mut best_candidate_index: i32 = -1;
                let mut best_d_ssim = BIG_FLOAT_VAL;

                let candidate_mse_of = |candidate: &CandidateEncoding,
                                        best_d_ssim: &mut f32|
                 -> f32 {
                    let mut d_ssim = 1.0
                        - compute_block_ssim_itp(
                            &bp.block_pixels_as_itp,
                            &candidate.comp_pixels_itp,
                        );
                    if d_ssim < *best_d_ssim {
                        *best_d_ssim = d_ssim;
                    }
                    d_ssim *= SSIM_WEIGHT;
                    let mut candidate_mse = MSE_WEIGHT
                        * compute_block_mse_itp(
                            &bp.block_pixels_as_itp,
                            &candidate.comp_pixels_itp,
                            global_cfg.delta_itp_dark_adjustment,
                        );
                    candidate_mse += d_ssim;
                    let mut total_deblock_penalty = 0f32;
                    if global_cfg.deblocking_flag {
                        total_deblock_penalty = calc_deblocking_penalty_itp(
                            bx,
                            by,
                            width,
                            height,
                            pass_src_img_itp,
                            &candidate.comp_pixels_itp,
                        ) * global_cfg.deblock_penalty_weight;
                    }
                    candidate_mse += total_deblock_penalty * SSIM_WEIGHT;
                    if (candidate.encoding_type == ET_BLOCK || candidate.encoding_type == ET_REUSE)
                        && complex_block
                    {
                        // Bias the encoder away from 2 level blocks and small grids on complex blocks
                        if candidate.coded_log_blk.weight_ise_range as u32 == ah::BISE_2_LEVELS {
                            candidate_mse *= TWO_LEVEL_PENALTY;
                        }
                        let (gw, gh) = (
                            candidate.coded_log_blk.grid_width,
                            candidate.coded_log_blk.grid_height,
                        );
                        if gw == 2 && gh == 2 {
                            candidate_mse *= COMPLEX_BLOCK_WEIGHT_GRID_2X2_MSE_PENALTY;
                        } else if gw.min(gh) <= 3 {
                            candidate_mse *= COMPLEX_BLOCK_WEIGHT_GRID_3X3_MSE_PENALTY;
                        } else if gw.min(gh) <= 4 {
                            candidate_mse *= COMPLEX_BLOCK_WEIGHT_GRID_4X4_MSE_PENALTY;
                        }
                    }
                    candidate_mse
                };

                let block_y_contrast_ratio = block_hy / (block_ly + 0.00000125);
                let gaussian1 = |best_d_ssim: f32| {
                    global_cfg.gaussian1_fallback
                        && outer_pass == 0
                        && very_complex_block
                        && best_d_ssim > SWITCH_TO_GAUSSIAN_FILTERED_THRESH1_D_SSIM
                };
                let gaussian2 = |best_d_ssim: f32| {
                    global_cfg.gaussian2_fallback
                        && comp_level >= 1
                        && outer_pass == 1
                        && very_complex_block
                        && best_d_ssim > SWITCH_TO_GAUSSIAN_FILTERED_THRESH2_D_SSIM
                        && block_hy >= 18.0
                        && block_y_contrast_ratio > 150.0
                        && block_avg_y >= 1.5
                };

                if global_cfg.lambda == 0.0 {
                    for (ci, candidate) in candidates.iter().enumerate() {
                        let candidate_t = candidate_mse_of(candidate, &mut best_d_ssim);
                        if (candidate_t as f64) < best_t {
                            best_t = candidate_t as f64;
                            best_candidate_index = ci as i32;
                        }
                    }
                    if gaussian1(best_d_ssim) || gaussian2(best_d_ssim) {
                        outer_pass += 1;
                        continue;
                    }
                } else {
                    // Compute block's perceptual weighting
                    let mut perceptual_scale = 0f32;
                    for y in 0..6 {
                        for x in 0..6 {
                            let sx = (bx * 6 + x).min(width - 1);
                            let sy = (by * 6 + y).min(height - 1);
                            let v = enc_state.smooth_block_mse_scales[(sx + sy * width) as usize];
                            perceptual_scale = if perceptual_scale > v {
                                perceptual_scale
                            } else {
                                v
                            };
                        }
                    }
                    // Very roughly normalize the computed distortion vs. bits.
                    perceptual_scale *= 10.0;
                    let mut tdb: Vec<(f64, f64, f32)> = Vec::with_capacity(candidates.len());
                    for (ci, candidate) in candidates.iter().enumerate() {
                        let candidate_mse = candidate_mse_of(candidate, &mut best_d_ssim);
                        let mut mode_penalty = 1.0f32;
                        if candidate.encoding_type == ET_SOLID {
                            mode_penalty *= SOLID_PENALTY;
                        } else if candidate.encoding_type == ET_REUSE {
                            mode_penalty *= REUSE_PENALTY;
                        } else if candidate.encoding_type == ET_RUN {
                            mode_penalty *= if complex_block {
                                RUN_PENALTY * 2.0
                            } else {
                                RUN_PENALTY
                            };
                        }
                        let candidate_bits = candidate.coder.get_total_bits() as f32;
                        let candidate_d = candidate_mse as f64 * mode_penalty as f64;
                        // pow(d, 2.0) is a multiply (both compilers fold it).
                        let candidate_t = perceptual_scale as f64 * (candidate_d * candidate_d)
                            + (candidate_bits * (global_cfg.lambda * 1000.0)) as f64;
                        tdb.push((candidate_t, candidate_d, candidate_bits));
                        if candidate_t < best_t {
                            best_t = candidate_t;
                            best_candidate_index = ci as i32;
                        }
                    }
                    for (c, v) in candidates.iter_mut().zip(tdb) {
                        c.t = v.0;
                        c.d = v.1;
                        c.bits = v.2;
                    }
                    if best_candidate_index < 0 {
                        // Should never happen
                        best_candidate_index = 0;
                    }
                    if gaussian1(best_d_ssim) || gaussian2(best_d_ssim) {
                        outer_pass += 1;
                        continue;
                    }
                    if global_cfg.rdo_candidate_diversity_boost {
                        // consider candidates along/near the Pareto front
                        let comp_bits = candidates[best_candidate_index as usize].bits;
                        let mut best_d = f64::MAX;
                        for (ci, candidate) in candidates.iter().enumerate() {
                            if candidate.bits
                                <= comp_bits
                                    * global_cfg.rdo_candidate_diversity_boost_bit_window_weight
                                && candidate.d < best_d
                            {
                                best_d = candidate.d;
                                best_candidate_index = ci as i32;
                            }
                        }
                    }
                    if global_cfg.jnd_optimization {
                        // a cheaper to code candidate that is nearly equivalent visually
                        let cur = &candidates[best_candidate_index as usize];
                        let mut new_best_candidate_bits = BIG_FLOAT_VAL;
                        let mut new_best_candidate_index: i32 = -1;
                        for (ci, candidate) in candidates.iter().enumerate() {
                            if ci as i32 == best_candidate_index || candidate.bits >= cur.bits {
                                continue;
                            }
                            let mut max_delta_itp = 0f32;
                            for i in 0..36 {
                                let delta_itp = compute_pixel_delta_itp(
                                    cur.comp_pixels_itp[i],
                                    candidate.comp_pixels_itp[i],
                                    bp.block_pixels_as_itp[i],
                                    global_cfg.delta_itp_dark_adjustment,
                                );
                                max_delta_itp = maxf(max_delta_itp, delta_itp);
                                if max_delta_itp >= global_cfg.jnd_delta_itp_thresh {
                                    break;
                                }
                            }
                            if max_delta_itp >= global_cfg.jnd_delta_itp_thresh {
                                continue;
                            }
                            if candidate.bits < new_best_candidate_bits {
                                new_best_candidate_bits = candidate.bits;
                                new_best_candidate_index = ci as i32;
                            }
                        }
                        if new_best_candidate_index != -1 {
                            best_candidate_index = new_best_candidate_index;
                        }
                    }
                }

                if best_candidate_index < 0 {
                    return false;
                }
                let best_candidate = &candidates[best_candidate_index as usize];
                if best_candidate.encoding_type == ET_RUN {
                    if prev_run_len == 0 {
                        if prev_encoding.get_total_bits() != 0 {
                            strip_coded_bits.append(&prev_encoding);
                        }
                        prev_encoding = best_candidate.coder.clone();
                        prev_run_len = 1;
                    } else {
                        prev_run_len += 1;
                        // Rewrite the previous encoding to extend the run length.
                        prev_encoding.restart();
                        prev_encoding.put_bits(RUN_CODE, RUN_CODE_LEN);
                        prev_encoding.put_vlc(prev_run_len - 1, 5);
                    }
                } else {
                    if prev_encoding.get_total_bits() != 0 {
                        strip_coded_bits.append(&prev_encoding);
                    }
                    prev_encoding = best_candidate.coder.clone();
                    prev_run_len = 0;
                }
                prev_comp_pixels = best_candidate.comp_pixels;
                if best_candidate.encoding_type != ET_RUN {
                    prev_non_run_candidate_encoding = best_candidate.clone();
                }
                let bi = bx as usize + by as usize * nbx;
                enc_state.coded_blocks[bi] = prev_non_run_candidate_encoding.clone();
                if !pack_astc_block(
                    &mut enc_state.final_astc_blocks[bi],
                    &best_candidate.decomp_log_blk,
                ) {
                    return false;
                }
                break;
            }
        }
    }
    if prev_encoding.get_total_bits() != 0 {
        strip_coded_bits.append(&prev_encoding);
    }
    enc_state.strip_bits[strip_index] = strip_coded_bits;
    true
}

#[inline]
fn minf32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `compress_photo`: encodes an HDR image to the UASTC HDR 6x6 intermediate format. Returns
/// (intermediate data, ASTC 6x6 blocks), or `None` on failure. `total_threads` is the job pool
/// size, which sets the strip count at lambda 0 (strips are encoded in order here; they are
/// independent, so the output only depends on the count). Image statistics are not ported.
pub fn compress_photo(
    orig_src_img: &ImageF,
    orig_global_cfg: &AstcHdr6x6GlobalConfig,
    total_threads: u32,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let (width, height) = (orig_src_img.get_width(), orig_src_img.get_height());
    if width == 0 || height == 0 {
        return None;
    }
    let mut global_cfg = *orig_global_cfg;
    let mut src_img = orig_src_img.clone();
    for p in src_img.get_pixels_mut() {
        for f in p.iter_mut().take(3) {
            if f.is_infinite() || f.is_nan() || *f < 0.0 {
                *f = 0.0;
            } else if *f > ASTC_HDR_MAX_VAL {
                *f = ASTC_HDR_MAX_VAL;
            }
        }
    }
    let num_blocks_x = width.div_ceil(6);
    let num_blocks_y = height.div_ceil(6);
    let total_blocks = (num_blocks_x * num_blocks_y) as usize;

    let mut smooth_block_mse_scales = Vec::new();
    if global_cfg.lambda != 0.0 {
        let (src_img_compressed, _) = tonemap_image_compressive2(&src_img);
        smooth_block_mse_scales =
            create_smooth_maps2(&src_img_compressed, &SmoothMapParams::default());
    }
    let mut src_img_filtered1 = ImageF::with_size(width, height);
    image_resample_f(
        &src_img,
        &mut src_img_filtered1,
        "gaussian",
        global_cfg.gaussian1_strength,
    );
    let mut src_img_filtered2 = ImageF::with_size(width, height);
    image_resample_f(
        &src_img,
        &mut src_img_filtered2,
        "gaussian",
        global_cfg.gaussian2_strength,
    );
    let src_img_itp = convert_rgb_image_to_itp(&src_img, &global_cfg);
    let src_img_filtered1_itp = convert_rgb_image_to_itp(&src_img_filtered1, &global_cfg);
    let src_img_filtered2_itp = convert_rgb_image_to_itp(&src_img_filtered2, &global_cfg);
    if global_cfg.lambda == 0.0 {
        global_cfg.favor_higher_compression = false;
    }
    let (total_strips, rows_per_strip) = calc_strip_size(
        global_cfg.lambda,
        num_blocks_y,
        total_threads.max(1),
        global_cfg.force_one_strip,
    )?;

    let master_coptions = AstcHdrCodecBaseOptions {
        q_log_bias: Q_LOG_BIAS_6X6,
        ..Default::default()
    };
    let mut enc_state = EncodeState {
        master_coptions,
        src_img,
        src_img_filtered1,
        src_img_filtered2,
        src_img_itp,
        src_img_filtered1_itp,
        src_img_filtered2_itp,
        smooth_block_mse_scales,
        strip_bits: vec![BitwiseCoder::new(); total_strips as usize],
        final_astc_blocks: vec![[0u8; 16]; total_blocks],
        coded_blocks: vec![CandidateEncoding::default(); total_blocks],
    };

    let mut coded_bits = BitwiseCoder::new();
    // For Basis v1.60 files write the original marker, otherwise write the new marker.
    coded_bits.put_bits(
        if global_cfg.write_basisu_1_6_compatible_files {
            super::astc_6x6_hdr::UASTC_6X6_HDR_SIG0
        } else {
            super::astc_6x6_hdr::UASTC_6X6_HDR_SIG1
        },
        16,
    );
    coded_bits.put_bits(width, 16);
    coded_bits.put_bits(height, 16);

    for strip_index in 0..total_strips {
        let strip_first_by = strip_index * rows_per_strip;
        let mut strip_last_by = (strip_first_by + rows_per_strip - 1).min(num_blocks_y);
        if strip_index == total_strips - 1 {
            strip_last_by = num_blocks_y - 1;
        }
        if !compress_strip_task(
            strip_index as usize,
            strip_first_by,
            strip_last_by,
            num_blocks_x,
            width,
            height,
            &global_cfg,
            &mut enc_state,
        ) {
            return None;
        }
    }
    for s in &enc_state.strip_bits {
        coded_bits.append(s);
    }
    coded_bits.put_bits(0xA742, 16);
    coded_bits.flush();
    let intermediate = coded_bits.get_bytes().clone();

    // Verify the output decodes to the blocks the encoder chose.
    let decoded = super::astc_6x6_hdr::decode_6x6_hdr(&intermediate)?;
    if decoded.blocks != enc_state.final_astc_blocks {
        return None;
    }
    let astc: Vec<u8> = enc_state
        .final_astc_blocks
        .iter()
        .flat_map(|b| b.iter().copied())
        .collect();
    Some((intermediate, astc))
}
