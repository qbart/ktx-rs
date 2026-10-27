// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of `astcenc_partition_tables.cpp` (compressor build): partitioning tables with
//! duplicate / degenerate partitioning removal and k-means coverage bitmaps.

#![allow(clippy::needless_range_loop)]

use super::internal::*;
use super::select_partition;
use super::vecmath::round_up_to_simd_multiple_vla;
use ktx_rs_macros::isa;

/// `BIT_PATTERN_WORDS`.
const BIT_PATTERN_WORDS: usize = (BLOCK_MAX_TEXELS * 2).div_ceil(64);

#[isa]
/// `generate_canonical_partitioning`.
fn generate_canonical_partitioning(
    texel_count: usize,
    partition_of_texel: &[u8],
    bit_pattern: &mut [u64; BIT_PATTERN_WORDS],
) {
    bit_pattern.fill(0);
    // Store a mapping to reorder the raw partitions so that the partitions are ordered such
    // that the lowest texel index in partition N is smaller than the lowest texel index in
    // partition N + 1.
    let mut mapped_index = [-1i32; BLOCK_MAX_PARTITIONS];
    let mut map_weight_count = 0;
    for i in 0..texel_count {
        let index = partition_of_texel[i] as usize;
        if mapped_index[index] < 0 {
            mapped_index[index] = map_weight_count;
            map_weight_count += 1;
        }
        let xlat_index = mapped_index[index] as u64;
        bit_pattern[i >> 5] |= xlat_index << (2 * (i & 0x1F));
    }
}

#[isa]
/// `generate_one_partition_info_entry`. Returns whether the partitioning has the
/// requested number of non-empty partitions.
fn generate_one_partition_info_entry(
    bsd: &mut BlockSizeDescriptor,
    partition_count: u32,
    partition_index: u32,
    partition_remap_index: usize,
    pi_index: usize,
) -> bool {
    let texels_per_block = bsd.texel_count as usize;
    let small_block = texels_per_block < 32;
    let (dim_x, dim_y, dim_z) = (bsd.dim_x as u32, bsd.dim_y as u32, bsd.dim_z as u32);

    let pi = &mut bsd.partitionings[pi_index];
    // Assign texels to partitions
    let mut texel_idx = 0usize;
    let mut counts = [0usize; BLOCK_MAX_PARTITIONS];
    for z in 0..dim_z {
        for y in 0..dim_y {
            for x in 0..dim_x {
                let part = select_partition(partition_index, x, y, z, partition_count, small_block);
                pi.partition_of_texel[texel_idx] = part;
                pi.texels_of_partition[part as usize][counts[part as usize]] = texel_idx as u8;
                counts[part as usize] += 1;
                texel_idx += 1;
            }
        }
    }

    // Fill loop tail so we can overfetch later
    for i in 0..partition_count as usize {
        let ptex_count = counts[i];
        let ptex_count_simd = round_up_to_simd_multiple_vla(ptex_count);
        for j in ptex_count..ptex_count_simd {
            pi.texels_of_partition[i][j] =
                pi.texels_of_partition[i][ptex_count.wrapping_sub(1) % BLOCK_MAX_TEXELS];
        }
    }

    // Populate the actual procedural partition count
    pi.partition_count = if counts[0] == 0 {
        0
    } else if counts[1] == 0 {
        1
    } else if counts[2] == 0 {
        2
    } else if counts[3] == 0 {
        3
    } else {
        4
    };

    // Populate the partition index
    pi.partition_index = partition_index as u16;

    // Populate the coverage bitmaps for 2/3/4 partitions
    for i in 0..BLOCK_MAX_PARTITIONS {
        pi.partition_texel_count[i] = counts[i] as u8;
    }
    let valid = pi.partition_count as u32 == partition_count;

    let texels_to_process = (bsd.texel_count as usize).min(BLOCK_MAX_KMEANS_TEXELS);
    let pot = bsd.partitionings[pi_index].partition_of_texel;
    let kmeans = bsd.kmeans_texels;
    let bitmaps: Option<&mut [u64]> = match partition_count {
        2 => Some(&mut bsd.coverage_bitmaps_2[partition_remap_index][..]),
        3 => Some(&mut bsd.coverage_bitmaps_3[partition_remap_index][..]),
        4 => Some(&mut bsd.coverage_bitmaps_4[partition_remap_index][..]),
        _ => None,
    };
    if let Some(bitmaps) = bitmaps {
        bitmaps.fill(0);
        for i in 0..texels_to_process {
            let idx = kmeans[i] as usize;
            bitmaps[pot[idx] as usize] |= 1u64 << i;
        }
    }
    valid
}

#[isa]
/// `build_partition_table_for_one_partition_count`.
fn build_partition_table_for_one_partition_count(
    bsd: &mut BlockSizeDescriptor,
    can_omit_partitionings: bool,
    partition_count_cutoff: u32,
    partition_count: u32,
    ptab: usize,
    canonical_patterns: &mut [[u64; BIT_PATTERN_WORDS]],
) {
    let pc = partition_count as usize;
    let mut next_index = 0usize;
    bsd.partitioning_count_selected[pc - 1] = 0;
    bsd.partitioning_count_all[pc - 1] = 0;

    // Skip tables larger than config max partition count if we can omit modes
    if can_omit_partitionings && partition_count > partition_count_cutoff {
        // Packed index is filled in only for cases that are not culled
        bsd.partitioning_packed_index[pc - 2].fill(BLOCK_BAD_PARTITIONING);
        return;
    }
    bsd.partitioning_packed_index[pc - 2].fill(BLOCK_BAD_PARTITIONING);

    // Iterate through twice
    //   - Pass 0: Keep selected partitionings
    //   - Pass 1: Keep non-selected partitionings (skip if in omit mode)
    let max_iter = if can_omit_partitionings { 1 } else { 2 };

    // Tracker for things we built in the first iteration
    let mut build = [false; BLOCK_MAX_PARTITIONINGS];
    for x in 0..max_iter {
        for i in 0..BLOCK_MAX_PARTITIONINGS {
            // Don't include things we built in the first pass
            if x == 1 && build[i] {
                continue;
            }
            let keep_useful = generate_one_partition_info_entry(
                bsd,
                partition_count,
                i as u32,
                next_index,
                ptab + next_index,
            );
            if x == 0 && !keep_useful {
                continue;
            }
            let mut pattern = [0u64; BIT_PATTERN_WORDS];
            generate_canonical_partitioning(
                bsd.texel_count as usize,
                &bsd.partitionings[ptab + next_index].partition_of_texel,
                &mut pattern,
            );
            canonical_patterns[next_index] = pattern;
            let keep_canonical = !canonical_patterns[..next_index].contains(&pattern);

            // Decide if we keep this one
            if keep_useful && keep_canonical {
                if x == 0 {
                    bsd.partitioning_packed_index[pc - 2][i] = next_index as u16;
                    bsd.partitioning_count_selected[pc - 1] += 1;
                    bsd.partitioning_count_all[pc - 1] += 1;
                    build[i] = true;
                    next_index += 1;
                }
            } else if x == 1 {
                bsd.partitioning_packed_index[pc - 2][i] = next_index as u16;
                bsd.partitioning_count_all[pc - 1] += 1;
                next_index += 1;
            }
        }
    }
}

#[isa]
/// `init_partition_tables`.
pub fn init_partition_tables(
    bsd: &mut BlockSizeDescriptor,
    can_omit_partitionings: bool,
    partition_count_cutoff: u32,
) {
    let par_tab2 = 0;
    let par_tab3 = par_tab2 + BLOCK_MAX_PARTITIONINGS;
    let par_tab4 = par_tab3 + BLOCK_MAX_PARTITIONINGS;
    let par_tab1 = par_tab4 + BLOCK_MAX_PARTITIONINGS;

    generate_one_partition_info_entry(bsd, 1, 0, 0, par_tab1);
    bsd.partitioning_count_selected[0] = 1;
    bsd.partitioning_count_all[0] = 1;

    let mut canonical_patterns = vec![[0u64; BIT_PATTERN_WORDS]; BLOCK_MAX_PARTITIONINGS];
    build_partition_table_for_one_partition_count(
        bsd,
        can_omit_partitionings,
        partition_count_cutoff,
        2,
        par_tab2,
        &mut canonical_patterns,
    );
    build_partition_table_for_one_partition_count(
        bsd,
        can_omit_partitionings,
        partition_count_cutoff,
        3,
        par_tab3,
        &mut canonical_patterns,
    );
    build_partition_table_for_one_partition_count(
        bsd,
        can_omit_partitionings,
        partition_count_cutoff,
        4,
        par_tab4,
        &mut canonical_patterns,
    );
}
