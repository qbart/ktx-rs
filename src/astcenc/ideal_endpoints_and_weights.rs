//! Port of `astcenc_ideal_endpoints_and_weights.cpp`: ideal endpoints and weights per
//! partition, weight decimation, weight quantization, weight set errors and the
//! least squares endpoint recomputation.
//!
//! Weight buffers are passed as slices; lanes beyond a slice read as zero (the C++ reads
//! the padded tail of larger buffers, so callers pass the full working buffers).

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::averages_and_directions::*;
use super::internal::*;
use super::mathlib;
use super::tables::QUANT_AND_XFER_TABLES;
use super::vecmath::*;

/// `unit2()`.
#[allow(clippy::excessive_precision)]
fn unit2() -> Vfloat4 {
    let v = 0.707106769084930420f32;
    Vfloat4::new(v, v, 0.0, 0.0)
}

/// `unit3()`.
#[allow(clippy::excessive_precision)]
fn unit3() -> Vfloat4 {
    let v = 0.577350258827209473f32;
    Vfloat4::new(v, v, v, 0.0)
}

/// `unit4()`.
fn unit4() -> Vfloat4 {
    Vfloat4::splat(0.5)
}

/// `loada` from a slice at `i` (missing lanes read as zero).
#[inline]
fn ld(p: &[f32], i: usize) -> Vfloat8 {
    Vfloat8::load(p.get(i..).unwrap_or_default())
}

/// `storea` into a slice at `i` (lanes past the end are dropped).
#[inline]
fn st(v: Vfloat8, p: &mut [f32], i: usize) {
    if let Some(d) = p.get_mut(i..) {
        v.store(d);
    }
}

/// `bilinear_infill_vla`.
fn bilinear_infill_vla(di: &DecimationInfo, weights: &[f32], index: usize) -> Vfloat8 {
    let w = |k: usize| Vfloat8::gatherf_byte_inds(weights, &di.texel_weights_tr[k][index..]);
    let t = |k: usize| ld(&di.texel_weight_contribs_float_tr[k], index);
    (w(0) * t(0) + w(1) * t(1)) + (w(2) * t(2) + w(3) * t(3))
}

/// `bilinear_infill_vla_2`.
fn bilinear_infill_vla_2(di: &DecimationInfo, weights: &[f32], index: usize) -> Vfloat8 {
    let w = |k: usize| Vfloat8::gatherf_byte_inds(weights, &di.texel_weights_tr[k][index..]);
    let t = |k: usize| ld(&di.texel_weight_contribs_float_tr[k], index);
    w(0) * t(0) + w(1) * t(1)
}

/// Shared tail of the per component ideal weight computations: the SIMD tail of the
/// weight arrays is zeroed.
fn zero_tail(ei: &mut EndpointsAndWeights, texel_count: usize) {
    let simd = round_up_to_simd_multiple_vla(texel_count).min(BLOCK_MAX_TEXELS);
    for i in texel_count..simd {
        ei.weights[i] = 0.0;
        ei.weight_error_scale[i] = 0.0;
    }
}

/// Normalizes the line parameters of one partition into weights in [0, 1]; returns
/// (lowparam, highparam, length_squared).
fn normalize_params(
    ei: &mut EndpointsAndWeights,
    pi: &PartitionInfo,
    i: usize,
    mut lowparam: f32,
    mut highparam: f32,
    error_weight: f32,
    is_constant_wes: &mut bool,
    partition0_len_sq: &mut f32,
) -> (f32, f32) {
    // It is possible for a uniform-color partition to produce length=0;
    // this causes NaN issues so set to small value to avoid this problem
    if highparam <= lowparam {
        lowparam = 0.0;
        highparam = 1e-7;
    }
    let length = highparam - lowparam;
    let length_squared = length * length;
    let scale = 1.0 / length;
    if i == 0 {
        *partition0_len_sq = length_squared;
    } else {
        *is_constant_wes = *is_constant_wes && length_squared == *partition0_len_sq;
    }
    for j in 0..pi.partition_texel_count[i] as usize {
        let tix = pi.texels_of_partition[i][j] as usize;
        let idx = mathlib::clamp1f((ei.weights[tix] - lowparam) * scale);
        ei.weights[tix] = idx;
        ei.weight_error_scale[tix] = length_squared * error_weight;
    }
    (lowparam, highparam)
}

/// `compute_ideal_colors_and_weights_1_comp`.
fn compute_ideal_colors_and_weights_1_comp(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ei: &mut EndpointsAndWeights,
    component: usize,
) {
    let partition_count = pi.partition_count as usize;
    ei.ep.partition_count = partition_count as u32;
    let texel_count = blk.texel_count as usize;
    let error_weight = blk.channel_weight.lane(component.min(3));
    let data_vr = blk.data(component);
    let sep_mask = Vint4::lane_id().eq(Vint4::splat(component as i32));

    let mut is_constant_wes = true;
    let mut partition0_len_sq = 0.0f32;
    for i in 0..partition_count {
        let mut lowvalue = 1e10f32;
        let mut highvalue = -1e10f32;
        let n = pi.partition_texel_count[i] as usize;
        for j in 0..n {
            let tix = pi.texels_of_partition[i][j] as usize;
            let value = data_vr[tix];
            lowvalue = mathlib::min(value, lowvalue);
            highvalue = mathlib::max(value, highvalue);
        }
        if highvalue <= lowvalue {
            lowvalue = 0.0;
            highvalue = 1e-7;
        }
        let length = highvalue - lowvalue;
        let length_squared = length * length;
        let scale = 1.0 / length;
        if i == 0 {
            partition0_len_sq = length_squared;
        } else {
            is_constant_wes = is_constant_wes && length_squared == partition0_len_sq;
        }
        for j in 0..n {
            let tix = pi.texels_of_partition[i][j] as usize;
            let value = mathlib::clamp1f((data_vr[tix] - lowvalue) * scale);
            ei.weights[tix] = value;
            ei.weight_error_scale[tix] = length_squared * error_weight;
        }
        ei.ep.endpt0[i] = Vfloat4::select(blk.data_min, Vfloat4::splat(lowvalue), sep_mask);
        ei.ep.endpt1[i] = Vfloat4::select(blk.data_max, Vfloat4::splat(highvalue), sep_mask);
    }
    zero_tail(ei, texel_count);
    ei.is_constant_weight_error_scale = is_constant_wes;
}

/// `compute_ideal_colors_and_weights_2_comp`.
fn compute_ideal_colors_and_weights_2_comp(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ei: &mut EndpointsAndWeights,
    component1: u32,
    component2: u32,
) {
    let partition_count = pi.partition_count as usize;
    ei.ep.partition_count = partition_count as u32;
    let texel_count = blk.texel_count as usize;
    let mut pms = [PartitionMetrics::default(); 4];

    let (error_weight, data_vr, data_vg) = if component1 == 0 && component2 == 1 {
        (
            blk.channel_weight.swz2(0, 1).hadd_s() / 2.0,
            blk.data(0),
            blk.data(1),
        )
    } else if component1 == 0 && component2 == 2 {
        (
            blk.channel_weight.swz2(0, 2).hadd_s() / 2.0,
            blk.data(0),
            blk.data(2),
        )
    } else {
        (
            blk.channel_weight.swz2(1, 2).hadd_s() / 2.0,
            blk.data(1),
            blk.data(2),
        )
    };
    compute_avgs_and_dirs_2_comp(pi, blk, component1, component2, &mut pms);

    let mut is_constant_wes = true;
    let mut partition0_len_sq = 0.0f32;
    let comp1_mask = Vint4::lane_id().eq(Vint4::splat(component1 as i32));
    let comp2_mask = Vint4::lane_id().eq(Vint4::splat(component2 as i32));
    for i in 0..partition_count {
        let mut dir = pms[i].dir;
        if dir.hadd_s() < 0.0 {
            dir = Vfloat4::zero() - dir;
        }
        let line = Line {
            a: pms[i].avg,
            b: normalize_safe(dir, unit2()),
        };
        let mut lowparam = 1e10f32;
        let mut highparam = -1e10f32;
        for j in 0..pi.partition_texel_count[i] as usize {
            let tix = pi.texels_of_partition[i][j] as usize;
            let point = Vfloat4::new(data_vr[tix], data_vg[tix], 0.0, 0.0);
            let param = (point - line.a).dot_s(line.b);
            ei.weights[tix] = param;
            lowparam = mathlib::min(param, lowparam);
            highparam = mathlib::max(param, highparam);
        }
        let (lowparam, highparam) = normalize_params(
            ei,
            pi,
            i,
            lowparam,
            highparam,
            error_weight,
            &mut is_constant_wes,
            &mut partition0_len_sq,
        );
        let lowvalue = line.a + line.b * lowparam;
        let highvalue = line.a + line.b * highparam;
        let ep0 = Vfloat4::select(blk.data_min, Vfloat4::splat(lowvalue.lane(0)), comp1_mask);
        let ep1 = Vfloat4::select(blk.data_max, Vfloat4::splat(highvalue.lane(0)), comp1_mask);
        ei.ep.endpt0[i] = Vfloat4::select(ep0, Vfloat4::splat(lowvalue.lane(1)), comp2_mask);
        ei.ep.endpt1[i] = Vfloat4::select(ep1, Vfloat4::splat(highvalue.lane(1)), comp2_mask);
    }
    zero_tail(ei, texel_count);
    ei.is_constant_weight_error_scale = is_constant_wes;
}

/// `compute_ideal_colors_and_weights_3_comp`.
fn compute_ideal_colors_and_weights_3_comp(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ei: &mut EndpointsAndWeights,
    omitted_component: u32,
) {
    let partition_count = pi.partition_count as usize;
    ei.ep.partition_count = partition_count as u32;
    let texel_count = blk.texel_count as usize;
    let mut pms = [PartitionMetrics::default(); 4];
    let cw = blk.channel_weight;
    // Like the C++: omitting component 0 weights components 0, 1 and 2.
    let (error_weight, planes) = match omitted_component {
        0 => (cw.swz3(0, 1, 2).hadd_s(), [1, 2, 3]),
        1 => (cw.swz3(0, 2, 3).hadd_s(), [0, 2, 3]),
        2 => (cw.swz3(0, 1, 3).hadd_s(), [0, 1, 3]),
        _ => (cw.swz3(0, 1, 2).hadd_s(), [0, 1, 2]),
    };
    let (data_vr, data_vg, data_vb) = (
        blk.data(planes[0]),
        blk.data(planes[1]),
        blk.data(planes[2]),
    );
    let error_weight = error_weight * (1.0f32 / 3.0f32);

    if omitted_component == 3 {
        compute_avgs_and_dirs_3_comp_rgb(pi, blk, &mut pms);
    } else {
        compute_avgs_and_dirs_3_comp(pi, blk, omitted_component, &mut pms);
    }

    let mut is_constant_wes = true;
    let mut partition0_len_sq = 0.0f32;
    for i in 0..partition_count {
        let mut dir = pms[i].dir;
        if dir.hadd_rgb_s() < 0.0 {
            dir = Vfloat4::zero() - dir;
        }
        let line = Line {
            a: pms[i].avg,
            b: normalize_safe(dir, unit3()),
        };
        let mut lowparam = 1e10f32;
        let mut highparam = -1e10f32;
        for j in 0..pi.partition_texel_count[i] as usize {
            let tix = pi.texels_of_partition[i][j] as usize;
            let point = Vfloat4::new3(data_vr[tix], data_vg[tix], data_vb[tix]);
            let param = (point - line.a).dot3_s(line.b);
            ei.weights[tix] = param;
            lowparam = mathlib::min(param, lowparam);
            highparam = mathlib::max(param, highparam);
        }
        let (lowparam, highparam) = normalize_params(
            ei,
            pi,
            i,
            lowparam,
            highparam,
            error_weight,
            &mut is_constant_wes,
            &mut partition0_len_sq,
        );
        let ep0 = line.a + line.b * lowparam;
        let ep1 = line.a + line.b * highparam;
        let bmin = blk.data_min;
        let bmax = blk.data_max;
        let (e0, e1) = match omitted_component {
            0 => (
                Vfloat4::new(bmin.lane(0), ep0.lane(0), ep0.lane(1), ep0.lane(2)),
                Vfloat4::new(bmax.lane(0), ep1.lane(0), ep1.lane(1), ep1.lane(2)),
            ),
            1 => (
                Vfloat4::new(ep0.lane(0), bmin.lane(1), ep0.lane(1), ep0.lane(2)),
                Vfloat4::new(ep1.lane(0), bmax.lane(1), ep1.lane(1), ep1.lane(2)),
            ),
            2 => (
                Vfloat4::new(ep0.lane(0), ep0.lane(1), bmin.lane(2), ep0.lane(2)),
                Vfloat4::new(ep1.lane(0), ep1.lane(1), bmax.lane(2), ep1.lane(2)),
            ),
            _ => (
                Vfloat4::new(ep0.lane(0), ep0.lane(1), ep0.lane(2), bmin.lane(3)),
                Vfloat4::new(ep1.lane(0), ep1.lane(1), ep1.lane(2), bmax.lane(3)),
            ),
        };
        ei.ep.endpt0[i] = e0;
        ei.ep.endpt1[i] = e1;
    }
    zero_tail(ei, texel_count);
    ei.is_constant_weight_error_scale = is_constant_wes;
}

/// `compute_ideal_colors_and_weights_4_comp`.
fn compute_ideal_colors_and_weights_4_comp(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ei: &mut EndpointsAndWeights,
) {
    let error_weight = blk.channel_weight.hadd_s() / 4.0;
    let partition_count = pi.partition_count as usize;
    let texel_count = blk.texel_count as usize;
    // Like the C++, the 4 component path leaves ei.ep.partition_count unchanged.
    let mut pms = [PartitionMetrics::default(); 4];
    compute_avgs_and_dirs_4_comp(pi, blk, &mut pms);

    let mut is_constant_wes = true;
    let mut partition0_len_sq = 0.0f32;
    for i in 0..partition_count {
        let mut dir = pms[i].dir;
        if dir.hadd_rgb_s() < 0.0 {
            dir = Vfloat4::zero() - dir;
        }
        let line = Line {
            a: pms[i].avg,
            b: normalize_safe(dir, unit4()),
        };
        let mut lowparam = 1e10f32;
        let mut highparam = -1e10f32;
        for j in 0..pi.partition_texel_count[i] as usize {
            let tix = pi.texels_of_partition[i][j] as usize;
            let point = blk.texel(tix);
            let param = (point - line.a).dot_s(line.b);
            ei.weights[tix] = param;
            lowparam = mathlib::min(param, lowparam);
            highparam = mathlib::max(param, highparam);
        }
        // It is possible for a uniform-color partition to produce length=0;
        // this causes NaN issues so set to small value to avoid this problem
        if highparam <= lowparam {
            lowparam = 0.0;
            highparam = 1e-7;
        }
        let length = highparam - lowparam;
        let length_squared = length * length;
        let scale = 1.0 / length;
        if i == 0 {
            partition0_len_sq = length_squared;
        } else {
            is_constant_wes = is_constant_wes && length_squared == partition0_len_sq;
        }
        ei.ep.endpt0[i] = line.a + line.b * lowparam;
        ei.ep.endpt1[i] = line.a + line.b * highparam;
        for j in 0..pi.partition_texel_count[i] as usize {
            let tix = pi.texels_of_partition[i][j] as usize;
            let idx = mathlib::clamp1f((ei.weights[tix] - lowparam) * scale);
            ei.weights[tix] = idx;
            ei.weight_error_scale[tix] = length_squared * error_weight;
        }
    }
    zero_tail(ei, texel_count);
    ei.is_constant_weight_error_scale = is_constant_wes;
}

/// `compute_ideal_colors_and_weights_1plane`.
pub fn compute_ideal_colors_and_weights_1plane(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ei: &mut EndpointsAndWeights,
) {
    let uses_alpha = !blk.is_constant_channel(3);
    if uses_alpha {
        compute_ideal_colors_and_weights_4_comp(blk, pi, ei);
    } else {
        compute_ideal_colors_and_weights_3_comp(blk, pi, ei, 3);
    }
}

/// `compute_ideal_colors_and_weights_2planes`.
pub fn compute_ideal_colors_and_weights_2planes(
    bsd: &BlockSizeDescriptor,
    blk: &ImageBlock,
    plane2_component: u32,
    ei1: &mut EndpointsAndWeights,
    ei2: &mut EndpointsAndWeights,
) {
    let pi = bsd.get_partition_info(1, 0);
    let uses_alpha = !blk.is_constant_channel(3);
    match plane2_component {
        0 => {
            // Separate weights for red
            if uses_alpha {
                compute_ideal_colors_and_weights_3_comp(blk, pi, ei1, 0);
            } else {
                compute_ideal_colors_and_weights_2_comp(blk, pi, ei1, 1, 2);
            }
            compute_ideal_colors_and_weights_1_comp(blk, pi, ei2, 0);
        }
        1 => {
            // Separate weights for green
            if uses_alpha {
                compute_ideal_colors_and_weights_3_comp(blk, pi, ei1, 1);
            } else {
                compute_ideal_colors_and_weights_2_comp(blk, pi, ei1, 0, 2);
            }
            compute_ideal_colors_and_weights_1_comp(blk, pi, ei2, 1);
        }
        2 => {
            // Separate weights for blue
            if uses_alpha {
                compute_ideal_colors_and_weights_3_comp(blk, pi, ei1, 2);
            } else {
                compute_ideal_colors_and_weights_2_comp(blk, pi, ei1, 0, 1);
            }
            compute_ideal_colors_and_weights_1_comp(blk, pi, ei2, 2);
        }
        _ => {
            // Separate weights for alpha
            compute_ideal_colors_and_weights_3_comp(blk, pi, ei1, 3);
            compute_ideal_colors_and_weights_1_comp(blk, pi, ei2, 3);
        }
    }
}

/// The infilled (or direct) weights of 8 texels at `i`.
fn infill(di: &DecimationInfo, w: &[f32], i: usize) -> Vfloat8 {
    if di.max_texel_weight_count > 2 {
        bilinear_infill_vla(di, w, i)
    } else if di.max_texel_weight_count > 1 {
        bilinear_infill_vla_2(di, w, i)
    } else {
        ld(w, i)
    }
}

/// `compute_error_of_weight_set_1plane`.
pub fn compute_error_of_weight_set_1plane(
    eai: &EndpointsAndWeights,
    di: &DecimationInfo,
    dec_weight_quant_uvalue: &[f32],
) -> f32 {
    let mut error_summav = Vfloat4::zero();
    let texel_count = di.texel_count as usize;
    let mut i = 0;
    while i < texel_count {
        let current_values = infill(di, dec_weight_quant_uvalue, i);
        let actual_values = ld(&eai.weights, i);
        let diff = current_values - actual_values;
        let significance = ld(&eai.weight_error_scale, i);
        let error = diff * diff * significance;
        Vfloat8::haccumulate(&mut error_summav, error);
        i += SIMD_WIDTH;
    }
    error_summav.hadd_s()
}

/// `compute_error_of_weight_set_2planes`.
pub fn compute_error_of_weight_set_2planes(
    eai1: &EndpointsAndWeights,
    eai2: &EndpointsAndWeights,
    di: &DecimationInfo,
    dec_weight_quant_uvalue_plane1: &[f32],
    dec_weight_quant_uvalue_plane2: &[f32],
) -> f32 {
    let mut error_summav = Vfloat4::zero();
    let texel_count = di.texel_count as usize;
    let mut i = 0;
    while i < texel_count {
        // Plane 1
        let current_values1 = infill(di, dec_weight_quant_uvalue_plane1, i);
        let actual_values1 = ld(&eai1.weights, i);
        let diff = current_values1 - actual_values1;
        let error1 = diff * diff * ld(&eai1.weight_error_scale, i);
        // Plane 2
        let current_values2 = infill(di, dec_weight_quant_uvalue_plane2, i);
        let actual_values2 = ld(&eai2.weights, i);
        let diff = current_values2 - actual_values2;
        let error2 = diff * diff * ld(&eai2.weight_error_scale, i);
        Vfloat8::haccumulate(&mut error_summav, error1 + error2);
        i += SIMD_WIDTH;
    }
    error_summav.hadd_s()
}

/// `compute_ideal_weights_for_decimation`.
pub fn compute_ideal_weights_for_decimation(
    ei: &EndpointsAndWeights,
    di: &DecimationInfo,
    dec_weight_ideal_value: &mut [f32],
) {
    let texel_count = di.texel_count as usize;
    let weight_count = di.weight_count as usize;
    let is_direct = texel_count == weight_count;

    // This function includes a copy of the epw from eai_in to eai_out, but if we're dealing
    // with direct weights then we don't need to do anything else.
    if is_direct {
        let mut i = 0;
        while i < texel_count {
            st(ld(&ei.weights, i), dec_weight_ideal_value, i);
            i += SIMD_WIDTH;
        }
        return;
    }

    // If we have a decimated weight grid use fancy weighting
    let constant_wes = ei.is_constant_weight_error_scale;
    let mut weight_error_scale = Vfloat8::splat(ei.weight_error_scale[0]);

    // Compute an initial average for each decimated weight
    let mut i = 0;
    while i < weight_count {
        let mut weight_weight = Vfloat8::splat(1e-10);
        let mut initial_weight = Vfloat8::zero();
        let weight_texel_count = Vint8::from_u8(&di.weight_texel_count[i..]);
        let max_texel_count = weight_texel_count.hmax_s() as usize;
        for j in 0..max_texel_count {
            let texel = &di.weight_texels_tr[j][i..];
            let weight = ld(&di.weights_texel_contribs_tr[j], i);
            if !constant_wes {
                weight_error_scale = Vfloat8::gatherf_byte_inds(&ei.weight_error_scale, texel);
            }
            let contrib_weight = weight * weight_error_scale;
            weight_weight += contrib_weight;
            initial_weight += Vfloat8::gatherf_byte_inds(&ei.weights, texel) * contrib_weight;
        }
        st(initial_weight / weight_weight, dec_weight_ideal_value, i);
        i += SIMD_WIDTH;
    }

    // Populate the interpolated weight grid based on the initial average
    // Process SIMD-width texel coordinates at at time while we can. Safe to
    // over-process full SIMD vectors - the tail is zeroed.
    let mut infilled_weights = [0.0f32; BLOCK_MAX_TEXELS];
    let mut i = 0;
    while i < texel_count {
        let weight = if di.max_texel_weight_count <= 2 {
            bilinear_infill_vla_2(di, dec_weight_ideal_value, i)
        } else {
            bilinear_infill_vla(di, dec_weight_ideal_value, i)
        };
        st(weight, &mut infilled_weights, i);
        i += SIMD_WIDTH;
    }

    // Perform a single iteration of refinement
    // Empirically determined step size; larger values don't help but smaller drops image quality
    const STEPSIZE: f32 = 0.25;
    const CHD_SCALE: f32 = -WEIGHTS_TEXEL_SUM;
    let mut i = 0;
    while i < weight_count {
        let weight_val = ld(dec_weight_ideal_value, i);
        // Accumulate error weighting of all the texels using this weight
        let mut error_change0 = Vfloat8::splat(1e-10);
        let mut error_change1 = Vfloat8::splat(0.0);
        let weight_texel_count = Vint8::from_u8(&di.weight_texel_count[i..]);
        let max_texel_count = weight_texel_count.hmax_s() as usize;
        for j in 0..max_texel_count {
            let texel = &di.weight_texels_tr[j][i..];
            let contrib_weight = ld(&di.weights_texel_contribs_tr[j], i);
            if !constant_wes {
                weight_error_scale = Vfloat8::gatherf_byte_inds(&ei.weight_error_scale, texel);
            }
            let scale = weight_error_scale * contrib_weight;
            let old_weight = Vfloat8::gatherf_byte_inds(&infilled_weights, texel);
            let ideal_weight = Vfloat8::gatherf_byte_inds(&ei.weights, texel);
            error_change0 += contrib_weight * scale;
            error_change1 += (old_weight - ideal_weight) * scale;
        }
        let mut step = (error_change1 * CHD_SCALE) / error_change0;
        step = Vfloat8::clamp(-STEPSIZE, STEPSIZE, step);
        // Update the weight; note this can store negative values
        st(weight_val + step, dec_weight_ideal_value, i);
        i += SIMD_WIDTH;
    }
}

/// `compute_quantized_weights_for_decimation`.
pub fn compute_quantized_weights_for_decimation(
    di: &DecimationInfo,
    mut low_bound: f32,
    mut high_bound: f32,
    dec_weight_ideal_value: &[f32],
    weight_set_out: &mut [f32],
    quantized_weight_set: &mut [u8],
    quant_level: u32,
) {
    let weight_count = di.weight_count as usize;
    let qat = &QUANT_AND_XFER_TABLES[quant_level as usize];
    const QUANT_LEVELS_M1: [f32; 12] = [
        1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 9.0, 11.0, 15.0, 19.0, 23.0, 31.0,
    ];
    let steps_m1 = Vint8::splat(get_quant_level(quant_level) as i32 - 1);
    let quant_level_m1 = QUANT_LEVELS_M1[quant_level as usize];

    // Quantize the weight set using both the specified low/high bounds and standard 0..1 bounds
    if high_bound <= low_bound {
        low_bound = 0.0;
        high_bound = 1.0;
    }
    let mut rscale = high_bound - low_bound;
    let scale = 1.0 / rscale;
    let scaled_low_bound = low_bound * scale;
    rscale *= 1.0 / 64.0;

    let scalev = Vfloat8::splat(scale);
    let scaled_low_boundv = Vfloat8::splat(scaled_low_bound);
    let quant_level_m1v = Vfloat8::splat(quant_level_m1);
    let rscalev = Vfloat8::splat(rscale);
    let low_boundv = Vfloat8::splat(low_bound);

    // The vtable lookups return the table entries (pshufb with in-range indices).
    let lookup = |idx: Vint8| Vint8 {
        m: idx
            .m
            .map(|i| qat.quant_to_unquant[(i & 31) as usize] as i32),
    };

    let mut i = 0;
    while i < weight_count {
        let mut ix = ld(dec_weight_ideal_value, i) * scalev - scaled_low_boundv;
        ix = ix.clampzo();
        // Look up the two closest indexes and return the one that was closest
        let ix1 = ix * quant_level_m1v;
        let weightl = ix1.float_to_int();
        let weighth = (weightl + Vint8::splat(1)).min(steps_m1);
        let ixli = lookup(weightl);
        let ixhi = lookup(weighth);
        let mut ixl = ixli.int_to_float();
        let ixh = ixhi.int_to_float();
        let mask = (ixl + ixh).lt(Vfloat8::splat(128.0) * ix);
        let weight = Vint8::select(ixli, ixhi, mask);
        ixl = Vfloat8::select(ixl, ixh, mask);
        // Invert the weight-scaling that was done initially
        st(ixl * rscalev + low_boundv, weight_set_out, i);
        let bytes = weight.pack_low_bytes();
        if let Some(d) = quantized_weight_set.get_mut(i..) {
            for (d, s) in d.iter_mut().zip(bytes) {
                *d = s;
            }
        }
        i += SIMD_WIDTH;
    }
}

/// `compute_rgbo_vector`.
fn compute_rgbo_vector(
    rgba_weight_sum: Vfloat4,
    weight_weight_sum: Vfloat4,
    rgbq_sum: Vfloat4,
    psum: f32,
) -> Vfloat4 {
    let x = rgba_weight_sum.lane(0);
    let y = rgba_weight_sum.lane(1);
    let z = rgba_weight_sum.lane(2);
    let p = weight_weight_sum.lane(0);
    let q = weight_weight_sum.lane(1);
    let r = weight_weight_sum.lane(2);
    let s = psum;
    let pp = p * p;
    let qq = q * q;
    let rr = r * r;
    let szmrr = s * z - rr;
    let dt = szmrr * y - z * qq;
    let yp = y * p;
    let qx = q * x;
    let yx = y * x;
    let mzyp = -z * yp;
    let mzqx = -z * qx;
    let mryx = -r * yx;
    let zqp = z * q * p;
    let ryp = r * yp;
    let rqx = r * qx;
    // Compute the reciprocal of matrix determinant
    let rdet = 1.0 / (dt * x + mzyp * p);
    // Actually compute the adjugate, and then apply reciprocal determinant
    let mat0 = Vfloat4::new(dt, zqp, ryp, mzyp);
    let mat1 = Vfloat4::new(zqp, szmrr * x - z * pp, rqx, mzqx);
    let mat2 = Vfloat4::new(ryp, rqx, (s * y - qq) * x - y * pp, mryx);
    let mat3 = Vfloat4::new(mzyp, mzqx, mryx, z * yx);
    let vect = rgbq_sum * rdet;
    Vfloat4::new(
        mat0.dot_s(vect),
        mat1.dot_s(vect),
        mat2.dot_s(vect),
        mat3.dot_s(vect),
    )
}

/// Unquantized weights as floats (`int_to_float(vint(dec_weights_uquant + i)) / 64`).
fn dequant_weights(dec_weights_uquant: &[u8], weight_count: usize, out: &mut [f32]) {
    let mut i = 0;
    while i < weight_count {
        let v = Vint8::from_u8(dec_weights_uquant.get(i..).unwrap_or_default());
        st(v.int_to_float() * Vfloat8::splat(1.0 / 64.0), out, i);
        i += SIMD_WIDTH;
    }
}

/// `recompute_ideal_colors_1plane`.
pub fn recompute_ideal_colors_1plane(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    di: &DecimationInfo,
    dec_weights_uquant: &[u8],
    ep: &mut Endpoints,
    rgbs_vectors: &mut [Vfloat4; 4],
    rgbo_vectors: &mut [Vfloat4; 4],
) {
    let weight_count = di.weight_count as usize;
    let total_texel_count = blk.texel_count as usize;
    let partition_count = pi.partition_count as usize;

    let mut dec_weight = [0.0f32; BLOCK_MAX_WEIGHTS];
    dequant_weights(dec_weights_uquant, weight_count, &mut dec_weight);

    let mut undec_weight = [0.0f32; BLOCK_MAX_TEXELS];
    let undec: &[f32] = if di.max_texel_weight_count == 1 {
        &dec_weight
    } else {
        let mut i = 0;
        while i < total_texel_count {
            let w = if di.max_texel_weight_count <= 2 {
                bilinear_infill_vla_2(di, &dec_weight, i)
            } else {
                bilinear_infill_vla(di, &dec_weight, i)
            };
            st(w, &mut undec_weight, i);
            i += SIMD_WIDTH;
        }
        &undec_weight
    };

    let mut rgba_sum = blk.data_mean * blk.texel_count as f32;
    for i in 0..partition_count {
        let texel_count = pi.partition_texel_count[i] as usize;
        let texel_indexes = &pi.texels_of_partition[i];

        // Only compute a partition mean if more than one partition
        if partition_count > 1 {
            rgba_sum = Vfloat4::zero();
            for j in 0..texel_count {
                rgba_sum += blk.texel(texel_indexes[j] as usize);
            }
        }
        rgba_sum = rgba_sum * blk.channel_weight;
        let rgba_weight_sum = (blk.channel_weight * texel_count as f32).max_s(1e-17);
        let scale_dir = normalize((rgba_sum / rgba_weight_sum).swz3(0, 1, 2));

        let mut scale_max = 0.0f32;
        let mut scale_min = 1e10f32;
        let mut wmin1 = 1.0f32;
        let mut wmax1 = 0.0f32;
        let mut left_sum_s = 0.0f32;
        let mut middle_sum_s = 0.0f32;
        let mut right_sum_s = 0.0f32;
        let mut color_vec_x = Vfloat4::zero();
        let mut color_vec_y = Vfloat4::zero();
        let mut scale_vec = Vfloat4::zero();
        let mut weight_weight_sum_s = 1e-17f32;
        let color_weight = blk.channel_weight;
        let ls_weight = color_weight.hadd_rgb_s();

        for j in 0..texel_count {
            let tix = texel_indexes[j] as usize;
            let rgba = blk.texel(tix);
            let idx0 = undec[tix];
            let om_idx0 = 1.0 - idx0;
            wmin1 = mathlib::min(idx0, wmin1);
            wmax1 = mathlib::max(idx0, wmax1);
            let scale = scale_dir.dot3_s(rgba);
            scale_min = mathlib::min(scale, scale_min);
            scale_max = mathlib::max(scale, scale_max);
            left_sum_s += om_idx0 * om_idx0;
            middle_sum_s += om_idx0 * idx0;
            right_sum_s += idx0 * idx0;
            weight_weight_sum_s += idx0;
            let color_idx = Vfloat4::splat(idx0);
            let cwprod = rgba;
            let cwiprod = cwprod * color_idx;
            color_vec_y += cwiprod;
            color_vec_x += cwprod - cwiprod;
            scale_vec += Vfloat4::new(om_idx0, idx0, 0.0, 0.0) * (scale * ls_weight);
        }

        let left_sum = Vfloat4::splat(left_sum_s) * color_weight;
        let middle_sum = Vfloat4::splat(middle_sum_s) * color_weight;
        let right_sum = Vfloat4::splat(right_sum_s) * color_weight;
        let lmrs_sum = Vfloat4::new3(left_sum_s, middle_sum_s, right_sum_s) * ls_weight;
        color_vec_x = color_vec_x * color_weight;
        color_vec_y = color_vec_y * color_weight;

        // Initialize the luminance and scale vectors with a reasonable default
        let scalediv = mathlib::clamp1f(scale_min / mathlib::max(scale_max, 1e-10));
        let sds = scale_dir * scale_max;
        rgbs_vectors[i] = Vfloat4::new(sds.lane(0), sds.lane(1), sds.lane(2), scalediv);

        if wmin1 >= wmax1 * 0.999 {
            // If all weights in the partition were equal, then just take average of all colors in
            // the partition and use that as both endpoint colors
            let avg = (color_vec_x + color_vec_y) / rgba_weight_sum;
            let notnan_mask = avg.eq(avg);
            ep.endpt0[i] = Vfloat4::select(ep.endpt0[i], avg, notnan_mask);
            ep.endpt1[i] = Vfloat4::select(ep.endpt1[i], avg, notnan_mask);
            rgbs_vectors[i] = Vfloat4::new(sds.lane(0), sds.lane(1), sds.lane(2), 1.0);
        } else {
            // Otherwise, complete the analytic calculation of ideal-endpoint-values for the given
            // set of texel weights and pixel colors
            let color_det1 = (left_sum * right_sum) - (middle_sum * middle_sum);
            let color_rdet1 = 1.0 / color_det1;
            let ls_det1 =
                (lmrs_sum.lane(0) * lmrs_sum.lane(2)) - (lmrs_sum.lane(1) * lmrs_sum.lane(1));
            let ls_rdet1 = 1.0 / ls_det1;
            let color_mss1 =
                (left_sum * left_sum) + (2.0 * middle_sum * middle_sum) + (right_sum * right_sum);
            let ls_mss1 = (lmrs_sum.lane(0) * lmrs_sum.lane(0))
                + (2.0 * lmrs_sum.lane(1) * lmrs_sum.lane(1))
                + (lmrs_sum.lane(2) * lmrs_sum.lane(2));
            let ep0 = (right_sum * color_vec_x - middle_sum * color_vec_y) * color_rdet1;
            let ep1 = (left_sum * color_vec_y - middle_sum * color_vec_x) * color_rdet1;
            let det_mask = color_det1.abs().gt(color_mss1 * 1e-4);
            let notnan_mask = ep0.eq(ep0) & ep1.eq(ep1);
            let full_mask = det_mask & notnan_mask;
            ep.endpt0[i] = Vfloat4::select(ep.endpt0[i], ep0, full_mask);
            ep.endpt1[i] = Vfloat4::select(ep.endpt1[i], ep1, full_mask);

            let scale_ep0 = (lmrs_sum.lane(2) * scale_vec.lane(0)
                - lmrs_sum.lane(1) * scale_vec.lane(1))
                * ls_rdet1;
            let scale_ep1 = (lmrs_sum.lane(0) * scale_vec.lane(1)
                - lmrs_sum.lane(1) * scale_vec.lane(0))
                * ls_rdet1;
            if ls_det1.abs() > (ls_mss1 * 1e-4)
                && !scale_ep0.is_nan()
                && !scale_ep1.is_nan()
                && scale_ep0 < scale_ep1
            {
                let scalediv2 = scale_ep0 / scale_ep1;
                let sdsm = scale_dir * scale_ep1;
                rgbs_vectors[i] = Vfloat4::new(sdsm.lane(0), sdsm.lane(1), sdsm.lane(2), scalediv2);
            }
        }

        // Calculations specific to mode #7, the HDR RGB-scale mode - skip if known LDR
        if blk.rgb_lns[0] != 0 || blk.alpha_lns[0] != 0 {
            let weight_weight_sum = Vfloat4::splat(weight_weight_sum_s) * color_weight;
            let psum = right_sum_s * color_weight.hadd_rgb_s();
            let mut rgbq_sum = color_vec_x + color_vec_y;
            rgbq_sum.set_lane(3, color_vec_y.hadd_rgb_s());
            let rgbovec = compute_rgbo_vector(rgba_weight_sum, weight_weight_sum, rgbq_sum, psum);
            rgbo_vectors[i] = rgbovec;
            // We can get a failure due to the use of a singular (non-invertible) matrix
            // If it failed, compute rgbo_vectors[] with a different method ...
            if rgbovec.dot_s(rgbovec).is_nan() {
                let v0 = ep.endpt0[i];
                let v1 = ep.endpt1[i];
                let avgdif = mathlib::max((v1 - v0).hadd_rgb_s() * (1.0f32 / 3.0f32), 0.0);
                let avg = (v0 + v1) * 0.5;
                let ep0 = avg - Vfloat4::splat(avgdif) * 0.5;
                rgbo_vectors[i] = Vfloat4::new(ep0.lane(0), ep0.lane(1), ep0.lane(2), avgdif);
            }
        }
    }
}

/// `recompute_ideal_colors_2planes`.
pub fn recompute_ideal_colors_2planes(
    blk: &ImageBlock,
    bsd: &BlockSizeDescriptor,
    di: &DecimationInfo,
    dec_weights_uquant_plane1: &[u8],
    dec_weights_uquant_plane2: &[u8],
    ep: &mut Endpoints,
    rgbs_vector: &mut Vfloat4,
    rgbo_vector: &mut Vfloat4,
    plane2_component: i32,
) {
    let weight_count = di.weight_count as usize;
    let total_texel_count = blk.texel_count as usize;

    let mut dec_weight_plane1 = [0.0f32; BLOCK_MAX_WEIGHTS_2PLANE];
    let mut dec_weight_plane2 = [0.0f32; BLOCK_MAX_WEIGHTS_2PLANE];
    dequant_weights(
        dec_weights_uquant_plane1,
        weight_count,
        &mut dec_weight_plane1,
    );
    dequant_weights(
        dec_weights_uquant_plane2,
        weight_count,
        &mut dec_weight_plane2,
    );

    let mut undec1 = [0.0f32; BLOCK_MAX_TEXELS];
    let mut undec2 = [0.0f32; BLOCK_MAX_TEXELS];
    let (u1, u2): (&[f32], &[f32]) = if di.max_texel_weight_count == 1 {
        (&dec_weight_plane1, &dec_weight_plane2)
    } else {
        let mut i = 0;
        while i < total_texel_count {
            let (w1, w2) = if di.max_texel_weight_count <= 2 {
                (
                    bilinear_infill_vla_2(di, &dec_weight_plane1, i),
                    bilinear_infill_vla_2(di, &dec_weight_plane2, i),
                )
            } else {
                (
                    bilinear_infill_vla(di, &dec_weight_plane1, i),
                    bilinear_infill_vla(di, &dec_weight_plane2, i),
                )
            };
            st(w1, &mut undec1, i);
            st(w2, &mut undec2, i);
            i += SIMD_WIDTH;
        }
        (&undec1, &undec2)
    };

    let texel_count = bsd.texel_count as usize;
    let rgba_weight_sum = (blk.channel_weight * texel_count as f32).max_s(1e-17);
    let scale_dir = normalize(blk.data_mean.swz3(0, 1, 2));

    let mut scale_max = 0.0f32;
    let mut scale_min = 1e10f32;
    let mut wmin1 = 1.0f32;
    let mut wmax1 = 0.0f32;
    let mut wmin2 = 1.0f32;
    let mut wmax2 = 0.0f32;
    let mut left1_sum_s = 0.0f32;
    let mut middle1_sum_s = 0.0f32;
    let mut right1_sum_s = 0.0f32;
    let mut left2_sum_s = 0.0f32;
    let mut middle2_sum_s = 0.0f32;
    let mut right2_sum_s = 0.0f32;
    let mut color_vec_x = Vfloat4::zero();
    let mut color_vec_y = Vfloat4::zero();
    let mut scale_vec = Vfloat4::zero();
    let mut weight_weight_sum = Vfloat4::splat(1e-17);

    let p2_mask = Vint4::lane_id().eq(Vint4::splat(plane2_component));
    let color_weight = blk.channel_weight;
    let ls_weight = color_weight.hadd_rgb_s();

    for j in 0..texel_count {
        let rgba = blk.texel(j);
        let idx0 = u1.get(j).copied().unwrap_or(0.0);
        let om_idx0 = 1.0 - idx0;
        wmin1 = mathlib::min(idx0, wmin1);
        wmax1 = mathlib::max(idx0, wmax1);
        let scale = scale_dir.dot3_s(rgba);
        scale_min = mathlib::min(scale, scale_min);
        scale_max = mathlib::max(scale, scale_max);
        left1_sum_s += om_idx0 * om_idx0;
        middle1_sum_s += om_idx0 * idx0;
        right1_sum_s += idx0 * idx0;

        let idx1 = u2.get(j).copied().unwrap_or(0.0);
        let om_idx1 = 1.0 - idx1;
        wmin2 = mathlib::min(idx1, wmin2);
        wmax2 = mathlib::max(idx1, wmax2);
        left2_sum_s += om_idx1 * om_idx1;
        middle2_sum_s += om_idx1 * idx1;
        right2_sum_s += idx1 * idx1;

        let color_idx = Vfloat4::select(Vfloat4::splat(idx0), Vfloat4::splat(idx1), p2_mask);
        let cwprod = rgba;
        let cwiprod = cwprod * color_idx;
        color_vec_y += cwiprod;
        color_vec_x += cwprod - cwiprod;
        scale_vec += Vfloat4::new(om_idx0, idx0, 0.0, 0.0) * (ls_weight * scale);
        weight_weight_sum += color_idx;
    }

    let left1_sum = Vfloat4::splat(left1_sum_s) * color_weight;
    let middle1_sum = Vfloat4::splat(middle1_sum_s) * color_weight;
    let right1_sum = Vfloat4::splat(right1_sum_s) * color_weight;
    let lmrs_sum = Vfloat4::new3(left1_sum_s, middle1_sum_s, right1_sum_s) * ls_weight;
    let left2_sum = Vfloat4::splat(left2_sum_s) * color_weight;
    let middle2_sum = Vfloat4::splat(middle2_sum_s) * color_weight;
    let right2_sum = Vfloat4::splat(right2_sum_s) * color_weight;
    color_vec_x = color_vec_x * color_weight;
    color_vec_y = color_vec_y * color_weight;

    // Initialize the luminance and scale vectors with a reasonable default
    let scalediv = mathlib::clamp1f(scale_min / mathlib::max(scale_max, 1e-10));
    let sds = scale_dir * scale_max;
    *rgbs_vector = Vfloat4::new(sds.lane(0), sds.lane(1), sds.lane(2), scalediv);

    if wmin1 >= wmax1 * 0.999 {
        // If all weights in the partition were equal, then just take average of all colors in
        // the partition and use that as both endpoint colors
        let avg = (color_vec_x + color_vec_y) / rgba_weight_sum;
        let p1_mask = Vint4::lane_id().ne(Vint4::splat(plane2_component));
        let notnan_mask = avg.eq(avg);
        let full_mask = p1_mask & notnan_mask;
        ep.endpt0[0] = Vfloat4::select(ep.endpt0[0], avg, full_mask);
        ep.endpt1[0] = Vfloat4::select(ep.endpt1[0], avg, full_mask);
        *rgbs_vector = Vfloat4::new(sds.lane(0), sds.lane(1), sds.lane(2), 1.0);
    } else {
        // Otherwise, complete the analytic calculation of ideal-endpoint-values for the given
        // set of texel weights and pixel colors
        let color_det1 = (left1_sum * right1_sum) - (middle1_sum * middle1_sum);
        let color_rdet1 = 1.0 / color_det1;
        let ls_det1 = (lmrs_sum.lane(0) * lmrs_sum.lane(2)) - (lmrs_sum.lane(1) * lmrs_sum.lane(1));
        let ls_rdet1 = 1.0 / ls_det1;
        let color_mss1 =
            (left1_sum * left1_sum) + (2.0 * middle1_sum * middle1_sum) + (right1_sum * right1_sum);
        let ls_mss1 = (lmrs_sum.lane(0) * lmrs_sum.lane(0))
            + (2.0 * lmrs_sum.lane(1) * lmrs_sum.lane(1))
            + (lmrs_sum.lane(2) * lmrs_sum.lane(2));
        let ep0 = (right1_sum * color_vec_x - middle1_sum * color_vec_y) * color_rdet1;
        let ep1 = (left1_sum * color_vec_y - middle1_sum * color_vec_x) * color_rdet1;
        let scale_ep0 = (lmrs_sum.lane(2) * scale_vec.lane(0)
            - lmrs_sum.lane(1) * scale_vec.lane(1))
            * ls_rdet1;
        let scale_ep1 = (lmrs_sum.lane(0) * scale_vec.lane(1)
            - lmrs_sum.lane(1) * scale_vec.lane(0))
            * ls_rdet1;
        let p1_mask = Vint4::lane_id().ne(Vint4::splat(plane2_component));
        let det_mask = color_det1.abs().gt(color_mss1 * 1e-4);
        let notnan_mask = ep0.eq(ep0) & ep1.eq(ep1);
        let full_mask = p1_mask & det_mask & notnan_mask;
        ep.endpt0[0] = Vfloat4::select(ep.endpt0[0], ep0, full_mask);
        ep.endpt1[0] = Vfloat4::select(ep.endpt1[0], ep1, full_mask);
        if ls_det1.abs() > (ls_mss1 * 1e-4)
            && !scale_ep0.is_nan()
            && !scale_ep1.is_nan()
            && scale_ep0 < scale_ep1
        {
            let scalediv2 = scale_ep0 / scale_ep1;
            let sdsm = scale_dir * scale_ep1;
            *rgbs_vector = Vfloat4::new(sdsm.lane(0), sdsm.lane(1), sdsm.lane(2), scalediv2);
        }
    }

    if wmin2 >= wmax2 * 0.999 {
        // If all weights in the partition were equal, then just take average of all colors in
        // the partition and use that as both endpoint colors
        let avg = (color_vec_x + color_vec_y) / rgba_weight_sum;
        let notnan_mask = avg.eq(avg);
        let full_mask = p2_mask & notnan_mask;
        ep.endpt0[0] = Vfloat4::select(ep.endpt0[0], avg, full_mask);
        ep.endpt1[0] = Vfloat4::select(ep.endpt1[0], avg, full_mask);
    } else {
        // Otherwise, complete the analytic calculation of ideal-endpoint-values for the given
        // set of texel weights and pixel colors
        let color_det2 = (left2_sum * right2_sum) - (middle2_sum * middle2_sum);
        let color_rdet2 = 1.0 / color_det2;
        let color_mss2 =
            (left2_sum * left2_sum) + (2.0 * middle2_sum * middle2_sum) + (right2_sum * right2_sum);
        let ep0 = (right2_sum * color_vec_x - middle2_sum * color_vec_y) * color_rdet2;
        let ep1 = (left2_sum * color_vec_y - middle2_sum * color_vec_x) * color_rdet2;
        let det_mask = color_det2.abs().gt(color_mss2 * 1e-4);
        let notnan_mask = ep0.eq(ep0) & ep1.eq(ep1);
        let full_mask = p2_mask & det_mask & notnan_mask;
        ep.endpt0[0] = Vfloat4::select(ep.endpt0[0], ep0, full_mask);
        ep.endpt1[0] = Vfloat4::select(ep.endpt1[0], ep1, full_mask);
    }

    // Calculations specific to mode #7, the HDR RGB-scale mode - skip if known LDR
    if blk.rgb_lns[0] != 0 || blk.alpha_lns[0] != 0 {
        weight_weight_sum = weight_weight_sum * color_weight;
        let psum = Vfloat4::select(right1_sum, right2_sum, p2_mask).dot3_s(color_weight);
        let mut rgbq_sum = color_vec_x + color_vec_y;
        rgbq_sum.set_lane(3, color_vec_y.hadd_rgb_s());
        *rgbo_vector = compute_rgbo_vector(rgba_weight_sum, weight_weight_sum, rgbq_sum, psum);
        // We can get a failure due to the use of a singular (non-invertible) matrix
        // If it failed, compute rgbo_vectors[] with a different method ...
        if rgbo_vector.dot_s(*rgbo_vector).is_nan() {
            let v0 = ep.endpt0[0];
            let v1 = ep.endpt1[0];
            let avgdif = mathlib::max((v1 - v0).hadd_rgb_s() * (1.0f32 / 3.0f32), 0.0);
            let avg = (v0 + v1) * 0.5;
            let ep0 = avg - Vfloat4::splat(avgdif) * 0.5;
            *rgbo_vector = Vfloat4::new(ep0.lane(0), ep0.lane(1), ep0.lane(2), avgdif);
        }
    }
}
