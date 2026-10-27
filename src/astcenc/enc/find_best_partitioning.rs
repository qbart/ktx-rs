// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of `astcenc_find_best_partitioning.cpp`: k-means clustering of the block texels, the
//! partitioning ordering by coverage mismatch and the candidate selection by line errors.

#![allow(clippy::needless_range_loop)]

use super::averages_and_directions::*;
use super::internal::*;
use super::vecmath::*;
use ktx_rs_macros::isa;

#[isa]
/// `unit3()` / `unit4()`.
#[allow(clippy::excessive_precision)]
fn unit3() -> Vfloat4 {
    let v = 0.577350258827209473f32;
    Vfloat4::new(v, v, v, 0.0)
}

#[isa]
fn unit4() -> Vfloat4 {
    Vfloat4::splat(0.5)
}

#[isa]
/// `kmeans_init`.
fn kmeans_init(
    blk: &ImageBlock,
    texel_count: usize,
    partition_count: usize,
    cluster_centers: &mut [Vfloat4; 4],
) {
    let mut clusters_selected = 0usize;
    let mut distances = [0.0f32; BLOCK_MAX_TEXELS];

    // Pick a random sample as first cluster center; 145897 from random.org
    let mut sample = 145897 % texel_count;
    let mut center_color = blk.texel(sample);
    cluster_centers[clusters_selected] = center_color;
    clusters_selected += 1;

    // Compute the distance to the first cluster center
    let mut distance_sum = 0.0f32;
    for i in 0..texel_count {
        let diff = blk.texel(i) - center_color;
        let distance = (diff * diff).dot_s(blk.channel_weight);
        distance_sum += distance;
        distances[i] = distance;
    }

    // More numbers from random.org for weighted-random center selection
    const CLUSTER_CUTOFFS: [f32; 9] = [
        0.626220, 0.932770, 0.275454, 0.318558, 0.240113, 0.009190, 0.347661, 0.731960, 0.156391,
    ];
    let mut cutoff = (clusters_selected - 1) + 3 * (partition_count - 2);

    // Pick the remaining samples as needed
    loop {
        // Pick the next center in a weighted-random fashion.
        let mut summa = 0.0f32;
        let distance_cutoff = distance_sum * CLUSTER_CUTOFFS[cutoff.min(8)];
        cutoff += 1;
        sample = 0;
        while sample < texel_count {
            summa += distances[sample];
            if summa >= distance_cutoff {
                break;
            }
            sample += 1;
        }
        // Clamp to a valid range and store the selected cluster center
        sample = sample.min(texel_count - 1);
        center_color = blk.texel(sample);
        cluster_centers[clusters_selected] = center_color;
        clusters_selected += 1;
        if clusters_selected >= partition_count {
            break;
        }
        // Compute the distance to the new cluster center, keep the min distance
        distance_sum = 0.0;
        for i in 0..texel_count {
            let diff = blk.texel(i) - center_color;
            let mut distance = (diff * diff).dot_s(blk.channel_weight);
            distance = super::mathlib::min(distance, distances[i]);
            distance_sum += distance;
            distances[i] = distance;
        }
    }
}

#[isa]
/// `kmeans_assign`.
fn kmeans_assign(
    blk: &ImageBlock,
    texel_count: usize,
    partition_count: usize,
    cluster_centers: &[Vfloat4; 4],
    partition_of_texel: &mut [u8; BLOCK_MAX_TEXELS],
) {
    let mut partition_texel_count = [0u8; BLOCK_MAX_PARTITIONS];
    // Find the best partition for every texel
    for i in 0..texel_count {
        let mut best_distance = f32::MAX;
        let mut best_partition = 0usize;
        let color = blk.texel(i);
        for j in 0..partition_count {
            let diff = color - cluster_centers[j];
            let distance = (diff * diff).dot_s(blk.channel_weight);
            if distance < best_distance {
                best_distance = distance;
                best_partition = j;
            }
        }
        partition_of_texel[i] = best_partition as u8;
        partition_texel_count[best_partition] =
            partition_texel_count[best_partition].wrapping_add(1);
    }

    // It is possible to get a situation where a partition ends up without any texels. In this
    // case, assign texel N to partition N. This is silly, but ensures that every partition
    // retains at least one texel. Reassigning a texel in this manner may cause another
    // partition to go empty, so if we actually did a reassignment, run the whole loop over again.
    loop {
        let mut problem_case = false;
        for i in 0..partition_count {
            if partition_texel_count[i] == 0 {
                let p = partition_of_texel[i] as usize;
                partition_texel_count[p] = partition_texel_count[p].wrapping_sub(1);
                partition_texel_count[i] = partition_texel_count[i].wrapping_add(1);
                partition_of_texel[i] = i as u8;
                problem_case = true;
            }
        }
        if !problem_case {
            break;
        }
    }
}

#[isa]
/// `kmeans_update`.
fn kmeans_update(
    blk: &ImageBlock,
    texel_count: usize,
    partition_count: usize,
    cluster_centers: &mut [Vfloat4; 4],
    partition_of_texel: &[u8; BLOCK_MAX_TEXELS],
) {
    let mut color_sum = [Vfloat4::zero(); BLOCK_MAX_PARTITIONS];
    let mut partition_texel_count = [0u8; BLOCK_MAX_PARTITIONS];
    // Find the center-of-gravity in each cluster
    for i in 0..texel_count {
        let partition = partition_of_texel[i] as usize & 3;
        color_sum[partition] += blk.texel(i);
        partition_texel_count[partition] = partition_texel_count[partition].wrapping_add(1);
    }
    // Set the center of gravity to be the new cluster center
    for i in 0..partition_count {
        let scale = 1.0 / partition_texel_count[i] as f32;
        cluster_centers[i] = color_sum[i] * scale;
    }
}

#[isa]
/// `partition_mismatch2`.
fn partition_mismatch2(a: &[u64], b: &[u64; 2]) -> u8 {
    let v1 = popcount(a[0] ^ b[0]) + popcount(a[1] ^ b[1]);
    let v2 = popcount(a[0] ^ b[1]) + popcount(a[1] ^ b[0]);
    (v1.min(v2) / 2) as u8
}

#[isa]
/// `partition_mismatch3`.
fn partition_mismatch3(a: &[u64], b: &[u64; 3]) -> u8 {
    let p = |i: usize, j: usize| popcount(a[i] ^ b[j]);
    let v0 = (p(1, 1) + p(2, 2)).min(p(1, 2) + p(2, 1)) + p(0, 0);
    let v1 = (p(1, 0) + p(2, 2)).min(p(1, 2) + p(2, 0)) + p(0, 1);
    let v2 = (p(1, 0) + p(2, 1)).min(p(1, 1) + p(2, 0)) + p(0, 2);
    (v0.min(v1).min(v2) / 2) as u8
}

#[isa]
/// `partition_mismatch4`.
fn partition_mismatch4(a: &[u64], b: &[u64; 4]) -> u8 {
    let p = |i: usize, j: usize| popcount(a[i] ^ b[j]);
    let mx23 = (p(2, 2) + p(3, 3)).min(p(2, 3) + p(3, 2));
    let mx13 = (p(2, 1) + p(3, 3)).min(p(2, 3) + p(3, 1));
    let mx12 = (p(2, 1) + p(3, 2)).min(p(2, 2) + p(3, 1));
    let mx03 = (p(2, 0) + p(3, 3)).min(p(2, 3) + p(3, 0));
    let mx02 = (p(2, 0) + p(3, 2)).min(p(2, 2) + p(3, 0));
    let mx01 = (p(2, 1) + p(3, 0)).min(p(2, 0) + p(3, 1));
    let v0 = p(0, 0) + (p(1, 1) + mx23).min(p(1, 2) + mx13).min(p(1, 3) + mx12);
    let v1 = p(0, 1) + (p(1, 0) + mx23).min(p(1, 2) + mx03).min(p(1, 3) + mx02);
    let v2 = p(0, 2) + (p(1, 1) + mx03).min(p(1, 0) + mx13).min(p(1, 3) + mx01);
    let v3 = p(0, 3) + (p(1, 1) + mx02).min(p(1, 2) + mx01).min(p(1, 0) + mx12);
    (v0.min(v1).min(v2.min(v3)) / 2) as u8
}

#[isa]
/// `count_partition_mismatch_bits`.
fn count_partition_mismatch_bits(
    bsd: &BlockSizeDescriptor,
    partition_count: usize,
    bitmaps: &[u64; 4],
    mismatch_counts: &mut [u8; BLOCK_MAX_PARTITIONINGS],
) {
    let active_count = bsd.partitioning_count_selected[partition_count - 1] as usize;
    for i in 0..active_count {
        mismatch_counts[i] = match partition_count {
            2 => partition_mismatch2(bitmaps, &bsd.coverage_bitmaps_2[i]),
            3 => partition_mismatch3(bitmaps, &bsd.coverage_bitmaps_3[i]),
            _ => partition_mismatch4(bitmaps, &bsd.coverage_bitmaps_4[i]),
        };
    }
}

#[isa]
/// `get_partition_ordering_by_mismatch_bits` (counting sort by mismatch count).
fn get_partition_ordering_by_mismatch_bits(
    texel_count: usize,
    partitioning_count: usize,
    mismatch_count: &[u8; BLOCK_MAX_PARTITIONINGS],
    partition_ordering: &mut [u16; BLOCK_MAX_PARTITIONINGS],
) -> usize {
    let mut mscount = [0u16; BLOCK_MAX_KMEANS_TEXELS];
    // Create the histogram of mismatch counts
    for i in 0..partitioning_count {
        let m = (mismatch_count[i] as usize).min(BLOCK_MAX_KMEANS_TEXELS - 1);
        mscount[m] = mscount[m].wrapping_add(1);
    }
    // Create a running sum from the histogram array
    // Cells store previous values only; i.e. exclude self after sum
    let mut sum = 0u16;
    for i in 0..texel_count.min(BLOCK_MAX_KMEANS_TEXELS) {
        let cnt = mscount[i];
        mscount[i] = sum;
        sum = sum.wrapping_add(cnt);
    }
    // Use the running sum as the index, incrementing after read to allow sequential entries
    // with the same count
    for i in 0..partitioning_count {
        let m = (mismatch_count[i] as usize).min(BLOCK_MAX_KMEANS_TEXELS - 1);
        let idx = mscount[m] as usize;
        mscount[m] = mscount[m].wrapping_add(1);
        if let Some(o) = partition_ordering.get_mut(idx) {
            *o = i as u16;
        }
    }
    partitioning_count
}

#[isa]
/// `compute_kmeans_partition_ordering`.
fn compute_kmeans_partition_ordering(
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    partition_count: usize,
    partition_ordering: &mut [u16; BLOCK_MAX_PARTITIONINGS],
) -> usize {
    let mut cluster_centers = [Vfloat4::zero(); 4];
    let mut texel_partitions = [0u8; BLOCK_MAX_TEXELS];
    let texel_count = bsd.texel_count as usize;

    // Use three passes of k-means clustering to partition the block data
    for i in 0..3 {
        if i == 0 {
            kmeans_init(blk, texel_count, partition_count, &mut cluster_centers);
        } else {
            kmeans_update(
                blk,
                texel_count,
                partition_count,
                &mut cluster_centers,
                &texel_partitions,
            );
        }
        kmeans_assign(
            blk,
            texel_count,
            partition_count,
            &cluster_centers,
            &mut texel_partitions,
        );
    }

    // Construct the block bitmaps of texel assignments to each partition
    let mut bitmaps = [0u64; BLOCK_MAX_PARTITIONS];
    let texels_to_process = texel_count.min(BLOCK_MAX_KMEANS_TEXELS);
    for i in 0..texels_to_process {
        let idx = bsd.kmeans_texels[i] as usize;
        bitmaps[texel_partitions[idx] as usize & 3] |= 1u64 << i;
    }

    // Count the mismatch between the block and the format's partition tables
    let mut mismatch_counts = [0u8; BLOCK_MAX_PARTITIONINGS];
    count_partition_mismatch_bits(bsd, partition_count, &bitmaps, &mut mismatch_counts);

    // Sort the partitions based on the number of mismatched bits
    get_partition_ordering_by_mismatch_bits(
        texels_to_process,
        bsd.partitioning_count_selected[partition_count - 1] as usize,
        &mismatch_counts,
        partition_ordering,
    )
}

#[isa]
/// `insert_result`.
fn insert_result(
    max_values: usize,
    this_error: f32,
    this_partition: u32,
    best_errors: &mut [f32],
    best_partitions: &mut [u32],
) {
    // Don't bother searching if the current worst error beats the new error
    if this_error >= best_errors[max_values - 1] {
        return;
    }
    // Else insert into the list in error-order
    for i in 0..max_values {
        // Existing result is better - move on ...
        if this_error > best_errors[i] {
            continue;
        }
        // Move existing results down one
        let mut j = max_values - 1;
        while j > i {
            best_errors[j] = best_errors[j - 1];
            best_partitions[j] = best_partitions[j - 1];
            j -= 1;
        }
        // Insert new result
        best_errors[i] = this_error;
        best_partitions[i] = this_partition;
        break;
    }
}

#[isa]
/// `find_best_partition_candidates`: returns the number of candidates written to
/// `best_partitions` (partition indices, not packed indices).
pub fn find_best_partition_candidates(
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    partition_count: u32,
    partition_search_limit: u32,
    best_partitions: &mut [u32; TUNE_MAX_PARTITIONING_CANDIDATES],
    requested_candidates: u32,
) -> u32 {
    // Constant used to estimate quantization error for a given partitioning; the optimal value
    // for this depends on bitrate. These values have been determined empirically.
    let texels_per_block = bsd.texel_count as u32;
    let mut weight_imprecision_estim = 0.055f32;
    if texels_per_block <= 20 {
        weight_imprecision_estim = 0.03;
    } else if texels_per_block <= 31 {
        weight_imprecision_estim = 0.04;
    } else if texels_per_block <= 41 {
        weight_imprecision_estim = 0.05;
    }
    weight_imprecision_estim *= weight_imprecision_estim;

    let pc = partition_count as usize;
    let mut partition_sequence = [0u16; BLOCK_MAX_PARTITIONINGS];
    let sequence_len =
        compute_kmeans_partition_ordering(bsd, blk, pc, &mut partition_sequence) as u32;
    let partition_search_limit = partition_search_limit.min(sequence_len) as usize;
    let requested_candidates = (partition_search_limit as u32)
        .min(requested_candidates)
        .min(TUNE_MAX_PARTITIONING_CANDIDATES as u32) as usize;

    let uses_alpha = !blk.is_constant_channel(3);

    // Partitioning errors assuming uncorrelated-chrominance endpoints
    let mut uncor_best_errors = [0.0f32; TUNE_MAX_PARTITIONING_CANDIDATES];
    let mut uncor_best_partitions = [0u32; TUNE_MAX_PARTITIONING_CANDIDATES];
    // Partitioning errors assuming same-chrominance endpoints
    let mut samec_best_errors = [0.0f32; TUNE_MAX_PARTITIONING_CANDIDATES];
    let mut samec_best_partitions = [0u32; TUNE_MAX_PARTITIONING_CANDIDATES];
    for i in 0..requested_candidates {
        uncor_best_errors[i] = ERROR_CALC_DEFAULT;
        samec_best_errors[i] = ERROR_CALC_DEFAULT;
    }

    for i in 0..partition_search_limit {
        let partition = partition_sequence[i] as u32;
        let pi = bsd.get_raw_partition_info(partition_count, partition);
        let mut pms = [PartitionMetrics::default(); 4];
        let (mut uncor_error, mut samec_error) = (0.0f32, 0.0f32);

        if uses_alpha {
            // Compute the error for the partitioning with alpha
            compute_avgs_and_dirs_4_comp(pi, blk, &mut pms);
            let mut uncor_lines = [Line::default(); 4];
            let mut samec_lines = [Line::default(); 4];
            let mut uncor_plines = [ProcessedLine::default(); 4];
            let mut samec_plines = [ProcessedLine::default(); 4];
            let mut line_lengths = [0.0f32; 4];
            for j in 0..pc {
                let pm = &pms[j];
                uncor_lines[j].a = pm.avg;
                uncor_lines[j].b = normalize_safe(pm.dir, unit4());
                uncor_plines[j].amod =
                    uncor_lines[j].a - uncor_lines[j].b * uncor_lines[j].a.dot(uncor_lines[j].b);
                uncor_plines[j].bs = uncor_lines[j].b;
                samec_lines[j].a = Vfloat4::zero();
                samec_lines[j].b = normalize_safe(pm.avg, unit4());
                samec_plines[j].amod = Vfloat4::zero();
                samec_plines[j].bs = samec_lines[j].b;
            }
            compute_error_squared_rgba(
                pi,
                blk,
                &uncor_plines,
                &samec_plines,
                &mut line_lengths,
                &mut uncor_error,
                &mut samec_error,
            );

            // Compute an estimate of error introduced by weight quantization imprecision.
            // This error is computed as follows, for each partition
            //     1: compute the principal-axis vector (full length) in error-space
            //     2: convert the principal-axis vector to regular RGB-space
            //     3: scale the vector by a constant that estimates average quantization error
            //     4: for each texel, square the vector, then do a dot-product with the texel's
            //        error weight; sum up the results across all texels.
            //     4(optimized): square the vector once, then do a dot-product with the average
            //        texel error, then multiply by the number of texels.
            for j in 0..pc {
                let tpp = pi.partition_texel_count[j] as f32;
                let error_weights = Vfloat4::splat(tpp * weight_imprecision_estim);
                let uncor_vector = uncor_lines[j].b * line_lengths[j];
                let samec_vector = samec_lines[j].b * line_lengths[j];
                uncor_error += (uncor_vector * uncor_vector).dot_s(error_weights);
                samec_error += (samec_vector * samec_vector).dot_s(error_weights);
            }
        } else {
            compute_avgs_and_dirs_3_comp_rgb(pi, blk, &mut pms);
            let mut plines = [PartitionLines3::default(); 4];
            for j in 0..pc {
                let pm = &pms[j];
                let pl = &mut plines[j];
                pl.uncor_line.a = pm.avg;
                pl.uncor_line.b = normalize_safe(pm.dir, unit3());
                pl.samec_line.a = Vfloat4::zero();
                pl.samec_line.b = normalize_safe(pm.avg, unit3());
                pl.uncor_pline.amod =
                    pl.uncor_line.a - pl.uncor_line.b * pl.uncor_line.a.dot3(pl.uncor_line.b);
                pl.uncor_pline.bs = pl.uncor_line.b;
                pl.samec_pline.amod = Vfloat4::zero();
                pl.samec_pline.bs = pl.samec_line.b;
            }
            compute_error_squared_rgb(pi, blk, &mut plines, &mut uncor_error, &mut samec_error);

            for j in 0..pc {
                let pl = &plines[j];
                let tpp = pi.partition_texel_count[j] as f32;
                let error_weights = Vfloat4::splat(tpp * weight_imprecision_estim);
                let uncor_vector = pl.uncor_line.b * pl.line_length;
                let samec_vector = pl.samec_line.b * pl.line_length;
                uncor_error += (uncor_vector * uncor_vector).dot3_s(error_weights);
                samec_error += (samec_vector * samec_vector).dot3_s(error_weights);
            }
        }

        insert_result(
            requested_candidates,
            uncor_error,
            partition,
            &mut uncor_best_errors,
            &mut uncor_best_partitions,
        );
        insert_result(
            requested_candidates,
            samec_error,
            partition,
            &mut samec_best_errors,
            &mut samec_best_partitions,
        );
    }

    let mut interleave = [0u32; 2 * TUNE_MAX_PARTITIONING_CANDIDATES];
    for i in 0..requested_candidates {
        interleave[2 * i] = bsd
            .get_raw_partition_info(partition_count, uncor_best_partitions[i])
            .partition_index as u32;
        interleave[2 * i + 1] = bsd
            .get_raw_partition_info(partition_count, samec_best_partitions[i])
            .partition_index as u32;
    }

    let mut bitmasks = [0u64; 1024 / 64];
    let mut emitted = 0usize;
    for i in 0..requested_candidates * 2 {
        let partition = interleave[i];
        let word = (partition / 64) as usize & 15;
        let bit = partition % 64;
        let written = bitmasks[word] & (1u64 << bit) != 0;
        if !written {
            best_partitions[emitted] = partition;
            bitmasks[word] |= 1u64 << bit;
            emitted += 1;
            if emitted == requested_candidates {
                break;
            }
        }
    }
    emitted as u32
}
