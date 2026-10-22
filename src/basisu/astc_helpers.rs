//! Port of `basisu_astc_helpers.h`: the ASTC logical block unpacker and texel
//! decoder (plus the block packer) used by the UASTC HDR transcoders.
#![allow(clippy::needless_range_loop)]

use super::hdr_tables::{ASTC_QUINT_ENCODE, ASTC_TRIT_ENCODE, QUINT_DECODE, TRIT_DECODE};
use std::sync::OnceLock;

pub const MIN_GRID_DIM: u32 = 2;
pub const MIN_BLOCK_DIM: u32 = 4;
pub const MAX_BLOCK_DIM: u32 = 12;
pub const MAX_BLOCK_PIXELS: usize = (MAX_BLOCK_DIM * MAX_BLOCK_DIM) as usize;
pub const MAX_GRID_WEIGHTS: u32 = 64;
pub const MAX_WEIGHT_INTERPOLANT_VALUE: u32 = 64;
pub const MAX_PARTITIONS: u32 = 4;
pub const MAX_DUAL_PLANE_PARTITIONS: u32 = 3;
pub const NUM_PARTITION_PATTERNS: u32 = 1024;
pub const MAX_ENDPOINTS: u32 = 18;

pub const CEM_LDR_LUM_DIRECT: u32 = 0;
pub const CEM_LDR_LUM_BASE_PLUS_OFS: u32 = 1;
pub const CEM_HDR_LUM_LARGE_RANGE: u32 = 2;
pub const CEM_HDR_LUM_SMALL_RANGE: u32 = 3;
pub const CEM_LDR_LUM_ALPHA_DIRECT: u32 = 4;
pub const CEM_LDR_LUM_ALPHA_BASE_PLUS_OFS: u32 = 5;
pub const CEM_LDR_RGB_BASE_SCALE: u32 = 6;
pub const CEM_HDR_RGB_BASE_SCALE: u32 = 7;
pub const CEM_LDR_RGB_DIRECT: u32 = 8;
pub const CEM_LDR_RGB_BASE_PLUS_OFFSET: u32 = 9;
pub const CEM_LDR_RGB_BASE_SCALE_PLUS_TWO_A: u32 = 10;
pub const CEM_HDR_RGB: u32 = 11;
pub const CEM_LDR_RGBA_DIRECT: u32 = 12;
pub const CEM_LDR_RGBA_BASE_PLUS_OFFSET: u32 = 13;
pub const CEM_HDR_RGB_LDR_ALPHA: u32 = 14;
pub const CEM_HDR_RGB_HDR_ALPHA: u32 = 15;

pub const BISE_2_LEVELS: u32 = 0;
pub const BISE_3_LEVELS: u32 = 1;
pub const BISE_4_LEVELS: u32 = 2;
pub const BISE_5_LEVELS: u32 = 3;
pub const BISE_6_LEVELS: u32 = 4;
pub const BISE_8_LEVELS: u32 = 5;
pub const BISE_10_LEVELS: u32 = 6;
pub const BISE_12_LEVELS: u32 = 7;
pub const BISE_16_LEVELS: u32 = 8;
pub const BISE_20_LEVELS: u32 = 9;
pub const BISE_24_LEVELS: u32 = 10;
pub const BISE_32_LEVELS: u32 = 11;
pub const BISE_40_LEVELS: u32 = 12;
pub const BISE_48_LEVELS: u32 = 13;
pub const BISE_64_LEVELS: u32 = 14;
pub const BISE_80_LEVELS: u32 = 15;
pub const BISE_96_LEVELS: u32 = 16;
pub const BISE_128_LEVELS: u32 = 17;
pub const BISE_160_LEVELS: u32 = 18;
pub const BISE_192_LEVELS: u32 = 19;
pub const BISE_256_LEVELS: u32 = 20;
pub const TOTAL_ISE_RANGES: u32 = 21;
pub const FIRST_VALID_ENDPOINT_ISE_RANGE: u32 = 4;
pub const LAST_VALID_ENDPOINT_ISE_RANGE: u32 = 20;
pub const FIRST_VALID_WEIGHT_ISE_RANGE: u32 = 0;
pub const LAST_VALID_WEIGHT_ISE_RANGE: u32 = 11;

/// `g_ise_range_table`: bits, trits, quints.
pub const ISE_RANGE_TABLE: [[u8; 3]; 21] = [
    [1, 0, 0],
    [0, 1, 0],
    [2, 0, 0],
    [0, 0, 1],
    [1, 1, 0],
    [3, 0, 0],
    [1, 0, 1],
    [2, 1, 0],
    [4, 0, 0],
    [2, 0, 1],
    [3, 1, 0],
    [5, 0, 0],
    [3, 0, 1],
    [4, 1, 0],
    [6, 0, 0],
    [4, 0, 1],
    [5, 1, 0],
    [7, 0, 0],
    [5, 0, 1],
    [6, 1, 0],
    [8, 0, 0],
];

pub type HalfFloat = u16;

/// Port of `log_astc_block`. `m_weights` and `m_solid_color` share storage in
/// C (a union); [`LogAstcBlock::solid_color`] reads the same bytes.
#[derive(Debug, Clone, Copy)]
pub struct LogAstcBlock {
    pub error_flag: bool,
    pub solid_color_flag_ldr: bool,
    pub solid_color_flag_hdr: bool,
    pub user_mode: u8,
    pub grid_width: u8,
    pub grid_height: u8,
    pub dual_plane: bool,
    pub weight_ise_range: u8,
    pub endpoint_ise_range: u8,
    pub color_component_selector: u8,
    pub num_partitions: u8,
    pub partition_id: u16,
    pub color_endpoint_modes: [u8; MAX_PARTITIONS as usize],
    pub weights: [u8; MAX_GRID_WEIGHTS as usize],
    pub endpoints: [u8; MAX_ENDPOINTS as usize],
}

impl Default for LogAstcBlock {
    fn default() -> Self {
        Self {
            error_flag: false,
            solid_color_flag_ldr: false,
            solid_color_flag_hdr: false,
            user_mode: 0,
            grid_width: 0,
            grid_height: 0,
            dual_plane: false,
            weight_ise_range: 0,
            endpoint_ise_range: 0,
            color_component_selector: 0,
            num_partitions: 0,
            partition_id: 0,
            color_endpoint_modes: [0; MAX_PARTITIONS as usize],
            weights: [0; MAX_GRID_WEIGHTS as usize],
            endpoints: [0; MAX_ENDPOINTS as usize],
        }
    }
}

impl LogAstcBlock {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// `m_solid_color[c]` (aliases `m_weights[2c..2c+2]`, little endian).
    pub fn solid_color(&self, c: usize) -> u16 {
        u16::from_le_bytes([self.weights[(c & 3) * 2], self.weights[(c & 3) * 2 + 1]])
    }

    pub fn set_solid_color(&mut self, c: usize, v: u16) {
        let b = v.to_le_bytes();
        self.weights[(c & 3) * 2] = b[0];
        self.weights[(c & 3) * 2 + 1] = b[1];
    }
}

#[inline]
fn ise(range: u32) -> [u8; 3] {
    ISE_RANGE_TABLE
        .get(range as usize)
        .copied()
        .unwrap_or([0, 0, 0])
}

/// Port of `get_ise_levels`.
#[inline]
pub fn get_ise_levels(ise_range: u32) -> u32 {
    let [b, t, q] = ise(ise_range);
    (1 + 2 * t as u32 + 4 * q as u32) << b
}

/// Port of `get_ise_sequence_bits`.
#[inline]
pub fn get_ise_sequence_bits(count: i32, range: u32) -> i32 {
    let [b, t, q] = ise(range);
    let mut total_bits = b as i32 * count;
    total_bits += (t as i32 * 8 * count + 4) / 5;
    total_bits += (q as i32 * 7 * count + 2) / 3;
    total_bits
}

/// Port of `weight_interpolate`.
#[inline]
pub fn weight_interpolate(l: u32, h: u32, w: u32) -> u32 {
    (l * (64 - w) + h * w + 32) >> 6
}

/// Port of `get_num_cem_values`.
#[inline]
pub fn get_num_cem_values(cem: u32) -> u32 {
    2 + 2 * (cem >> 2)
}

fn bit_replication_scale(src: u32, num_src_bits: i32, num_dst_bits: i32) -> u32 {
    let mut dst = 0u32;
    let mut shift = num_dst_bits - num_src_bits;
    while shift > -num_src_bits {
        dst |= if shift >= 0 {
            src << shift
        } else {
            src >> -shift
        };
        shift -= num_src_bits;
    }
    dst
}

/// Port of `dequant_bise_endpoint`.
pub fn dequant_bise_endpoint(val: u32, ise_range: u32) -> u32 {
    match ise_range {
        5 => bit_replication_scale(val, 3, 8),
        8 => bit_replication_scale(val, 4, 8),
        11 => bit_replication_scale(val, 5, 8),
        14 => bit_replication_scale(val, 6, 8),
        17 => bit_replication_scale(val, 7, 8),
        20 => val,
        4 | 6 | 7 | 9 | 10 | 12 | 13 | 15 | 16 | 18 | 19 => {
            let [num_bits, _, num_quints] = ise(ise_range);
            let num_bits = num_bits as u32;
            let range_index = (num_bits * 2 + u32::from(num_quints != 0)) - 2;
            let bits = val & ((1 << num_bits) - 1);
            let tval = val >> num_bits;
            let a = bits & 1;
            let b = (bits >> 1) & 1;
            let c = (bits >> 2) & 1;
            let d = (bits >> 3) & 1;
            let e = (bits >> 4) & 1;
            let f = (bits >> 5) & 1;
            let aa = if a != 0 { 511 } else { 0 };
            let bb = match range_index {
                2 => (b << 1) | (b << 2) | (b << 4) | (b << 8),
                3 => (b << 2) | (b << 3) | (b << 8),
                4 => b | (c << 1) | (b << 2) | (c << 3) | (b << 7) | (c << 8),
                5 => c | (b << 1) | (c << 2) | (b << 7) | (c << 8),
                6 => b | (c << 1) | (d << 2) | (b << 6) | (c << 7) | (d << 8),
                7 => c | (d << 1) | (b << 6) | (c << 7) | (d << 8),
                8 => d | (e << 1) | (b << 5) | (c << 6) | (d << 7) | (e << 8),
                9 => e | (b << 5) | (c << 6) | (d << 7) | (e << 8),
                10 => f | (b << 4) | (c << 5) | (d << 6) | (e << 7) | (f << 8),
                _ => 0,
            };
            const C_VALS: [u32; 11] = [204, 113, 93, 54, 44, 26, 22, 13, 11, 6, 5];
            let cc = C_VALS.get(range_index as usize).copied().unwrap_or(0);
            let mut u = tval * cc + bb;
            u ^= aa;
            (aa & 0x80) | (u >> 2)
        }
        _ => 0,
    }
}

/// Port of `dequant_bise_weight`.
pub fn dequant_bise_weight(val: u32, ise_range: u32) -> u32 {
    let mut u = match ise_range {
        0 => {
            if val != 0 {
                63
            } else {
                0
            }
        }
        1 => [0u32, 32, 63].get(val as usize).copied().unwrap_or(0),
        2 => bit_replication_scale(val, 2, 6),
        3 => [0u32, 16, 32, 47, 63]
            .get(val as usize)
            .copied()
            .unwrap_or(0),
        5 => bit_replication_scale(val, 3, 6),
        8 => bit_replication_scale(val, 4, 6),
        11 => bit_replication_scale(val, 5, 6),
        4 | 6 | 7 | 9 | 10 => {
            let [num_bits, _, num_quints] = ise(ise_range);
            let num_bits = num_bits as u32;
            let range_index = num_bits * 2 + u32::from(num_quints != 0);
            let bits = val & ((1 << num_bits) - 1);
            let d = val >> num_bits;
            const C_TABLE: [u32; 5] = [50, 28, 23, 13, 11];
            let a = bits & 1;
            let b = (bits >> 1) & 1;
            let c = (bits >> 2) & 1;
            let aa = if a == 0 { 0 } else { 0x7F };
            let bb = match range_index {
                4 => (b << 6) | (b << 2) | b,
                5 => (b << 6) | (b << 1),
                6 => (c << 6) | (b << 5) | (c << 1) | b,
                _ => 0,
            };
            let cc = C_TABLE
                .get(range_index.wrapping_sub(2) as usize)
                .copied()
                .unwrap_or(0);
            let mut u = d * cc + bb;
            u ^= aa;
            (aa & 0x20) | (u >> 2)
        }
        _ => 0,
    };
    if u > 32 {
        u += 1;
    }
    u
}

fn find_nearest_bise(v: i32, ise_range: u32, weight_flag: bool) -> u32 {
    let total_levels = get_ise_levels(ise_range);
    let mut best_e = i32::MAX;
    let mut best_index = 0;
    for i in 0..total_levels {
        let qv = if weight_flag {
            dequant_bise_weight(i, ise_range)
        } else {
            dequant_bise_endpoint(i, ise_range)
        } as i32;
        let e = (v - qv).abs();
        if e < best_e {
            best_e = e;
            best_index = i;
            if best_e == 0 {
                break;
            }
        }
    }
    best_index
}

/// Port of `dequant_table`.
#[derive(Debug, Clone, Default)]
pub struct DequantTable {
    /// [0-255] or [0-64] value to nearest ISE symbol, [256] or [65].
    pub val_to_ise: Vec<u8>,
    /// ISE symbol to [0,255] or [0,64] value, [levels].
    pub ise_to_val: Vec<u8>,
    /// Level rank index of an ISE symbol, [levels].
    pub ise_to_rank: Vec<u8>,
    /// ISE symbol of a level rank (inverse of `ise_to_rank`), [levels].
    pub rank_to_ise: Vec<u8>,
}

static EMPTY_TABLE: DequantTable = DequantTable {
    val_to_ise: Vec::new(),
    ise_to_val: Vec::new(),
    ise_to_rank: Vec::new(),
    rank_to_ise: Vec::new(),
};

/// Port of `create_quant_tables`.
fn create_quant_tables(ise_range: u32, weight_flag: bool) -> DequantTable {
    let num_levels = get_ise_levels(ise_range) as usize;
    let num_dequant_vals = if weight_flag {
        MAX_WEIGHT_INTERPOLANT_VALUE + 1
    } else {
        256
    };
    let dequant = |i: u32| {
        if weight_flag {
            dequant_bise_weight(i, ise_range)
        } else {
            dequant_bise_endpoint(i, ise_range)
        }
    };
    let mut tab = DequantTable {
        val_to_ise: vec![0; num_dequant_vals as usize],
        ise_to_val: vec![0; num_levels],
        ise_to_rank: vec![0; num_levels],
        rank_to_ise: vec![0; num_levels],
    };
    for i in 0..num_dequant_vals {
        let bise_index = find_nearest_bise(i as i32, ise_range, weight_flag);
        if let Some(slot) = tab.val_to_ise.get_mut(i as usize) {
            *slot = bise_index as u8;
        }
        if let Some(slot) = tab.ise_to_val.get_mut(bise_index as usize) {
            *slot = dequant(bise_index) as u8;
        }
    }
    let [_, trits, quints] = ise(ise_range);
    if trits == 0 && quints == 0 {
        // Only bits
        for i in 0..num_levels {
            tab.ise_to_rank[i] = i as u8;
            tab.rank_to_ise[i] = i as u8;
        }
    } else {
        // Range has trits or quints: sort by dequantized value.
        let mut vals: Vec<u32> = (0..num_levels as u32)
            .map(|i| (dequant(i) << 16) | i)
            .collect();
        vals.sort_unstable();
        for (rank, v) in vals.iter().enumerate() {
            let ise_val = *v as u8;
            if let Some(slot) = tab.ise_to_rank.get_mut(ise_val as usize) {
                *slot = rank as u8;
            }
            tab.rank_to_ise[rank] = ise_val;
        }
    }
    tab
}

/// Port of `dequant_tables`.
pub struct DequantTables {
    weights: Vec<DequantTable>,
    endpoints: Vec<DequantTable>,
}

impl DequantTables {
    /// `get_weight_tab(range)` (an empty table when out of range).
    pub fn weight_tab(&self, range: u32) -> &DequantTable {
        self.weights
            .get(range.wrapping_sub(FIRST_VALID_WEIGHT_ISE_RANGE) as usize)
            .unwrap_or(&EMPTY_TABLE)
    }

    /// `get_endpoint_tab(range)` (an empty table when out of range).
    pub fn endpoint_tab(&self, range: u32) -> &DequantTable {
        self.endpoints
            .get(range.wrapping_sub(FIRST_VALID_ENDPOINT_ISE_RANGE) as usize)
            .unwrap_or(&EMPTY_TABLE)
    }

    /// `get_weight_tab(range).m_ISE_to_val`.
    pub fn weight_ise_to_val(&self, range: u32) -> &[u8] {
        &self.weight_tab(range).ise_to_val
    }

    /// `get_endpoint_tab(range).m_ISE_to_val`.
    pub fn endpoint_ise_to_val(&self, range: u32) -> &[u8] {
        &self.endpoint_tab(range).ise_to_val
    }
}

/// `g_dequant_tables` (initialized once, like `init_tables`).
pub fn dequant_tables() -> &'static DequantTables {
    static T: OnceLock<DequantTables> = OnceLock::new();
    T.get_or_init(|| DequantTables {
        weights: (FIRST_VALID_WEIGHT_ISE_RANGE..=LAST_VALID_WEIGHT_ISE_RANGE)
            .map(|r| create_quant_tables(r, true))
            .collect(),
        endpoints: (FIRST_VALID_ENDPOINT_ISE_RANGE..=LAST_VALID_ENDPOINT_ISE_RANGE)
            .map(|r| create_quant_tables(r, false))
            .collect(),
    })
}

/// Port of `is_cem_ldr`.
pub fn is_cem_ldr(mode: u32) -> bool {
    matches!(
        mode,
        CEM_LDR_LUM_DIRECT
            | CEM_LDR_LUM_BASE_PLUS_OFS
            | CEM_LDR_LUM_ALPHA_DIRECT
            | CEM_LDR_LUM_ALPHA_BASE_PLUS_OFS
            | CEM_LDR_RGB_BASE_SCALE
            | CEM_LDR_RGB_DIRECT
            | CEM_LDR_RGB_BASE_PLUS_OFFSET
            | CEM_LDR_RGB_BASE_SCALE_PLUS_TWO_A
            | CEM_LDR_RGBA_DIRECT
            | CEM_LDR_RGBA_BASE_PLUS_OFFSET
    )
}

/// Port of `weighted_sample`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeightedSample {
    pub src_x: u8,
    pub src_y: u8,
    /// [y][x], scaled by 16, round by adding 8.
    pub weights: [[u8; 2]; 2],
}

/// Port of `compute_upsample_weights` (`block_width * block_height` samples).
pub fn compute_upsample_weights(
    block_width: u32,
    block_height: u32,
    weight_grid_width: u32,
    weight_grid_height: u32,
    out: &mut [WeightedSample],
) {
    if block_width < 2 || block_height < 2 {
        return;
    }
    let scale_x = (1024 + block_width / 2) / (block_width - 1);
    let scale_y = (1024 + block_height / 2) / (block_height - 1);
    for texel_y in 0..block_height {
        for texel_x in 0..block_width {
            let g_x = (scale_x * texel_x * weight_grid_width.wrapping_sub(1)).wrapping_add(32) >> 6;
            let g_y =
                (scale_y * texel_y * weight_grid_height.wrapping_sub(1)).wrapping_add(32) >> 6;
            let (f_x, f_y) = (g_x & 0xf, g_y & 0xf);
            let w11 = (f_x * f_y + 8) >> 4;
            if let Some(s) = out.get_mut((texel_x + texel_y * block_width) as usize) {
                s.src_x = (g_x >> 4) as u8;
                s.src_y = (g_y >> 4) as u8;
                s.weights = [
                    // C's u32 math: 16 - fX - fY may wrap before + w11.
                    [
                        16u32.wrapping_sub(f_x + f_y).wrapping_add(w11) as u8,
                        (f_x - w11) as u8,
                    ],
                    [(f_y - w11) as u8, w11 as u8],
                ];
            }
        }
    }
}

/// Port of `upsample_weight_grid`.
pub fn upsample_weight_grid(
    bx: u32,
    by: u32,
    wx: u32,
    wy: u32,
    src_weights: &[u8],
    dst_weights: &mut [u8],
) {
    let total_src_weights = (wx * wy) as usize;
    let total_dst_weights = (bx * by) as usize;
    if total_src_weights == total_dst_weights {
        for i in 0..total_src_weights {
            if let (Some(d), Some(&s)) = (dst_weights.get_mut(i), src_weights.get(i)) {
                *d = s;
            }
        }
        return;
    }
    let mut weights = [WeightedSample::default(); MAX_BLOCK_PIXELS];
    compute_upsample_weights(bx, by, wx, wy, &mut weights);
    let src = |i: u32| u32::from(src_weights.get(i as usize).copied().unwrap_or(0));
    for (i, ps) in weights.iter().take(total_dst_weights).enumerate() {
        let [[w00, w01], [w10, w11]] = ps.weights.map(|r| r.map(u32::from));
        let (sx, sy) = (u32::from(ps.src_x), u32::from(ps.src_y));
        let mut total = 8;
        if w00 != 0 {
            total += src(sx + sy * wx) * w00;
        }
        if w01 != 0 {
            total += src(sx + 1 + sy * wx) * w01;
        }
        if w10 != 0 {
            total += src(sx + (sy + 1) * wx) * w10;
        }
        if w11 != 0 {
            total += src(sx + 1 + (sy + 1) * wx) * w11;
        }
        if let Some(d) = dst_weights.get_mut(i) {
            *d = (total >> 4) as u8;
        }
    }
}

#[inline]
fn hash52(v: u32) -> u32 {
    let mut p = v;
    p ^= p >> 15;
    p = p.wrapping_sub(p << 17);
    p = p.wrapping_add(p << 7);
    p = p.wrapping_add(p << 4);
    p ^= p >> 5;
    p = p.wrapping_add(p << 16);
    p ^= p >> 7;
    p ^= p >> 3;
    p ^= p << 6;
    p ^= p >> 17;
    p
}

/// Port of `compute_texel_partition` (the precomputed 2/3-subset table in C
/// returns the same values).
pub fn compute_texel_partition(
    seed_in: u32,
    x_in: u32,
    y_in: u32,
    z_in: u32,
    num_partitions: i32,
    small_block: bool,
) -> u32 {
    let (x, y, z) = if small_block {
        (x_in << 1, y_in << 1, z_in << 1)
    } else {
        (x_in, y_in, z_in)
    };
    let seed = seed_in.wrapping_add(1024u32.wrapping_mul(num_partitions.wrapping_sub(1) as u32));
    let rnum = hash52(seed);
    let nib = |s: u32| ((rnum >> s) & 0xf) as u8;
    let mut s = [
        nib(0),
        nib(4),
        nib(8),
        nib(12),
        nib(16),
        nib(20),
        nib(24),
        nib(28),
        nib(18),
        nib(22),
        nib(26),
        (rnum.rotate_left(2) & 0xf) as u8,
    ];
    for v in s.iter_mut() {
        *v = v.wrapping_mul(*v);
    }
    let sh_a = if seed & 2 != 0 { 4 } else { 5 };
    let sh_b = if num_partitions == 3 { 6 } else { 5 };
    let sh1 = if seed & 1 != 0 { sh_a } else { sh_b };
    let sh2 = if seed & 1 != 0 { sh_b } else { sh_a };
    let sh3 = if seed & 0x10 != 0 { sh1 } else { sh2 };
    let shifts = [sh1, sh2, sh1, sh2, sh1, sh2, sh1, sh2, sh3, sh3, sh3, sh3];
    for (v, sh) in s.iter_mut().zip(shifts) {
        *v >>= sh;
    }
    let sd = |i: usize| u32::from(s[i]);
    let a = 0x3f
        & (sd(0)
            .wrapping_mul(x)
            .wrapping_add(sd(1).wrapping_mul(y))
            .wrapping_add(sd(10).wrapping_mul(z))
            .wrapping_add(rnum >> 14));
    let b = 0x3f
        & (sd(2)
            .wrapping_mul(x)
            .wrapping_add(sd(3).wrapping_mul(y))
            .wrapping_add(sd(11).wrapping_mul(z))
            .wrapping_add(rnum >> 10));
    let c = if num_partitions >= 3 {
        0x3f & (sd(4)
            .wrapping_mul(x)
            .wrapping_add(sd(5).wrapping_mul(y))
            .wrapping_add(sd(8).wrapping_mul(z))
            .wrapping_add(rnum >> 6))
    } else {
        0
    };
    let d = if num_partitions >= 4 {
        0x3f & (sd(6)
            .wrapping_mul(x)
            .wrapping_add(sd(7).wrapping_mul(y))
            .wrapping_add(sd(9).wrapping_mul(z))
            .wrapping_add(rnum >> 2))
    } else {
        0
    };
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

#[inline]
fn blue_contract(r: i32, g: i32, b: i32, a: i32) -> [i32; 4] {
    [(r + b) >> 1, (g + b) >> 1, b, a]
}

/// Port of `bit_transfer_signed`.
#[inline]
pub fn bit_transfer_signed(a: &mut i32, b: &mut i32) {
    *b >>= 1;
    *b |= *a & 0x80;
    *a >>= 1;
    *a &= 0x3F;
    if (*a & 0x20) != 0 {
        *a -= 0x40;
    }
}

#[inline]
fn sign_extend(src: i32, num_src_bits: i32) -> i32 {
    let negative = (src & (1 << (num_src_bits - 1))) != 0;
    if negative {
        src | !((1 << num_src_bits) - 1)
    } else {
        src & ((1 << num_src_bits) - 1)
    }
}

/// Port of `decode_endpoint`: `endpoints[comp][lh]`.
pub fn decode_endpoint(cem_index: u32, endpoints: &mut [[i32; 2]; 4], pe: &[u8]) {
    let v = |i: usize| i32::from(pe.get(i).copied().unwrap_or(0));
    let (v0, v1) = (v(0), v(1));
    let set = |ep: &mut [[i32; 2]; 4], e0: [i32; 4], e1: [i32; 4]| {
        for c in 0..4 {
            ep[c] = [e0[c], e1[c]];
        }
    };
    let clamp_all = |ep: &mut [[i32; 2]; 4]| {
        for c in ep.iter_mut() {
            c[0] = c[0].clamp(0, 255);
            c[1] = c[1].clamp(0, 255);
        }
    };
    match cem_index {
        CEM_LDR_LUM_DIRECT => set(endpoints, [v0, v0, v0, 0xFF], [v1, v1, v1, 0xFF]),
        CEM_LDR_LUM_BASE_PLUS_OFS => {
            let l0 = (v0 >> 2) | (v1 & 0xc0);
            let l1 = (l0 + (v1 & 0x3f)).min(0xFF);
            set(endpoints, [l0, l0, l0, 0xFF], [l1, l1, l1, 0xFF]);
        }
        CEM_LDR_LUM_ALPHA_DIRECT => {
            let (v2, v3) = (v(2), v(3));
            set(endpoints, [v0, v0, v0, v2], [v1, v1, v1, v3]);
        }
        CEM_LDR_LUM_ALPHA_BASE_PLUS_OFS => {
            let (mut v0, mut v1, mut v2, mut v3) = (v0, v1, v(2), v(3));
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            let l1 = v0 + v1;
            set(endpoints, [v0, v0, v0, v2], [l1, l1, l1, v2 + v3]);
            clamp_all(endpoints);
        }
        CEM_LDR_RGB_BASE_SCALE => {
            let (v2, v3) = (v(2), v(3));
            set(
                endpoints,
                [(v0 * v3) >> 8, (v1 * v3) >> 8, (v2 * v3) >> 8, 0xFF],
                [v0, v1, v2, 0xFF],
            );
        }
        CEM_LDR_RGB_DIRECT => {
            let (v2, v3, v4, v5) = (v(2), v(3), v(4), v(5));
            if (v1 + v3 + v5) >= (v0 + v2 + v4) {
                set(endpoints, [v0, v2, v4, 0xFF], [v1, v3, v5, 0xFF]);
            } else {
                set(
                    endpoints,
                    blue_contract(v1, v3, v5, 0xFF),
                    blue_contract(v0, v2, v4, 0xFF),
                );
            }
        }
        CEM_LDR_RGB_BASE_PLUS_OFFSET => {
            let (mut v0, mut v1, mut v2, mut v3, mut v4, mut v5) = (v0, v1, v(2), v(3), v(4), v(5));
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            bit_transfer_signed(&mut v5, &mut v4);
            if (v1 + v3 + v5) >= 0 {
                set(
                    endpoints,
                    [v0, v2, v4, 0xFF],
                    [v0 + v1, v2 + v3, v4 + v5, 0xFF],
                );
            } else {
                set(
                    endpoints,
                    blue_contract(v0 + v1, v2 + v3, v4 + v5, 0xFF),
                    blue_contract(v0, v2, v4, 0xFF),
                );
            }
            clamp_all(endpoints);
        }
        CEM_LDR_RGB_BASE_SCALE_PLUS_TWO_A => {
            let (v2, v3, v4, v5) = (v(2), v(3), v(4), v(5));
            set(
                endpoints,
                [(v0 * v3) >> 8, (v1 * v3) >> 8, (v2 * v3) >> 8, v4],
                [v0, v1, v2, v5],
            );
        }
        CEM_LDR_RGBA_DIRECT => {
            let (v2, v3, v4, v5, v6, v7) = (v(2), v(3), v(4), v(5), v(6), v(7));
            if (v1 + v3 + v5) >= (v0 + v2 + v4) {
                set(endpoints, [v0, v2, v4, v6], [v1, v3, v5, v7]);
            } else {
                set(
                    endpoints,
                    blue_contract(v1, v3, v5, v7),
                    blue_contract(v0, v2, v4, v6),
                );
            }
        }
        CEM_LDR_RGBA_BASE_PLUS_OFFSET => {
            let (mut v0, mut v1, mut v2, mut v3) = (v0, v1, v(2), v(3));
            let (mut v4, mut v5, mut v6, mut v7) = (v(4), v(5), v(6), v(7));
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            bit_transfer_signed(&mut v5, &mut v4);
            bit_transfer_signed(&mut v7, &mut v6);
            if (v1 + v3 + v5) >= 0 {
                set(
                    endpoints,
                    [v0, v2, v4, v6],
                    [v0 + v1, v2 + v3, v4 + v5, v6 + v7],
                );
            } else {
                set(
                    endpoints,
                    blue_contract(v0 + v1, v2 + v3, v4 + v5, v6 + v7),
                    blue_contract(v0, v2, v4, v6),
                );
            }
            clamp_all(endpoints);
        }
        CEM_HDR_LUM_LARGE_RANGE => {
            let (y0, y1) = if v1 >= v0 {
                (v0 << 4, v1 << 4)
            } else {
                ((v1 << 4) + 8, (v0 << 4) - 8)
            };
            set(endpoints, [y0, y0, y0, 0x780], [y1, y1, y1, 0x780]);
        }
        CEM_HDR_LUM_SMALL_RANGE => {
            let (y0, d) = if (v0 & 0x80) != 0 {
                (((v1 & 0xE0) << 4) | ((v0 & 0x7F) << 2), (v1 & 0x1F) << 2)
            } else {
                (((v1 & 0xF0) << 4) | ((v0 & 0x7F) << 1), (v1 & 0x0F) << 1)
            };
            let y1 = (y0 + d).min(0xFFF);
            set(endpoints, [y0, y0, y0, 0x780], [y1, y1, y1, 0x780]);
        }
        CEM_HDR_RGB_BASE_SCALE => {
            let (e0, e1, _) = decode_mode7_ise20(v0, v1, v(2), v(3));
            set(
                endpoints,
                [e0[0], e0[1], e0[2], 0x780],
                [e1[0], e1[1], e1[2], 0x780],
            );
        }
        CEM_HDR_RGB_HDR_ALPHA | CEM_HDR_RGB_LDR_ALPHA | CEM_HDR_RGB => {
            let (e0, e1) = decode_mode11_ise20(v0, v1, v(2), v(3), v(4), v(5));
            let (mut a0, mut a1) = (0x780, 0x780);
            if cem_index == CEM_HDR_RGB_LDR_ALPHA {
                a0 = v(6);
                a1 = v(7);
            } else if cem_index == CEM_HDR_RGB_HDR_ALPHA {
                let (mut v6, mut v7) = (v(6), v(7));
                let mode = ((v6 >> 7) & 1) | ((v7 >> 6) & 2);
                v6 &= 0x7F;
                v7 &= 0x7F;
                if mode == 3 {
                    a0 = v6 << 5;
                    a1 = v7 << 5;
                } else {
                    v6 |= (v7 << (mode + 1)) & 0x780;
                    v7 &= 0x3F >> mode;
                    v7 ^= 0x20 >> mode;
                    v7 -= 0x20 >> mode;
                    v6 = ((v6 as u32) << (4 - mode)) as i32;
                    v7 = ((v7 as u32) << (4 - mode)) as i32;
                    v7 = v7.wrapping_add(v6).clamp(0, 0xFFF);
                    a0 = v6;
                    a1 = v7;
                }
            }
            set(
                endpoints,
                [e0[0], e0[1], e0[2], a0],
                [e1[0], e1[1], e1[2], a1],
            );
        }
        _ => {
            for c in endpoints.iter_mut() {
                *c = [0, 0];
            }
        }
    }
}

/// CEM 7 decode to qlog12 (`decode_mode7_to_qlog12_ise20` / `decode_endpoint`):
/// returns (low, high, scale).
pub fn decode_mode7_ise20(v0: i32, v1: i32, v2: i32, v3: i32) -> ([i32; 3], [i32; 3], i32) {
    let modeval = ((v0 & 0xC0) >> 6) | ((v1 & 0x80) >> 5) | ((v2 & 0x80) >> 4);
    let (majcomp, mode) = if (modeval & 0xC) != 0xC {
        (modeval >> 2, modeval & 3)
    } else if modeval != 0xF {
        (modeval & 3, 4)
    } else {
        (0, 5)
    };
    let mut red = v0 & 0x3f;
    let mut green = v1 & 0x1f;
    let mut blue = v2 & 0x1f;
    let mut scale = v3 & 0x1f;
    let x0 = (v1 >> 6) & 1;
    let x1 = (v1 >> 5) & 1;
    let x2 = (v2 >> 6) & 1;
    let x3 = (v2 >> 5) & 1;
    let x4 = (v3 >> 7) & 1;
    let x5 = (v3 >> 6) & 1;
    let x6 = (v3 >> 5) & 1;
    let ohm = 1 << mode;
    if ohm & 0x30 != 0 {
        green |= x0 << 6;
    }
    if ohm & 0x3A != 0 {
        green |= x1 << 5;
    }
    if ohm & 0x30 != 0 {
        blue |= x2 << 6;
    }
    if ohm & 0x3A != 0 {
        blue |= x3 << 5;
    }
    if ohm & 0x3D != 0 {
        scale |= x6 << 5;
    }
    if ohm & 0x2D != 0 {
        scale |= x5 << 6;
    }
    if ohm & 0x04 != 0 {
        scale |= x4 << 7;
    }
    if ohm & 0x3B != 0 {
        red |= x4 << 6;
    }
    if ohm & 0x04 != 0 {
        red |= x3 << 6;
    }
    if ohm & 0x10 != 0 {
        red |= x5 << 7;
    }
    if ohm & 0x0F != 0 {
        red |= x2 << 7;
    }
    if ohm & 0x05 != 0 {
        red |= x1 << 8;
    }
    if ohm & 0x0A != 0 {
        red |= x0 << 8;
    }
    if ohm & 0x05 != 0 {
        red |= x0 << 9;
    }
    if ohm & 0x02 != 0 {
        red |= x6 << 9;
    }
    if ohm & 0x01 != 0 {
        red |= x3 << 10;
    }
    if ohm & 0x02 != 0 {
        red |= x5 << 10;
    }
    const SHAMTS: [i32; 6] = [1, 1, 2, 3, 4, 5];
    let shamt = SHAMTS[(mode as usize).min(5)];
    red <<= shamt;
    green <<= shamt;
    blue <<= shamt;
    scale <<= shamt;
    if mode != 5 {
        green = red - green;
        blue = red - blue;
    }
    if majcomp == 1 {
        std::mem::swap(&mut red, &mut green);
    }
    if majcomp == 2 {
        std::mem::swap(&mut red, &mut blue);
    }
    (
        [
            (red - scale).clamp(0, 0xFFF),
            (green - scale).clamp(0, 0xFFF),
            (blue - scale).clamp(0, 0xFFF),
        ],
        [
            red.clamp(0, 0xFFF),
            green.clamp(0, 0xFFF),
            blue.clamp(0, 0xFFF),
        ],
        scale,
    )
}

/// CEM 11 decode to qlog12 (`decode_mode11_to_qlog12_ise20` / `decode_endpoint`):
/// returns (low, high).
pub fn decode_mode11_ise20(
    v0: i32,
    v1: i32,
    v2: i32,
    v3: i32,
    v4: i32,
    v5: i32,
) -> ([i32; 3], [i32; 3]) {
    let majcomp = ((v4 & 0x80) >> 7) | ((v5 & 0x80) >> 6);
    if majcomp == 3 {
        return (
            [v0 << 4, v2 << 4, (v4 & 0x7f) << 5],
            [v1 << 4, v3 << 4, (v5 & 0x7f) << 5],
        );
    }
    let mode = ((v1 & 0x80) >> 7) | ((v2 & 0x80) >> 6) | ((v3 & 0x80) >> 5);
    let mut va = v0 | ((v1 & 0x40) << 2);
    let mut vb0 = v2 & 0x3f;
    let mut vb1 = v3 & 0x3f;
    let mut vc = v1 & 0x3f;
    const DBITSTAB: [i32; 8] = [7, 6, 7, 6, 5, 6, 5, 6];
    let dbits = DBITSTAB[(mode as usize) & 7];
    let mut vd0 = sign_extend(v4 & 0x7f, dbits);
    let mut vd1 = sign_extend(v5 & 0x7f, dbits);
    let x0 = (v2 >> 6) & 1;
    let x1 = (v3 >> 6) & 1;
    let x2 = (v4 >> 6) & 1;
    let x3 = (v5 >> 6) & 1;
    let x4 = (v4 >> 5) & 1;
    let x5 = (v5 >> 5) & 1;
    let ohm = 1 << mode;
    if ohm & 0xA4 != 0 {
        va |= x0 << 9;
    }
    if ohm & 0x08 != 0 {
        va |= x2 << 9;
    }
    if ohm & 0x50 != 0 {
        va |= x4 << 9;
    }
    if ohm & 0x50 != 0 {
        va |= x5 << 10;
    }
    if ohm & 0xA0 != 0 {
        va |= x1 << 10;
    }
    if ohm & 0xC0 != 0 {
        va |= x2 << 11;
    }
    if ohm & 0x04 != 0 {
        vc |= x1 << 6;
    }
    if ohm & 0xE8 != 0 {
        vc |= x3 << 6;
    }
    if ohm & 0x20 != 0 {
        vc |= x2 << 7;
    }
    if ohm & 0x5B != 0 {
        vb0 |= x0 << 6;
    }
    if ohm & 0x5B != 0 {
        vb1 |= x1 << 6;
    }
    if ohm & 0x12 != 0 {
        vb0 |= x2 << 7;
    }
    if ohm & 0x12 != 0 {
        vb1 |= x3 << 7;
    }
    let shamt = (mode >> 1) ^ 3;
    let shl = |v: i32| ((v as u32) << shamt) as i32;
    va = shl(va);
    vb0 = shl(vb0);
    vb1 = shl(vb1);
    vc = shl(vc);
    vd0 = shl(vd0);
    vd1 = shl(vd1);
    let mut e1 = [
        va.clamp(0, 0xFFF),
        (va - vb0).clamp(0, 0xFFF),
        (va - vb1).clamp(0, 0xFFF),
    ];
    let mut e0 = [
        (va - vc).clamp(0, 0xFFF),
        (va - vb0 - vc - vd0).clamp(0, 0xFFF),
        (va - vb1 - vc - vd1).clamp(0, 0xFFF),
    ];
    if majcomp != 0 {
        let m = majcomp as usize;
        e0.swap(0, m);
        e1.swap(0, m);
    }
    (e0, e1)
}

/// Port of `is_half_inf_or_nan`.
#[inline]
pub fn is_half_inf_or_nan(v: HalfFloat) -> bool {
    ((v >> 10) & 31) == 31
}

/// Port of `qlog16_to_half`.
#[inline]
pub fn qlog16_to_half(k: i32) -> HalfFloat {
    let e = (k & 0xF800) >> 11;
    let m = k & 0x7FF;
    let mt = if m < 512 {
        3 * m
    } else if m >= 1536 {
        5 * m - 2048
    } else {
        4 * m - 512
    };
    ((e << 10) + (mt >> 3)) as HalfFloat
}

/// Port of `astc_helpers::float_to_half` (matches Intel's F32TO16).
pub fn float_to_half(val: f32, toward_zero: bool) -> HalfFloat {
    let bits = val.to_bits() as i32;
    let flt_m = bits & 0x7FFFFF;
    let flt_e = (bits >> 23) & 0xFF;
    let s = (bits >> 31) & 1;
    let mut e = 0;
    let mut m = 0;
    if flt_e == 0xff {
        e = 31;
        if flt_m != 0 {
            m = 1;
        }
    } else if flt_e != 0 {
        let new_exp = flt_e - 127;
        if new_exp > 15 {
            e = 31;
        } else if new_exp < -14 {
            let x = 16777216.0f32 * val.abs();
            m = if toward_zero {
                x.trunc() as i32
            } else {
                x.round_ties_even() as i32
            };
        } else {
            e = new_exp + 15;
            let x = flt_m as f32 * (1.0f32 / 8192.0f32);
            m = if toward_zero {
                x.trunc() as i32
            } else {
                x.round_ties_even() as i32
            };
        }
    }
    if m == 1024 {
        e += 1;
        m = 0;
    }
    ((s << 15) | (e << 10) | m) as HalfFloat
}

/// Port of `astc_helpers::half_to_float`.
pub fn half_to_float(hval: HalfFloat) -> f32 {
    let s = (u32::from(hval) >> 15) & 1;
    let mut e = (u32::from(hval) >> 10) & 0x1F;
    let mut m = u32::from(hval) & 0x3FF;
    if e == 0 {
        if m == 0 {
            return f32::from_bits(s << 31);
        }
        while m & 0x400 == 0 {
            m <<= 1;
            e = e.wrapping_sub(1);
        }
        e = e.wrapping_add(1);
        m &= !0x400;
    } else if e == 31 {
        return f32::from_bits((s << 31) | 0x7f800000 | (m << 13));
    }
    e = e.wrapping_add(127 - 15);
    m <<= 13;
    f32::from_bits(m | (e << 23) | (s << 31))
}

const RGB9E5_EXP_BIAS: i32 = 15;
const RGB9E5_MANTISSA_BITS: i32 = 9;
const MAX_RGB9E5_MANTISSA: i32 = (1 << RGB9E5_MANTISSA_BITS) - 1;
pub const MAX_RGB9E5: f32 = 65408.0; // 0xff80

/// Exact `powf(2.0f, k)` for the small integer exponents used here.
#[inline]
fn exp2i(k: i32) -> f32 {
    f32::from_bits(((k + 127) as u32) << 23)
}

#[inline]
fn clampf(a: f32, l: f32, h: f32) -> f32 {
    if a < l {
        l
    } else if a > h {
        h
    } else {
        a
    }
}

#[inline]
fn floor_log2(x: f32) -> i32 {
    ((x.to_bits() >> 23) & 0xFF) as i32 - 127
}

#[inline]
fn maximumf(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// Port of `pack_rgb9e5`.
pub fn pack_rgb9e5(r: f32, g: f32, b: f32) -> u32 {
    let r = clampf(r, 0.0, MAX_RGB9E5);
    let g = clampf(g, 0.0, MAX_RGB9E5);
    let b = clampf(b, 0.0, MAX_RGB9E5);
    let maxrgb = maximumf(maximumf(r, g), b);
    let mut exp_shared = (-RGB9E5_EXP_BIAS - 1).max(floor_log2(maxrgb)) + 1 + RGB9E5_EXP_BIAS;
    let mut denom = exp2i(exp_shared - RGB9E5_EXP_BIAS - RGB9E5_MANTISSA_BITS);
    let maxm = ((maxrgb / denom) + 0.5).floor() as i32;
    if maxm == MAX_RGB9E5_MANTISSA + 1 {
        denom *= 2.0;
        exp_shared += 1;
    }
    let rm = ((r / denom) + 0.5).floor() as i32;
    let gm = ((g / denom) + 0.5).floor() as i32;
    let bm = ((b / denom) + 0.5).floor() as i32;
    (rm as u32) | ((gm as u32) << 9) | ((bm as u32) << 18) | ((exp_shared as u32) << 27)
}

#[inline]
fn clz17(x: u32) -> i32 {
    let mut x = x & 0x1FFFF;
    if x == 0 {
        return 17;
    }
    let mut n = 0;
    while (x & 0x10000) == 0 {
        x <<= 1;
        n += 1;
    }
    n
}

fn pack_rgb9e5_ldr_astc(mut cr: i32, mut cg: i32, mut cb: i32) -> u32 {
    let mut lz = clz17((cr | cg | cb | 1) as u32);
    if cr == 65535 {
        cr = 65536;
        lz = 0;
    }
    if cg == 65535 {
        cg = 65536;
        lz = 0;
    }
    if cb == 65535 {
        cb = 65536;
        lz = 0;
    }
    cr <<= lz;
    cg <<= lz;
    cb <<= lz;
    cr = (cr >> 8) & 0x1FF;
    cg = (cg >> 8) & 0x1FF;
    cb = (cb >> 8) & 0x1FF;
    let exponent = (16 - lz) as u32;
    (exponent << 27) | ((cb as u32) << 18) | ((cg as u32) << 9) | cr as u32
}

fn pack_rgb9e5_hdr_astc(mut cr: i32, mut cg: i32, mut cb: i32) -> u32 {
    for c in [&mut cr, &mut cg, &mut cb] {
        if *c > 0x7c00 {
            *c = 0;
        } else if *c == 0x7c00 {
            *c = 0x7bff;
        }
    }
    let re = (cr >> 10) & 0x1F;
    let ge = (cg >> 10) & 0x1F;
    let be = (cb >> 10) & 0x1F;
    let rex = if re == 0 { 1 } else { re };
    let gex = if ge == 0 { 1 } else { ge };
    let bex = if be == 0 { 1 } else { be };
    let xm = ((cr | cg | cb) & 0x200) >> 9;
    let xe = re | ge | be;
    let (rshift, gshift, bshift, expo);
    if xe == 0 {
        expo = xm;
        rshift = xm;
        gshift = xm;
        bshift = xm;
    } else if re >= ge && re >= be {
        expo = rex + 1;
        rshift = 2;
        gshift = rex - gex + 2;
        bshift = rex - bex + 2;
    } else if ge >= be {
        expo = gex + 1;
        rshift = gex - rex + 2;
        gshift = 2;
        bshift = gex - bex + 2;
    } else {
        expo = bex + 1;
        rshift = bex - rex + 2;
        gshift = bex - gex + 2;
        bshift = 2;
    }
    let shr = |v: i32, s: i32| (v as u32).checked_shr(s as u32).unwrap_or(0) as i32;
    let rm = shr((cr & 0x3FF) | if re == 0 { 0 } else { 0x400 }, rshift) & 0x1FF;
    let gm = shr((cg & 0x3FF) | if ge == 0 { 0 } else { 0x400 }, gshift) & 0x1FF;
    let bm = shr((cb & 0x3FF) | if be == 0 { 0 } else { 0x400 }, bshift) & 0x1FF;
    ((expo as u32) << 27) | ((bm as u32) << 18) | ((gm as u32) << 9) | rm as u32
}

/// Port of `decode_mode`, carrying the output pixel buffer
/// (`blk_width * blk_height` texels).
pub enum DecodePixels<'a> {
    Srgb8(&'a mut [[u8; 4]]),
    Ldr8(&'a mut [[u8; 4]]),
    Hdr16(&'a mut [[HalfFloat; 4]]),
    Rgb9e5(&'a mut [u32]),
}

fn write_error_block(pixels: &mut DecodePixels, num_blk_pixels: usize) {
    match pixels {
        DecodePixels::Hdr16(p) => p
            .iter_mut()
            .take(num_blk_pixels)
            .for_each(|x| *x = [0xFFFF; 4]),
        DecodePixels::Rgb9e5(p) => {
            let purple = pack_rgb9e5(1.0, 0.0, 1.0);
            p.iter_mut().take(num_blk_pixels).for_each(|x| *x = purple);
        }
        DecodePixels::Srgb8(p) | DecodePixels::Ldr8(p) => p
            .iter_mut()
            .take(num_blk_pixels)
            .for_each(|x| *x = 0xFFFF00FFu32.to_le_bytes()),
    }
}

/// Port of `decode_block`.
pub fn decode_block(
    log_blk: &LogAstcBlock,
    pixels: &mut DecodePixels,
    blk_width: u32,
    blk_height: u32,
) -> bool {
    let num_blk_pixels = (blk_width * blk_height) as usize;
    let fail = |pixels: &mut DecodePixels| {
        write_error_block(pixels, num_blk_pixels);
        false
    };

    if log_blk.error_flag {
        return fail(pixels);
    }

    if log_blk.solid_color_flag_ldr {
        let sc = |c: usize| log_blk.solid_color(c);
        match pixels {
            DecodePixels::Hdr16(p) => {
                let mut h = [0u16; 4];
                for (c, hc) in h.iter_mut().enumerate() {
                    *hc = if sc(c) == 0xFFFF {
                        0x3C00
                    } else {
                        float_to_half(f32::from(sc(c)) * (1.0 / 65536.0), true)
                    };
                }
                p.iter_mut().take(num_blk_pixels).for_each(|x| *x = h);
            }
            DecodePixels::Rgb9e5(p) => {
                let f = |c: usize| {
                    if sc(c) == 0xFFFF {
                        1.0
                    } else {
                        f32::from(sc(c)) * (1.0 / 65536.0)
                    }
                };
                let packed = pack_rgb9e5(f(0), f(1), f(2));
                p.iter_mut().take(num_blk_pixels).for_each(|x| *x = packed);
            }
            DecodePixels::Srgb8(p) | DecodePixels::Ldr8(p) => {
                let v = [
                    (sc(0) >> 8) as u8,
                    (sc(1) >> 8) as u8,
                    (sc(2) >> 8) as u8,
                    (sc(3) >> 8) as u8,
                ];
                p.iter_mut().take(num_blk_pixels).for_each(|x| *x = v);
            }
        }
        return true;
    } else if log_blk.solid_color_flag_hdr {
        let sc = |c: usize| log_blk.solid_color(c);
        match pixels {
            DecodePixels::Hdr16(p) => {
                let v = [sc(0), sc(1), sc(2), sc(3)];
                p.iter_mut().take(num_blk_pixels).for_each(|x| *x = v);
            }
            DecodePixels::Rgb9e5(p) => {
                let packed = pack_rgb9e5(
                    half_to_float(sc(0)),
                    half_to_float(sc(1)),
                    half_to_float(sc(2)),
                );
                p.iter_mut().take(num_blk_pixels).for_each(|x| *x = packed);
            }
            _ => return fail(pixels),
        }
        return true;
    }

    let gw = u32::from(log_blk.grid_width);
    let gh = u32::from(log_blk.grid_height);
    let ep_range = u32::from(log_blk.endpoint_ise_range);
    let w_range = u32::from(log_blk.weight_ise_range);
    let num_parts = u32::from(log_blk.num_partitions);
    if gw < 2 || gh < 2 || gw > blk_width || gh > blk_height {
        return fail(pixels);
    }
    if !(FIRST_VALID_ENDPOINT_ISE_RANGE..=LAST_VALID_ENDPOINT_ISE_RANGE).contains(&ep_range)
        || !(FIRST_VALID_WEIGHT_ISE_RANGE..=LAST_VALID_WEIGHT_ISE_RANGE).contains(&w_range)
        || !(1..=MAX_PARTITIONS).contains(&num_parts)
        || (log_blk.dual_plane && num_parts > MAX_DUAL_PLANE_PARTITIONS)
        || u32::from(log_blk.partition_id) >= NUM_PARTITION_PATTERNS
        || (num_parts == 1 && log_blk.partition_id > 0)
        || log_blk.color_component_selector > 3
    {
        return fail(pixels);
    }

    let total_endpoint_levels = get_ise_levels(ep_range);
    let total_weight_levels = get_ise_levels(w_range);

    let mut is_ldr_endpoints = [false; MAX_PARTITIONS as usize];
    let mut total_cem_vals = 0;
    for i in 0..num_parts as usize {
        let cem = u32::from(log_blk.color_endpoint_modes[i]);
        if cem > 15 {
            return fail(pixels);
        }
        total_cem_vals += get_num_cem_values(cem);
        is_ldr_endpoints[i] = is_cem_ldr(cem);
    }
    if total_cem_vals > MAX_ENDPOINTS {
        return fail(pixels);
    }

    let tabs = dequant_tables();
    let endpoint_dequant = tabs.endpoint_ise_to_val(ep_range);
    let mut dequantized_endpoints = [0u8; MAX_ENDPOINTS as usize];
    for i in 0..total_cem_vals as usize {
        let e = u32::from(log_blk.endpoints[i]);
        if e >= total_endpoint_levels {
            return fail(pixels);
        }
        dequantized_endpoints[i] = endpoint_dequant.get(e as usize).copied().unwrap_or(0);
    }

    let weight_dequant = tabs.weight_ise_to_val(w_range);
    let mut dequantized_weights = [[0u8; MAX_BLOCK_PIXELS]; 2];
    let total_weight_vals = (if log_blk.dual_plane { 2 } else { 1 }) * gw * gh;
    for i in 0..total_weight_vals as usize {
        let Some(&w) = log_blk.weights.get(i) else {
            return fail(pixels);
        };
        if u32::from(w) >= total_weight_levels {
            return fail(pixels);
        }
        let (plane, grid) = if log_blk.dual_plane {
            (i & 1, i >> 1)
        } else {
            (0, i)
        };
        dequantized_weights[plane][grid] = weight_dequant.get(w as usize).copied().unwrap_or(0);
    }

    let mut upsampled_weights = [[0u8; MAX_BLOCK_PIXELS]; 2];
    upsample_weight_grid(
        blk_width,
        blk_height,
        gw,
        gh,
        &dequantized_weights[0],
        &mut upsampled_weights[0],
    );
    if log_blk.dual_plane {
        upsample_weight_grid(
            blk_width,
            blk_height,
            gw,
            gh,
            &dequantized_weights[1],
            &mut upsampled_weights[1],
        );
    }

    let mut endpoints = [[[0i32; 2]; 4]; 4];
    let mut endpoint_val_index = 0usize;
    for subset in 0..num_parts as usize {
        let cem = u32::from(log_blk.color_endpoint_modes[subset]);
        decode_endpoint(
            cem,
            &mut endpoints[subset],
            dequantized_endpoints
                .get(endpoint_val_index..)
                .unwrap_or(&[]),
        );
        endpoint_val_index += get_num_cem_values(cem) as usize;
    }

    let small_block = num_blk_pixels < 31;
    let ccs = if log_blk.dual_plane {
        u32::from(log_blk.color_component_selector)
    } else {
        u32::MAX
    };
    let subset_of = |x: u32, y: u32| -> usize {
        if num_parts > 1 {
            compute_texel_partition(
                u32::from(log_blk.partition_id),
                x,
                y,
                0,
                num_parts as i32,
                small_block,
            ) as usize
        } else {
            0
        }
    };
    let weight = |c: u32, pixel_index: usize| -> u32 {
        u32::from(upsampled_weights[usize::from(c == ccs)][pixel_index])
    };

    let srgb = matches!(pixels, DecodePixels::Srgb8(_));
    let mut success = true;
    for y in 0..blk_height {
        for x in 0..blk_width {
            let pixel_index = (x + y * blk_width) as usize;
            let subset = subset_of(x, y).min(3);
            let ep = &endpoints[subset];
            let ldr = is_ldr_endpoints[subset];
            match pixels {
                DecodePixels::Rgb9e5(p) => {
                    let mut comp = [0i32; 3];
                    for c in 0..3u32 {
                        let w = weight(c, pixel_index);
                        let [le, he] = ep[c as usize];
                        if ldr {
                            let le = (le << 8) | le;
                            let he = (he << 8) | he;
                            comp[c as usize] = weight_interpolate(le as u32, he as u32, w) as i32;
                        } else {
                            let qlog16 = weight_interpolate((le << 4) as u32, (he << 4) as u32, w);
                            let mut h = qlog16_to_half(qlog16 as i32);
                            if is_half_inf_or_nan(h) {
                                h = 0x7BFF;
                            }
                            comp[c as usize] = i32::from(h);
                        }
                    }
                    let packed = if ldr {
                        pack_rgb9e5_ldr_astc(comp[0], comp[1], comp[2])
                    } else {
                        pack_rgb9e5_hdr_astc(comp[0], comp[1], comp[2])
                    };
                    if let Some(px) = p.get_mut(pixel_index) {
                        *px = packed;
                    }
                }
                DecodePixels::Hdr16(p) => {
                    let mut out = [0u16; 4];
                    for c in 0..4u32 {
                        let w = weight(c, pixel_index);
                        let [le, he] = ep[c as usize];
                        let cem = u32::from(log_blk.color_endpoint_modes[subset]);
                        out[c as usize] = if ldr || (cem == CEM_HDR_RGB_LDR_ALPHA && c == 3) {
                            let le = (le << 8) | le;
                            let he = (he << 8) | he;
                            let k = weight_interpolate(le as u32, he as u32, w);
                            if k == 0xFFFF {
                                0x3C00
                            } else {
                                float_to_half(k as f32 * (1.0 / 65536.0), true)
                            }
                        } else {
                            let qlog16 = weight_interpolate((le << 4) as u32, (he << 4) as u32, w);
                            let o = qlog16_to_half(qlog16 as i32);
                            if is_half_inf_or_nan(o) { 0x7BFF } else { o }
                        };
                    }
                    if let Some(px) = p.get_mut(pixel_index) {
                        *px = out;
                    }
                }
                DecodePixels::Srgb8(p) | DecodePixels::Ldr8(p) => {
                    let Some(px) = p.get_mut(pixel_index) else {
                        continue;
                    };
                    if !ldr {
                        *px = 0xFFFF00FFu32.to_le_bytes();
                        success = false;
                        continue;
                    }
                    for c in 0..4u32 {
                        let w = weight(c, pixel_index);
                        let [le, he] = ep[c as usize];
                        let (le, he) = if srgb {
                            ((le << 8) | 0x80, (he << 8) | 0x80)
                        } else {
                            ((le << 8) | le, (he << 8) | he)
                        };
                        px[c as usize] = (weight_interpolate(le as u32, he as u32, w) >> 8) as u8;
                    }
                }
            }
        }
    }
    success
}

#[inline]
fn get_bits(bits: u128, ofs: u32, len: u32) -> u32 {
    let mask = if len >= 64 {
        u64::MAX
    } else {
        (1u64 << len) - 1
    };
    (bits.checked_shr(ofs).unwrap_or(0) as u64 & mask) as u32
}

#[inline]
fn next_bits(bits: u128, ofs: &mut u32, len: u32) -> u32 {
    let x = get_bits(bits, *ofs, len);
    *ofs += len;
    x
}

fn decode_void_extent(bits: u128, log_blk: &mut LogAstcBlock) -> bool {
    if get_bits(bits, 10, 2) != 0b11 {
        return false;
    }
    let mut ofs = 12;
    let min_s = next_bits(bits, &mut ofs, 13);
    let max_s = next_bits(bits, &mut ofs, 13);
    let min_t = next_bits(bits, &mut ofs, 13);
    let max_t = next_bits(bits, &mut ofs, 13);
    let all_ones = min_s == 0x1FFF && max_s == 0x1FFF && min_t == 0x1FFF && max_t == 0x1FFF;
    if !all_ones && (min_s >= max_s || min_t >= max_t) {
        return false;
    }
    if get_bits(bits, 9, 1) != 0 {
        log_blk.solid_color_flag_hdr = true;
    } else {
        log_blk.solid_color_flag_ldr = true;
    }
    for c in 0..4 {
        log_blk.set_solid_color(c, get_bits(bits, 64 + 16 * c as u32, 16) as u16);
    }
    if log_blk.solid_color_flag_hdr {
        for c in 0..4 {
            if is_half_inf_or_nan(log_blk.solid_color(c)) {
                return false;
            }
        }
    }
    true
}

/// `s_dec_rows`: Dp_ofs, P_ofs, W_ofs, W_size, H_ofs, H_size, W_bias, H_bias, p0_ofs, p1_ofs, p2_ofs.
const DEC_ROWS: [[i8; 11]; 10] = [
    [10, 9, 7, 2, 5, 2, 4, 2, 4, 0, 1],
    [10, 9, 7, 2, 5, 2, 8, 2, 4, 0, 1],
    [10, 9, 5, 2, 7, 2, 2, 8, 4, 0, 1],
    [10, 9, 5, 2, 7, 1, 2, 6, 4, 0, 1],
    [10, 9, 7, 1, 5, 2, 2, 2, 4, 0, 1],
    [10, 9, 0, 0, 5, 2, 12, 2, 4, 2, 3],
    [10, 9, 5, 2, 0, 0, 2, 12, 4, 2, 3],
    [10, 9, 0, 0, 0, 0, 6, 10, 4, 2, 3],
    [10, 9, 0, 0, 0, 0, 10, 6, 4, 2, 3],
    [-1, -1, 5, 2, 9, 2, 6, 6, 4, 2, 3],
];

fn decode_config(bits: u128, log_blk: &mut LogAstcBlock) -> bool {
    if get_bits(bits, 0, 4) == 0 {
        return false;
    }
    if get_bits(bits, 0, 2) == 0 && get_bits(bits, 6, 3) == 0b111 && get_bits(bits, 2, 4) != 0b1111
    {
        return false;
    }
    if get_bits(bits, 0, 9) == 0b111111100 {
        return decode_void_extent(bits, log_blk);
    }
    let x0_2 = get_bits(bits, 0, 2);
    let x2_2 = get_bits(bits, 2, 2);
    let x5_4 = get_bits(bits, 5, 4);
    let x8_1 = get_bits(bits, 8, 1);
    let x7_2 = get_bits(bits, 7, 2);
    let row_index = if x0_2 == 0 {
        if x7_2 == 0b00 {
            5
        } else if x7_2 == 0b01 {
            6
        } else if x5_4 == 0b1100 {
            7
        } else if x5_4 == 0b1101 {
            8
        } else if x7_2 == 0b10 {
            9
        } else {
            return false;
        }
    } else if x2_2 == 0b00 {
        0
    } else if x2_2 == 0b01 {
        1
    } else if x2_2 == 0b10 {
        2
    } else if x8_1 == 0 {
        3
    } else {
        4
    };
    let r = DEC_ROWS[row_index];
    let [
        dp_ofs,
        p_ofs,
        w_ofs,
        w_size,
        h_ofs,
        h_size,
        w_bias,
        h_bias,
        p0,
        p1,
        p2,
    ] = r;
    let mut w = w_bias as u32;
    let mut h = h_bias as u32;
    let pf = p_ofs >= 0 && get_bits(bits, p_ofs as u32, 1) != 0;
    let dp = dp_ofs >= 0 && get_bits(bits, dp_ofs as u32, 1) != 0;
    if w_size != 0 {
        w += get_bits(bits, w_ofs as u32, w_size as u32);
    }
    if h_size != 0 {
        h += get_bits(bits, h_ofs as u32, h_size as u32);
    }
    let p = get_bits(bits, p0 as u32, 1)
        | (get_bits(bits, p1 as u32, 1) << 1)
        | (get_bits(bits, p2 as u32, 1) << 2);
    if p < 2 {
        return false;
    }
    log_blk.grid_width = w as u8;
    log_blk.grid_height = h as u8;
    log_blk.weight_ise_range = ((p - 2) + u32::from(pf) * BISE_10_LEVELS) as u8;
    log_blk.dual_plane = dp;
    true
}

fn decode_trit_block(vals: &mut [u8], bits: u128, ofs: &mut u32, bits_per_val: u32) {
    const T_BITS: [u32; 5] = [2, 2, 1, 2, 1];
    let mut m = [0u32; 5];
    let mut t = 0u32;
    let mut t_ofs = 0;
    for c in 0..vals.len().min(5) {
        if bits_per_val != 0 {
            m[c] = next_bits(bits, ofs, bits_per_val);
        }
        t |= next_bits(bits, ofs, T_BITS[c]) << t_ofs;
        t_ofs += T_BITS[c];
    }
    let trits = TRIT_DECODE[(t & 0xFF) as usize];
    for (i, v) in vals.iter_mut().take(5).enumerate() {
        *v = ((u32::from(trits[i]) << bits_per_val) | m[i]) as u8;
    }
}

fn decode_quint_block(vals: &mut [u8], bits: u128, ofs: &mut u32, bits_per_val: u32) {
    const T_BITS: [u32; 3] = [3, 2, 2];
    let mut m = [0u32; 3];
    let mut t = 0u32;
    let mut t_ofs = 0;
    for c in 0..vals.len().min(3) {
        if bits_per_val != 0 {
            m[c] = next_bits(bits, ofs, bits_per_val);
        }
        t |= next_bits(bits, ofs, T_BITS[c]) << t_ofs;
        t_ofs += T_BITS[c];
    }
    let quints = QUINT_DECODE[(t & 0x7F) as usize];
    for (i, v) in vals.iter_mut().take(3).enumerate() {
        *v = ((u32::from(quints[i]) << bits_per_val) | m[i]) as u8;
    }
}

fn decode_bise_bits(ise_range: u32, vals: &mut [u8], num_vals: usize, bits: u128, mut ofs: u32) {
    let [b, t, q] = ise(ise_range);
    let bits_per_val = u32::from(b);
    let vals = match vals.get_mut(..num_vals) {
        Some(v) => v,
        None => return,
    };
    if t != 0 {
        for chunk in vals.chunks_mut(5) {
            decode_trit_block(chunk, bits, &mut ofs, bits_per_val);
        }
    } else if q != 0 {
        for chunk in vals.chunks_mut(3) {
            decode_quint_block(chunk, bits, &mut ofs, bits_per_val);
        }
    } else {
        for v in vals.iter_mut() {
            *v = next_bits(bits, &mut ofs, bits_per_val) as u8;
        }
    }
}

/// Port of `decode_bise` (from a 128-bit little endian block).
pub fn decode_bise(
    ise_range: u32,
    vals: &mut [u8],
    num_vals: u32,
    bits128: &[u8; 16],
    bit_ofs: u32,
) {
    decode_bise_bits(
        ise_range,
        vals,
        num_vals as usize,
        u128::from_le_bytes(*bits128),
        bit_ofs,
    );
}

/// Port of `unpack_block`: physical ASTC block to logical block.
pub fn unpack_block(
    astc_block: &[u8; 16],
    log_blk: &mut LogAstcBlock,
    blk_width: u32,
    blk_height: u32,
) -> bool {
    log_blk.clear();
    log_blk.error_flag = true;

    let bits = u128::from_le_bytes(*astc_block);
    let rev_bits = bits.reverse_bits();

    if !decode_config(bits, log_blk) {
        return false;
    }
    if log_blk.solid_color_flag_hdr || log_blk.solid_color_flag_ldr {
        log_blk.error_flag = false;
        return true;
    }
    if u32::from(log_blk.grid_width) > blk_width || u32::from(log_blk.grid_height) > blk_height {
        return false;
    }

    let total_grid_weights = (if log_blk.dual_plane { 2 } else { 1 })
        * (u32::from(log_blk.grid_width) * u32::from(log_blk.grid_height));
    let total_weight_bits = get_ise_sequence_bits(
        total_grid_weights as i32,
        u32::from(log_blk.weight_ise_range),
    );
    if total_grid_weights == 0
        || total_grid_weights > MAX_GRID_WEIGHTS
        || !(24..=96).contains(&total_weight_bits)
    {
        return false;
    }
    let total_weight_bits = total_weight_bits as u32;
    let end_of_weight_bit_ofs = 128 - total_weight_bits;
    let mut total_extra_bits = 0u32;

    log_blk.num_partitions = (get_bits(bits, 11, 2) + 1) as u8;
    let num_parts = u32::from(log_blk.num_partitions);
    if num_parts == 1 {
        log_blk.color_endpoint_modes[0] = get_bits(bits, 13, 4) as u8;
    } else {
        if log_blk.dual_plane && num_parts == 4 {
            return false;
        }
        log_blk.partition_id = get_bits(bits, 13, 10) as u16;
        let mut cem_bits = get_bits(bits, 23, 6);
        if (cem_bits & 3) == 0 {
            for i in 0..num_parts as usize {
                log_blk.color_endpoint_modes[i] = (cem_bits >> 2) as u8;
            }
        } else {
            let first_cem_index = ((cem_bits & 3) - 1) * 4;
            total_extra_bits = 3 * num_parts - 4;
            if total_weight_bits + total_extra_bits > 128 {
                return false;
            }
            let mut cem_bit_pos = end_of_weight_bit_ofs - total_extra_bits;
            let mut c = [0u32; 4];
            let mut m = [0u32; 4];
            cem_bits >>= 2;
            for ci in c.iter_mut().take(num_parts as usize) {
                *ci = cem_bits & 1;
                cem_bits >>= 1;
            }
            match num_parts {
                2 => {
                    m[0] = cem_bits & 3;
                    m[1] = next_bits(bits, &mut cem_bit_pos, 2);
                }
                3 => {
                    m[0] = cem_bits & 1;
                    m[0] |= next_bits(bits, &mut cem_bit_pos, 1) << 1;
                    m[1] = next_bits(bits, &mut cem_bit_pos, 2);
                    m[2] = next_bits(bits, &mut cem_bit_pos, 2);
                }
                _ => {
                    for mi in m.iter_mut() {
                        *mi = next_bits(bits, &mut cem_bit_pos, 2);
                    }
                }
            }
            for i in 0..num_parts as usize {
                log_blk.color_endpoint_modes[i] = (first_cem_index + c[i] * 4 + m[i]) as u8;
            }
        }
    }

    if log_blk.dual_plane {
        total_extra_bits += 2;
        if total_extra_bits > end_of_weight_bit_ofs {
            return false;
        }
        let ccs_bit_pos = end_of_weight_bit_ofs - total_extra_bits;
        log_blk.color_component_selector = get_bits(bits, ccs_bit_pos, 2) as u8;
    }

    let mut config_bit_pos = 11 + 2;
    if num_parts == 1 {
        config_bit_pos += 4;
    } else {
        config_bit_pos += 10 + 6;
    }
    let total_config_bits = config_bit_pos + total_extra_bits;
    let num_remaining_bits = 128 - total_config_bits as i32 - total_weight_bits as i32;
    if num_remaining_bits < 0 {
        return false;
    }

    let mut total_cem_vals = 0;
    for j in 0..num_parts as usize {
        total_cem_vals += get_num_cem_values(u32::from(log_blk.color_endpoint_modes[j]));
    }
    if total_cem_vals > MAX_ENDPOINTS {
        return false;
    }

    let mut endpoint_ise_range = -1i32;
    for k in (1..=20).rev() {
        if get_ise_sequence_bits(total_cem_vals as i32, k as u32) <= num_remaining_bits {
            endpoint_ise_range = k;
            break;
        }
    }
    if endpoint_ise_range < FIRST_VALID_ENDPOINT_ISE_RANGE as i32 {
        return false;
    }
    log_blk.endpoint_ise_range = endpoint_ise_range as u8;

    decode_bise_bits(
        endpoint_ise_range as u32,
        &mut log_blk.endpoints,
        total_cem_vals as usize,
        bits,
        config_bit_pos,
    );
    decode_bise_bits(
        u32::from(log_blk.weight_ise_range),
        &mut log_blk.weights,
        total_grid_weights as usize,
        rev_bits,
        0,
    );

    log_blk.error_flag = false;
    true
}

/// Port of `astc_set_bits` (byte-wise, like C).
fn astc_set_bits(out: &mut [u8], bit_pos: &mut u32, mut value: u32, mut total_bits: u32) {
    while total_bits != 0 {
        let bits_to_write = total_bits.min(8 - (*bit_pos & 7));
        if let Some(b) = out.get_mut((*bit_pos >> 3) as usize) {
            *b |= value.wrapping_shl(*bit_pos & 7) as u8;
        }
        *bit_pos += bits_to_write;
        total_bits -= bits_to_write;
        value = value.checked_shr(bits_to_write).unwrap_or(0);
    }
}

/// Port of `astc_set_bits_1_to_9`.
fn astc_set_bits_1_to_9(buf: &mut [u8], bit_offset: &mut u32, code: u32, codesize: u32) {
    if codesize != 0 {
        let byte_bit_offset = *bit_offset & 7;
        let val = code << byte_bit_offset;
        let index = (*bit_offset >> 3) as usize;
        if let Some(b) = buf.get_mut(index) {
            *b |= val as u8;
        }
        if codesize > 8 - byte_bit_offset
            && let Some(b) = buf.get_mut(index + 1)
        {
            *b |= (val >> 8) as u8;
        }
        *bit_offset += codesize;
    }
}

#[inline]
fn astc_extract_bits(bits: u32, low: u32, high: u32) -> u32 {
    (bits >> low) & ((1 << (high - low + 1)) - 1)
}

/// Port of `astc_encode_quints`: 3 values, quints and `n` bits each.
fn astc_encode_quints(out: &mut [u8], values: &[u8; 5], bit_pos: &mut u32, n: u32) {
    let bit_mask = (1u32 << n) - 1;
    let mut quints = 0u32;
    let mut bits = [0u32; 3];
    for (i, mul) in [1u32, 5, 25].into_iter().enumerate() {
        quints += u32::from(values[i] >> n) * mul;
        bits[i] = u32::from(values[i]) & bit_mask;
    }
    let t = u32::from(ASTC_QUINT_ENCODE.get(quints as usize).copied().unwrap_or(0));
    astc_set_bits(
        out,
        bit_pos,
        bits[0]
            | (astc_extract_bits(t, 0, 2) << n)
            | (bits[1] << (3 + n))
            | (astc_extract_bits(t, 3, 4) << (3 + n * 2))
            | (bits[2] << (5 + n * 2))
            | (astc_extract_bits(t, 5, 6) << (5 + n * 3)),
        7 + n * 3,
    );
}

/// Port of `astc_encode_trits`: 5 values, trits and `n` bits each.
fn astc_encode_trits(out: &mut [u8], values: &[u8; 5], bit_pos: &mut u32, n: u32) {
    let bit_mask = (1u32 << n) - 1;
    let mut trits = 0u32;
    let mut bits = [0u32; 5];
    for (i, mul) in [1u32, 3, 9, 27, 81].into_iter().enumerate() {
        trits += u32::from(values[i] >> n) * mul;
        bits[i] = u32::from(values[i]) & bit_mask;
    }
    let t = u32::from(ASTC_TRIT_ENCODE.get(trits as usize).copied().unwrap_or(0));
    astc_set_bits(
        out,
        bit_pos,
        bits[0] | (astc_extract_bits(t, 0, 1) << n) | (bits[1] << (2 + n)),
        n * 2 + 2,
    );
    astc_set_bits(
        out,
        bit_pos,
        astc_extract_bits(t, 2, 3)
            | (bits[2] << 2)
            | (astc_extract_bits(t, 4, 4) << (2 + n))
            | (bits[3] << (3 + n))
            | (astc_extract_bits(t, 5, 6) << (3 + n * 2))
            | (bits[4] << (5 + n * 2))
            | (astc_extract_bits(t, 7, 7) << (5 + n * 3)),
        n * 3 + 6,
    );
}

/// Port of `encode_bise`: ORs `num_vals` BISE-packed values into `dst` at `bit_pos`.
pub fn encode_bise(
    dst: &mut [u8; 16],
    src_vals: &[u8],
    mut bit_pos: u32,
    num_vals: u32,
    range: u32,
) {
    let mut temp = [0u8; 20];
    let [num_bits, trits, quints] = ise(range);
    let num_bits = u32::from(num_bits);
    let group_size = if trits != 0 {
        5
    } else if quints != 0 {
        3
    } else {
        0
    };
    let val = |i: usize| src_vals.get(i).copied().unwrap_or(0);
    if group_size != 0 {
        // Pack each group of 5 or 3 values (always writes a whole group).
        let total_groups = num_vals.div_ceil(group_size);
        for group_index in 0..total_groups {
            let mut vals = [0u8; 5];
            let limit = group_size.min(num_vals - group_index * group_size);
            for i in 0..limit {
                vals[i as usize] = val((group_index * group_size + i) as usize);
            }
            if group_size == 5 {
                astc_encode_trits(&mut temp, &vals, &mut bit_pos, num_bits);
            } else {
                astc_encode_quints(&mut temp, &vals, &mut bit_pos, num_bits);
            }
        }
    } else {
        for i in 0..num_vals as usize {
            astc_set_bits_1_to_9(&mut temp, &mut bit_pos, u32::from(val(i)), num_bits);
        }
    }
    for (d, t) in dst.iter_mut().zip(temp) {
        *d |= t;
    }
}

#[inline]
fn is_packable(value: i32, num_bits: u32) -> bool {
    value >= 0 && value < (1 << num_bits)
}

/// Port of `get_config_bits` (tables 81-82).
fn get_config_bits(log_block: &LogAstcBlock) -> Option<u32> {
    let w = i32::from(log_block.grid_width);
    let h = i32::from(log_block.grid_height);
    let hp = u32::from(log_block.weight_ise_range >= 6); // high precision
    let dp_p = (u32::from(log_block.dual_plane) << 1) | hp;
    // p from the weight range, bits rearranged to p0 p2 p1
    let mut p =
        (2 + u32::from(log_block.weight_ise_range)).wrapping_sub(if hp != 0 { 6 } else { 0 });
    p = (p >> 1) + ((p & 1) << 2);
    let (wu, hu) = (w as u32, h as u32);

    if is_packable(w - 4, 2) && is_packable(h - 2, 2) {
        return Some((dp_p << 9) | ((wu - 4) << 7) | ((hu - 2) << 5) | ((p & 4) << 2) | (p & 3));
    }
    if is_packable(w - 8, 2) && is_packable(h - 2, 2) {
        return Some(
            (dp_p << 9) | ((wu - 8) << 7) | ((hu - 2) << 5) | ((p & 4) << 2) | 4 | (p & 3),
        );
    }
    if is_packable(w - 2, 2) && is_packable(h - 8, 2) {
        return Some(
            (dp_p << 9) | ((hu - 8) << 7) | ((wu - 2) << 5) | ((p & 4) << 2) | 8 | (p & 3),
        );
    }
    if is_packable(w - 2, 2) && is_packable(h - 6, 1) {
        return Some(
            (dp_p << 9) | ((hu - 6) << 7) | ((wu - 2) << 5) | ((p & 4) << 2) | 12 | (p & 3),
        );
    }
    if is_packable(w - 2, 1) && is_packable(h - 2, 2) {
        return Some((dp_p << 9) | (wu << 7) | ((hu - 2) << 5) | ((p & 4) << 2) | 12 | (p & 3));
    }
    if w == 12 && is_packable(h - 2, 2) {
        return Some((dp_p << 9) | ((hu - 2) << 5) | (p << 2));
    }
    if h == 12 && is_packable(w - 2, 2) {
        return Some((dp_p << 9) | (1 << 7) | ((wu - 2) << 5) | (p << 2));
    }
    if w == 6 && h == 10 {
        return Some((dp_p << 9) | (3 << 7) | (p << 2));
    }
    if w == 10 && h == 6 {
        return Some((dp_p << 9) | (0b1101 << 5) | (p << 2));
    }
    // W+6 H+6 (no dual plane or high prec)
    if dp_p == 0 && is_packable(w - 6, 2) && is_packable(h - 6, 2) {
        return Some(((hu - 6) << 9) | 256 | ((wu - 6) << 5) | (p << 2));
    }
    None
}

fn pack_void_extent(blk: &mut [u8; 16], ldr: bool, c: [u16; 4]) {
    blk.fill(0xFF);
    blk[0] = 0b1111_1100;
    if ldr {
        blk[1] = 0b1111_1101;
    }
    for (i, v) in c.iter().enumerate() {
        blk[8 + i * 2..10 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
}

/// Port of `pack_void_extent_ldr`.
pub fn pack_void_extent_ldr(blk: &mut [u8; 16], r: u16, g: u16, b: u16, a: u16) {
    pack_void_extent(blk, true, [r, g, b, a]);
}

/// Port of `pack_void_extent_hdr` (half float components).
pub fn pack_void_extent_hdr(blk: &mut [u8; 16], rh: u16, gh: u16, bh: u16, ah: u16) {
    pack_void_extent(blk, false, [rh, gh, bh, ah]);
}

/// Port of `pack_astc_block`: logical block to physical ASTC block (no stats or
/// validate flags; the logical endpoint ISE range must be the one the layout implies).
pub fn pack_astc_block(phys_block: &mut [u8; 16], log_block: &LogAstcBlock) -> bool {
    *phys_block = [0; 16];
    if log_block.error_flag {
        return false;
    }
    let solid = |c| log_block.solid_color(c);
    if log_block.solid_color_flag_ldr {
        pack_void_extent_ldr(phys_block, solid(0), solid(1), solid(2), solid(3));
        return true;
    } else if log_block.solid_color_flag_hdr {
        pack_void_extent_hdr(phys_block, solid(0), solid(1), solid(2), solid(3));
        return true;
    }

    let num_parts = u32::from(log_block.num_partitions);
    if !(1..=MAX_PARTITIONS).contains(&num_parts) {
        return false;
    }
    // Max usable weight range is 11
    if u32::from(log_block.weight_ise_range) > LAST_VALID_WEIGHT_ISE_RANGE {
        return false;
    }
    // See 23.24 Illegal Encodings, [0,5] is the minimum ISE encoding for endpoints
    if !(FIRST_VALID_ENDPOINT_ISE_RANGE..=LAST_VALID_ENDPOINT_ISE_RANGE)
        .contains(&u32::from(log_block.endpoint_ise_range))
    {
        return false;
    }
    if log_block.color_component_selector > 3 {
        return false;
    }

    let Some(config_bits) = get_config_bits(log_block) else {
        return false;
    };
    let mut bit_pos = 0u32;
    astc_set_bits(phys_block, &mut bit_pos, config_bits, 11);

    let total_grid_weights = (if log_block.dual_plane { 2 } else { 1 })
        * (u32::from(log_block.grid_width) * u32::from(log_block.grid_height));
    let total_weight_bits = get_ise_sequence_bits(
        total_grid_weights as i32,
        u32::from(log_block.weight_ise_range),
    );
    // 18.24 Illegal Encodings
    if total_grid_weights == 0
        || total_grid_weights > MAX_GRID_WEIGHTS
        || !(24..=96).contains(&total_weight_bits)
    {
        return false;
    }
    let total_weight_bits = total_weight_bits as u32;

    let mut total_extra_bits = 0u32;
    astc_set_bits(phys_block, &mut bit_pos, num_parts - 1, 2);
    let cems = &log_block.color_endpoint_modes;

    if num_parts > 1 {
        if u32::from(log_block.partition_id) >= NUM_PARTITION_PATTERNS {
            return false;
        }
        astc_set_bits(
            phys_block,
            &mut bit_pos,
            u32::from(log_block.partition_id),
            10,
        );

        let parts = &cems[..num_parts as usize];
        let highest_cem = parts.iter().copied().max().map_or(0, u32::from);
        let lowest_cem = parts.iter().copied().min().map_or(u32::MAX, u32::from);
        if highest_cem > 15 {
            return false;
        }
        // Ensure CEM range is contiguous
        if (highest_cem >> 2) > 1 + (lowest_cem >> 2) {
            return false;
        }
        // See tables 79/80
        let mut encoded_cem = u32::from(cems[0]) << 2;
        if lowest_cem != highest_cem {
            encoded_cem = 3.min(1 + (lowest_cem >> 2));
            for (j, &cem) in parts.iter().enumerate() {
                let m = i32::from(cem & 3);
                let c = i32::from(cem >> 2) - ((encoded_cem & 3) as i32 - 1);
                if (c & 1) != c {
                    return false;
                }
                encoded_cem |= ((c << (2 + j)) | (m << (2 + num_parts as usize + 2 * j))) as u32;
            }
            total_extra_bits = 3 * num_parts - 4;
            if total_weight_bits + total_extra_bits > 128 {
                return false;
            }
            let mut cem_bit_pos = 128 - total_weight_bits - total_extra_bits;
            astc_set_bits(
                phys_block,
                &mut cem_bit_pos,
                encoded_cem >> 6,
                total_extra_bits,
            );
        }
        astc_set_bits(phys_block, &mut bit_pos, encoded_cem & 0x3f, 6);
    } else {
        if log_block.partition_id != 0 || cems[0] > 15 {
            return false;
        }
        astc_set_bits(phys_block, &mut bit_pos, u32::from(cems[0]), 4);
    }

    if log_block.dual_plane {
        if num_parts > 3 {
            return false;
        }
        total_extra_bits += 2;
        let mut ccs_bit_pos = 128 - total_weight_bits - total_extra_bits;
        astc_set_bits(
            phys_block,
            &mut ccs_bit_pos,
            u32::from(log_block.color_component_selector),
            2,
        );
    }

    let total_config_bits = bit_pos + total_extra_bits;
    let num_remaining_bits = 128 - total_config_bits as i32 - total_weight_bits as i32;
    if num_remaining_bits < 0 {
        return false;
    }
    let total_cem_vals: u32 = cems[..num_parts as usize]
        .iter()
        .map(|&c| 2 + 2 * (u32::from(c) >> 2))
        .sum();
    if total_cem_vals > MAX_ENDPOINTS {
        return false;
    }
    let mut endpoint_ise_range = -1i32;
    for k in (1..=20).rev() {
        if get_ise_sequence_bits(total_cem_vals as i32, k as u32) <= num_remaining_bits {
            endpoint_ise_range = k;
            break;
        }
    }
    if endpoint_ise_range < FIRST_VALID_ENDPOINT_ISE_RANGE as i32 {
        return false;
    }
    // Ensure the caller utilized the right endpoint ISE range.
    if i32::from(log_block.endpoint_ise_range) != endpoint_ise_range {
        return false;
    }

    // Pack endpoints forwards
    encode_bise(
        phys_block,
        &log_block.endpoints,
        bit_pos,
        total_cem_vals,
        endpoint_ise_range as u32,
    );
    // Pack weights backwards
    let mut weight_data = [0u8; 16];
    encode_bise(
        &mut weight_data,
        &log_block.weights,
        0,
        total_grid_weights,
        u32::from(log_block.weight_ise_range),
    );
    let packed = u128::from_le_bytes(*phys_block) | u128::from_le_bytes(weight_data).reverse_bits();
    *phys_block = packed.to_le_bytes();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_round_trip() {
        for h in [0u16, 1, 0x3C00, 0x7BFF, 0x0400, 0x03FF, 0x8001] {
            assert_eq!(float_to_half(half_to_float(h), false), h);
        }
    }

    #[test]
    fn dequant_tables_cover_all_levels() {
        let t = dequant_tables();
        assert_eq!(t.endpoint_ise_to_val(20).len(), 256);
        assert_eq!(t.weight_ise_to_val(11).len(), 32);
        assert_eq!(t.weight_ise_to_val(0), &[0, 64]);
    }

    #[test]
    fn rank_tables_sort_by_value() {
        let t = dequant_tables();
        for range in FIRST_VALID_ENDPOINT_ISE_RANGE..=LAST_VALID_ENDPOINT_ISE_RANGE {
            let tab = t.endpoint_tab(range);
            for (rank, &ise) in tab.rank_to_ise.iter().enumerate() {
                assert_eq!(tab.ise_to_rank[ise as usize] as usize, rank);
                if rank > 0 {
                    let prev = tab.rank_to_ise[rank - 1] as usize;
                    assert!(tab.ise_to_val[prev] <= tab.ise_to_val[ise as usize]);
                }
            }
            for v in 0..256 {
                let ise = tab.val_to_ise[v] as usize;
                assert!(ise < tab.ise_to_val.len());
            }
        }
    }
}
