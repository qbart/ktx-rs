// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of `astcenc_block_sizes.cpp` (compressor build): block mode decoding, decimation
//! tables and the block size descriptor with percentile based mode selection.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::get_ise_sequence_bitcount;
use super::internal::*;
use super::mathlib::{rand, rand_init};
use super::partition_tables::init_partition_tables;
use super::percentile_tables::get_packed_table;
use super::vecmath::round_up_to_simd_multiple_vla;
use ktx_rs_macros::isa;

/// Result of `decode_block_mode_2d/3d`: weights (x, y, z), dual plane, quant mode, bits.
struct DecodedMode {
    x: u32,
    y: u32,
    z: u32,
    dual: bool,
    quant: u32,
    bits: u32,
}

#[isa]
fn finish(x: u32, y: u32, z: u32, d: u32, h: u32, base_quant_mode: u32) -> Option<DecodedMode> {
    let weight_count = x * y * z * (d + 1);
    let quant = (base_quant_mode.wrapping_sub(2)).wrapping_add(6 * h);
    let bits = get_ise_sequence_bitcount(weight_count, quant as usize);
    let valid = weight_count as usize <= BLOCK_MAX_WEIGHTS
        && (BLOCK_MIN_WEIGHT_BITS..=BLOCK_MAX_WEIGHT_BITS).contains(&bits);
    valid.then_some(DecodedMode {
        x,
        y,
        z,
        dual: d != 0,
        quant,
        bits,
    })
}

#[isa]
/// `decode_block_mode_2d`.
fn decode_block_mode_2d(block_mode: u32) -> Option<DecodedMode> {
    let mut base_quant_mode = (block_mode >> 4) & 1;
    let mut h = (block_mode >> 9) & 1;
    let mut d = (block_mode >> 10) & 1;
    let a = (block_mode >> 5) & 0x3;
    let (wx, wy);
    if block_mode & 3 != 0 {
        base_quant_mode |= (block_mode & 3) << 1;
        let mut b = (block_mode >> 7) & 3;
        match (block_mode >> 2) & 3 {
            0 => {
                wx = b + 4;
                wy = a + 2;
            }
            1 => {
                wx = b + 8;
                wy = a + 2;
            }
            2 => {
                wx = a + 2;
                wy = b + 8;
            }
            _ => {
                b &= 1;
                if block_mode & 0x100 != 0 {
                    wx = b + 2;
                    wy = a + 2;
                } else {
                    wx = a + 2;
                    wy = b + 6;
                }
            }
        }
    } else {
        base_quant_mode |= ((block_mode >> 2) & 3) << 1;
        if (block_mode >> 2) & 3 == 0 {
            return None;
        }
        let b = (block_mode >> 9) & 3;
        match (block_mode >> 7) & 3 {
            0 => {
                wx = 12;
                wy = a + 2;
            }
            1 => {
                wx = a + 2;
                wy = 12;
            }
            2 => {
                wx = a + 6;
                wy = b + 6;
                d = 0;
                h = 0;
            }
            _ => match (block_mode >> 5) & 3 {
                0 => {
                    wx = 6;
                    wy = 10;
                }
                1 => {
                    wx = 10;
                    wy = 6;
                }
                _ => return None,
            },
        }
    }
    finish(wx, wy, 1, d, h, base_quant_mode)
}

#[isa]
/// `decode_block_mode_3d`.
fn decode_block_mode_3d(block_mode: u32) -> Option<DecodedMode> {
    let mut base_quant_mode = (block_mode >> 4) & 1;
    let mut h = (block_mode >> 9) & 1;
    let mut d = (block_mode >> 10) & 1;
    let a = (block_mode >> 5) & 0x3;
    let (mut wx, mut wy, mut wz);
    if block_mode & 3 != 0 {
        base_quant_mode |= (block_mode & 3) << 1;
        let b = (block_mode >> 7) & 3;
        let c = (block_mode >> 2) & 0x3;
        wx = a + 2;
        wy = b + 2;
        wz = c + 2;
    } else {
        base_quant_mode |= ((block_mode >> 2) & 3) << 1;
        if (block_mode >> 2) & 3 == 0 {
            return None;
        }
        let b = (block_mode >> 9) & 3;
        if (block_mode >> 7) & 3 != 3 {
            d = 0;
            h = 0;
        }
        match (block_mode >> 7) & 3 {
            0 => {
                wx = 6;
                wy = b + 2;
                wz = a + 2;
            }
            1 => {
                wx = a + 2;
                wy = 6;
                wz = b + 2;
            }
            2 => {
                wx = a + 2;
                wy = b + 2;
                wz = 6;
            }
            _ => {
                wx = 2;
                wy = 2;
                wz = 2;
                match (block_mode >> 5) & 3 {
                    0 => wx = 6,
                    1 => wy = 6,
                    2 => wz = 6,
                    _ => return None,
                }
            }
        }
    }
    finish(wx, wy, wz, d, h, base_quant_mode)
}

/// `dt_init_working_buffers`.
struct DtInitWorkingBuffers {
    weight_count_of_texel: [u8; BLOCK_MAX_TEXELS],
    grid_weights_of_texel: [[u8; 4]; BLOCK_MAX_TEXELS],
    weights_of_texel: [[u8; 4]; BLOCK_MAX_TEXELS],
    texel_count_of_weight: [u8; BLOCK_MAX_WEIGHTS],
    texels_of_weight: [[u8; BLOCK_MAX_TEXELS]; BLOCK_MAX_WEIGHTS],
    texel_weights_of_weight: [[u8; BLOCK_MAX_TEXELS]; BLOCK_MAX_WEIGHTS],
}

impl DtInitWorkingBuffers {
    #[isa]
    fn new() -> Box<DtInitWorkingBuffers> {
        Box::new(DtInitWorkingBuffers {
            weight_count_of_texel: [0; BLOCK_MAX_TEXELS],
            grid_weights_of_texel: [[0; 4]; BLOCK_MAX_TEXELS],
            weights_of_texel: [[0; 4]; BLOCK_MAX_TEXELS],
            texel_count_of_weight: [0; BLOCK_MAX_WEIGHTS],
            texels_of_weight: [[0; BLOCK_MAX_TEXELS]; BLOCK_MAX_WEIGHTS],
            texel_weights_of_weight: [[0; BLOCK_MAX_TEXELS]; BLOCK_MAX_WEIGHTS],
        })
    }

    #[isa]
    /// Records the (weight, contribution) pairs of one texel.
    fn add(&mut self, texel: usize, qweight: &[u32; 4], weight: &[u32; 4], max_tcw: &mut u8) {
        for i in 0..4 {
            if weight[i] != 0 {
                let q = qweight[i] as usize;
                let n = self.weight_count_of_texel[texel] as usize;
                self.grid_weights_of_texel[texel][n] = qweight[i] as u8;
                self.weights_of_texel[texel][n] = weight[i] as u8;
                self.weight_count_of_texel[texel] += 1;
                let m = self.texel_count_of_weight[q] as usize;
                self.texels_of_weight[q][m] = texel as u8;
                self.texel_weights_of_weight[q][m] = weight[i] as u8;
                self.texel_count_of_weight[q] += 1;
                *max_tcw = (*max_tcw).max(self.texel_count_of_weight[q]);
            }
        }
    }
}

#[isa]
/// Common tail of `init_decimation_info_2d/3d`. `zero_first` selects the 3D order of the
/// per texel loops (zero all 4 then fill) vs the 2D order (fill then zero the rest).
fn finish_decimation_info(
    di: &mut DecimationInfo,
    wb: &DtInitWorkingBuffers,
    texels_per_block: usize,
    weights_per_block: usize,
    max_texel_count_of_weight: u8,
    zero_first: bool,
) {
    let mut max_texel_weight_count = 0u8;
    for i in 0..texels_per_block {
        di.texel_weight_count[i] = wb.weight_count_of_texel[i];
        max_texel_weight_count = max_texel_weight_count.max(di.texel_weight_count[i]);
        let n = wb.weight_count_of_texel[i] as usize;
        let zero = |di: &mut DecimationInfo, j: usize| {
            di.texel_weight_contribs_int_tr[j][i] = 0;
            di.texel_weight_contribs_float_tr[j][i] = 0.0;
            di.texel_weights_tr[j][i] = 0;
        };
        if zero_first {
            for j in 0..4 {
                zero(di, j);
            }
        }
        for j in 0..n {
            di.texel_weight_contribs_int_tr[j][i] = wb.weights_of_texel[i][j];
            di.texel_weight_contribs_float_tr[j][i] =
                wb.weights_of_texel[i][j] as f32 * (1.0 / WEIGHTS_TEXEL_SUM);
            di.texel_weights_tr[j][i] = wb.grid_weights_of_texel[i][j];
        }
        if !zero_first {
            for j in n..4 {
                zero(di, j);
            }
        }
    }
    di.max_texel_weight_count = max_texel_weight_count;

    for i in 0..weights_per_block {
        let texel_count_wt = wb.texel_count_of_weight[i] as usize;
        di.weight_texel_count[i] = texel_count_wt as u8;
        for j in 0..texel_count_wt {
            let texel = wb.texels_of_weight[i][j];
            di.weight_texels_tr[j][i] = texel;
            di.weights_texel_contribs_tr[j][i] = wb.texel_weights_of_weight[i][j] as f32;
            // perform a layer of array unrolling. An aspect of this unrolling is that
            // one of the texel-weight indexes is an identity-mapped index; we will use this
            // fact to reorder the indexes so that the first one is the identity index.
            di.texel_contrib_for_weight[j][i] = 0.0;
            for k in 0..4 {
                let dttw = di.texel_weights_tr[k][texel as usize];
                let dttwf = di.texel_weight_contribs_float_tr[k][texel as usize];
                if dttw as usize == i && dttwf != 0.0 {
                    di.texel_contrib_for_weight[j][i] = dttwf;
                    break;
                }
            }
        }
        // Initialize array tail so we can over-fetch with SIMD later to avoid loop tails
        // Match last texel in active lane in SIMD group, for better gathers
        let last_texel = di.weight_texels_tr[texel_count_wt.saturating_sub(1)][i];
        for j in texel_count_wt..max_texel_count_of_weight as usize {
            di.weight_texels_tr[j][i] = last_texel;
            di.weights_texel_contribs_tr[j][i] = 0.0;
        }
    }

    // Initialize array tail so we can over-fetch with SIMD later to avoid loop tails
    let texels_per_block_simd = round_up_to_simd_multiple_vla(texels_per_block);
    for i in texels_per_block..texels_per_block_simd {
        di.texel_weight_count[i] = 0;
        for j in 0..4 {
            di.texel_weight_contribs_float_tr[j][i] = 0.0;
            di.texel_weights_tr[j][i] = 0;
            di.texel_weight_contribs_int_tr[j][i] = 0;
        }
    }

    // Initialize array tail so we can over-fetch with SIMD later to avoid loop tails
    // Match last texel in active lane in SIMD group, for better gathers
    let last_texel_count_wt = wb.texel_count_of_weight[weights_per_block - 1] as usize;
    let last_texel =
        di.weight_texels_tr[last_texel_count_wt.saturating_sub(1)][weights_per_block - 1];
    let weights_per_block_simd = round_up_to_simd_multiple_vla(weights_per_block);
    for i in weights_per_block..weights_per_block_simd {
        di.weight_texel_count[i] = 0;
        for j in 0..max_texel_count_of_weight as usize {
            di.weight_texels_tr[j][i] = last_texel;
            di.weights_texel_contribs_tr[j][i] = 0.0;
        }
    }
}

#[isa]
/// `init_decimation_info_2d`.
fn init_decimation_info_2d(
    texels_x: u32,
    texels_y: u32,
    weights_x: u32,
    weights_y: u32,
    di: &mut DecimationInfo,
    wb: &mut DtInitWorkingBuffers,
) {
    let texels_per_block = (texels_x * texels_y) as usize;
    let weights_per_block = (weights_x * weights_y) as usize;
    let mut max_texel_count_of_weight = 0u8;
    wb.texel_count_of_weight[..weights_per_block].fill(0);
    wb.weight_count_of_texel[..texels_per_block].fill(0);

    for y in 0..texels_y {
        for x in 0..texels_x {
            let texel = (y * texels_x + x) as usize;
            let x_weight =
                (((1024 + texels_x / 2) / (texels_x - 1)) * x * (weights_x - 1) + 32) >> 6;
            let y_weight =
                (((1024 + texels_y / 2) / (texels_y - 1)) * y * (weights_y - 1) + 32) >> 6;
            let x_weight_frac = x_weight & 0xF;
            let y_weight_frac = y_weight & 0xF;
            let x_weight_int = x_weight >> 4;
            let y_weight_int = y_weight >> 4;
            let mut qweight = [0u32; 4];
            qweight[0] = x_weight_int + y_weight_int * weights_x;
            qweight[1] = qweight[0] + 1;
            qweight[2] = qweight[0] + weights_x;
            qweight[3] = qweight[2] + 1;
            // Truncated-precision bilinear interpolation
            let prod = x_weight_frac * y_weight_frac;
            let mut weight = [0u32; 4];
            weight[3] = (prod + 8) >> 4;
            weight[1] = x_weight_frac.wrapping_sub(weight[3]);
            weight[2] = y_weight_frac.wrapping_sub(weight[3]);
            weight[0] = (16u32
                .wrapping_sub(x_weight_frac)
                .wrapping_sub(y_weight_frac))
            .wrapping_add(weight[3]);
            wb.add(texel, &qweight, &weight, &mut max_texel_count_of_weight);
        }
    }
    finish_decimation_info(
        di,
        wb,
        texels_per_block,
        weights_per_block,
        max_texel_count_of_weight,
        false,
    );
    di.texel_count = texels_per_block as u8;
    di.weight_count = weights_per_block as u8;
    di.weight_x = weights_x as u8;
    di.weight_y = weights_y as u8;
    di.weight_z = 1;
}

#[isa]
/// `init_decimation_info_3d`.
fn init_decimation_info_3d(
    texels: (u32, u32, u32),
    weights: (u32, u32, u32),
    di: &mut DecimationInfo,
    wb: &mut DtInitWorkingBuffers,
) {
    let (texels_x, texels_y, texels_z) = texels;
    let (weights_x, weights_y, weights_z) = weights;
    let texels_per_block = (texels_x * texels_y * texels_z) as usize;
    let weights_per_block = (weights_x * weights_y * weights_z) as usize;
    let mut max_texel_count_of_weight = 0u8;
    wb.texel_count_of_weight[..weights_per_block].fill(0);
    wb.weight_count_of_texel[..texels_per_block].fill(0);

    for z in 0..texels_z {
        for y in 0..texels_y {
            for x in 0..texels_x {
                let texel = ((z * texels_y + y) * texels_x + x) as usize;
                let x_weight = ((((1024 + texels_x / 2) / (texels_x - 1)) * x * (weights_x - 1)
                    + 32)
                    >> 6) as i32;
                let y_weight = ((((1024 + texels_y / 2) / (texels_y - 1)) * y * (weights_y - 1)
                    + 32)
                    >> 6) as i32;
                let z_weight = ((((1024 + texels_z / 2) / (texels_z - 1)) * z * (weights_z - 1)
                    + 32)
                    >> 6) as i32;
                let x_weight_frac = x_weight & 0xF;
                let y_weight_frac = y_weight & 0xF;
                let z_weight_frac = z_weight & 0xF;
                let x_weight_int = x_weight >> 4;
                let y_weight_int = y_weight >> 4;
                let z_weight_int = z_weight >> 4;
                let mut qweight = [0i32; 4];
                qweight[0] = (z_weight_int * weights_y as i32 + y_weight_int) * weights_x as i32
                    + x_weight_int;
                qweight[3] = ((z_weight_int + 1) * weights_y as i32 + (y_weight_int + 1))
                    * weights_x as i32
                    + (x_weight_int + 1);

                // simplex interpolation
                let fs = x_weight_frac;
                let ft = y_weight_frac;
                let fp = z_weight_frac;
                let cas =
                    (((fs > ft) as i32) << 2) + (((ft > fp) as i32) << 1) + ((fs > fp) as i32);
                let n = weights_x as i32;
                let nm = (weights_x * weights_y) as i32;
                let (s1, s2, w0, w1, w2, w3) = match cas {
                    7 => (1, n, 16 - fs, fs - ft, ft - fp, fp),
                    3 => (n, 1, 16 - ft, ft - fs, fs - fp, fp),
                    5 => (1, nm, 16 - fs, fs - fp, fp - ft, ft),
                    4 => (nm, 1, 16 - fp, fp - fs, fs - ft, ft),
                    2 => (n, nm, 16 - ft, ft - fp, fp - fs, fs),
                    _ => (nm, n, 16 - fp, fp - ft, ft - fs, fs),
                };
                qweight[1] = qweight[0] + s1;
                qweight[2] = qweight[1] + s2;
                let qw = qweight.map(|v| v as u32);
                let w = [w0, w1, w2, w3].map(|v| v as u32);
                wb.add(texel, &qw, &w, &mut max_texel_count_of_weight);
            }
        }
    }
    finish_decimation_info(
        di,
        wb,
        texels_per_block,
        weights_per_block,
        max_texel_count_of_weight,
        true,
    );
    di.texel_count = texels_per_block as u8;
    di.weight_count = weights_per_block as u8;
    di.weight_x = weights_x as u8;
    di.weight_y = weights_y as u8;
    di.weight_z = weights_z as u8;
}

#[isa]
/// `assign_kmeans_texels`.
fn assign_kmeans_texels(bsd: &mut BlockSizeDescriptor) {
    // Use all texels for kmeans on a small block
    if bsd.texel_count as usize <= BLOCK_MAX_KMEANS_TEXELS {
        for i in 0..bsd.texel_count {
            bsd.kmeans_texels[i as usize] = i;
        }
        return;
    }
    // Select a random subset of BLOCK_MAX_KMEANS_TEXELS for kmeans on a large block
    let mut rng_state = rand_init();
    // Initialize array used for tracking used indices
    let mut seen = [false; BLOCK_MAX_TEXELS];
    // Assign 64 random indices, retrying if we see repeats
    let mut arr_elements_set = 0;
    while arr_elements_set < BLOCK_MAX_KMEANS_TEXELS {
        let mut texel = rand(&mut rng_state) as u8;
        texel %= bsd.texel_count;
        if !seen[texel as usize] {
            bsd.kmeans_texels[arr_elements_set] = texel;
            arr_elements_set += 1;
            seen[texel as usize] = true;
        }
    }
}

#[isa]
/// `maxprec` values of a decimation entry.
fn maxprecs(weight_count: u32, try_2planes: bool) -> (i32, i32) {
    let mut maxprec_1plane = -1;
    let mut maxprec_2planes = -1;
    for i in 0..12 {
        let bits_1plane = get_ise_sequence_bitcount(weight_count, i);
        if (BLOCK_MIN_WEIGHT_BITS..=BLOCK_MAX_WEIGHT_BITS).contains(&bits_1plane) {
            maxprec_1plane = i as i32;
        }
        if try_2planes {
            let bits_2planes = get_ise_sequence_bitcount(2 * weight_count, i);
            if (BLOCK_MIN_WEIGHT_BITS..=BLOCK_MAX_WEIGHT_BITS).contains(&bits_2planes) {
                maxprec_2planes = i as i32;
            }
        }
    }
    (maxprec_1plane, maxprec_2planes)
}

#[isa]
/// `construct_dt_entry_2d`.
fn construct_dt_entry_2d(
    texels_x: u32,
    texels_y: u32,
    weights_x: u32,
    weights_y: u32,
    bsd: &mut BlockSizeDescriptor,
    wb: &mut DtInitWorkingBuffers,
    index: usize,
) {
    let weight_count = weights_x * weights_y;
    let try_2planes = (2 * weight_count) as usize <= BLOCK_MAX_WEIGHTS;
    init_decimation_info_2d(
        texels_x,
        texels_y,
        weights_x,
        weights_y,
        &mut bsd.decimation_tables[index],
        wb,
    );
    let (maxprec_1plane, maxprec_2planes) = maxprecs(weight_count, try_2planes);
    let dm = &mut bsd.decimation_modes[index];
    dm.maxprec_1plane = maxprec_1plane as i8;
    dm.maxprec_2planes = maxprec_2planes as i8;
    dm.refprec_1plane = 0;
    dm.refprec_2planes = 0;
}

#[isa]
/// `get_2d_percentile_table`.
pub fn get_2d_percentile_table(dim_x: u32, dim_y: u32) -> Vec<f32> {
    let mut unpacked = vec![1.0f32; WEIGHTS_MAX_BLOCK_MODES];
    let Some(apt) = get_packed_table(dim_x, dim_y) else {
        return unpacked;
    };
    for i in 0..2 {
        let itemcount = apt.item_count[i] as usize;
        let difscale = apt.difscales[i] as u32;
        let mut accum = apt.initial_percs[i] as u32;
        for &item in apt.items[i].iter().take(itemcount) {
            let idx = (item & 0x7FF) as usize;
            let weight = ((item >> 11) & 0x1F) as u32;
            accum += weight;
            unpacked[idx] = accum as f32 / difscale as f32;
        }
    }
    unpacked
}

#[isa]
/// `construct_block_size_descriptor_2d`.
fn construct_block_size_descriptor_2d(
    texels_x: u32,
    texels_y: u32,
    can_omit_modes: bool,
    mode_cutoff: f32,
    bsd: &mut BlockSizeDescriptor,
) {
    const MAX_DMI: usize = 12 * 16 + 12;
    let mut decimation_mode_index = [-1i32; MAX_DMI];
    let mut wb = DtInitWorkingBuffers::new();

    bsd.dim_x = texels_x as u8;
    bsd.dim_y = texels_y as u8;
    bsd.dim_z = 1;
    bsd.texel_count = (texels_x * texels_y) as u8;

    let percentiles = get_2d_percentile_table(texels_x, texels_y);
    let always_cutoff = 0.0f32;

    // Construct the list of block formats referencing the decimation tables
    let mut packed_bm_idx = 0usize;
    let mut packed_dm_idx = 0usize;
    let mut bm_counts = [0u32; 4];
    let mut dm_counts = [0u32; 4];

    // Clear out the list of block modes
    bsd.block_mode_packed_index.fill(BLOCK_BAD_BLOCK_MODE);

    // Iterate four times to build a usefully ordered list:
    //   - Pass 0 - keep selected single plane "always" block modes
    //   - Pass 1 - keep selected single plane "non-always" block modes
    //   - Pass 2 - keep select dual plane block modes
    //   - Pass 3 - keep everything else that's legal
    let limit = if can_omit_modes { 3 } else { 4 };
    for j in 0..limit {
        for i in 0..WEIGHTS_MAX_BLOCK_MODES {
            // Skip modes we've already included in a previous pass
            if bsd.block_mode_packed_index[i] != BLOCK_BAD_BLOCK_MODE {
                continue;
            }
            // Decode parameters
            let Some(m) = decode_block_mode_2d(i as u32) else {
                continue;
            };
            // Always skip invalid encodings for the current block size
            if m.x > texels_x || m.y > texels_y {
                continue;
            }
            // Selectively skip dual plane encodings
            if (j <= 1 && m.dual) || (j == 2 && !m.dual) {
                continue;
            }
            // Always skip encodings we can't physically encode based on
            // generic encoding bit availability
            let limit_bits = if m.dual { 109 } else { 111 };
            if (limit_bits as u32).wrapping_sub(m.bits) == 0 {
                continue;
            }
            // Determine whether to keep or skip this mode
            let percentile_hit = if j == 0 {
                percentiles[i] <= always_cutoff
            } else {
                percentiles[i] <= mode_cutoff
            };
            if j != 3 && !percentile_hit {
                continue;
            }
            // Allocate and initialize the decimation table entry if we've not used it yet
            let key = (m.y * 16 + m.x) as usize;
            let mut decimation_mode = decimation_mode_index[key];
            if decimation_mode < 0 {
                construct_dt_entry_2d(texels_x, texels_y, m.x, m.y, bsd, &mut wb, packed_dm_idx);
                decimation_mode_index[key] = packed_dm_idx as i32;
                decimation_mode = packed_dm_idx as i32;
                dm_counts[j] += 1;
                packed_dm_idx += 1;
            }
            let bm = &mut bsd.block_modes[packed_bm_idx];
            bm.decimation_mode = decimation_mode as u8;
            bm.quant_mode = m.quant as u8;
            bm.is_dual_plane = m.dual;
            bm.weight_bits = m.bits as u8;
            bm.mode_index = i as u16;
            let quant = bm.get_weight_quant_mode();
            let dm = &mut bsd.decimation_modes[decimation_mode as usize];
            if m.dual {
                dm.set_ref_2plane(quant);
            } else {
                dm.set_ref_1plane(quant);
            }
            bsd.block_mode_packed_index[i] = packed_bm_idx as u16;
            packed_bm_idx += 1;
            bm_counts[j] += 1;
        }
    }

    bsd.block_mode_count_1plane_always = bm_counts[0];
    bsd.block_mode_count_1plane_selected = bm_counts[0] + bm_counts[1];
    bsd.block_mode_count_1plane_2plane_selected = bm_counts[0] + bm_counts[1] + bm_counts[2];
    bsd.block_mode_count_all = bm_counts[0] + bm_counts[1] + bm_counts[2] + bm_counts[3];

    bsd.decimation_mode_count_always = dm_counts[0];
    bsd.decimation_mode_count_selected = dm_counts[0] + dm_counts[1] + dm_counts[2];
    bsd.decimation_mode_count_all = dm_counts[0] + dm_counts[1] + dm_counts[2] + dm_counts[3];

    // Ensure the end of the array contains valid data (should never get read)
    for i in bsd.decimation_mode_count_all as usize..WEIGHTS_MAX_DECIMATION_MODES {
        bsd.decimation_modes[i] = DecimationMode {
            maxprec_1plane: -1,
            maxprec_2planes: -1,
            refprec_1plane: 0,
            refprec_2planes: 0,
        };
    }

    // Determine the texels to use for kmeans clustering.
    assign_kmeans_texels(bsd);
}

#[isa]
/// `construct_block_size_descriptor_3d`.
fn construct_block_size_descriptor_3d(
    texels_x: u32,
    texels_y: u32,
    texels_z: u32,
    bsd: &mut BlockSizeDescriptor,
) {
    // Store a remap table for storing packed decimation modes.
    // Indexing uses [Z * 64 + Y *  8 + X] and max size for each axis is 6.
    const MAX_DMI: usize = 6 * 64 + 6 * 8 + 6;
    let mut decimation_mode_index = [-1i32; MAX_DMI];
    let mut decimation_mode_count = 0usize;
    let mut wb = DtInitWorkingBuffers::new();

    bsd.dim_x = texels_x as u8;
    bsd.dim_y = texels_y as u8;
    bsd.dim_z = texels_z as u8;
    bsd.texel_count = (texels_x * texels_y * texels_z) as u8;

    // gather all the infill-modes that can be used with the current block size
    for weights_x in 2..=texels_x {
        for weights_y in 2..=texels_y {
            for weights_z in 2..=texels_z {
                let weight_count = weights_x * weights_y * weights_z;
                if weight_count as usize > BLOCK_MAX_WEIGHTS {
                    continue;
                }
                decimation_mode_index[(weights_z * 64 + weights_y * 8 + weights_x) as usize] =
                    decimation_mode_count as i32;
                init_decimation_info_3d(
                    (texels_x, texels_y, texels_z),
                    (weights_x, weights_y, weights_z),
                    &mut bsd.decimation_tables[decimation_mode_count],
                    &mut wb,
                );
                let (maxprec_1plane, mut maxprec_2planes) = maxprecs(weight_count, true);
                if (2 * weight_count) as usize > BLOCK_MAX_WEIGHTS {
                    maxprec_2planes = -1;
                }
                bsd.decimation_modes[decimation_mode_count] = DecimationMode {
                    maxprec_1plane: maxprec_1plane as i8,
                    maxprec_2planes: maxprec_2planes as i8,
                    refprec_1plane: if maxprec_1plane == -1 { 0 } else { 0xFFFF },
                    refprec_2planes: if maxprec_2planes == -1 { 0 } else { 0xFFFF },
                };
                decimation_mode_count += 1;
            }
        }
    }

    // Ensure the end of the array contains valid data (should never get read)
    for i in decimation_mode_count..WEIGHTS_MAX_DECIMATION_MODES {
        bsd.decimation_modes[i] = DecimationMode {
            maxprec_1plane: -1,
            maxprec_2planes: -1,
            refprec_1plane: 0,
            refprec_2planes: 0,
        };
    }

    bsd.decimation_mode_count_always = 0; // Skipped for 3D modes
    bsd.decimation_mode_count_selected = decimation_mode_count as u32;
    bsd.decimation_mode_count_all = decimation_mode_count as u32;

    // Construct the list of block formats referencing the decimation tables
    bsd.block_mode_packed_index.fill(BLOCK_BAD_BLOCK_MODE);

    let mut packed_idx = 0usize;
    let mut bm_counts = [0u32; 2];

    // Iterate two times to build a usefully ordered list:
    //   - Pass 0 - keep valid single plane block modes
    //   - Pass 1 - keep valid dual plane block modes
    for j in 0..2 {
        for i in 0..WEIGHTS_MAX_BLOCK_MODES {
            // Skip modes we've already included in a previous pass
            if bsd.block_mode_packed_index[i] != BLOCK_BAD_BLOCK_MODE {
                continue;
            }
            let Some(m) = decode_block_mode_3d(i as u32) else {
                continue;
            };
            if m.x > texels_x || m.y > texels_y || m.z > texels_z {
                continue;
            }
            // Skip modes that are invalid, too large, or not selected by the heuristic
            if (j == 0 && m.dual) || (j == 1 && !m.dual) {
                continue;
            }
            // Always skip encodings we can't physically encode based on
            // generic encoding bit availability
            let limit_bits = if m.dual { 109u32 } else { 111 };
            if limit_bits.wrapping_sub(m.bits) == 0 {
                continue;
            }
            let decimation_mode = decimation_mode_index[(m.z * 64 + m.y * 8 + m.x) as usize];
            let bm = &mut bsd.block_modes[packed_idx];
            bm.decimation_mode = decimation_mode as u8;
            bm.quant_mode = m.quant as u8;
            bm.weight_bits = m.bits as u8;
            bm.is_dual_plane = m.dual;
            bm.mode_index = i as u16;
            bsd.block_mode_packed_index[i] = packed_idx as u16;
            bm_counts[j] += 1;
            packed_idx += 1;
        }
    }

    bsd.block_mode_count_1plane_always = 0; // Skipped for 3D modes
    bsd.block_mode_count_1plane_selected = bm_counts[0];
    bsd.block_mode_count_1plane_2plane_selected = bm_counts[0] + bm_counts[1];
    bsd.block_mode_count_all = bm_counts[0] + bm_counts[1];

    // Determine the texels to use for kmeans clustering.
    assign_kmeans_texels(bsd);
}

#[isa]
/// `init_block_size_descriptor`.
pub fn init_block_size_descriptor(
    texels_x: u32,
    texels_y: u32,
    texels_z: u32,
    can_omit_modes: bool,
    partition_count_cutoff: u32,
    mode_cutoff: f32,
    bsd: &mut BlockSizeDescriptor,
) {
    if texels_z > 1 {
        construct_block_size_descriptor_3d(texels_x, texels_y, texels_z, bsd);
    } else {
        construct_block_size_descriptor_2d(texels_x, texels_y, can_omit_modes, mode_cutoff, bsd);
    }
    init_partition_tables(bsd, can_omit_modes, partition_count_cutoff);
}
