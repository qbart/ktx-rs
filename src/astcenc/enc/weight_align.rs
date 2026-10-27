//! Port of `astcenc_weight_align.cpp`: the angular-sum weight alignment that picks the
//! low / high weight bounds per decimation mode and weight quant level.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use std::sync::OnceLock;

use super::internal::*;
use super::vecmath::*;
use ktx_macros::isa;

const ANGULAR_STEPS: usize = 32;
const SINCOS_STEPS: usize = 64;
const STEPS_FOR_QUANT_LEVEL: [u8; 12] = [2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32];

struct SinCos {
    sin_table: [[f32; ANGULAR_STEPS]; SINCOS_STEPS],
    cos_table: [[f32; ANGULAR_STEPS]; SINCOS_STEPS],
}

#[isa]
/// `prepare_angular_tables` (computed once).
fn tables() -> &'static SinCos {
    static TABLES: OnceLock<SinCos> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = SinCos {
            sin_table: [[0.0; ANGULAR_STEPS]; SINCOS_STEPS],
            cos_table: [[0.0; ANGULAR_STEPS]; SINCOS_STEPS],
        };
        for i in 0..ANGULAR_STEPS {
            let angle_step = (i + 1) as f32;
            for j in 0..SINCOS_STEPS {
                let a = (2.0 * PI / (SINCOS_STEPS as f32 - 1.0)) * angle_step * j as f32;
                t.sin_table[j][i] = a.sin();
                t.cos_table[j][i] = a.cos();
            }
        }
        t
    })
}

#[isa]
/// `prepare_angular_tables`.
pub fn prepare_angular_tables() {
    let _ = tables();
}

#[isa]
#[inline]
fn ld(p: &[f32], i: usize) -> Vfloat8 {
    Vfloat8::load(p.get(i..).unwrap_or_default())
}

#[isa]
/// `compute_angular_offsets`.
fn compute_angular_offsets(
    weight_count: usize,
    dec_weight_ideal_value: &[f32],
    max_angular_steps: usize,
    offsets: &mut [f32; ANGULAR_STEPS],
) {
    let t = tables();
    let mut isamplev = [0i32; BLOCK_MAX_WEIGHTS + 8];
    // Precompute isample; arrays are always allocated 64 elements long
    let mut i = 0;
    while i < weight_count {
        // Add 2^23 and interpreting bits extracts round-to-nearest int
        let ideal_weight = ld(dec_weight_ideal_value, i).clampzo();
        let sample = ideal_weight * (SINCOS_STEPS as f32 - 1.0);
        let isample = sample.float_to_int_rtn();
        isample.store(&mut isamplev[i..]);
        i += SIMD_WIDTH;
    }

    // Arrays are multiple of SIMD width (ANGULAR_STEPS), safe to overshoot max
    let mult = Vfloat8::splat(1.0 / (2.0 * PI));
    let mut i = 0;
    while i < max_angular_steps {
        let mut anglesum_x = Vfloat8::zero();
        let mut anglesum_y = Vfloat8::zero();
        for j in 0..weight_count {
            let isample = (isamplev[j] as usize).min(SINCOS_STEPS - 1);
            anglesum_x += Vfloat8::loada(&t.cos_table[isample][i..]);
            anglesum_y += Vfloat8::loada(&t.sin_table[isample][i..]);
        }
        let mut angle = atan2(anglesum_y, anglesum_x);
        angle = Vfloat8::select(Vfloat8::zero(), angle, angle.eq(angle));
        let ofs = angle * mult;
        ofs.store(&mut offsets[i..]);
        i += SIMD_WIDTH;
    }
}

#[isa]
/// `compute_lowest_and_highest_weight`.
fn compute_lowest_and_highest_weight(
    weight_count: usize,
    dec_weight_ideal_value: &[f32],
    max_angular_steps: usize,
    max_quant_steps: u32,
    offsets: &[f32; ANGULAR_STEPS],
    lowest_weight: &mut [f32; ANGULAR_STEPS],
    weight_span: &mut [i32; ANGULAR_STEPS],
    error: &mut [f32; ANGULAR_STEPS],
    cut_low_weight_error: &mut [f32; ANGULAR_STEPS],
    cut_high_weight_error: &mut [f32; ANGULAR_STEPS],
) {
    let mut rcp_stepsize = Vint8::lane_id().int_to_float() + Vfloat8::splat(1.0);

    // Compute minimum/maximum weights in the weight array. Our remapping
    // is monotonic, so the min/max rounded weights relate to the min/max
    // unrounded weights in a straightforward way.
    let mut min_weight = Vfloat8::splat(f32::MAX);
    let mut max_weight = Vfloat8::splat(-f32::MAX);
    let mut lane_id = Vint8::lane_id();
    let mut i = 0;
    while i < weight_count {
        let active = lane_id.lt(Vint8::splat(weight_count as i32));
        lane_id += Vint8::splat(SIMD_WIDTH as i32);
        let weights = ld(dec_weight_ideal_value, i);
        min_weight = Vfloat8::select(min_weight, weights, active).min(min_weight);
        max_weight = Vfloat8::select(max_weight, weights, active).max(max_weight);
        i += SIMD_WIDTH;
    }
    let min_weight = min_weight.hmin();
    let max_weight = max_weight.hmax();

    // Arrays are ANGULAR_STEPS long, so always safe to run full vectors
    let mut sp = 0;
    while sp < max_angular_steps {
        let mut errval = Vfloat8::zero();
        let mut cut_low_weight_err = Vfloat8::zero();
        let mut cut_high_weight_err = Vfloat8::zero();
        let offset = Vfloat8::loada(&offsets[sp..]);

        // We know the min and max weight values, so we can figure out
        // the corresponding indices before we enter the loop.
        let minidx = (min_weight * rcp_stepsize - offset).round();
        let maxidx = (max_weight * rcp_stepsize - offset).round();

        for j in 0..weight_count {
            let sval = Vfloat8::splat(dec_weight_ideal_value.get(j).copied().unwrap_or(0.0))
                * rcp_stepsize
                - offset;
            let svalrte = sval.round();
            let diff = sval - svalrte;
            errval += diff * diff;

            // Accumulate errors for minimum index
            let mask = svalrte.eq(minidx);
            let accum = cut_low_weight_err + Vfloat8::splat(1.0) - Vfloat8::splat(2.0) * diff;
            cut_low_weight_err = Vfloat8::select(cut_low_weight_err, accum, mask);

            // Accumulate errors for maximum index
            let mask = svalrte.eq(maxidx);
            let accum = cut_high_weight_err + Vfloat8::splat(1.0) + Vfloat8::splat(2.0) * diff;
            cut_high_weight_err = Vfloat8::select(cut_high_weight_err, accum, mask);
        }

        // Write out min weight and weight span; clamp span to a usable range
        let mut span = (maxidx - minidx + Vfloat8::splat(1.0)).float_to_int();
        span = span.min(Vint8::splat(max_quant_steps as i32 + 3));
        span = span.max(Vint8::splat(2));
        minidx.store(&mut lowest_weight[sp..]);
        span.store(&mut weight_span[sp..]);

        // The cut_(lowest/highest)_weight_error indicate the error that results from forcing
        // samples that should have had the weight value one step (up/down).
        let ssize = 1.0 / rcp_stepsize;
        let errscale = ssize * ssize;
        (errval * errscale).store(&mut error[sp..]);
        (cut_low_weight_err * errscale).store(&mut cut_low_weight_error[sp..]);
        (cut_high_weight_err * errscale).store(&mut cut_high_weight_error[sp..]);

        rcp_stepsize += Vfloat8::splat(SIMD_WIDTH as f32);
        sp += SIMD_WIDTH;
    }
}

#[isa]
/// `compute_angular_endpoints_for_quant_levels`.
fn compute_angular_endpoints_for_quant_levels(
    weight_count: usize,
    dec_weight_ideal_value: &[f32],
    max_quant_level: usize,
    low_value: &mut [f32; TUNE_MAX_ANGULAR_QUANT + 1],
    high_value: &mut [f32; TUNE_MAX_ANGULAR_QUANT + 1],
) {
    let max_quant_steps = STEPS_FOR_QUANT_LEVEL[max_quant_level] as usize;
    let max_angular_steps = STEPS_FOR_QUANT_LEVEL[max_quant_level] as usize;

    let mut angular_offsets = [0.0f32; ANGULAR_STEPS];
    compute_angular_offsets(
        weight_count,
        dec_weight_ideal_value,
        max_angular_steps,
        &mut angular_offsets,
    );

    let mut lowest_weight = [0.0f32; ANGULAR_STEPS];
    let mut weight_span = [0i32; ANGULAR_STEPS];
    let mut error = [0.0f32; ANGULAR_STEPS];
    let mut cut_low_weight_error = [0.0f32; ANGULAR_STEPS];
    let mut cut_high_weight_error = [0.0f32; ANGULAR_STEPS];
    compute_lowest_and_highest_weight(
        weight_count,
        dec_weight_ideal_value,
        max_angular_steps,
        max_quant_steps as u32,
        &angular_offsets,
        &mut lowest_weight,
        &mut weight_span,
        &mut error,
        &mut cut_low_weight_error,
        &mut cut_high_weight_error,
    );

    // For each quantization level, find the best error terms. Use packed vectors so data-dependent
    // branches can become selects. This involves some integer to float casts, but the values are
    // small enough so they never round the wrong way.
    let mut best_results = [Vfloat4::zero(); 36];
    for r in best_results.iter_mut().take(max_quant_steps + 4) {
        *r = Vfloat4::new(ERROR_CALC_DEFAULT, -1.0, 0.0, 0.0);
    }
    let sel = |best_result: Vfloat4, new_result: Vfloat4, err: f32| {
        let mask = Vfloat4::splat(best_result.lane(0)).gt(Vfloat4::splat(err));
        Vfloat4::select(best_result, new_result, mask)
    };
    for i in 0..max_angular_steps {
        let i_flt = i as f32;
        let idx_span = (weight_span[i] as usize).min(35);
        let error_cut_low = error[i] + cut_low_weight_error[i];
        let error_cut_high = error[i] + cut_high_weight_error[i];
        let error_cut_low_high = error[i] + cut_low_weight_error[i] + cut_high_weight_error[i];

        // Check best error against record N
        best_results[idx_span] = sel(
            best_results[idx_span],
            Vfloat4::new(error[i], i_flt, 0.0, 0.0),
            error[i],
        );

        // Check best error against record N-1 with either cut low or cut high
        let mut best_result = best_results[idx_span - 1];
        best_result = sel(
            best_result,
            Vfloat4::new(error_cut_low, i_flt, 1.0, 0.0),
            error_cut_low,
        );
        best_results[idx_span - 1] = sel(
            best_result,
            Vfloat4::new(error_cut_high, i_flt, 0.0, 0.0),
            error_cut_high,
        );

        // Check best error against record N-2 with both cut low and high
        best_results[idx_span - 2] = sel(
            best_results[idx_span - 2],
            Vfloat4::new(error_cut_low_high, i_flt, 1.0, 0.0),
            error_cut_low_high,
        );
    }

    for i in 0..=max_quant_level {
        let q = STEPS_FOR_QUANT_LEVEL[i] as usize;
        let bsi = cvtt(best_results[q].lane(1)).max(0) as usize;
        let lwi = lowest_weight[bsi] + best_results[q].lane(2);
        let hwi = lwi + q as f32 - 1.0;
        let stepsize = 1.0 / (1.0 + bsi as f32);
        low_value[i] = (angular_offsets[bsi] + lwi) * stepsize;
        high_value[i] = (angular_offsets[bsi] + hwi) * stepsize;
    }
}

#[isa]
/// `compute_angular_endpoints_1plane`. The ideal decimated weights are read from
/// `tmpbuf.dec_weights_ideal`, which is what every C++ caller passes.
pub fn compute_angular_endpoints_1plane(
    only_always: bool,
    bsd: &BlockSizeDescriptor,
    max_weight_quant: u32,
    tmpbuf: &mut CompressionWorkingBuffers,
) {
    let CompressionWorkingBuffers {
        dec_weights_ideal: dec_weight_ideal_value,
        weight_low_value1: low_value,
        weight_high_value1: high_value,
        weight_low_values1: low_values,
        weight_high_values1: high_values,
        ..
    } = tmpbuf;

    let max_decimation_modes = if only_always {
        bsd.decimation_mode_count_always
    } else {
        bsd.decimation_mode_count_selected
    } as usize;
    for i in 0..max_decimation_modes {
        let dm = &bsd.decimation_modes[i];
        if !dm.is_ref_1plane(max_weight_quant) {
            continue;
        }
        let weight_count = bsd.get_decimation_info(i).weight_count as usize;
        let max_precision = (dm.maxprec_1plane as u32)
            .min(TUNE_MAX_ANGULAR_QUANT as u32)
            .min(max_weight_quant);
        compute_angular_endpoints_for_quant_levels(
            weight_count,
            dec_weight_ideal_value
                .get(i * BLOCK_MAX_WEIGHTS..)
                .unwrap_or_default(),
            max_precision as usize,
            &mut low_values[i],
            &mut high_values[i],
        );
    }

    let max_block_modes = if only_always {
        bsd.block_mode_count_1plane_always
    } else {
        bsd.block_mode_count_1plane_selected
    } as usize;
    for i in 0..max_block_modes {
        let bm = &bsd.block_modes[i];
        let quant_mode = bm.quant_mode as usize;
        let decim_mode = bm.decimation_mode as usize;
        if quant_mode <= TUNE_MAX_ANGULAR_QUANT {
            low_value[i] = low_values[decim_mode][quant_mode];
            high_value[i] = high_values[decim_mode][quant_mode];
        } else {
            low_value[i] = 0.0;
            high_value[i] = 1.0;
        }
    }
}

#[isa]
/// `compute_angular_endpoints_2planes` (ideal weights from `tmpbuf.dec_weights_ideal`).
pub fn compute_angular_endpoints_2planes(
    bsd: &BlockSizeDescriptor,
    max_weight_quant: u32,
    tmpbuf: &mut CompressionWorkingBuffers,
) {
    let CompressionWorkingBuffers {
        dec_weights_ideal: dec_weight_ideal_value,
        weight_low_value1: low_value1,
        weight_high_value1: high_value1,
        weight_low_values1: low_values1,
        weight_high_values1: high_values1,
        weight_low_value2: low_value2,
        weight_high_value2: high_value2,
        weight_low_values2: low_values2,
        weight_high_values2: high_values2,
        ..
    } = tmpbuf;

    for i in 0..bsd.decimation_mode_count_selected as usize {
        let dm = &bsd.decimation_modes[i];
        if !dm.is_ref_2plane(max_weight_quant) {
            continue;
        }
        let weight_count = bsd.get_decimation_info(i).weight_count as usize;
        let max_precision = (dm.maxprec_2planes as u32)
            .min(TUNE_MAX_ANGULAR_QUANT as u32)
            .min(max_weight_quant);
        compute_angular_endpoints_for_quant_levels(
            weight_count,
            dec_weight_ideal_value
                .get(i * BLOCK_MAX_WEIGHTS..)
                .unwrap_or_default(),
            max_precision as usize,
            &mut low_values1[i],
            &mut high_values1[i],
        );
        compute_angular_endpoints_for_quant_levels(
            weight_count,
            dec_weight_ideal_value
                .get(i * BLOCK_MAX_WEIGHTS + WEIGHTS_PLANE2_OFFSET..)
                .unwrap_or_default(),
            max_precision as usize,
            &mut low_values2[i],
            &mut high_values2[i],
        );
    }

    let start = bsd.block_mode_count_1plane_selected as usize;
    let end = bsd.block_mode_count_1plane_2plane_selected as usize;
    for i in start..end {
        let bm = &bsd.block_modes[i];
        let quant_mode = bm.quant_mode as usize;
        let decim_mode = bm.decimation_mode as usize;
        if quant_mode <= TUNE_MAX_ANGULAR_QUANT {
            low_value1[i] = low_values1[decim_mode][quant_mode];
            high_value1[i] = high_values1[decim_mode][quant_mode];
            low_value2[i] = low_values2[decim_mode][quant_mode];
            high_value2[i] = high_values2[decim_mode][quant_mode];
        } else {
            low_value1[i] = 0.0;
            high_value1[i] = 1.0;
            low_value2[i] = 0.0;
            high_value2[i] = 1.0;
        }
    }
}
