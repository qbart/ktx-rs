// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of `astcenc_compress_symbolic.cpp`: the per block search over partition counts,
//! planes and block modes, with weight realignment and refinement.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::color_quantize::pack_color_endpoints;
use super::decompress_symbolic::*;
use super::find_best_partitioning::find_best_partition_candidates;
use super::ideal_endpoints_and_weights::*;
use super::internal::*;
use super::mathlib;
use super::pick_best_endpoint_format::compute_ideal_endpoint_formats;
use super::symbolic_physical::symbolic_to_physical;
use super::tables::QUANT_AND_XFER_TABLES;
use super::vecmath::*;
use super::weight_align::{compute_angular_endpoints_1plane, compute_angular_endpoints_2planes};
use super::{ASTCENC_FLG_MAP_RGBM, AstcencConfig, AstcencProfile};
use ktx_macros::isa;

#[isa]
/// `merge_endpoints`: merges two planes of endpoints into a single vector.
fn merge_endpoints(
    ep_plane1: &Endpoints,
    ep_plane2: &Endpoints,
    component_plane2: u32,
    result: &mut Endpoints,
) {
    let partition_count = ep_plane1.partition_count;
    let sep_mask = Vint4::lane_id().eq(Vint4::splat(component_plane2 as i32));

    result.partition_count = partition_count;
    result.endpt0[0] = Vfloat4::select(ep_plane1.endpt0[0], ep_plane2.endpt0[0], sep_mask);
    result.endpt1[0] = Vfloat4::select(ep_plane1.endpt1[0], ep_plane2.endpt1[0], sep_mask);
}

#[isa]
/// Unpacked endpoints of every partition of `scb`.
fn unpack_endpoints(
    decode_mode: AstcencProfile,
    scb: &SymbolicCompressedBlock,
) -> ([Vint4; BLOCK_MAX_PARTITIONS], [Vint4; BLOCK_MAX_PARTITIONS]) {
    let mut endpnt0 = [Vint4::zero(); BLOCK_MAX_PARTITIONS];
    let mut endpnt1 = [Vint4::zero(); BLOCK_MAX_PARTITIONS];
    for pa_idx in 0..scb.partition_count as usize {
        let (e0, e1, _, _) = unpack_color_endpoints(
            decode_mode,
            scb.color_formats[pa_idx],
            &scb.color_values[pa_idx],
        );
        endpnt0[pa_idx] = e0;
        endpnt1[pa_idx] = e1;
    }
    (endpnt0, endpnt1)
}

#[isa]
/// Per partition endpoint base and 1/64 step of a plane (`plane_mask` lanes zeroed).
fn plane_offsets(
    partition_count: usize,
    endpnt0: &[Vint4; BLOCK_MAX_PARTITIONS],
    endpnt1: &[Vint4; BLOCK_MAX_PARTITIONS],
    plane_mask: Vmask4,
) -> (
    [Vfloat4; BLOCK_MAX_PARTITIONS],
    [Vfloat4; BLOCK_MAX_PARTITIONS],
) {
    let mut endpnt0f = [Vfloat4::zero(); BLOCK_MAX_PARTITIONS];
    let mut offset = [Vfloat4::zero(); BLOCK_MAX_PARTITIONS];
    for pa_idx in 0..partition_count {
        let mut epd = endpnt1[pa_idx] - endpnt0[pa_idx];
        epd = Vint4::select(epd, Vint4::zero(), plane_mask);
        endpnt0f[pa_idx] = endpnt0[pa_idx].int_to_float();
        offset[pa_idx] = epd.int_to_float() * (1.0 / 64.0);
    }
    (endpnt0f, offset)
}

#[isa]
/// `realign_weights_undecimated`: tries moving each weight of an undecimated grid one
/// quantization step up or down.
fn realign_weights_undecimated(
    decode_mode: AstcencProfile,
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    scb: &mut SymbolicCompressedBlock,
) -> bool {
    let partition_count = scb.partition_count as usize;
    let pi = bsd.get_partition_info(partition_count as u32, u32::from(scb.partition_index));

    let bm = bsd.get_block_mode(scb.block_mode as usize);
    let qat = &QUANT_AND_XFER_TABLES[bm.quant_mode as usize];

    let max_plane = usize::from(bm.is_dual_plane);
    let mut plane_mask = Vint4::lane_id().eq(Vint4::splat(i32::from(scb.plane2_component)));

    // Decode the color endpoints
    let (endpnt0, endpnt1) = unpack_endpoints(decode_mode, scb);

    let mut adjustments = false;

    // For each plane and partition ...
    for pl_idx in 0..=max_plane {
        let (endpnt0f, offset) = plane_offsets(partition_count, &endpnt0, &endpnt1, plane_mask);
        let base = pl_idx * WEIGHTS_PLANE2_OFFSET;

        // For each weight compute previous, current, and next errors
        for texel in 0..bsd.texel_count as usize {
            let uqw = i32::from(scb.weights[base + texel]);

            let prev_and_next = u32::from(qat.prev_next_values[uqw as usize]);
            let uqw_down = (prev_and_next & 0xFF) as i32;
            let uqw_up = ((prev_and_next >> 8) & 0xFF) as i32;

            // Interpolate the colors to create the diffs
            let weight_base = uqw as f32;
            let weight_down = (uqw_down - uqw) as f32;
            let weight_up = (uqw_up - uqw) as f32;

            let partition = pi.partition_of_texel[texel] as usize;
            let color_offset = offset[partition];
            let color_base = endpnt0f[partition];

            let color = color_base + color_offset * weight_base;
            let orig_color = blk.texel(texel);
            let error_weight = blk.channel_weight;

            let color_diff = color - orig_color;
            let color_diff_down = color_diff + color_offset * weight_down;
            let color_diff_up = color_diff + color_offset * weight_up;

            let error_base = (color_diff * color_diff).dot_s(error_weight);
            let error_down = (color_diff_down * color_diff_down).dot_s(error_weight);
            let error_up = (color_diff_up * color_diff_up).dot_s(error_weight);

            // Check if the prev or next error is better, and if so use it
            if error_up < error_base && error_up < error_down && uqw < 64 {
                scb.weights[base + texel] = uqw_up as u8;
                adjustments = true;
            } else if error_down < error_base && uqw > 0 {
                scb.weights[base + texel] = uqw_down as u8;
                adjustments = true;
            }
        }

        // Prepare iteration for plane 2
        plane_mask = !plane_mask;
    }

    adjustments
}

#[isa]
/// `realign_weights_decimated`: tries moving each weight of a decimated grid one
/// quantization step up or down.
fn realign_weights_decimated(
    decode_mode: AstcencProfile,
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    scb: &mut SymbolicCompressedBlock,
) -> bool {
    let partition_count = scb.partition_count as usize;
    let pi = bsd.get_partition_info(partition_count as u32, u32::from(scb.partition_index));

    let bm = bsd.get_block_mode(scb.block_mode as usize);
    let qat = &QUANT_AND_XFER_TABLES[bm.quant_mode as usize];

    let di = bsd.get_decimation_info(bm.decimation_mode as usize);
    let weight_count = di.weight_count as usize;

    let max_plane = usize::from(bm.is_dual_plane);
    let mut plane_mask = Vint4::lane_id().eq(Vint4::splat(i32::from(scb.plane2_component)));

    // Decode the color endpoints
    let (endpnt0, endpnt1) = unpack_endpoints(decode_mode, scb);

    let mut adjustments = false;

    // For each plane and partition ...
    for pl_idx in 0..=max_plane {
        let (endpnt0f, offset) = plane_offsets(partition_count, &endpnt0, &endpnt1, plane_mask);
        let base = pl_idx * WEIGHTS_PLANE2_OFFSET;

        // Create an unquantized weight grid for this decimation level (in groups of 8)
        let mut uq_weightsf = [0.0f32; BLOCK_MAX_WEIGHTS];
        let mut we_idx = 0;
        while we_idx < weight_count {
            for k in we_idx..(we_idx + SIMD_WIDTH).min(BLOCK_MAX_WEIGHTS) {
                uq_weightsf[k] = f32::from(scb.weights.get(base + k).copied().unwrap_or(0));
            }
            we_idx += SIMD_WIDTH;
        }

        // For each weight compute previous, current, and next errors
        for we_idx in 0..weight_count {
            let uqw = i32::from(scb.weights[base + we_idx]);
            let prev_and_next = u32::from(qat.prev_next_values[uqw as usize]);

            let uqw_base = uq_weightsf[we_idx];
            let uqw_down = (prev_and_next & 0xFF) as f32;
            let uqw_up = ((prev_and_next >> 8) & 0xFF) as f32;

            let uqw_diff_down = uqw_down - uqw_base;
            let uqw_diff_up = uqw_up - uqw_base;

            let mut error_basev = Vfloat4::zero();
            let mut error_downv = Vfloat4::zero();
            let mut error_upv = Vfloat4::zero();

            // Interpolate the colors to create the diffs
            let texels_to_evaluate = di.weight_texel_count[we_idx] as usize;
            for te_idx in 0..texels_to_evaluate {
                let texel = di.weight_texels_tr[te_idx][we_idx] as usize;

                let tw_base = di.texel_contrib_for_weight[te_idx][we_idx];

                let w = |j: usize| uq_weightsf[di.texel_weights_tr[j][texel] as usize];
                let c = |j: usize| di.texel_weight_contribs_float_tr[j][texel];
                let weight_base = (w(0) * c(0) + w(1) * c(1)) + (w(2) * c(2) + w(3) * c(3));

                // Ideally this is integer rounded, but IQ gain it isn't worth the overhead
                let weight_down = weight_base + uqw_diff_down * tw_base - weight_base;
                let weight_up = weight_base + uqw_diff_up * tw_base - weight_base;

                let partition = pi.partition_of_texel[texel] as usize;
                let color_offset = offset[partition];
                let color_base = endpnt0f[partition];

                let color = color_base + color_offset * weight_base;
                let orig_color = blk.texel(texel);

                let color_diff = color - orig_color;
                let color_down_diff = color_diff + color_offset * weight_down;
                let color_up_diff = color_diff + color_offset * weight_up;

                error_basev += color_diff * color_diff;
                error_downv += color_down_diff * color_down_diff;
                error_upv += color_up_diff * color_up_diff;
            }

            let error_weight = blk.channel_weight;
            let error_base = (error_basev * error_weight).hadd_s();
            let error_down = (error_downv * error_weight).hadd_s();
            let error_up = (error_upv * error_weight).hadd_s();

            // Check if the prev or next error is better, and if so use it
            if error_up < error_base && error_up < error_down && uqw < 64 {
                uq_weightsf[we_idx] = uqw_up;
                scb.weights[base + we_idx] = uqw_up as u8;
                adjustments = true;
            } else if error_down < error_base && uqw > 0 {
                uq_weightsf[we_idx] = uqw_down;
                scb.weights[base + we_idx] = uqw_down as u8;
                adjustments = true;
            }
        }

        // Prepare iteration for plane 2
        plane_mask = !plane_mask;
    }

    adjustments
}

#[isa]
/// `compute_ideal_endpoint_formats` with the weight bit counts and errors of `tmpbuf`.
fn ideal_endpoint_formats(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    ep: &Endpoints,
    tune_candidate_limit: u32,
    start_block_mode: u32,
    end_block_mode: u32,
    partition_format_specifiers: &mut [[u8; BLOCK_MAX_PARTITIONS]; TUNE_MAX_TRIAL_CANDIDATES],
    block_mode: &mut [i32; TUNE_MAX_TRIAL_CANDIDATES],
    quant_level: &mut [u32; TUNE_MAX_TRIAL_CANDIDATES],
    quant_level_mod: &mut [u32; TUNE_MAX_TRIAL_CANDIDATES],
    tmpbuf: &mut CompressionWorkingBuffers,
) -> u32 {
    let qwt_bitcounts = std::mem::take(&mut tmpbuf.qwt_bitcounts);
    let qwt_errors = std::mem::take(&mut tmpbuf.qwt_errors);
    let count = compute_ideal_endpoint_formats(
        pi,
        blk,
        ep,
        &qwt_bitcounts,
        &qwt_errors,
        tune_candidate_limit,
        start_block_mode,
        end_block_mode,
        partition_format_specifiers,
        block_mode,
        quant_level,
        quant_level_mod,
        tmpbuf,
    );
    tmpbuf.qwt_bitcounts = qwt_bitcounts;
    tmpbuf.qwt_errors = qwt_errors;
    count
}

/// `compute_symbolic_block_difference_*` selector.
type DifferenceFn =
    fn(&AstcencConfig, &BlockSizeDescriptor, &SymbolicCompressedBlock, &ImageBlock) -> f32;

#[isa]
/// Smallest weight per lane that results in a color value greater than 1.
fn min_weight_cutoff(ep: &Endpoints, i: usize, min_ep: Vfloat4) -> Vfloat4 {
    let e = (Vfloat4::splat(1.0) - ep.endpt0[i]) / (ep.endpt1[i] - ep.endpt0[i]);
    let use_ep = e.gt(Vfloat4::splat(0.5)) & e.lt(min_ep);
    Vfloat4::select(min_ep, e, use_ep)
}

/// Outcome of one refinement trial: whether to continue refining, stop this candidate,
/// or stop the candidate search.
enum Trial {
    Continue,
    NextCandidate,
    Done,
}

#[isa]
/// Shared pre / post realignment test of the candidate loops.
fn trial_test(
    errorval: f32,
    threshold_base: f32,
    iters_remaining: u32,
    workscb: &mut SymbolicCompressedBlock,
    scb: &mut SymbolicCompressedBlock,
    best_errorval_in_mode: &mut f32,
    best_errorval_in_scb: &mut f32,
    tune_errorval_threshold: f32,
) -> Trial {
    let mut errorval = errorval;
    if errorval == -ERROR_CALC_DEFAULT {
        errorval = -errorval;
        workscb.block_type = SYM_BTYPE_ERROR;
    }

    *best_errorval_in_mode = mathlib::min(errorval, *best_errorval_in_mode);

    // Average refinement improvement is 3.5% per iteration, so skip blocks that are
    // unlikely to catch up with the best block we have already
    let threshold = (0.045 * iters_remaining as f32) + threshold_base;
    if errorval > (threshold * *best_errorval_in_scb) {
        return Trial::NextCandidate;
    }

    if errorval < *best_errorval_in_scb {
        *best_errorval_in_scb = errorval;
        workscb.errorval = errorval;
        *scb = *workscb;

        if errorval < tune_errorval_threshold {
            // Skip remaining candidates - this is "good enough"
            return Trial::Done;
        }
    }
    Trial::Continue
}

#[isa]
/// `compress_symbolic_block_for_partition_1plane`: compresses a block using a chosen
/// partitioning and 1 plane of weights.
fn compress_symbolic_block_for_partition_1plane(
    config: &AstcencConfig,
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    only_always: bool,
    tune_errorval_threshold: f32,
    partition_count: u32,
    partition_index: u32,
    scb: &mut SymbolicCompressedBlock,
    tmpbuf: &mut CompressionWorkingBuffers,
    quant_limit: i32,
) -> f32 {
    let max_weight_quant = (QUANT_32 as i32).min(quant_limit);

    let mut compute_difference: DifferenceFn = compute_symbolic_block_difference_1plane;
    if partition_count == 1 && config.flags & ASTCENC_FLG_MAP_RGBM == 0 {
        compute_difference = compute_symbolic_block_difference_1plane_1partition;
    }

    let pi = bsd.get_partition_info(partition_count, partition_index);

    // Compute ideal weights and endpoint colors, with no quantization or decimation
    compute_ideal_colors_and_weights_1plane(blk, pi, &mut tmpbuf.ei1);
    let ei = tmpbuf.ei1.clone();

    // For each decimation mode, compute an ideal set of weights with no quantization
    let max_decimation_modes = if only_always {
        bsd.decimation_mode_count_always
    } else {
        bsd.decimation_mode_count_selected
    } as usize;
    for i in 0..max_decimation_modes {
        let dm = bsd.get_decimation_mode(i);
        if !dm.is_ref_1plane(max_weight_quant as u32) {
            continue;
        }

        let di = bsd.get_decimation_info(i);
        compute_ideal_weights_for_decimation(
            &ei,
            di,
            &mut tmpbuf.dec_weights_ideal[i * BLOCK_MAX_WEIGHTS..],
        );
    }

    // Compute maximum colors for the endpoints and ideal weights, then for each endpoint and
    // ideal weight pair, compute the smallest weight that will result in a color value > 1
    let mut min_ep = Vfloat4::splat(10.0);
    for i in 0..partition_count as usize {
        min_ep = min_weight_cutoff(&ei.ep, i, min_ep);
    }

    let min_wt_cutoff = min_ep.hmin_s();

    // For each mode, use the angular method to compute a shift
    compute_angular_endpoints_1plane(only_always, bsd, max_weight_quant as u32, tmpbuf);

    // For each mode (which specifies a decimation and a quantization):
    //     * Compute number of bits needed for the quantized weights
    //     * Generate an optimized set of quantized weights
    //     * Compute quantization errors for the mode
    const FREE_BITS_FOR_PARTITION_COUNT: [i32; 4] = [
        115 - 4,
        111 - 4 - PARTITION_INDEX_BITS as i32,
        108 - 4 - PARTITION_INDEX_BITS as i32,
        105 - 4 - PARTITION_INDEX_BITS as i32,
    ];

    let max_block_modes = if only_always {
        bsd.block_mode_count_1plane_always
    } else {
        bsd.block_mode_count_1plane_selected
    } as usize;
    for i in 0..max_block_modes {
        let bm = bsd.block_modes[i];

        if i32::from(bm.quant_mode) > max_weight_quant {
            tmpbuf.qwt_errors[i] = 1e38;
            continue;
        }

        let bitcount =
            FREE_BITS_FOR_PARTITION_COUNT[partition_count as usize - 1] - i32::from(bm.weight_bits);
        if bitcount <= 0 {
            tmpbuf.qwt_errors[i] = 1e38;
            continue;
        }

        if tmpbuf.weight_high_value1[i] > 1.02 * min_wt_cutoff {
            tmpbuf.weight_high_value1[i] = 1.0;
        }

        let decimation_mode = bm.decimation_mode as usize;
        let di = bsd.get_decimation_info(decimation_mode);

        tmpbuf.qwt_bitcounts[i] = bitcount as i8;

        let mut dec_weights_uquantf = [0.0f32; BLOCK_MAX_WEIGHTS];

        // Generate the optimized set of weights for the weight mode
        let CompressionWorkingBuffers {
            dec_weights_ideal,
            dec_weights_uquant,
            weight_low_value1,
            weight_high_value1,
            ..
        } = &mut *tmpbuf;
        compute_quantized_weights_for_decimation(
            di,
            weight_low_value1[i],
            weight_high_value1[i],
            &dec_weights_ideal[BLOCK_MAX_WEIGHTS * decimation_mode..],
            &mut dec_weights_uquantf,
            &mut dec_weights_uquant[BLOCK_MAX_WEIGHTS * i..],
            bm.get_weight_quant_mode(),
        );

        // Compute weight quantization errors for the block mode
        tmpbuf.qwt_errors[i] = compute_error_of_weight_set_1plane(&ei, di, &dec_weights_uquantf);
    }

    // Decide the optimal combination of color endpoint encodings and weight encodings
    let mut partition_format_specifiers = [[0u8; BLOCK_MAX_PARTITIONS]; TUNE_MAX_TRIAL_CANDIDATES];
    let mut block_mode_index = [0i32; TUNE_MAX_TRIAL_CANDIDATES];

    let mut color_quant_level = [0u32; TUNE_MAX_TRIAL_CANDIDATES];
    let mut color_quant_level_mod = [0u32; TUNE_MAX_TRIAL_CANDIDATES];

    let candidate_count = ideal_endpoint_formats(
        pi,
        blk,
        &ei.ep,
        config.tune_candidate_limit,
        0,
        max_block_modes as u32,
        &mut partition_format_specifiers,
        &mut block_mode_index,
        &mut color_quant_level,
        &mut color_quant_level_mod,
        tmpbuf,
    );

    // Iterate over the N believed-to-be-best modes to find out which one is actually best
    let mut best_errorval_in_mode = ERROR_CALC_DEFAULT;
    let mut best_errorval_in_scb = scb.errorval;

    'candidates: for i in 0..candidate_count as usize {
        let bm_packed_index = block_mode_index[i] as usize;
        let qw_bm = bsd.block_modes[bm_packed_index];

        let decimation_mode = qw_bm.decimation_mode as usize;
        let di = bsd.get_decimation_info(decimation_mode);

        // Recompute the ideal color endpoints before storing them
        let mut rgbs_colors = [Vfloat4::zero(); BLOCK_MAX_PARTITIONS];
        let mut rgbo_colors = [Vfloat4::zero(); BLOCK_MAX_PARTITIONS];

        let mut workscb = SymbolicCompressedBlock::default();
        let mut workep = ei.ep;

        let src = BLOCK_MAX_WEIGHTS * bm_packed_index;
        for j in 0..di.weight_count as usize {
            workscb.weights[j] = tmpbuf.dec_weights_uquant[src + j];
        }

        for l in 0..config.tune_refinement_limit {
            recompute_ideal_colors_1plane(
                blk,
                pi,
                di,
                &workscb.weights,
                &mut workep,
                &mut rgbs_colors,
                &mut rgbo_colors,
            );

            // Quantize the chosen color, tracking if worth trying the mod value
            let mut all_same = color_quant_level[i] != color_quant_level_mod[i];
            for j in 0..partition_count as usize {
                workscb.color_formats[j] = pack_color_endpoints(
                    workep.endpt0[j],
                    workep.endpt1[j],
                    rgbs_colors[j],
                    rgbo_colors[j],
                    partition_format_specifiers[i][j],
                    &mut workscb.color_values[j],
                    color_quant_level[i],
                );

                all_same = all_same && workscb.color_formats[j] == workscb.color_formats[0];
            }

            // If all the color endpoint modes are the same, we get a few more bits to store
            // colors; requantize all the colors and see if the endpoint modes remain the same.
            workscb.color_formats_matched = 0;
            if partition_count >= 2 && all_same {
                let mut colorvals = [[0u8; 8]; BLOCK_MAX_PARTITIONS];
                let mut color_formats_mod = [0u8; BLOCK_MAX_PARTITIONS];
                let mut all_same_mod = true;
                for j in 0..partition_count as usize {
                    color_formats_mod[j] = pack_color_endpoints(
                        workep.endpt0[j],
                        workep.endpt1[j],
                        rgbs_colors[j],
                        rgbo_colors[j],
                        partition_format_specifiers[i][j],
                        &mut colorvals[j],
                        color_quant_level_mod[i],
                    );

                    // Early out as soon as it's no longer possible to use mod
                    if color_formats_mod[j] != color_formats_mod[0] {
                        all_same_mod = false;
                        break;
                    }
                }

                if all_same_mod {
                    workscb.color_formats_matched = 1;
                    workscb.color_values = colorvals;
                    workscb.color_formats = color_formats_mod;
                }
            }

            // Store header fields
            workscb.partition_count = partition_count as u8;
            workscb.partition_index = partition_index as u16;
            workscb.plane2_component = -1;
            workscb.quant_mode = if workscb.color_formats_matched != 0 {
                color_quant_level_mod[i]
            } else {
                color_quant_level[i]
            };
            workscb.block_mode = qw_bm.mode_index;
            workscb.block_type = SYM_BTYPE_NONCONST;

            // Pre-realign test
            if l == 0 {
                let errorval = compute_difference(config, bsd, &workscb, blk);
                match trial_test(
                    errorval,
                    1.08,
                    config.tune_refinement_limit - l,
                    &mut workscb,
                    scb,
                    &mut best_errorval_in_mode,
                    &mut best_errorval_in_scb,
                    tune_errorval_threshold,
                ) {
                    Trial::Continue => {}
                    Trial::NextCandidate => continue 'candidates,
                    Trial::Done => break 'candidates,
                }
            }

            // Perform a final pass over the weights to try to improve them.
            let adjustments = if di.weight_count != bsd.texel_count {
                realign_weights_decimated(config.profile, bsd, blk, &mut workscb)
            } else {
                realign_weights_undecimated(config.profile, bsd, blk, &mut workscb)
            };

            // Post-realign test
            let errorval = compute_difference(config, bsd, &workscb, blk);
            match trial_test(
                errorval,
                1.0,
                config.tune_refinement_limit - 1 - l,
                &mut workscb,
                scb,
                &mut best_errorval_in_mode,
                &mut best_errorval_in_scb,
                tune_errorval_threshold,
            ) {
                Trial::Continue => {}
                Trial::NextCandidate => continue 'candidates,
                Trial::Done => break 'candidates,
            }

            if !adjustments {
                break;
            }
        }
    }

    best_errorval_in_mode
}

#[isa]
/// `compress_symbolic_block_for_partition_2planes`: compresses a block using a single
/// partition and 2 planes of weights.
fn compress_symbolic_block_for_partition_2planes(
    config: &AstcencConfig,
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    tune_errorval_threshold: f32,
    plane2_component: u32,
    scb: &mut SymbolicCompressedBlock,
    tmpbuf: &mut CompressionWorkingBuffers,
    quant_limit: i32,
) -> f32 {
    let max_weight_quant = (QUANT_32 as i32).min(quant_limit);

    // Compute ideal weights and endpoint colors, with no quantization or decimation
    {
        let CompressionWorkingBuffers { ei1, ei2, .. } = &mut *tmpbuf;
        compute_ideal_colors_and_weights_2planes(bsd, blk, plane2_component, ei1, ei2);
    }
    let ei1 = tmpbuf.ei1.clone();
    let ei2 = tmpbuf.ei2.clone();

    // For each decimation mode, compute an ideal set of weights with no quantization
    for i in 0..bsd.decimation_mode_count_selected as usize {
        let dm = bsd.get_decimation_mode(i);
        if !dm.is_ref_2plane(max_weight_quant as u32) {
            continue;
        }

        let di = bsd.get_decimation_info(i);

        compute_ideal_weights_for_decimation(
            &ei1,
            di,
            &mut tmpbuf.dec_weights_ideal[i * BLOCK_MAX_WEIGHTS..],
        );

        compute_ideal_weights_for_decimation(
            &ei2,
            di,
            &mut tmpbuf.dec_weights_ideal[i * BLOCK_MAX_WEIGHTS + WEIGHTS_PLANE2_OFFSET..],
        );
    }

    // Compute maximum colors for the endpoints and ideal weights, then for each endpoint and
    // ideal weight pair, compute the smallest weight that will result in a color value > 1
    let mut min_ep1 = min_weight_cutoff(&ei1.ep, 0, Vfloat4::splat(10.0));
    let min_ep2 = min_weight_cutoff(&ei2.ep, 0, Vfloat4::splat(10.0));

    let err_max = Vfloat4::splat(ERROR_CALC_DEFAULT);
    let err_mask = Vint4::lane_id().eq(Vint4::splat(plane2_component as i32));

    // Set the plane2 component to max error in ep1
    min_ep1 = Vfloat4::select(min_ep1, err_max, err_mask);

    let min_wt_cutoff1 = min_ep1.hmin_s();

    // Set the minwt2 to the plane2 component min in ep2
    let min_wt_cutoff2 = Vfloat4::select(err_max, min_ep2, err_mask).hmin_s();

    compute_angular_endpoints_2planes(bsd, max_weight_quant as u32, tmpbuf);

    // For each mode (which specifies a decimation and a quantization):
    //     * Compute number of bits needed for the quantized weights
    //     * Generate an optimized set of quantized weights
    //     * Compute quantization errors for the mode
    let start_2plane = bsd.block_mode_count_1plane_selected as usize;
    let end_2plane = bsd.block_mode_count_1plane_2plane_selected as usize;

    for i in start_2plane..end_2plane {
        let bm = bsd.block_modes[i];

        if i32::from(bm.quant_mode) > max_weight_quant {
            tmpbuf.qwt_errors[i] = 1e38;
            continue;
        }

        tmpbuf.qwt_bitcounts[i] = (109 - i32::from(bm.weight_bits)) as i8;

        if tmpbuf.weight_high_value1[i] > 1.02 * min_wt_cutoff1 {
            tmpbuf.weight_high_value1[i] = 1.0;
        }

        if tmpbuf.weight_high_value2[i] > 1.02 * min_wt_cutoff2 {
            tmpbuf.weight_high_value2[i] = 1.0;
        }

        let decimation_mode = bm.decimation_mode as usize;
        let di = bsd.get_decimation_info(decimation_mode);

        let mut dec_weights_uquantf = [0.0f32; BLOCK_MAX_WEIGHTS];

        // Generate the optimized set of weights for the mode
        let CompressionWorkingBuffers {
            dec_weights_ideal,
            dec_weights_uquant,
            weight_low_value1,
            weight_high_value1,
            weight_low_value2,
            weight_high_value2,
            ..
        } = &mut *tmpbuf;
        let ideal = BLOCK_MAX_WEIGHTS * decimation_mode;
        let uquant = BLOCK_MAX_WEIGHTS * i;
        compute_quantized_weights_for_decimation(
            di,
            weight_low_value1[i],
            weight_high_value1[i],
            &dec_weights_ideal[ideal..],
            &mut dec_weights_uquantf,
            &mut dec_weights_uquant[uquant..],
            bm.get_weight_quant_mode(),
        );

        compute_quantized_weights_for_decimation(
            di,
            weight_low_value2[i],
            weight_high_value2[i],
            &dec_weights_ideal[ideal + WEIGHTS_PLANE2_OFFSET..],
            &mut dec_weights_uquantf[WEIGHTS_PLANE2_OFFSET..],
            &mut dec_weights_uquant[uquant + WEIGHTS_PLANE2_OFFSET..],
            bm.get_weight_quant_mode(),
        );

        // Compute weight quantization errors for the block mode
        tmpbuf.qwt_errors[i] = compute_error_of_weight_set_2planes(
            &ei1,
            &ei2,
            di,
            &dec_weights_uquantf,
            &dec_weights_uquantf[WEIGHTS_PLANE2_OFFSET..],
        );
    }

    // Decide the optimal combination of color endpoint encodings and weight encodings
    let mut partition_format_specifiers = [[0u8; BLOCK_MAX_PARTITIONS]; TUNE_MAX_TRIAL_CANDIDATES];
    let mut block_mode_index = [0i32; TUNE_MAX_TRIAL_CANDIDATES];

    let mut color_quant_level = [0u32; TUNE_MAX_TRIAL_CANDIDATES];
    let mut color_quant_level_mod = [0u32; TUNE_MAX_TRIAL_CANDIDATES];

    let mut epm = Endpoints::default();
    merge_endpoints(&ei1.ep, &ei2.ep, plane2_component, &mut epm);

    let pi = bsd.get_partition_info(1, 0);
    let candidate_count = ideal_endpoint_formats(
        pi,
        blk,
        &epm,
        config.tune_candidate_limit,
        bsd.block_mode_count_1plane_selected,
        bsd.block_mode_count_1plane_2plane_selected,
        &mut partition_format_specifiers,
        &mut block_mode_index,
        &mut color_quant_level,
        &mut color_quant_level_mod,
        tmpbuf,
    );

    // Iterate over the N believed-to-be-best modes to find out which one is actually best
    let mut best_errorval_in_mode = ERROR_CALC_DEFAULT;
    let mut best_errorval_in_scb = scb.errorval;

    'candidates: for i in 0..candidate_count as usize {
        let bm_packed_index = block_mode_index[i] as usize;
        let qw_bm = bsd.block_modes[bm_packed_index];

        let decimation_mode = qw_bm.decimation_mode as usize;
        let di = bsd.get_decimation_info(decimation_mode);

        let mut rgbs_color = Vfloat4::zero();
        let mut rgbo_color = Vfloat4::zero();

        let mut workscb = SymbolicCompressedBlock::default();
        let mut workep = epm;

        let src = BLOCK_MAX_WEIGHTS * bm_packed_index;
        for j in 0..di.weight_count as usize {
            workscb.weights[j] = tmpbuf.dec_weights_uquant[src + j];
            workscb.weights[j + WEIGHTS_PLANE2_OFFSET] =
                tmpbuf.dec_weights_uquant[src + WEIGHTS_PLANE2_OFFSET + j];
        }

        for l in 0..config.tune_refinement_limit {
            let (w1, w2) = workscb.weights.split_at(WEIGHTS_PLANE2_OFFSET);
            recompute_ideal_colors_2planes(
                blk,
                bsd,
                di,
                w1,
                w2,
                &mut workep,
                &mut rgbs_color,
                &mut rgbo_color,
                plane2_component as i32,
            );

            // Quantize the chosen color
            workscb.color_formats[0] = pack_color_endpoints(
                workep.endpt0[0],
                workep.endpt1[0],
                rgbs_color,
                rgbo_color,
                partition_format_specifiers[i][0],
                &mut workscb.color_values[0],
                color_quant_level[i],
            );

            // Store header fields
            workscb.partition_count = 1;
            workscb.partition_index = 0;
            workscb.quant_mode = color_quant_level[i];
            workscb.color_formats_matched = 0;
            workscb.block_mode = qw_bm.mode_index;
            workscb.plane2_component = plane2_component as i8;
            workscb.block_type = SYM_BTYPE_NONCONST;

            // Pre-realign test
            if l == 0 {
                let errorval = compute_symbolic_block_difference_2plane(config, bsd, &workscb, blk);
                match trial_test(
                    errorval,
                    1.08,
                    config.tune_refinement_limit - l,
                    &mut workscb,
                    scb,
                    &mut best_errorval_in_mode,
                    &mut best_errorval_in_scb,
                    tune_errorval_threshold,
                ) {
                    Trial::Continue => {}
                    Trial::NextCandidate => continue 'candidates,
                    Trial::Done => break 'candidates,
                }
            }

            // Perform a final pass over the weights to try to improve them.
            let adjustments = if di.weight_count != bsd.texel_count {
                realign_weights_decimated(config.profile, bsd, blk, &mut workscb)
            } else {
                realign_weights_undecimated(config.profile, bsd, blk, &mut workscb)
            };

            // Post-realign test
            let errorval = compute_symbolic_block_difference_2plane(config, bsd, &workscb, blk);
            match trial_test(
                errorval,
                1.0,
                config.tune_refinement_limit - 1 - l,
                &mut workscb,
                scb,
                &mut best_errorval_in_mode,
                &mut best_errorval_in_scb,
                tune_errorval_threshold,
            ) {
                Trial::Continue => {}
                Trial::NextCandidate => continue 'candidates,
                Trial::Done => break 'candidates,
            }

            if !adjustments {
                break;
            }
        }
    }

    best_errorval_in_mode
}

#[isa]
/// `prepare_block_statistics`: the lowest absolute channel correlation of the block.
fn prepare_block_statistics(texels_per_block: usize, blk: &ImageBlock) -> f32 {
    // Compute covariance matrix, as a collection of 10 scalars that form the upper-triangular
    // row of the matrix. The matrix is symmetric, so this is all we need for this use case.
    let (mut rs, mut gs, mut bs, mut as_) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut rr_var, mut gg_var, mut bb_var, mut aa_var) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut rg_cov, mut rb_cov, mut ra_cov) = (0.0f32, 0.0f32, 0.0f32);
    let (mut gb_cov, mut ga_cov, mut ba_cov) = (0.0f32, 0.0f32, 0.0f32);
    let mut weight_sum = 0.0f32;

    for i in 0..texels_per_block {
        let weight = blk.channel_weight.hadd_s() / 4.0;
        weight_sum += weight;

        let r = blk.data_r[i];
        let g = blk.data_g[i];
        let b = blk.data_b[i];
        let a = blk.data_a[i];

        let rw = r * weight;
        rs += rw;
        rr_var += r * rw;
        rg_cov += g * rw;
        rb_cov += b * rw;
        ra_cov += a * rw;

        let gw = g * weight;
        gs += gw;
        gg_var += g * gw;
        gb_cov += b * gw;
        ga_cov += a * gw;

        let bw = b * weight;
        bs += bw;
        bb_var += b * bw;
        ba_cov += a * bw;

        let aw = a * weight;
        as_ += aw;
        aa_var += a * aw;
    }

    let rpt = 1.0 / mathlib::max(weight_sum, 1e-7);

    rr_var -= rs * (rs * rpt);
    rg_cov -= gs * (rs * rpt);
    rb_cov -= bs * (rs * rpt);
    ra_cov -= as_ * (rs * rpt);

    gg_var -= gs * (gs * rpt);
    gb_cov -= bs * (gs * rpt);
    ga_cov -= as_ * (gs * rpt);

    bb_var -= bs * (bs * rpt);
    ba_cov -= as_ * (bs * rpt);

    aa_var -= as_ * (as_ * rpt);

    // These will give a NaN if a channel is constant - these are fixed up in the next step
    rg_cov *= mathlib::rsqrt(rr_var * gg_var);
    rb_cov *= mathlib::rsqrt(rr_var * bb_var);
    ra_cov *= mathlib::rsqrt(rr_var * aa_var);
    gb_cov *= mathlib::rsqrt(gg_var * bb_var);
    ga_cov *= mathlib::rsqrt(gg_var * aa_var);
    ba_cov *= mathlib::rsqrt(bb_var * aa_var);

    let fix = |v: f32| if v.is_nan() { 1.0 } else { v };
    let (rg_cov, rb_cov, ra_cov) = (fix(rg_cov), fix(rb_cov), fix(ra_cov));
    let (gb_cov, ga_cov, ba_cov) = (fix(gb_cov), fix(ga_cov), fix(ba_cov));

    let mut lowest_correlation = mathlib::min(rg_cov.abs(), rb_cov.abs());
    lowest_correlation = mathlib::min(lowest_correlation, ra_cov.abs());
    lowest_correlation = mathlib::min(lowest_correlation, gb_cov.abs());
    lowest_correlation = mathlib::min(lowest_correlation, ga_cov.abs());
    lowest_correlation = mathlib::min(lowest_correlation, ba_cov.abs());

    lowest_correlation
}

#[isa]
/// UNORM16 constant color of the block origin texel.
fn constant_u16(blk: &ImageBlock) -> [i32; 4] {
    let color_f32 = Vfloat4::clamp(0.0, 1.0, blk.origin_texel) * 65535.0;
    color_f32.float_to_int_rtn().m
}

#[isa]
/// `compress_block`: compresses one image block into a 16 byte physical ASTC block.
pub fn compress_block(
    config: &AstcencConfig,
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    pcb: &mut [u8; 16],
    tmpbuf: &mut CompressionWorkingBuffers,
) {
    let decode_mode = config.profile;
    let mut scb = SymbolicCompressedBlock::default();

    // Set stricter block targets for luminance data as we have more bits to play with
    let block_is_l = blk.is_luminance();
    let block_is_l_scale = if block_is_l { 1.0 / 1.5 } else { 1.0 };

    // Set slightly stricter block targets for lumalpha data as we have more bits to play with
    let block_is_la = blk.is_luminancealpha();
    let block_is_la_scale = if block_is_la { 1.0 / 1.05 } else { 1.0 };

    let max_partitions = config.tune_partition_count_limit;

    let requested_partition_indices = [
        config.tune_2partition_index_limit,
        config.tune_3partition_index_limit,
        config.tune_4partition_index_limit,
    ];

    let requested_partition_trials = [
        config.tune_2partitioning_candidate_limit,
        config.tune_3partitioning_candidate_limit,
        config.tune_4partitioning_candidate_limit,
    ];

    // Detected a constant-color block
    if blk.data_min.eq(blk.data_max).all() {
        scb.partition_count = 0;

        // Encode as FP16 if using HDR
        if decode_mode == AstcencProfile::Hdr || decode_mode == AstcencProfile::HdrRgbLdrA {
            scb.block_type = SYM_BTYPE_CONST_F16;
            scb.constant_color = float_to_float16_v(blk.origin_texel).m;
        } else {
            // Encode as UNORM16 if NOT using HDR
            scb.block_type = SYM_BTYPE_CONST_U16;
            scb.constant_color = constant_u16(blk);
        }

        symbolic_to_physical(bsd, &scb, pcb);
        return;
    }

    let error_weight_sum = blk.channel_weight.hadd_s() * f32::from(bsd.texel_count);
    let error_threshold =
        config.tune_db_limit * error_weight_sum * block_is_l_scale * block_is_la_scale;

    // Set SCB and mode errors to a very high error value
    scb.errorval = ERROR_CALC_DEFAULT;
    scb.block_type = SYM_BTYPE_ERROR;

    let mut best_errorvals_for_pcount = [ERROR_CALC_DEFAULT; BLOCK_MAX_PARTITIONS];

    let exit_thresholds_for_pcount = [
        0.0,
        config.tune_2partition_early_out_limit_factor,
        config.tune_3partition_early_out_limit_factor,
        0.0,
    ];

    // Trial using 1 plane of weights and 1 partition. Most of the time we test it twice, first
    // with a mode cutoff of 0 and then with the specified mode cutoff. This causes an early-out
    // that speeds up encoding of easy blocks.
    let errorval_mult = [1.0 / config.tune_mse_overshoot, 1.0];

    let errorval_overshoot = 1.0 / config.tune_mse_overshoot;

    // Only enable MODE0 fast path if enabled
    // Never enable for 3D blocks as no "always" block modes are available
    let mut start_trial = 1;
    if config.tune_search_mode0_enable >= TUNE_MIN_SEARCH_MODE0 && bsd.dim_z == 1 {
        start_trial = 0;
    }

    'tests: {
        let mut quant_limit = QUANT_32 as i32;
        for i in start_trial..2 {
            let errorval = compress_symbolic_block_for_partition_1plane(
                config,
                bsd,
                blk,
                i == 0,
                error_threshold * errorval_mult[i] * errorval_overshoot,
                1,
                0,
                &mut scb,
                tmpbuf,
                QUANT_32 as i32,
            );

            // Record the quant level so we can use the filter later searches
            if scb.block_type != SYM_BTYPE_ERROR {
                let bm = bsd.get_block_mode(scb.block_mode as usize);
                quant_limit = bm.get_weight_quant_mode() as i32;
            }

            best_errorvals_for_pcount[0] = mathlib::min(best_errorvals_for_pcount[0], errorval);
            if errorval < (error_threshold * errorval_mult[i]) {
                break 'tests;
            }
        }

        let lowest_correl = prepare_block_statistics(bsd.texel_count as usize, blk);
        let block_skip_two_plane = lowest_correl > config.tune_2plane_early_out_limit_correlation;

        // Test the four possible 1-partition, 2-planes modes. Do this in reverse, as
        // alpha is the most likely to be non-correlated if it is present in the data.
        for i in (0..BLOCK_MAX_COMPONENTS).rev() {
            if block_skip_two_plane {
                continue;
            }

            if blk.grayscale && i != 3 {
                continue;
            }

            if blk.is_constant_channel(i) {
                continue;
            }

            let errorval = compress_symbolic_block_for_partition_2planes(
                config,
                bsd,
                blk,
                error_threshold * errorval_overshoot,
                i as u32,
                &mut scb,
                tmpbuf,
                quant_limit,
            );

            // If attempting two planes is much worse than the best one plane result
            // then further two plane searches are unlikely to help so move on ...
            if errorval > (best_errorvals_for_pcount[0] * 1.85) {
                break;
            }

            if errorval < error_threshold {
                break 'tests;
            }
        }

        // Find best blocks for 2, 3 and 4 partitions
        for partition_count in 2..=max_partitions as usize {
            let mut partition_indices = [0u32; TUNE_MAX_PARTITIONING_CANDIDATES];

            let requested_indices = requested_partition_indices[partition_count - 2];

            let mut requested_trials = requested_partition_trials[partition_count - 2];
            requested_trials = requested_trials.min(requested_indices);

            let actual_trials = find_best_partition_candidates(
                bsd,
                blk,
                partition_count as u32,
                requested_indices,
                &mut partition_indices,
                requested_trials,
            );

            let best_error_in_prev = best_errorvals_for_pcount[partition_count - 2];

            for i in 0..actual_trials as usize {
                let errorval = compress_symbolic_block_for_partition_1plane(
                    config,
                    bsd,
                    blk,
                    false,
                    error_threshold * errorval_overshoot,
                    partition_count as u32,
                    partition_indices[i],
                    &mut scb,
                    tmpbuf,
                    quant_limit,
                );

                best_errorvals_for_pcount[partition_count - 1] =
                    mathlib::min(best_errorvals_for_pcount[partition_count - 1], errorval);

                // If using N partitions doesn't improve much over using N-1 partitions then skip
                // trying N+1. Error can dramatically improve if the data is correlated or
                // non-correlated and aligns with a partitioning that suits that encoding, so for
                // this inner loop check add a large error scale because the "other" trial could
                // be a lot better.
                let best_error = best_errorvals_for_pcount[partition_count - 1];
                let best_error_scale = exit_thresholds_for_pcount[partition_count - 1] * 1.85;
                if best_error > (best_error_in_prev * best_error_scale) {
                    break 'tests;
                }

                if errorval < error_threshold {
                    break 'tests;
                }
            }

            // If using N partitions doesn't improve much over using N-1 partitions then skip
            // trying N+1
            let best_error = best_errorvals_for_pcount[partition_count - 1];
            let best_error_scale = exit_thresholds_for_pcount[partition_count - 1];
            if best_error > (best_error_in_prev * best_error_scale) {
                break 'tests;
            }
        }
    }

    // If we still have an error block then convert to something we can encode
    if scb.block_type == SYM_BTYPE_ERROR {
        scb.block_type = SYM_BTYPE_CONST_U16;
        scb.constant_color = constant_u16(blk);
    }

    // Compress to a physical block
    symbolic_to_physical(bsd, &scb, pcb);
}
