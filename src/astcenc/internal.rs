//! Port of the compressor data structures of `astcenc_internal.h`.
//!
//! Arrays keep the C sizes and SIMD padding (AVX2 build: 8 lanes) so that padded table
//! contents match the original.

#![allow(clippy::large_stack_arrays)]

use super::vecmath::Vfloat4;

pub const BLOCK_MAX_TEXELS: usize = 216;
pub const BLOCK_MAX_COMPONENTS: usize = 4;
pub const BLOCK_MAX_PARTITIONS: usize = 4;
pub const BLOCK_MAX_PARTITIONINGS: usize = 1024;
pub const BLOCK_MAX_KMEANS_TEXELS: usize = 64;
pub const BLOCK_MAX_WEIGHTS: usize = 64;
pub const BLOCK_MAX_WEIGHTS_2PLANE: usize = BLOCK_MAX_WEIGHTS / 2;
pub const BLOCK_MIN_WEIGHT_BITS: u32 = 24;
pub const BLOCK_MAX_WEIGHT_BITS: u32 = 96;
pub const BLOCK_BAD_BLOCK_MODE: u16 = 0xFFFF;
pub const BLOCK_BAD_PARTITIONING: u16 = 0xFFFF;
pub const PARTITION_INDEX_BITS: u32 = 10;
pub const WEIGHTS_PLANE2_OFFSET: usize = BLOCK_MAX_WEIGHTS_2PLANE;
pub const WEIGHTS_TEXEL_SUM: f32 = 16.0;
pub const WEIGHTS_MAX_BLOCK_MODES: usize = 2048;
pub const WEIGHTS_MAX_DECIMATION_MODES: usize = 87;
pub const ERROR_CALC_DEFAULT: f32 = 1e30;
pub const TUNE_MIN_SEARCH_MODE0: f32 = 0.85;
pub const TUNE_MAX_TRIAL_CANDIDATES: usize = 8;
pub const TUNE_MAX_PARTITIONING_CANDIDATES: usize = 8;
pub const TUNE_MAX_ANGULAR_QUANT: usize = 7;

// endpoint_formats
pub const FMT_LUMINANCE: u8 = 0;
pub const FMT_LUMINANCE_DELTA: u8 = 1;
pub const FMT_HDR_LUMINANCE_LARGE_RANGE: u8 = 2;
pub const FMT_HDR_LUMINANCE_SMALL_RANGE: u8 = 3;
pub const FMT_LUMINANCE_ALPHA: u8 = 4;
pub const FMT_LUMINANCE_ALPHA_DELTA: u8 = 5;
pub const FMT_RGB_SCALE: u8 = 6;
pub const FMT_HDR_RGB_SCALE: u8 = 7;
pub const FMT_RGB: u8 = 8;
pub const FMT_RGB_DELTA: u8 = 9;
pub const FMT_RGB_SCALE_ALPHA: u8 = 10;
pub const FMT_HDR_RGB: u8 = 11;
pub const FMT_RGBA: u8 = 12;
pub const FMT_RGBA_DELTA: u8 = 13;
pub const FMT_HDR_RGB_LDR_ALPHA: u8 = 14;
pub const FMT_HDR_RGBA: u8 = 15;

// quant_method
pub const QUANT_2: u32 = 0;
pub const QUANT_3: u32 = 1;
pub const QUANT_4: u32 = 2;
pub const QUANT_5: u32 = 3;
pub const QUANT_6: u32 = 4;
pub const QUANT_8: u32 = 5;
pub const QUANT_10: u32 = 6;
pub const QUANT_12: u32 = 7;
pub const QUANT_16: u32 = 8;
pub const QUANT_20: u32 = 9;
pub const QUANT_24: u32 = 10;
pub const QUANT_32: u32 = 11;
pub const QUANT_40: u32 = 12;
pub const QUANT_48: u32 = 13;
pub const QUANT_64: u32 = 14;
pub const QUANT_80: u32 = 15;
pub const QUANT_96: u32 = 16;
pub const QUANT_128: u32 = 17;
pub const QUANT_160: u32 = 18;
pub const QUANT_192: u32 = 19;
pub const QUANT_256: u32 = 20;

/// `get_quant_level`.
pub fn get_quant_level(method: u32) -> u32 {
    const LEVELS: [u32; 21] = [
        2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256,
    ];
    LEVELS.get(method as usize).copied().unwrap_or(0)
}

/// Port of `partition_metrics`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PartitionMetrics {
    pub avg: Vfloat4,
    pub dir: Vfloat4,
}

/// Port of `partition_info`.
#[derive(Clone)]
pub struct PartitionInfo {
    pub partition_count: u16,
    pub partition_index: u16,
    pub partition_texel_count: [u8; BLOCK_MAX_PARTITIONS],
    pub partition_of_texel: [u8; BLOCK_MAX_TEXELS],
    pub texels_of_partition: [[u8; BLOCK_MAX_TEXELS]; BLOCK_MAX_PARTITIONS],
}

impl Default for PartitionInfo {
    fn default() -> Self {
        PartitionInfo {
            partition_count: 0,
            partition_index: 0,
            partition_texel_count: [0; BLOCK_MAX_PARTITIONS],
            partition_of_texel: [0; BLOCK_MAX_TEXELS],
            texels_of_partition: [[0; BLOCK_MAX_TEXELS]; BLOCK_MAX_PARTITIONS],
        }
    }
}

/// Port of `decimation_info` (compressor build).
#[derive(Clone)]
pub struct DecimationInfo {
    pub texel_count: u8,
    pub max_texel_weight_count: u8,
    pub weight_count: u8,
    pub weight_x: u8,
    pub weight_y: u8,
    pub weight_z: u8,
    pub texel_weight_count: [u8; BLOCK_MAX_TEXELS],
    pub texel_weights_tr: [[u8; BLOCK_MAX_TEXELS]; 4],
    pub texel_weight_contribs_int_tr: [[u8; BLOCK_MAX_TEXELS]; 4],
    pub texel_weight_contribs_float_tr: [[f32; BLOCK_MAX_TEXELS]; 4],
    pub weight_texel_count: [u8; BLOCK_MAX_WEIGHTS],
    pub weight_texels_tr: [[u8; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
    pub weights_texel_contribs_tr: [[f32; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
    pub texel_contrib_for_weight: [[f32; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
}

impl Default for DecimationInfo {
    fn default() -> Self {
        DecimationInfo {
            texel_count: 0,
            max_texel_weight_count: 0,
            weight_count: 0,
            weight_x: 0,
            weight_y: 0,
            weight_z: 0,
            texel_weight_count: [0; BLOCK_MAX_TEXELS],
            texel_weights_tr: [[0; BLOCK_MAX_TEXELS]; 4],
            texel_weight_contribs_int_tr: [[0; BLOCK_MAX_TEXELS]; 4],
            texel_weight_contribs_float_tr: [[0.0; BLOCK_MAX_TEXELS]; 4],
            weight_texel_count: [0; BLOCK_MAX_WEIGHTS],
            weight_texels_tr: [[0; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
            weights_texel_contribs_tr: [[0.0; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
            texel_contrib_for_weight: [[0.0; BLOCK_MAX_WEIGHTS]; BLOCK_MAX_TEXELS],
        }
    }
}

/// Port of `block_mode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockMode {
    pub mode_index: u16,
    pub decimation_mode: u8,
    pub quant_mode: u8,
    pub weight_bits: u8,
    pub is_dual_plane: bool,
}

impl BlockMode {
    pub fn get_weight_quant_mode(&self) -> u32 {
        self.quant_mode as u32
    }
}

/// Port of `decimation_mode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecimationMode {
    pub maxprec_1plane: i8,
    pub maxprec_2planes: i8,
    pub refprec_1plane: u16,
    pub refprec_2planes: u16,
}

impl DecimationMode {
    pub fn set_ref_1plane(&mut self, weight_quant: u32) {
        self.refprec_1plane |= 1 << weight_quant;
    }
    pub fn is_ref_1plane(&self, max_weight_quant: u32) -> bool {
        let mask = ((1u32 << (max_weight_quant + 1)) - 1) as u16;
        self.refprec_1plane & mask != 0
    }
    pub fn set_ref_2plane(&mut self, weight_quant: u32) {
        self.refprec_2planes |= 1 << weight_quant;
    }
    pub fn is_ref_2plane(&self, max_weight_quant: u32) -> bool {
        let mask = ((1u32 << (max_weight_quant + 1)) - 1) as u16;
        self.refprec_2planes & mask != 0
    }
}

/// Port of `block_size_descriptor` (compressor build).
pub struct BlockSizeDescriptor {
    pub dim_x: u8,
    pub dim_y: u8,
    pub dim_z: u8,
    pub texel_count: u8,
    pub decimation_mode_count_always: u32,
    pub decimation_mode_count_selected: u32,
    pub decimation_mode_count_all: u32,
    pub block_mode_count_1plane_always: u32,
    pub block_mode_count_1plane_selected: u32,
    pub block_mode_count_1plane_2plane_selected: u32,
    pub block_mode_count_all: u32,
    pub partitioning_count_selected: [u32; BLOCK_MAX_PARTITIONS],
    pub partitioning_count_all: [u32; BLOCK_MAX_PARTITIONS],
    pub decimation_modes: [DecimationMode; WEIGHTS_MAX_DECIMATION_MODES],
    pub decimation_tables: Vec<DecimationInfo>,
    pub block_mode_packed_index: [u16; WEIGHTS_MAX_BLOCK_MODES],
    pub block_modes: [BlockMode; WEIGHTS_MAX_BLOCK_MODES],
    /// `(3 * BLOCK_MAX_PARTITIONINGS) + 1` entries: 2, 3, 4 partition tables then the
    /// single 1 partition entry.
    pub partitionings: Vec<PartitionInfo>,
    pub partitioning_packed_index: [[u16; BLOCK_MAX_PARTITIONINGS]; 3],
    pub kmeans_texels: [u8; BLOCK_MAX_KMEANS_TEXELS],
    pub coverage_bitmaps_2: Vec<[u64; 2]>,
    pub coverage_bitmaps_3: Vec<[u64; 3]>,
    pub coverage_bitmaps_4: Vec<[u64; 4]>,
}

impl BlockSizeDescriptor {
    /// Zero initialised descriptor.
    pub fn empty() -> Box<BlockSizeDescriptor> {
        Box::new(BlockSizeDescriptor {
            dim_x: 0,
            dim_y: 0,
            dim_z: 0,
            texel_count: 0,
            decimation_mode_count_always: 0,
            decimation_mode_count_selected: 0,
            decimation_mode_count_all: 0,
            block_mode_count_1plane_always: 0,
            block_mode_count_1plane_selected: 0,
            block_mode_count_1plane_2plane_selected: 0,
            block_mode_count_all: 0,
            partitioning_count_selected: [0; BLOCK_MAX_PARTITIONS],
            partitioning_count_all: [0; BLOCK_MAX_PARTITIONS],
            decimation_modes: [DecimationMode::default(); WEIGHTS_MAX_DECIMATION_MODES],
            decimation_tables: vec![DecimationInfo::default(); WEIGHTS_MAX_DECIMATION_MODES],
            block_mode_packed_index: [0; WEIGHTS_MAX_BLOCK_MODES],
            block_modes: [BlockMode::default(); WEIGHTS_MAX_BLOCK_MODES],
            partitionings: vec![PartitionInfo::default(); 3 * BLOCK_MAX_PARTITIONINGS + 1],
            partitioning_packed_index: [[0; BLOCK_MAX_PARTITIONINGS]; 3],
            kmeans_texels: [0; BLOCK_MAX_KMEANS_TEXELS],
            coverage_bitmaps_2: vec![[0; 2]; BLOCK_MAX_PARTITIONINGS],
            coverage_bitmaps_3: vec![[0; 3]; BLOCK_MAX_PARTITIONINGS],
            coverage_bitmaps_4: vec![[0; 4]; BLOCK_MAX_PARTITIONINGS],
        })
    }

    /// `get_block_mode`.
    pub fn get_block_mode(&self, block_mode: usize) -> &BlockMode {
        let packed = self.block_mode_packed_index[block_mode] as usize;
        &self.block_modes[packed.min(WEIGHTS_MAX_BLOCK_MODES - 1)]
    }

    pub fn get_decimation_mode(&self, decimation_mode: usize) -> &DecimationMode {
        &self.decimation_modes[decimation_mode]
    }

    pub fn get_decimation_info(&self, decimation_mode: usize) -> &DecimationInfo {
        &self.decimation_tables[decimation_mode]
    }

    /// Start index in `partitionings` of `get_partition_table(partition_count)`.
    pub fn partition_table_base(partition_count: u32) -> usize {
        let pc = if partition_count == 1 {
            5
        } else {
            partition_count
        };
        (pc as usize - 2) * BLOCK_MAX_PARTITIONINGS
    }

    /// `get_partition_info`.
    pub fn get_partition_info(&self, partition_count: u32, index: u32) -> &PartitionInfo {
        let mut packed_index = 0usize;
        if partition_count >= 2 {
            packed_index = self.partitioning_packed_index[partition_count as usize - 2]
                [index as usize] as usize;
        }
        let i = Self::partition_table_base(partition_count) + packed_index;
        &self.partitionings[i.min(self.partitionings.len() - 1)]
    }

    /// `get_raw_partition_info`.
    pub fn get_raw_partition_info(
        &self,
        partition_count: u32,
        packed_index: u32,
    ) -> &PartitionInfo {
        let i = Self::partition_table_base(partition_count) + packed_index as usize;
        &self.partitionings[i.min(self.partitionings.len() - 1)]
    }
}
