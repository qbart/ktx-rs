//! Port of `encoder/basisu_frontend.cpp`: the ETC1S frontend (endpoint / selector vector
//! quantization and refinement) of the BasisLZ encoder.
//!
//! The C++ job pool splits are kept where they change results (the per 2048 block chunks of
//! `find_optimal_selector_clusters_for_each_block`); all other jobs write disjoint outputs and
//! are run in order. OpenCL paths, global codebooks and the debug image dumps are not ported
//! (libktx builds basisu without OpenCL and never passes global codebooks).
//! `BASISU_FRONTEND_VERIFY` aborts in C++; here a failed check makes `compress` return false.

#![allow(clippy::needless_range_loop)]

use super::enc::{ColorRgba, color_distance, std_sort_by};
use super::etc_enc::{
    BasisEtcQuality, Etc1Optimizer, Etc1OptimizerParams, Etc1OptimizerResults, EtcBlock,
    G_ETC1_PIXEL_COORDS, G_ETC1_PIXEL_INDICES, unpack_etc1,
};
use super::vq::{TreeVectorQuant, generate_hierarchical_codebook_threaded};

/// `pixel_block`: 4x4 pixels, row major.
pub type PixelBlock = [ColorRgba; 16];

pub const BASISU_DEFAULT_ETC1S_COMPRESSION_LEVEL: u32 = 2;
pub const BASISU_MAX_ETC1S_COMPRESSION_LEVEL: u32 = 6;

pub const C_MAX_ENDPOINT_CLUSTERS: u32 = 16128;
pub const C_MAX_SELECTOR_CLUSTERS: u32 = 16128;

/// `basist::basis_texture_type`.
pub const C_BASIS_TEX_TYPE_2D: u32 = 0;
pub const C_BASIS_TEX_TYPE_2D_ARRAY: u32 = 1;
pub const C_BASIS_TEX_TYPE_CUBEMAP_ARRAY: u32 = 2;
pub const C_BASIS_TEX_TYPE_VIDEO_FRAMES: u32 = 3;
pub const C_BASIS_TEX_TYPE_VOLUME: u32 = 4;

const C_MAX_CODEBOOK_CREATION_THREADS: u32 = 8;
const BASISU_MAX_ENDPOINT_REFINEMENT_STEPS: u32 = 3;
const BASISU_ENDPOINT_PARENT_CODEBOOK_SIZE: u32 = 16;
const BASISU_SELECTOR_PARENT_CODEBOOK_SIZE_COMP_LEVEL_01: u32 = 32;
const BASISU_SELECTOR_PARENT_CODEBOOK_SIZE_COMP_LEVEL_DEFAULT: u32 = 16;

/// `basisu_frontend::params`.
#[derive(Debug, Clone)]
pub struct BasisuFrontendParams {
    pub source_blocks: Vec<PixelBlock>,
    pub max_endpoint_clusters: u32,
    pub max_selector_clusters: u32,
    pub compression_level: u32,
    pub perceptual: bool,
    pub debug_stats: bool,
    pub debug_images: bool,
    pub dump_endpoint_clusterization: bool,
    pub validate: bool,
    pub multithreaded: bool,
    pub disable_hierarchical_endpoint_codebooks: bool,
    pub tex_type: u32,
    /// `m_pJob_pool->get_total_threads()` (0 = no job pool).
    pub job_pool_threads: u32,
}

impl Default for BasisuFrontendParams {
    fn default() -> Self {
        BasisuFrontendParams {
            source_blocks: Vec::new(),
            max_endpoint_clusters: 256,
            max_selector_clusters: 256,
            compression_level: BASISU_DEFAULT_ETC1S_COMPRESSION_LEVEL,
            perceptual: true,
            debug_stats: false,
            debug_images: false,
            dump_endpoint_clusterization: true,
            validate: false,
            multithreaded: false,
            disable_hierarchical_endpoint_codebooks: false,
            tex_type: C_BASIS_TEX_TYPE_2D,
            job_pool_threads: 0,
        }
    }
}

/// `endpoint_cluster_etc_params`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointClusterEtcParams {
    /// `[use_individual_mode]`
    pub color_unscaled: [ColorRgba; 2],
    pub inten_table: [u32; 2],
    pub color_error: [u64; 2],
    pub subblocks: Vec<u32>,
    pub color_used: [bool; 2],
    pub valid: bool,
}

impl EndpointClusterEtcParams {
    /// `operator==`: colors and intensity tables only.
    fn same(&self, o: &Self) -> bool {
        self.color_unscaled == o.color_unscaled && self.inten_table == o.inten_table
    }

    /// `operator<`.
    fn less(&self, o: &Self) -> bool {
        for i in 0..2 {
            if self.color_unscaled[i] < o.color_unscaled[i] {
                return true;
            } else if self.color_unscaled[i] != o.color_unscaled[i] {
                return false;
            }
        }
        self.inten_table[0] < o.inten_table[0]
            || (self.inten_table[0] == o.inten_table[0] && self.inten_table[1] < o.inten_table[1])
    }
}

/// `subblock_endpoint_quant_err`.
#[derive(Debug, Clone, Copy, Default)]
struct SubblockEndpointQuantErr {
    total_err: u64,
    cluster_index: u32,
    #[allow(dead_code)]
    cluster_subblock_index: u32,
    block_index: u32,
    subblock_index: u32,
}

impl SubblockEndpointQuantErr {
    fn less(&self, r: &Self) -> bool {
        if self.total_err < r.total_err {
            true
        } else if self.total_err == r.total_err {
            if self.block_index < r.block_index {
                true
            } else if self.block_index == r.block_index {
                self.subblock_index < r.subblock_index
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// `std::sort` + `std::unique` + `erase`.
fn sort_unique(v: &mut Vec<u32>) {
    v.sort_unstable();
    v.dedup();
}

/// `basisu_frontend`.
#[derive(Default)]
pub struct BasisuFrontend {
    params: BasisuFrontendParams,
    total_blocks: u32,
    total_pixels: u32,

    endpoint_refinement: bool,
    use_hierarchical_endpoint_codebooks: bool,
    use_hierarchical_selector_codebooks: bool,
    num_endpoint_codebook_iterations: u32,
    num_selector_codebook_iterations: u32,

    encoded_blocks: Vec<EtcBlock>,
    orig_encoded_blocks: Vec<EtcBlock>,
    etc1_blocks_etc1s: Vec<EtcBlock>,

    endpoint_clusterizer: TreeVectorQuant<6>,
    endpoint_clusters: Vec<Vec<u32>>,
    endpoint_parent_clusters: Vec<Vec<u32>>,
    block_parent_endpoint_cluster: Vec<u8>,
    endpoint_clusters_within_each_parent_cluster: Vec<Vec<u32>>,
    endpoint_cluster_etc_params: Vec<EndpointClusterEtcParams>,
    block_endpoint_clusters_indices: Vec<[u32; 2]>,

    selector_cluster_block_indices: Vec<Vec<u32>>,
    optimized_cluster_selectors: Vec<EtcBlock>,
    selector_parent_cluster_block_indices: Vec<Vec<u32>>,
    block_parent_selector_cluster: Vec<u8>,
    selector_clusters_within_each_parent_cluster: Vec<Vec<u32>>,
    block_selector_cluster_index: Vec<u32>,

    subblock_endpoint_quant_err_vec: Vec<SubblockEndpointQuantErr>,

    /// Set by a failed `BASISU_FRONTEND_VERIFY`.
    verify_failed: bool,
}

fn new_optimizer_results(n: u32) -> Etc1OptimizerResults {
    Etc1OptimizerResults {
        n,
        selectors: vec![0; n as usize],
        ..Default::default()
    }
}

impl BasisuFrontend {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn verify(&mut self, c: bool) {
        if !c {
            self.verify_failed = true;
        }
    }

    pub fn init(&mut self, p: &BasisuFrontendParams) -> bool {
        if p.max_endpoint_clusters < 1 || p.max_endpoint_clusters > C_MAX_ENDPOINT_CLUSTERS {
            return false;
        }
        if p.max_selector_clusters < 1 || p.max_selector_clusters > C_MAX_SELECTOR_CLUSTERS {
            return false;
        }
        self.params = p.clone();
        self.encoded_blocks = vec![EtcBlock::default(); p.source_blocks.len()];
        self.num_endpoint_codebook_iterations = 1;
        self.num_selector_codebook_iterations = 1;

        let (refine, hier, iters) = match p.compression_level {
            0 => (false, true, 1),
            1 | 2 => (true, true, 1),
            3 => (true, false, 1),
            4 => (true, true, BASISU_MAX_ENDPOINT_REFINEMENT_STEPS),
            5 => (true, false, BASISU_MAX_ENDPOINT_REFINEMENT_STEPS),
            _ => (true, false, BASISU_MAX_ENDPOINT_REFINEMENT_STEPS * 2),
        };
        self.endpoint_refinement = refine;
        self.use_hierarchical_endpoint_codebooks = hier;
        self.use_hierarchical_selector_codebooks = hier;
        self.num_endpoint_codebook_iterations = iters;
        self.num_selector_codebook_iterations = iters;

        if self.params.disable_hierarchical_endpoint_codebooks {
            self.use_hierarchical_endpoint_codebooks = false;
        }
        true
    }

    pub fn compress(&mut self) -> bool {
        self.total_blocks = self.params.source_blocks.len() as u32;
        self.total_pixels = self.total_blocks * 16;
        self.verify_failed = false;

        // Encode the initial high quality ETC1S texture
        self.init_etc1_images();

        // First quantize the ETC1S endpoints
        self.init_endpoint_training_vectors();
        self.generate_endpoint_clusters();
        if self.verify_failed {
            return false;
        }

        for refine_endpoint_step in 0..self.num_endpoint_codebook_iterations {
            if self.params.validate {
                let ok = self.check_etc1s_constraints()
                    && self.validate_endpoint_cluster_hierarchy(false);
                self.verify(ok);
            }
            if refine_endpoint_step != 0 {
                self.introduce_new_endpoint_clusters();
            }
            if self.params.validate {
                let ok = self.validate_endpoint_cluster_hierarchy(false);
                self.verify(ok);
            }
            self.generate_endpoint_codebook(refine_endpoint_step);

            let mut early_out = false;
            if self.endpoint_refinement {
                if self.refine_endpoint_clusterization() == 0 {
                    early_out = true;
                }
                if self.params.tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES
                    && refine_endpoint_step == 0
                    && self.num_endpoint_codebook_iterations == 1
                {
                    self.eliminate_redundant_or_empty_endpoint_clusters();
                    self.generate_endpoint_codebook(refine_endpoint_step.max(1));
                }
            }
            if self.params.validate {
                let ok = self.validate_endpoint_cluster_hierarchy(false);
                self.verify(ok);
            }
            self.eliminate_redundant_or_empty_endpoint_clusters();
            if self.params.validate {
                let ok = self.validate_endpoint_cluster_hierarchy(false);
                self.verify(ok);
            }
            if self.verify_failed {
                return false;
            }
            if early_out {
                break;
            }
        }

        if self.params.validate {
            let ok = self.check_etc1s_constraints();
            self.verify(ok);
        }

        self.generate_block_endpoint_clusters();
        self.create_initial_packed_texture();

        // Now quantize the ETC1S selectors
        self.generate_selector_clusters();
        if self.verify_failed {
            return false;
        }
        if self.use_hierarchical_selector_codebooks {
            self.compute_selector_clusters_within_each_parent_cluster();
        }

        if self.params.compression_level == 0 {
            self.create_optimized_selector_codebook(0);
            self.find_optimal_selector_clusters_for_each_block();
            self.introduce_special_selector_clusters();
        } else {
            for refine_selector_steps in 0..self.num_selector_codebook_iterations {
                self.create_optimized_selector_codebook(refine_selector_steps);
                self.find_optimal_selector_clusters_for_each_block();
                self.introduce_special_selector_clusters();
                if (self.params.compression_level >= 4
                    || self.params.tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES)
                    && self.refine_block_endpoints_given_selectors() == 0
                {
                    break;
                }
            }
        }

        self.optimize_selector_codebook();
        self.finalize();

        if self.verify_failed {
            return false;
        }
        if self.params.validate && !self.validate_output() {
            return false;
        }
        true
    }

    // ---- accessors ----

    pub fn get_params(&self) -> &BasisuFrontendParams {
        &self.params
    }

    pub fn get_source_pixel_block(&self, i: u32) -> &PixelBlock {
        &self.params.source_blocks[i as usize]
    }

    pub fn get_total_output_blocks(&self) -> u32 {
        self.encoded_blocks.len() as u32
    }

    pub fn get_output_block(&self, block_index: u32) -> &EtcBlock {
        &self.encoded_blocks[block_index as usize]
    }

    pub fn get_output_blocks(&self) -> &[EtcBlock] {
        &self.encoded_blocks
    }

    pub fn get_etc1s_block(&self, block_index: u32) -> &EtcBlock {
        &self.etc1_blocks_etc1s[block_index as usize]
    }

    pub fn get_diff_flag(&self, block_index: u32) -> bool {
        self.encoded_blocks[block_index as usize].get_diff_bit()
    }

    pub fn get_total_endpoint_clusters(&self) -> u32 {
        self.endpoint_clusters.len() as u32
    }

    pub fn get_subblock_endpoint_cluster_index(
        &self,
        block_index: u32,
        subblock_index: u32,
    ) -> u32 {
        self.block_endpoint_clusters_indices[block_index as usize][subblock_index as usize]
    }

    pub fn get_endpoint_cluster_unscaled_color(
        &self,
        cluster_index: u32,
        individual_mode: bool,
    ) -> &ColorRgba {
        &self.endpoint_cluster_etc_params[cluster_index as usize].color_unscaled
            [individual_mode as usize]
    }

    pub fn get_endpoint_cluster_inten_table(
        &self,
        cluster_index: u32,
        individual_mode: bool,
    ) -> u32 {
        self.endpoint_cluster_etc_params[cluster_index as usize].inten_table
            [individual_mode as usize]
    }

    pub fn get_endpoint_cluster_color_is_used(
        &self,
        cluster_index: u32,
        individual_mode: bool,
    ) -> bool {
        self.endpoint_cluster_etc_params[cluster_index as usize].color_used
            [individual_mode as usize]
    }

    pub fn get_total_selector_clusters(&self) -> u32 {
        self.selector_cluster_block_indices.len() as u32
    }

    pub fn get_block_selector_cluster_index(&self, block_index: u32) -> u32 {
        self.block_selector_cluster_index[block_index as usize]
    }

    pub fn get_selector_cluster_selector_bits(&self, cluster_index: u32) -> &EtcBlock {
        &self.optimized_cluster_selectors[cluster_index as usize]
    }

    pub fn get_selector_cluster_block_indices(&self, selector_cluster_index: u32) -> &[u32] {
        &self.selector_cluster_block_indices[selector_cluster_index as usize]
    }

    /// `get_opencl_failed` (OpenCL is never used).
    pub fn get_opencl_failed(&self) -> bool {
        false
    }

    // ---- implementation ----

    fn max_codebook_threads(&self) -> u32 {
        let mut max_threads = if self.params.multithreaded {
            let hw = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
            hw.min(C_MAX_CODEBOOK_CREATION_THREADS)
        } else {
            0
        };
        if self.params.job_pool_threads != 0 {
            max_threads = max_threads.min(self.params.job_pool_threads);
        }
        max_threads
    }

    fn introduce_special_selector_clusters(&mut self) {
        let mut total_blocks_relocated = 0u32;
        let initial_selector_clusters = self.selector_cluster_block_indices.len();
        let mut block_relocated_flags = vec![false; self.total_blocks as usize];

        // Make sure the selector codebook always has pure flat blocks for each possible
        // selector, to avoid obvious artifacts.
        for sel in 0..4u32 {
            let mut blk = EtcBlock::default();
            for j in 0..16 {
                blk.set_selector(j & 3, j >> 2, sel);
            }
            if self
                .optimized_cluster_selectors
                .iter()
                .any(|s| s.get_raw_selector_bits() == blk.get_raw_selector_bits())
            {
                continue;
            }
            let new_selector_cluster_index = self.optimized_cluster_selectors.len() as u32;
            self.optimized_cluster_selectors.push(blk);
            // vector_ensure_element_is_valid
            if self.selector_cluster_block_indices.len() <= new_selector_cluster_index as usize {
                self.selector_cluster_block_indices
                    .resize(new_selector_cluster_index as usize + 1, Vec::new());
            }

            for block_index in 0..self.total_blocks {
                let bi = block_index as usize;
                if self.orig_encoded_blocks[bi].get_raw_selector_bits()
                    != blk.get_raw_selector_bits()
                {
                    continue;
                }
                // See if using flat selectors actually decreases the block's error.
                let old_selector_cluster_index = self.block_selector_cluster_index[bi];
                let mut cur_blk = EtcBlock::default();
                let endpoint_cluster_index =
                    self.get_subblock_endpoint_cluster_index(block_index, 0);
                cur_blk.set_block_color5_etc1s(
                    self.get_endpoint_cluster_unscaled_color(endpoint_cluster_index, false),
                );
                cur_blk.set_inten_tables_etc1s(
                    self.get_endpoint_cluster_inten_table(endpoint_cluster_index, false),
                );
                cur_blk.set_raw_selector_bits(
                    self.get_selector_cluster_selector_bits(old_selector_cluster_index)
                        .get_raw_selector_bits(),
                );
                cur_blk.set_flip_bit(true);
                let src = &self.params.source_blocks[bi];
                let cur_err = cur_blk.evaluate_etc1_error(src, self.params.perceptual, -1);
                cur_blk.set_raw_selector_bits(blk.get_raw_selector_bits());
                let new_err = cur_blk.evaluate_etc1_error(src, self.params.perceptual, -1);
                if new_err >= cur_err {
                    continue;
                }
                // Change the block to use the new cluster
                self.block_selector_cluster_index[bi] = new_selector_cluster_index;
                self.selector_cluster_block_indices[new_selector_cluster_index as usize]
                    .push(block_index);
                block_relocated_flags[bi] = true;
                total_blocks_relocated += 1;
                self.encoded_blocks[bi].set_raw_selector_bits(blk.get_raw_selector_bits());
            }
        }

        if total_blocks_relocated != 0 {
            for block_indices in
                self.selector_cluster_block_indices[..initial_selector_clusters].iter_mut()
            {
                block_indices.retain(|&b| !block_relocated_flags[b as usize]);
            }
        }
    }

    /// Changes the number and ordering of the selector codebook clusters.
    fn optimize_selector_codebook(&mut self) {
        let n = self.optimized_cluster_selectors.len();
        let mut selector_cluster_was_used = vec![false; n];
        for i in 0..self.total_blocks as usize {
            selector_cluster_was_used[self.block_selector_cluster_index[i] as usize] = true;
        }
        let mut old_to_new = vec![0i32; n];
        let mut new_to_old: Vec<u32> = Vec::new();
        let mut total_new_entries = 0u32;
        let mut selector_hashmap: std::collections::HashMap<u32, u32> =
            std::collections::HashMap::new();
        for i in 0..n {
            if !selector_cluster_was_used[i] {
                old_to_new[i] = -1;
                continue;
            }
            let raw_selector_bits = self.optimized_cluster_selectors[i].get_raw_selector_bits();
            if let Some(&e) = selector_hashmap.get(&raw_selector_bits) {
                old_to_new[i] = e as i32;
                continue;
            }
            selector_hashmap.insert(raw_selector_bits, total_new_entries);
            old_to_new[i] = total_new_entries as i32;
            total_new_entries += 1;
            new_to_old.push(i as u32);
        }

        let mut ok = true;
        for i in 0..self.block_selector_cluster_index.len() {
            let o = old_to_new[self.block_selector_cluster_index[i] as usize];
            ok &= o >= 0 && o < total_new_entries as i32;
            self.block_selector_cluster_index[i] = o.max(0) as u32;
        }
        self.verify(ok);

        let mut new_optimized_cluster_selectors = vec![
            EtcBlock::default();
            if n != 0 {
                total_new_entries as usize
            } else {
                0
            }
        ];
        let mut new_selector_cluster_indices: Vec<Vec<u32>> = vec![
            Vec::new();
            if self.selector_cluster_block_indices.is_empty() {
                0
            } else {
                total_new_entries as usize
            }
        ];
        if n != 0 {
            for i in 0..total_new_entries as usize {
                new_optimized_cluster_selectors[i] =
                    self.optimized_cluster_selectors[new_to_old[i] as usize];
            }
        }
        for i in 0..self.block_selector_cluster_index.len() {
            let c = self.block_selector_cluster_index[i] as usize;
            if let Some(v) = new_selector_cluster_indices.get_mut(c) {
                v.push(i as u32);
            } else {
                self.verify_failed = true;
            }
        }
        self.optimized_cluster_selectors = new_optimized_cluster_selectors;
        self.selector_cluster_block_indices = new_selector_cluster_indices;

        // This isn't strictly necessary - doing it for completeness/future sanity.
        for v in self.selector_clusters_within_each_parent_cluster.iter_mut() {
            for j in v.iter_mut() {
                *j = old_to_new[*j as usize] as u32;
            }
        }
    }

    fn init_etc1_images(&mut self) {
        let total = self.total_blocks as usize;
        self.etc1_blocks_etc1s = vec![EtcBlock::default(); total];
        let quality = match self.params.compression_level {
            0 => BasisEtcQuality::Fast,
            1 => BasisEtcQuality::Medium,
            BASISU_MAX_ETC1S_COMPRESSION_LEVEL => BasisEtcQuality::Uber,
            _ => BasisEtcQuality::Slow,
        };
        let mut optimizer = Etc1Optimizer::new();
        for block_index in 0..total {
            let source_blk = &self.params.source_blocks[block_index];
            let mut p = Etc1OptimizerParams {
                num_src_pixels: 16,
                src_pixels: source_blk,
                ..Default::default()
            };
            p.pack.quality = quality;
            p.pack.perceptual = self.params.perceptual;
            let mut r = new_optimizer_results(16);
            optimizer.init(&p);
            if !optimizer.compute(&p, &mut r) {
                self.verify_failed = true;
            }
            let blk = &mut self.etc1_blocks_etc1s[block_index];
            *blk = EtcBlock::default();
            blk.set_block_color5_etc1s(&r.block_color_unscaled);
            blk.set_inten_tables_etc1s(r.block_inten_table);
            blk.set_flip_bit(true);
            for y in 0..4u32 {
                for x in 0..4u32 {
                    blk.set_selector(x, y, r.selectors[(x + y * 4) as usize] as u32);
                }
            }
        }
    }

    fn init_endpoint_training_vectors(&mut self) {
        let tv = self.endpoint_clusterizer.get_training_vecs_mut();
        tv.clear();
        tv.reserve(self.etc1_blocks_etc1s.len() * 2);
        for blk in &self.etc1_blocks_etc1s {
            let mut block_colors = [[0u8; 4]; 2];
            blk.get_block_low_high_colors(&mut block_colors, 0);
            let s = 1.0f32 / 255.0;
            let v = [
                block_colors[0][0] as f32 * s,
                block_colors[0][1] as f32 * s,
                block_colors[0][2] as f32 * s,
                block_colors[1][0] as f32 * s,
                block_colors[1][1] as f32 * s,
                block_colors[1][2] as f32 * s,
            ];
            tv.push((v, 1));
            tv.push((v, 1));
        }
    }

    fn generate_endpoint_clusters(&mut self) {
        let parent_codebook_size = if self.params.max_endpoint_clusters >= 256 {
            BASISU_ENDPOINT_PARENT_CODEBOOK_SIZE
        } else {
            0
        };
        let max_threads = self.max_codebook_threads();
        let mut clusters = Vec::new();
        let mut parents = Vec::new();
        let status = generate_hierarchical_codebook_threaded(
            &mut self.endpoint_clusterizer,
            self.params.max_endpoint_clusters,
            if self.use_hierarchical_endpoint_codebooks {
                parent_codebook_size
            } else {
                0
            },
            &mut clusters,
            &mut parents,
            max_threads,
            true,
        );
        self.endpoint_clusters = clusters;
        self.endpoint_parent_clusters = parents;
        self.verify(status);

        if self.use_hierarchical_endpoint_codebooks {
            if self.endpoint_parent_clusters.is_empty() {
                let mut all = Vec::with_capacity(self.total_blocks as usize * 2);
                for i in 0..self.total_blocks {
                    all.push(i * 2);
                    all.push(i * 2 + 1);
                }
                self.endpoint_parent_clusters = vec![all];
            }
            self.block_parent_endpoint_cluster = vec![0xFF; self.total_blocks as usize];
            for (pi, cluster) in self.endpoint_parent_clusters.iter().enumerate() {
                for &j in cluster {
                    self.block_parent_endpoint_cluster[(j >> 1) as usize] = pi as u8;
                }
            }
            let mut ok = self
                .block_parent_endpoint_cluster
                .iter()
                .all(|&p| p != 0xFF);
            // Ensure that all the blocks within each cluster are all in the same parent cluster.
            for cluster in &self.endpoint_clusters {
                if let Some(&first) = cluster.first() {
                    let p = self.block_parent_endpoint_cluster[(first >> 1) as usize];
                    ok &= cluster
                        .iter()
                        .all(|&j| self.block_parent_endpoint_cluster[(j >> 1) as usize] == p);
                }
            }
            self.verify(ok);
        }
    }

    /// Sets `block_endpoint_clusters_indices` from `endpoint_clusters`.
    fn generate_block_endpoint_clusters(&mut self) {
        self.block_endpoint_clusters_indices
            .resize(self.total_blocks as usize, [0; 2]);
        for (cluster_index, cluster) in self.endpoint_clusters.iter().enumerate() {
            for &t in cluster {
                self.block_endpoint_clusters_indices[(t >> 1) as usize][(t & 1) as usize] =
                    cluster_index as u32;
            }
        }
        if self.params.validate {
            let ok = self
                .block_endpoint_clusters_indices
                .iter()
                .all(|c| c[0] == c[1]);
            self.verify(ok);
        }
    }

    fn compute_endpoint_clusters_within_each_parent_cluster(&mut self) {
        self.generate_block_endpoint_clusters();
        let mut within = vec![Vec::new(); self.endpoint_parent_clusters.len()];
        // Note: It's possible that some blocks got moved into the same cluster, but live in
        // different parent clusters.
        for block_index in 0..self.total_blocks as usize {
            let cluster_index = self.block_endpoint_clusters_indices[block_index][0];
            let parent = self.block_parent_endpoint_cluster[block_index] as usize;
            within[parent].push(cluster_index);
        }
        let mut ok = true;
        for v in within.iter_mut() {
            ok &= !v.is_empty();
            sort_unique(v);
        }
        self.verify(ok);
        self.endpoint_clusters_within_each_parent_cluster = within;
    }

    fn compute_endpoint_subblock_error_vec(&mut self) {
        self.subblock_endpoint_quant_err_vec.clear();
        for (cluster_index, cluster_indices) in self.endpoint_clusters.iter().enumerate() {
            let etc_params = &self.endpoint_cluster_etc_params[cluster_index];
            for (iter, &t) in cluster_indices.iter().enumerate() {
                let block_index = t >> 1;
                let subblock_index = t & 1;
                let src = &self.params.source_blocks[block_index as usize];
                let mut block_colors = [[0u8; 4]; 4];
                // C++ passes scaled = true for the unscaled color here; kept as is.
                EtcBlock::get_block_colors5(
                    &mut block_colors,
                    &etc_params.color_unscaled[0],
                    etc_params.inten_table[0],
                    true,
                );
                let mut total_err = 0u64;
                for i in 0..8 {
                    let c = &src[G_ETC1_PIXEL_INDICES[1][subblock_index as usize][i] as usize];
                    let mut best_err = u64::MAX;
                    for s in 0..4 {
                        let err = color_distance(self.params.perceptual, c, &block_colors[s], false)
                            as u64;
                        if err < best_err {
                            best_err = err;
                        }
                    }
                    total_err += best_err;
                }
                self.subblock_endpoint_quant_err_vec
                    .push(SubblockEndpointQuantErr {
                        total_err,
                        cluster_index: cluster_index as u32,
                        cluster_subblock_index: iter as u32,
                        block_index,
                        subblock_index,
                    });
            }
        }
        std_sort_by(&mut self.subblock_endpoint_quant_err_vec, |a, b| a.less(b));
    }

    fn introduce_new_endpoint_clusters(&mut self) {
        self.generate_block_endpoint_clusters();
        let mut num_new_endpoint_clusters =
            self.params.max_endpoint_clusters as i32 - self.endpoint_clusters.len() as i32;
        if num_new_endpoint_clusters <= 0 {
            return;
        }
        self.compute_endpoint_subblock_error_vec();

        let num_orig_endpoint_clusters = self.endpoint_clusters.len();
        let mut training_vector_was_relocated = std::collections::HashSet::new();
        let mut cluster_sizes: Vec<u32> = self
            .endpoint_clusters
            .iter()
            .map(|c| c.len() as u32)
            .collect();
        let mut ignore_cluster = std::collections::HashSet::new();

        while num_new_endpoint_clusters != 0 {
            let Some(subblock_to_move) = self.subblock_endpoint_quant_err_vec.pop() else {
                break;
            };
            if ignore_cluster.contains(&subblock_to_move.cluster_index) {
                continue;
            }
            let training_vector_index =
                subblock_to_move.block_index * 2 + subblock_to_move.subblock_index;
            let ci = subblock_to_move.cluster_index as usize;
            if cluster_sizes[ci] <= 2 {
                continue;
            }
            if training_vector_was_relocated.contains(&training_vector_index)
                || training_vector_was_relocated.contains(&(training_vector_index ^ 1))
            {
                continue;
            }
            self.endpoint_clusters
                .push(vec![training_vector_index, training_vector_index ^ 1]);
            self.endpoint_cluster_etc_params
                .push(EndpointClusterEtcParams::default());
            training_vector_was_relocated.insert(training_vector_index);
            training_vector_was_relocated.insert(training_vector_index ^ 1);
            cluster_sizes[ci] -= 2;
            ignore_cluster.insert(subblock_to_move.cluster_index);
            num_new_endpoint_clusters -= 1;
        }

        let mut ok = true;
        for cluster_indices in self.endpoint_clusters[..num_orig_endpoint_clusters].iter_mut() {
            let before = cluster_indices.len();
            cluster_indices.retain(|t| !training_vector_was_relocated.contains(t));
            if cluster_indices.len() != before {
                ok &= !cluster_indices.is_empty();
            }
        }
        self.verify(ok);
        self.generate_block_endpoint_clusters();
    }

    /// Gathers the pixels of each endpoint cluster and computes optimized ETC1S endpoints. If
    /// `step >= 1`, new endpoints are only used if they decrease the quantization error.
    fn generate_endpoint_codebook(&mut self, step: u32) {
        self.endpoint_cluster_etc_params.resize(
            self.endpoint_clusters.len(),
            EndpointClusterEtcParams::default(),
        );
        let quality = if self.params.compression_level <= 1 {
            BasisEtcQuality::Medium
        } else if self.params.compression_level == BASISU_MAX_ETC1S_COMPRESSION_LEVEL {
            BasisEtcQuality::Uber
        } else {
            BasisEtcQuality::Slow
        };
        let perceptual = self.params.perceptual;
        let mut optimizer = Etc1Optimizer::new();
        let mut ok = true;
        for cluster_index in 0..self.endpoint_clusters.len() {
            let cluster_indices = &self.endpoint_clusters[cluster_index];
            ok &= !cluster_indices.is_empty();
            let total_pixels = cluster_indices.len() as u32 * 8;
            let mut cluster_pixels = Vec::with_capacity(total_pixels as usize);
            for &t in cluster_indices {
                let src = &self.params.source_blocks[(t >> 1) as usize];
                for i in 0..8 {
                    cluster_pixels.push(src[G_ETC1_PIXEL_INDICES[1][(t & 1) as usize][i] as usize]);
                }
            }

            let mut new_subblock_params = EndpointClusterEtcParams::default();
            {
                let mut p = Etc1OptimizerParams {
                    num_src_pixels: total_pixels,
                    src_pixels: &cluster_pixels,
                    use_color4: false,
                    ..Default::default()
                };
                p.pack.perceptual = perceptual;
                p.pack.quality = quality;
                let mut r = new_optimizer_results(total_pixels);
                optimizer.init(&p);
                if !optimizer.compute(&p, &mut r) {
                    ok = false;
                }
                new_subblock_params.color_unscaled[0] = r.block_color_unscaled;
                new_subblock_params.inten_table[0] = r.block_inten_table;
                new_subblock_params.color_error[0] = r.error;
            }

            let prev = &mut self.endpoint_cluster_etc_params[cluster_index];
            let mut use_new_subblock_params = false;
            if step == 0 || !prev.valid {
                use_new_subblock_params = true;
            } else {
                let mut block_colors = [[0u8; 4]; 4];
                EtcBlock::get_block_colors5(
                    &mut block_colors,
                    &prev.color_unscaled[0],
                    prev.inten_table[0],
                    false,
                );
                let mut total_prev_err = 0u64;
                for c in &cluster_pixels {
                    let mut best_err = u64::MAX;
                    for s in 0..4 {
                        let err = color_distance(perceptual, c, &block_colors[s], false) as u64;
                        if err < best_err {
                            best_err = err;
                        }
                    }
                    total_prev_err += best_err;
                }
                // See if we should update this cluster's endpoints (if the error has actually
                // fallen)
                if total_prev_err > new_subblock_params.color_error[0] {
                    use_new_subblock_params = true;
                }
            }
            if use_new_subblock_params {
                new_subblock_params.valid = true;
                *prev = new_subblock_params;
            }
        }
        self.verify(ok);
    }

    fn check_etc1s_constraints(&self) -> bool {
        let mut block_clusters = vec![[0u32; 2]; self.total_blocks as usize];
        for (cluster_index, cluster) in self.endpoint_clusters.iter().enumerate() {
            for &t in cluster {
                block_clusters[(t >> 1) as usize][(t & 1) as usize] = cluster_index as u32;
            }
        }
        block_clusters.iter().all(|c| c[0] == c[1])
    }

    /// For each block, determines which ETC1S endpoint cluster encodes it with the lowest
    /// error, reassigning blocks to clusters. Returns the number of reassigned blocks.
    fn refine_endpoint_clusterization(&mut self) -> u32 {
        if self.use_hierarchical_endpoint_codebooks {
            self.compute_endpoint_clusters_within_each_parent_cluster();
        }
        let total = self.total_blocks as usize;
        let mut block_clusters = vec![[0u32; 2]; total];
        for (cluster_index, cluster) in self.endpoint_clusters.iter().enumerate() {
            for &t in cluster {
                block_clusters[(t >> 1) as usize][(t & 1) as usize] = cluster_index as u32;
            }
        }

        let perceptual = self.params.perceptual;
        let mut best_cluster_indices = vec![0u32; total];
        let mut ok = true;
        for block_index in 0..total {
            let cluster_index = block_clusters[block_index][0];
            ok &= cluster_index == block_clusters[block_index][1];
            let src = &self.params.source_blocks[block_index];

            let mut best_cluster_err = i64::MAX as u64;
            let mut best_cluster_index = 0u32;

            let parent = if self.block_parent_endpoint_cluster.is_empty() {
                0
            } else {
                self.block_parent_endpoint_cluster[block_index] as usize
            };
            let cluster_list: Option<&Vec<u32>> = self
                .endpoint_clusters_within_each_parent_cluster
                .get(parent);
            let total_clusters = if self.use_hierarchical_endpoint_codebooks {
                cluster_list.map_or(0, |c| c.len())
            } else {
                self.endpoint_clusters.len()
            };
            let cur_inten = self.endpoint_cluster_etc_params[cluster_index as usize].inten_table[0];

            for i in 0..total_clusters {
                let cluster_iter = if self.use_hierarchical_endpoint_codebooks {
                    cluster_list.map_or(0, |c| c[i])
                } else {
                    i as u32
                };
                let ep = &self.endpoint_cluster_etc_params[cluster_iter as usize];
                let cluster_etc_inten = ep.inten_table[0];

                let total_err = if cluster_etc_inten > cur_inten {
                    // Can't assign it here - may result in too much error when selector quant
                    // occurs
                    i64::MAX as u64
                } else {
                    let mut subblock_colors = [[0u8; 4]; 4];
                    EtcBlock::get_block_colors5(
                        &mut subblock_colors,
                        &ep.color_unscaled[0],
                        cluster_etc_inten,
                        false,
                    );
                    // find_lowest_error_{perceptual,linear}_rgb_4_N (the SSE kernels early out
                    // the same way: the partial sum exceeds the best error)
                    let mut total_err = 0u64;
                    for p in src.iter() {
                        let mut best_err = u64::MAX;
                        for c in &subblock_colors {
                            let err = color_distance(perceptual, p, c, false) as u64;
                            best_err = best_err.min(err);
                            if best_err == 0 {
                                break;
                            }
                        }
                        total_err += best_err;
                        if total_err > best_cluster_err {
                            break;
                        }
                    }
                    total_err
                };

                if total_err < best_cluster_err
                    || (cluster_iter == cluster_index && total_err == best_cluster_err)
                {
                    best_cluster_err = total_err;
                    best_cluster_index = cluster_iter;
                    if best_cluster_err == 0 {
                        break;
                    }
                }
            }
            best_cluster_indices[block_index] = best_cluster_index;
        }
        self.verify(ok);

        let mut optimized_endpoint_clusters = vec![Vec::new(); self.endpoint_clusters.len()];
        let mut total_subblocks_reassigned = 0u32;
        for block_index in 0..total {
            let training_vector_index = block_index as u32 * 2;
            let orig_cluster_index = block_clusters[block_index][0];
            let best_cluster_index = best_cluster_indices[block_index];
            optimized_endpoint_clusters[best_cluster_index as usize].push(training_vector_index);
            optimized_endpoint_clusters[best_cluster_index as usize]
                .push(training_vector_index + 1);
            if best_cluster_index != orig_cluster_index {
                total_subblocks_reassigned += 1;
            }
        }
        self.endpoint_clusters = optimized_endpoint_clusters;
        total_subblocks_reassigned
    }

    fn eliminate_redundant_or_empty_endpoint_clusters(&mut self) {
        // Step 1: Sort endpoint clusters by the base colors/intens
        let n = self.endpoint_clusters.len();
        let mut sorted = vec![0u32; n];
        for (i, v) in sorted.iter_mut().enumerate() {
            *v = i as u32;
        }
        {
            let params = &self.endpoint_cluster_etc_params;
            std_sort_by(&mut sorted, |&a, &b| {
                params[a as usize].less(&params[b as usize])
            });
        }
        let clusters = std::mem::take(&mut self.endpoint_clusters);
        let etc_params = std::mem::take(&mut self.endpoint_cluster_etc_params);
        self.endpoint_clusters = sorted
            .iter()
            .map(|&j| clusters[j as usize].clone())
            .collect();
        self.endpoint_cluster_etc_params = sorted
            .iter()
            .map(|&j| etc_params[j as usize].clone())
            .collect();

        // Step 2: Eliminate redundant endpoint clusters, or empty endpoint clusters
        let mut new_endpoint_clusters: Vec<Vec<u32>> = Vec::new();
        let mut new_subblock_etc_params: Vec<EndpointClusterEtcParams> = Vec::new();
        let mut i = 0;
        while i < n {
            if self.endpoint_clusters[i].is_empty() {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < n {
                if !self.endpoint_cluster_etc_params[i].same(&self.endpoint_cluster_etc_params[j]) {
                    break;
                }
                j += 1;
            }
            let mut c = self.endpoint_clusters[i].clone();
            for k in i + 1..j {
                c.extend_from_slice(&self.endpoint_clusters[k]);
            }
            new_endpoint_clusters.push(c);
            new_subblock_etc_params.push(self.endpoint_cluster_etc_params[i].clone());
            i = j;
        }
        if self.endpoint_clusters.len() != new_endpoint_clusters.len() {
            self.endpoint_clusters = new_endpoint_clusters;
            self.endpoint_cluster_etc_params = new_subblock_etc_params;
        }
    }

    fn create_initial_packed_texture(&mut self) {
        let mut ok = true;
        for block_index in 0..self.total_blocks as usize {
            let [cluster0, cluster1] = self.block_endpoint_clusters_indices[block_index];
            ok &= cluster0 == cluster1;
            let p0 = &self.endpoint_cluster_etc_params[cluster0 as usize];
            let p1 = &self.endpoint_cluster_etc_params[cluster1 as usize];
            let blk = &mut self.encoded_blocks[block_index];
            blk.set_block_color5(&p0.color_unscaled[0], &p1.color_unscaled[0]);
            blk.set_flip_bit(true);
            blk.set_inten_table(0, p0.inten_table[0]);
            blk.set_inten_table(1, p1.inten_table[0]);
            blk.determine_selectors(
                &self.params.source_blocks[block_index],
                self.params.perceptual,
                0,
                2,
            );
        }
        self.verify(ok);
        self.orig_encoded_blocks = self.encoded_blocks.clone();
    }

    fn compute_selector_clusters_within_each_parent_cluster(&mut self) {
        let mut block_selector_cluster_indices = vec![0u32; self.total_blocks as usize];
        for (cluster_index, cluster) in self.selector_cluster_block_indices.iter().enumerate() {
            for &b in cluster {
                block_selector_cluster_indices[b as usize] = cluster_index as u32;
            }
        }
        let mut within = vec![Vec::new(); self.selector_parent_cluster_block_indices.len()];
        for block_index in 0..self.total_blocks as usize {
            let parent = self.block_parent_selector_cluster[block_index] as usize;
            within[parent].push(block_selector_cluster_indices[block_index]);
        }
        let mut ok = true;
        for v in within.iter_mut() {
            ok &= !v.is_empty();
            sort_unique(v);
        }
        self.verify(ok);
        self.selector_clusters_within_each_parent_cluster = within;
    }

    fn generate_selector_clusters(&mut self) {
        let mut selector_clusterizer = TreeVectorQuant::<16>::new();
        for blk in &self.encoded_blocks {
            let mut v = [0.0f32; 16];
            for y in 0..4u32 {
                for x in 0..4u32 {
                    v[(x + y * 4) as usize] = blk.get_selector(x, y) as f32;
                }
            }
            let subblock_index = if blk.get_inten_table(0) > blk.get_inten_table(1) {
                0
            } else {
                1
            };
            let mut block_colors = [[0u8; 4]; 2];
            blk.get_block_low_high_colors(&mut block_colors, subblock_index);
            let dist = color_distance(
                self.params.perceptual,
                &block_colors[0],
                &block_colors[1],
                false,
            );
            const COLOR_DIST_TO_WEIGHT: u32 = 300;
            const MAX_WEIGHT: u32 = 4096;
            let weight = (dist / COLOR_DIST_TO_WEIGHT).clamp(1, MAX_WEIGHT);
            selector_clusterizer.add_training_vec(&v, weight as u64);
        }

        let selector_parent_codebook_size = if self.params.compression_level <= 1 {
            BASISU_SELECTOR_PARENT_CODEBOOK_SIZE_COMP_LEVEL_01
        } else {
            BASISU_SELECTOR_PARENT_CODEBOOK_SIZE_COMP_LEVEL_DEFAULT
        };
        let parent_codebook_size = if self.params.max_selector_clusters >= 256 {
            selector_parent_codebook_size
        } else {
            0
        };
        let max_threads = self.max_codebook_threads();
        let mut clusters = Vec::new();
        let mut parents = Vec::new();
        let status = generate_hierarchical_codebook_threaded(
            &mut selector_clusterizer,
            self.params.max_selector_clusters,
            if self.use_hierarchical_selector_codebooks {
                parent_codebook_size
            } else {
                0
            },
            &mut clusters,
            &mut parents,
            max_threads,
            false,
        );
        self.selector_cluster_block_indices = clusters;
        self.selector_parent_cluster_block_indices = parents;
        self.verify(status);

        if self.use_hierarchical_selector_codebooks {
            if self.selector_parent_cluster_block_indices.is_empty() {
                self.selector_parent_cluster_block_indices = vec![(0..self.total_blocks).collect()];
            }
            self.block_parent_selector_cluster = vec![0xFF; self.total_blocks as usize];
            for (pi, cluster) in self
                .selector_parent_cluster_block_indices
                .iter()
                .enumerate()
            {
                for &b in cluster {
                    self.block_parent_selector_cluster[b as usize] = pi as u8;
                }
            }
            let mut ok = self
                .block_parent_selector_cluster
                .iter()
                .all(|&p| p != 0xFF);
            for cluster in &self.selector_cluster_block_indices {
                if let Some(&first) = cluster.first() {
                    let p = self.block_parent_selector_cluster[first as usize];
                    ok &= cluster
                        .iter()
                        .all(|&b| self.block_parent_selector_cluster[b as usize] == p);
                }
            }
            self.verify(ok);
        }
    }

    /// For each selector cluster and each of the 16 pixels, picks the selector minimizing the
    /// error across all blocks of the cluster.
    fn create_optimized_selector_codebook(&mut self, _iter: u32) {
        let total_selector_clusters = self.selector_cluster_block_indices.len();
        self.optimized_cluster_selectors
            .resize(total_selector_clusters, EtcBlock::default());
        let perceptual = self.params.perceptual;
        for cluster_index in 0..total_selector_clusters {
            let cluster_block_indices = &self.selector_cluster_block_indices[cluster_index];
            if cluster_block_indices.is_empty() {
                continue;
            }
            let mut total_err = [[[0u64; 4]; 4]; 4];
            for &block_index in cluster_block_indices {
                let blk = &self.encoded_blocks[block_index as usize];
                let mut blk_colors = [[0u8; 4]; 4];
                blk.get_block_colors(&mut blk_colors, 0);
                let src = &self.params.source_blocks[block_index as usize];
                for y in 0..4 {
                    for x in 0..4 {
                        let orig_color = &src[x + y * 4];
                        for s in 0..4 {
                            total_err[y][x][s] +=
                                color_distance(perceptual, &blk_colors[s], orig_color, false)
                                    as u64;
                        }
                    }
                }
            }
            for y in 0..4 {
                for x in 0..4 {
                    let mut best_err = total_err[y][x][0];
                    let mut best_sel = 0u32;
                    for s in 1..4 {
                        if total_err[y][x][s] < best_err {
                            best_err = total_err[y][x][s];
                            best_sel = s as u32;
                        }
                    }
                    self.optimized_cluster_selectors[cluster_index]
                        .set_selector(x as u32, y as u32, best_sel);
                }
            }
        }
    }

    /// For each block, determines which quantized selectors best encode it given its quantized
    /// endpoints. May leave empty clusters.
    fn find_optimal_selector_clusters_for_each_block(&mut self) {
        if self.params.validate {
            let n = self.optimized_cluster_selectors.len() as u32;
            let ok = self.selector_cluster_block_indices.len() == n as usize
                && self
                    .selector_clusters_within_each_parent_cluster
                    .iter()
                    .all(|v| v.iter().all(|&c| c < n));
            self.verify(ok);
        }
        let total = self.total_blocks as usize;
        self.block_selector_cluster_index.resize(total, 0);

        if self.params.compression_level == 0 {
            // Just leave the blocks in their original selector clusters.
            for (sci, cluster) in self.selector_cluster_block_indices.iter().enumerate() {
                for &b in cluster {
                    self.block_selector_cluster_index[b as usize] = sci as u32;
                    self.encoded_blocks[b as usize].set_raw_selector_bits(
                        self.optimized_cluster_selectors[sci].get_raw_selector_bits(),
                    );
                }
            }
            return;
        }

        let mut unpacked = vec![0u8; 16 * self.optimized_cluster_selectors.len()];
        for (ci, s) in self.optimized_cluster_selectors.iter().enumerate() {
            for y in 0..4u32 {
                for x in 0..4u32 {
                    unpacked[ci * 16 + (y * 4 + x) as usize] = s.get_selector(x, y) as u8;
                }
            }
        }

        let perceptual = self.params.perceptual;
        const N: usize = 2048;
        let mut first_index = 0;
        while first_index < total {
            let last_index = total.min(first_index + N);
            let mut prev_best_cluster_index = 0u32;
            for block_index in first_index..last_index {
                let block = &self.params.source_blocks[block_index];
                if block_index > first_index && *block == self.params.source_blocks[block_index - 1]
                {
                    self.encoded_blocks[block_index].set_raw_selector_bits(
                        self.optimized_cluster_selectors[prev_best_cluster_index as usize]
                            .get_raw_selector_bits(),
                    );
                    self.block_selector_cluster_index[block_index] = prev_best_cluster_index;
                    continue;
                }

                let mut trial_block_colors = [[0u8; 4]; 4];
                self.encoded_blocks[block_index].get_block_colors_etc1s(&mut trial_block_colors);

                // precompute errors for the i-th block pixel and selector sel: [sel][i]
                let mut trial_errors = [[0u32; 16]; 4];
                for sel in 0..4 {
                    for i in 0..16 {
                        trial_errors[sel][i] =
                            color_distance(perceptual, &block[i], &trial_block_colors[sel], false);
                    }
                }
                let min_err = |i: usize| {
                    trial_errors[0][i]
                        .min(trial_errors[1][i])
                        .min(trial_errors[2][i])
                        .min(trial_errors[3][i]) as u64
                };
                let min_possible_error_0_15: u64 = (0..16).map(min_err).sum();
                let min_possible_error_4_15: u64 = (4..16).map(min_err).sum();
                let min_possible_error_8_15: u64 = (8..16).map(min_err).sum();
                let min_possible_error_12_15: u64 = (12..16).map(min_err).sum();

                let mut best_cluster_err = i64::MAX as u64;
                let mut best_cluster_index = 0u32;

                let parent = if self.block_parent_selector_cluster.is_empty() {
                    0
                } else {
                    self.block_parent_selector_cluster[block_index] as usize
                };
                let cluster_list = self
                    .selector_clusters_within_each_parent_cluster
                    .get(parent);
                let total_clusters = if self.use_hierarchical_selector_codebooks {
                    cluster_list.map_or(0, |c| c.len())
                } else {
                    self.selector_cluster_block_indices.len()
                };

                for cluster_iter in 0..total_clusters {
                    let cluster_index = if self.use_hierarchical_selector_codebooks {
                        cluster_list.map_or(0, |c| c[cluster_iter])
                    } else {
                        cluster_iter as u32
                    };
                    let s =
                        &unpacked[cluster_index as usize * 16..cluster_index as usize * 16 + 16];
                    let te = |i: usize| trial_errors[s[i] as usize][i] as u64;

                    let mut trial_err = te(0) + te(1) + te(2) + te(3);
                    if trial_err + min_possible_error_4_15 >= best_cluster_err {
                        continue;
                    }
                    trial_err += te(4) + te(5) + te(6) + te(7);
                    if trial_err + min_possible_error_8_15 >= best_cluster_err {
                        continue;
                    }
                    trial_err += te(8) + te(9) + te(10) + te(11);
                    if trial_err + min_possible_error_12_15 >= best_cluster_err {
                        continue;
                    }
                    trial_err += te(12) + te(13) + te(14) + te(15);
                    if trial_err < best_cluster_err {
                        best_cluster_err = trial_err;
                        best_cluster_index = cluster_index;
                        if best_cluster_err == min_possible_error_0_15 {
                            break;
                        }
                    }
                }

                self.encoded_blocks[block_index].set_raw_selector_bits(
                    self.optimized_cluster_selectors[best_cluster_index as usize]
                        .get_raw_selector_bits(),
                );
                self.block_selector_cluster_index[block_index] = best_cluster_index;
                prev_best_cluster_index = best_cluster_index;
            }
            first_index = last_index;
        }

        for v in self.selector_cluster_block_indices.iter_mut() {
            v.clear();
        }
        for block_index in 0..total {
            let best = self.block_selector_cluster_index[block_index] as usize;
            if self.selector_cluster_block_indices.len() <= best {
                self.selector_cluster_block_indices
                    .resize(best + 1, Vec::new());
            }
            self.selector_cluster_block_indices[best].push(block_index as u32);
        }
    }

    fn refine_block_endpoints_given_selectors(&mut self) -> u32 {
        for block_index in 0..self.total_blocks {
            let ec = self.block_endpoint_clusters_indices[block_index as usize];
            self.endpoint_cluster_etc_params[ec[0] as usize]
                .subblocks
                .push(block_index * 2);
            self.endpoint_cluster_etc_params[ec[1] as usize]
                .subblocks
                .push(block_index * 2 + 1);
        }

        let perceptual = self.params.perceptual;
        let mut total_subblocks_refined = 0u32;
        let mut optimizer = Etc1Optimizer::new();

        for endpoint_cluster_index in 0..self.endpoint_cluster_etc_params.len() {
            let subblocks = self.endpoint_cluster_etc_params[endpoint_cluster_index]
                .subblocks
                .clone();

            let mut subblock_colors: [Vec<ColorRgba>; 2] = [Vec::new(), Vec::new()];
            let mut subblock_selectors: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
            let mut cur_subblock_err = [0u64; 2];

            for &t in &subblocks {
                let block_index = (t >> 1) as usize;
                let subblock_index = (t & 1) as usize;
                let blk = &self.encoded_blocks[block_index];
                let use_individual_mode = !blk.get_diff_bit() as usize;
                let src = &self.params.source_blocks[block_index];
                let mut unpacked_block_pixels = [[0u8; 4]; 16];
                unpack_etc1(blk, &mut unpacked_block_pixels, false);
                for i in 0..8 {
                    let pixel_index = G_ETC1_PIXEL_INDICES[1][subblock_index][i] as usize;
                    let (cx, cy) = G_ETC1_PIXEL_COORDS[1][subblock_index][i];
                    subblock_colors[use_individual_mode].push(src[pixel_index]);
                    cur_subblock_err[use_individual_mode] += color_distance(
                        perceptual,
                        &src[pixel_index],
                        &unpacked_block_pixels[pixel_index],
                        false,
                    ) as u64;
                    subblock_selectors[use_individual_mode]
                        .push(blk.get_selector(cx as u32, cy as u32) as u8);
                }
            }

            let mut results = [new_optimizer_results(0), new_optimizer_results(0)];
            let mut results_valid = [false; 2];
            for uim in 0..2 {
                let total_pixels = subblock_colors[uim].len() as u32;
                if total_pixels == 0 {
                    continue;
                }
                let mut p = Etc1OptimizerParams {
                    num_src_pixels: total_pixels,
                    src_pixels: &subblock_colors[uim],
                    use_color4: uim != 0,
                    force_selectors: Some(&subblock_selectors[uim]),
                    ..Default::default()
                };
                p.pack.perceptual = perceptual;
                p.pack.quality = BasisEtcQuality::Uber;
                results[uim] = new_optimizer_results(total_pixels);
                optimizer.init(&p);
                if !optimizer.compute(&p, &mut results[uim]) {
                    continue;
                }
                if results[uim].error < cur_subblock_err[uim] {
                    results_valid[uim] = true;
                }
            }

            for uim in 0..2 {
                if !results_valid[uim] {
                    continue;
                }
                let num_passes = if uim != 0 { 1 } else { 2 };
                let mut all_passed5 = true;
                for pass in 0..num_passes {
                    for &t in &subblocks {
                        let block_index = (t >> 1) as usize;
                        let subblock_index = t & 1;
                        let blk = &mut self.encoded_blocks[block_index];
                        if !blk.get_diff_bit() != (uim != 0) {
                            continue;
                        }
                        let sp = &mut self.endpoint_cluster_etc_params[endpoint_cluster_index];
                        if uim != 0 {
                            let r = &results[1];
                            blk.set_base4_color(
                                subblock_index,
                                EtcBlock::pack_color4(&r.block_color_unscaled, false, 127),
                            );
                            blk.set_inten_table(subblock_index, r.block_inten_table);
                            sp.color_error[1] = r.error;
                            sp.inten_table[1] = r.block_inten_table;
                            sp.color_unscaled[1] = r.block_color_unscaled;
                            total_subblocks_refined += 1;
                        } else {
                            let r = &results[0];
                            let base_color5 = blk.get_base5_color();
                            let delta_color3 = blk.get_delta3_color();
                            let (r0, g0, b0) = EtcBlock::unpack_color5_rgb(base_color5, false);
                            let (_, r1, g1, b1) = EtcBlock::unpack_color5_delta_rgb(
                                base_color5,
                                delta_color3,
                                false,
                                255,
                            );
                            let mut rr = [r0, r1];
                            let mut gg = [g0, g1];
                            let mut bb = [b0, b1];
                            rr[subblock_index as usize] = r.block_color_unscaled[0] as u32;
                            gg[subblock_index as usize] = r.block_color_unscaled[1] as u32;
                            bb[subblock_index as usize] = r.block_color_unscaled[2] as u32;
                            let colors = [
                                [rr[0] as u8, gg[0] as u8, bb[0] as u8, 255],
                                [rr[1] as u8, gg[1] as u8, bb[1] as u8, 255],
                            ];
                            if !EtcBlock::try_pack_color5_delta3(&colors) {
                                all_passed5 = false;
                                break;
                            }
                            if pass == 1 && all_passed5 {
                                blk.set_block_color5(&colors[0], &colors[1]);
                                blk.set_inten_table(subblock_index, r.block_inten_table);
                                sp.color_error[0] = r.error;
                                sp.inten_table[0] = r.block_inten_table;
                                sp.color_unscaled[0] = r.block_color_unscaled;
                                total_subblocks_refined += 1;
                            }
                        }
                    }
                }
            }
        }
        total_subblocks_refined
    }

    fn finalize(&mut self) {
        for block_index in 0..self.total_blocks {
            for subblock_index in 0..2 {
                let c = self.get_subblock_endpoint_cluster_index(block_index, subblock_index);
                self.endpoint_cluster_etc_params[c as usize].color_used[0] = true;
            }
        }
    }

    /// `reoptimize_remapped_endpoints`: the backend remapped the block endpoints during RDO,
    /// so reoptimize the endpoint codebook (bottom up, some leaves can be combined).
    pub fn reoptimize_remapped_endpoints(
        &mut self,
        new_block_endpoints: &[u32],
        old_to_new_endpoint_cluster_indices: &mut Vec<i32>,
        optimize_final_codebook: bool,
        block_selector_indices: Option<&[u32]>,
    ) {
        let n_clusters = self.endpoint_clusters.len();
        let mut new_endpoint_cluster_block_indices = vec![Vec::new(); n_clusters];
        for (i, &e) in new_block_endpoints.iter().enumerate() {
            new_endpoint_cluster_block_indices[e as usize].push(i as u32);
        }
        let mut cluster_valid = vec![false; n_clusters];
        let mut cluster_improved = vec![false; n_clusters];
        let perceptual = self.params.perceptual;
        let quality = if self.params.compression_level == BASISU_MAX_ETC1S_COMPRESSION_LEVEL {
            BasisEtcQuality::Uber
        } else {
            BasisEtcQuality::Slow
        };
        let mut optimizer = Etc1Optimizer::new();
        let mut ok = true;

        for cluster_index in 0..n_clusters {
            let cluster_block_indices = &new_endpoint_cluster_block_indices[cluster_index];
            if cluster_block_indices.is_empty() {
                continue;
            }
            let total_pixels = cluster_block_indices.len() as u32 * 16;
            let mut cluster_pixels: Vec<ColorRgba> = Vec::with_capacity(total_pixels as usize);
            let mut force_selectors = vec![0u8; total_pixels as usize];

            let mut blk = EtcBlock::default();
            blk.set_block_color5_etc1s(
                self.get_endpoint_cluster_unscaled_color(cluster_index as u32, false),
            );
            blk.set_inten_tables_etc1s(
                self.get_endpoint_cluster_inten_table(cluster_index as u32, false),
            );
            blk.set_flip_bit(true);

            let mut cur_err = 0u64;
            for (k, &block_index) in cluster_block_indices.iter().enumerate() {
                let src = &self.params.source_blocks[block_index as usize];
                cluster_pixels.extend_from_slice(src);
                let selector_cluster_index = match block_selector_indices {
                    Some(s) => s[block_index as usize],
                    None => self.get_block_selector_cluster_index(block_index),
                };
                let blk_selectors = self.get_selector_cluster_selector_bits(selector_cluster_index);
                blk.set_raw_selector_bits(blk_selectors.get_raw_selector_bits());
                cur_err += blk.evaluate_etc1_error(src, perceptual, -1);
                for y in 0..4u32 {
                    for x in 0..4u32 {
                        force_selectors[k * 16 + (x + y * 4) as usize] =
                            blk_selectors.get_selector(x, y) as u8;
                    }
                }
            }

            let mut new_params = EndpointClusterEtcParams::default();
            {
                let mut p = Etc1OptimizerParams {
                    num_src_pixels: total_pixels,
                    src_pixels: &cluster_pixels,
                    use_color4: false,
                    force_selectors: Some(&force_selectors),
                    ..Default::default()
                };
                p.pack.perceptual = perceptual;
                p.pack.quality = quality;
                let mut r = new_optimizer_results(total_pixels);
                optimizer.init(&p);
                if !optimizer.compute(&p, &mut r) {
                    ok = false;
                }
                new_params.color_unscaled[0] = r.block_color_unscaled;
                new_params.inten_table[0] = r.block_inten_table;
                new_params.color_error[0] = r.error;
                new_params.color_used[0] = true;
                new_params.valid = true;
            }
            if new_params.color_error[0] < cur_err {
                self.endpoint_cluster_etc_params[cluster_index] = new_params;
                cluster_improved[cluster_index] = true;
            }
            cluster_valid[cluster_index] = true;
        }

        old_to_new_endpoint_cluster_indices.clear();
        old_to_new_endpoint_cluster_indices.resize(n_clusters, -1);
        let mut total_new_endpoint_clusters = 0i32;
        for old in 0..n_clusters {
            if cluster_valid[old] {
                old_to_new_endpoint_cluster_indices[old] = total_new_endpoint_clusters;
                total_new_endpoint_clusters += 1;
            }
        }

        if optimize_final_codebook {
            let mut new_etc_params =
                vec![EndpointClusterEtcParams::default(); total_new_endpoint_clusters as usize];
            for old in 0..n_clusters {
                let nw = old_to_new_endpoint_cluster_indices[old];
                if nw >= 0 {
                    new_etc_params[nw as usize] = self.endpoint_cluster_etc_params[old].clone();
                }
            }
            let mut new_endpoint_clusters = vec![Vec::new(); total_new_endpoint_clusters as usize];
            for (block_index, &old) in new_block_endpoints.iter().enumerate() {
                let nw = old_to_new_endpoint_cluster_indices[old as usize];
                if nw < 0 || nw >= total_new_endpoint_clusters {
                    ok = false;
                    continue;
                }
                let b = block_index as u32;
                new_endpoint_clusters[nw as usize].push(b * 2);
                new_endpoint_clusters[nw as usize].push(b * 2 + 1);
                new_etc_params[nw as usize].subblocks.push(b * 2);
                new_etc_params[nw as usize].subblocks.push(b * 2 + 1);
                self.block_endpoint_clusters_indices[block_index] = [nw as u32; 2];
            }
            self.endpoint_clusters = new_endpoint_clusters;
            self.endpoint_cluster_etc_params = new_etc_params;

            self.eliminate_redundant_or_empty_endpoint_clusters();

            for new_cluster_index in 0..self.endpoint_clusters.len() {
                for &sb in &self.endpoint_clusters[new_cluster_index] {
                    let block_index = (sb >> 1) as usize;
                    self.block_endpoint_clusters_indices[block_index] =
                        [new_cluster_index as u32; 2];
                    let old = new_block_endpoints[block_index] as usize;
                    old_to_new_endpoint_cluster_indices[old] = new_cluster_index as i32;
                }
            }

            for block_index in 0..self.encoded_blocks.len() {
                let c = self.get_subblock_endpoint_cluster_index(block_index as u32, 0);
                let color = *self.get_endpoint_cluster_unscaled_color(c, false);
                let inten = self.get_endpoint_cluster_inten_table(c, false);
                self.encoded_blocks[block_index].set_block_color5_etc1s(&color);
                self.encoded_blocks[block_index].set_inten_tables_etc1s(inten);
            }
        }
        self.verify(ok);
    }

    /// Endpoint clusterization hierarchy integrity checker (does not check for empty clusters).
    fn validate_endpoint_cluster_hierarchy(&self, ensure_clusters_have_same_parents: bool) -> bool {
        if self.endpoint_parent_clusters.is_empty() {
            return true;
        }
        let n = self.total_blocks as usize * 2;
        let mut subblock_parent_indices = vec![-1i32; n];
        let mut subblock_cluster_indices = vec![-1i32; n];
        for (pi, parent) in self.endpoint_parent_clusters.iter().enumerate() {
            for &s in parent {
                let s = s as usize;
                if s >= n || subblock_parent_indices[s] != -1 {
                    return false;
                }
                subblock_parent_indices[s] = pi as i32;
            }
        }
        if subblock_parent_indices.contains(&-1) {
            return false;
        }
        for (ci, cluster) in self.endpoint_clusters.iter().enumerate() {
            let mut parent_index = 0;
            for (i, &s) in cluster.iter().enumerate() {
                let s = s as usize;
                if s >= n || subblock_cluster_indices[s] != -1 {
                    return false;
                }
                subblock_cluster_indices[s] = ci as i32;
                if ensure_clusters_have_same_parents {
                    if i == 0 {
                        parent_index = subblock_parent_indices[s];
                    } else if subblock_parent_indices[s] != parent_index {
                        return false;
                    }
                }
            }
        }
        !subblock_cluster_indices.contains(&-1)
    }

    /// Very slow; only intended for debugging (`-validate_etc1s`).
    fn validate_output(&self) -> bool {
        if !self.check_etc1s_constraints() {
            return false;
        }
        for block_index in 0..self.total_blocks {
            let out = self.get_output_block(block_index);
            if !out.get_flip_bit() || !self.get_diff_flag(block_index) {
                return false;
            }
            let mut blk = EtcBlock::default();
            blk.set_flip_bit(true);
            blk.set_diff_bit(true);
            let e0 = self.get_subblock_endpoint_cluster_index(block_index, 0);
            let e1 = self.get_subblock_endpoint_cluster_index(block_index, 1);
            if e0 != e1 {
                return false;
            }
            if !blk.set_block_color5_check(
                self.get_endpoint_cluster_unscaled_color(e0, false),
                self.get_endpoint_cluster_unscaled_color(e1, false),
            ) {
                return false;
            }
            if !self.get_endpoint_cluster_color_is_used(e0, false) {
                return false;
            }
            blk.set_inten_table(0, self.get_endpoint_cluster_inten_table(e0, false));
            blk.set_inten_table(1, self.get_endpoint_cluster_inten_table(e1, false));
            let sci = self.get_block_selector_cluster_index(block_index);
            if sci >= self.get_total_selector_clusters()
                || !self
                    .get_selector_cluster_block_indices(sci)
                    .contains(&block_index)
            {
                return false;
            }
            blk.set_raw_selector_bits(
                self.get_selector_cluster_selector_bits(sci)
                    .get_raw_selector_bits(),
            );
            if out.get_flip_bit() != blk.get_flip_bit()
                || out.get_diff_bit() != blk.get_diff_bit()
                || out.get_inten_table(0) != blk.get_inten_table(0)
                || out.get_inten_table(1) != blk.get_inten_table(1)
                || out.get_base5_color() != blk.get_base5_color()
                || out.get_delta3_color() != blk.get_delta3_color()
                || out.get_raw_selector_bits() != blk.get_raw_selector_bits()
            {
                return false;
            }
        }
        true
    }
}
