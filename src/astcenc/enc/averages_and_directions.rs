//! Port of `astcenc_averages_and_directions.cpp`: partition averages, dominant
//! directions and the squared error against the uncorrelated / same chroma lines.
//! Loops run 8 lanes wide with 4 lane accumulators like the AVX2 build.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::internal::*;
use super::vecmath::*;
use ktx_macros::isa;

#[isa]
/// Masked 8-wide accumulation of `data[i..i + 8]` into `acc` (`haccumulate(acc, loada(..), m)`).
#[inline]
fn acc_masked(acc: &mut Vfloat4, data: &[f32], i: usize, m: Vmask8) {
    Vfloat8::haccumulate_masked(acc, Vfloat8::loada(&data[i..]), m);
}

#[isa]
/// `compute_partition_averages_rgb` / `_rgba` with `comps` components (3 or 4).
fn compute_partition_averages(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    comps: usize,
    averages: &mut [Vfloat4; 4],
) {
    let partition_count = pi.partition_count as usize;
    let texel_count = blk.texel_count as usize;
    let mean = if comps == 3 {
        blk.data_mean.swz3(0, 1, 2)
    } else {
        blk.data_mean
    };

    if partition_count == 1 {
        averages[0] = mean;
        return;
    }

    // One accumulator per (partition < N - 1, component)
    let mut pp = [[Vfloat4::zero(); 4]; 3];
    let mut lane_id = Vint8::lane_id();
    let mut i = 0;
    while i < texel_count {
        let texel_partition = Vint8::from_u8(&pi.partition_of_texel[i..]);
        let lane_mask = lane_id.lt(Vint8::splat(texel_count as i32));
        lane_id += Vint8::splat(SIMD_WIDTH as i32);
        let mut masks = [Vmask8::default(); 3];
        for p in 0..partition_count - 1 {
            masks[p] = lane_mask & texel_partition.eq(Vint8::splat(p as i32));
        }
        for c in 0..comps {
            for p in 0..partition_count - 1 {
                acc_masked(&mut pp[p][c], blk.data(c), i, masks[p]);
            }
        }
        i += SIMD_WIDTH;
    }

    let block_total = mean * blk.texel_count as f32;
    let total = |p: usize| {
        let s = |c: usize| pp[p][c].hadd_s();
        if comps == 3 {
            Vfloat4::new3(s(0), s(1), s(2))
        } else {
            Vfloat4::new(s(0), s(1), s(2), s(3))
        }
    };
    let mut last = block_total;
    for p in 0..partition_count - 1 {
        let t = total(p);
        last = last - t;
        averages[p] = t / pi.partition_texel_count[p] as f32;
    }
    averages[partition_count - 1] = last / pi.partition_texel_count[partition_count - 1] as f32;
}

#[isa]
/// Shared direction search: sum of the positive half spaces per axis, pick the longest.
fn best_direction(texels: impl Iterator<Item = Vfloat4>, average: Vfloat4, axes: usize) -> Vfloat4 {
    let mut sums = [Vfloat4::zero(); 4];
    let zero = Vfloat4::zero();
    for t in texels {
        let texel_datum = t - average;
        for (a, sum) in sums.iter_mut().enumerate().take(axes) {
            let tdm = texel_datum.swz4(a, a, a, a).gt(zero);
            *sum += Vfloat4::select(zero, texel_datum, tdm);
        }
    }
    let mut best_vector = sums[0];
    let mut best_sum = sums[0].dot(sums[0]);
    for a in 1..axes {
        let prod = sums[a].dot(sums[a]);
        let mask = prod.gt(best_sum);
        best_vector = Vfloat4::select(best_vector, sums[a], mask);
        if a + 1 < axes {
            best_sum = Vfloat4::select(best_sum, prod, mask);
        }
    }
    best_vector
}

#[isa]
/// `compute_avgs_and_dirs_4_comp`.
pub fn compute_avgs_and_dirs_4_comp(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    pm: &mut [PartitionMetrics; 4],
) {
    let partition_count = pi.partition_count as usize;
    let mut partition_averages = [Vfloat4::zero(); 4];
    compute_partition_averages(pi, blk, 4, &mut partition_averages);
    for partition in 0..partition_count {
        let texel_indexes = &pi.texels_of_partition[partition];
        let texel_count = pi.partition_texel_count[partition] as usize;
        let average = partition_averages[partition];
        pm[partition].avg = average;
        pm[partition].dir = best_direction(
            texel_indexes[..texel_count]
                .iter()
                .map(|&i| blk.texel(i as usize)),
            average,
            4,
        );
    }
}

#[isa]
/// `compute_avgs_and_dirs_3_comp`.
pub fn compute_avgs_and_dirs_3_comp(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    omitted_component: u32,
    pm: &mut [PartitionMetrics; 4],
) {
    let mut partition_averages = [Vfloat4::zero(); 4];
    compute_partition_averages(pi, blk, 4, &mut partition_averages);
    let (lanes, planes): ([usize; 3], [usize; 3]) = match omitted_component {
        0 => ([1, 2, 3], [1, 2, 3]),
        1 => ([0, 2, 3], [0, 2, 3]),
        2 => ([0, 1, 3], [0, 1, 3]),
        _ => ([0, 1, 2], [0, 1, 2]),
    };
    for avg in partition_averages.iter_mut() {
        *avg = avg.swz3(lanes[0], lanes[1], lanes[2]);
    }
    let (vr, vg, vb) = (
        blk.data(planes[0]),
        blk.data(planes[1]),
        blk.data(planes[2]),
    );
    let partition_count = pi.partition_count as usize;
    for partition in 0..partition_count {
        let texel_indexes = &pi.texels_of_partition[partition];
        let texel_count = pi.partition_texel_count[partition] as usize;
        let average = partition_averages[partition];
        pm[partition].avg = average;
        let texels = texel_indexes[..texel_count].iter().map(|&i| {
            let i = i as usize;
            Vfloat4::new3(vr[i], vg[i], vb[i])
        });
        pm[partition].dir = best_direction(texels, average, 3);
    }
}

#[isa]
/// `compute_avgs_and_dirs_3_comp_rgb`.
pub fn compute_avgs_and_dirs_3_comp_rgb(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    pm: &mut [PartitionMetrics; 4],
) {
    let partition_count = pi.partition_count as usize;
    let mut partition_averages = [Vfloat4::zero(); 4];
    compute_partition_averages(pi, blk, 3, &mut partition_averages);
    for partition in 0..partition_count {
        let texel_indexes = &pi.texels_of_partition[partition];
        let texel_count = pi.partition_texel_count[partition] as usize;
        let average = partition_averages[partition];
        pm[partition].avg = average;
        pm[partition].dir = best_direction(
            texel_indexes[..texel_count]
                .iter()
                .map(|&i| blk.texel3(i as usize)),
            average,
            3,
        );
    }
}

#[isa]
/// `compute_avgs_and_dirs_2_comp`.
pub fn compute_avgs_and_dirs_2_comp(
    pt: &PartitionInfo,
    blk: &ImageBlock,
    component1: u32,
    component2: u32,
    pm: &mut [PartitionMetrics; 4],
) {
    let (mut average, vr, vg) = if component1 == 0 && component2 == 1 {
        (blk.data_mean.swz2(0, 1), blk.data(0), blk.data(1))
    } else if component1 == 0 && component2 == 2 {
        (blk.data_mean.swz2(0, 2), blk.data(0), blk.data(2))
    } else {
        (blk.data_mean.swz2(1, 2), blk.data(1), blk.data(2))
    };
    let partition_count = pt.partition_count as usize;
    for partition in 0..partition_count {
        let texel_indexes = &pt.texels_of_partition[partition];
        let texel_count = pt.partition_texel_count[partition] as usize;
        let idx = &texel_indexes[..texel_count];
        // Only compute a partition mean if more than one partition
        if partition_count > 1 {
            average = Vfloat4::zero();
            for &i in idx {
                average += Vfloat4::new(vr[i as usize], vg[i as usize], 0.0, 0.0);
            }
            average = average / texel_count as f32;
        }
        pm[partition].avg = average;
        let texels = idx
            .iter()
            .map(|&i| Vfloat4::new(vr[i as usize], vg[i as usize], 0.0, 0.0));
        pm[partition].dir = best_direction(texels, average, 2);
    }
}

#[isa]
/// Gathers 8 texels of component plane `c` by byte indices.
#[inline]
fn gather(blk: &ImageBlock, c: usize, idx: &[u8]) -> Vfloat8 {
    Vfloat8::gatherf_byte_inds(blk.data(c), idx)
}

#[isa]
/// `compute_error_squared_rgba`.
pub fn compute_error_squared_rgba(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    uncor_plines: &[ProcessedLine; 4],
    samec_plines: &[ProcessedLine; 4],
    line_lengths: &mut [f32; 4],
    uncor_error: &mut f32,
    samec_error: &mut f32,
) {
    let partition_count = pi.partition_count as usize;
    let mut uncor_errorsumv = Vfloat4::zero();
    let mut samec_errorsumv = Vfloat4::zero();

    for partition in 0..partition_count {
        let texel_indexes = &pi.texels_of_partition[partition];
        let l_uncor = uncor_plines[partition];
        let l_samec = samec_plines[partition];
        let texel_count = pi.partition_texel_count[partition] as usize;

        let ub = |k: usize| Vfloat8::splat(l_uncor.bs.lane(k));
        let ua = |k: usize| Vfloat8::splat(l_uncor.amod.lane(k));
        let sb = |k: usize| Vfloat8::splat(l_samec.bs.lane(k));
        let ew = |k: usize| Vfloat8::splat(blk.channel_weight.lane(k));

        let mut uncor_loparamv = Vfloat8::splat(1e10);
        let mut uncor_hiparamv = Vfloat8::splat(-1e10);

        let mut lane_ids = Vint8::lane_id();
        let mut i = 0;
        while i < texel_count {
            let mask = lane_ids.lt(Vint8::splat(texel_count as i32));
            let texel_idxs = &texel_indexes[i..];
            let d = [
                gather(blk, 0, texel_idxs),
                gather(blk, 1, texel_idxs),
                gather(blk, 2, texel_idxs),
                gather(blk, 3, texel_idxs),
            ];

            let uncor_param = (d[0] * ub(0)) + (d[1] * ub(1)) + (d[2] * ub(2)) + (d[3] * ub(3));
            uncor_loparamv = uncor_param.min(uncor_loparamv);
            uncor_hiparamv = uncor_param.max(uncor_hiparamv);

            let ud = |k: usize| (ua(k) - d[k]) + (uncor_param * ub(k));
            let (u0, u1, u2, u3) = (ud(0), ud(1), ud(2), ud(3));
            let uncor_err =
                (ew(0) * u0 * u0) + (ew(1) * u1 * u1) + (ew(2) * u2 * u2) + (ew(3) * u3 * u3);
            Vfloat8::haccumulate_masked(&mut uncor_errorsumv, uncor_err, mask);

            let samec_param = (d[0] * sb(0)) + (d[1] * sb(1)) + (d[2] * sb(2)) + (d[3] * sb(3));
            let sd = |k: usize| samec_param * sb(k) - d[k];
            let (s0, s1, s2, s3) = (sd(0), sd(1), sd(2), sd(3));
            let samec_err =
                (ew(0) * s0 * s0) + (ew(1) * s1 * s1) + (ew(2) * s2 * s2) + (ew(3) * s3 * s3);
            Vfloat8::haccumulate_masked(&mut samec_errorsumv, samec_err, mask);

            lane_ids += Vint8::splat(SIMD_WIDTH as i32);
            i += SIMD_WIDTH;
        }

        // Turn very small numbers and NaNs into a small number
        let uncor_linelen = uncor_hiparamv.hmax_s() - uncor_loparamv.hmin_s();
        line_lengths[partition] = super::mathlib::max(uncor_linelen, 1e-7);
    }

    *uncor_error = uncor_errorsumv.hadd_s();
    *samec_error = samec_errorsumv.hadd_s();
}

#[isa]
/// `compute_error_squared_rgb`.
pub fn compute_error_squared_rgb(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    plines: &mut [PartitionLines3; 4],
    uncor_error: &mut f32,
    samec_error: &mut f32,
) {
    let partition_count = pi.partition_count as usize;
    let mut uncor_errorsumv = Vfloat4::zero();
    let mut samec_errorsumv = Vfloat4::zero();

    for partition in 0..partition_count {
        let pl = &mut plines[partition];
        let texel_indexes = &pi.texels_of_partition[partition];
        let texel_count = pi.partition_texel_count[partition] as usize;
        let l_uncor = pl.uncor_pline;
        let l_samec = pl.samec_pline;

        let ub = |k: usize| Vfloat8::splat(l_uncor.bs.lane(k));
        let ua = |k: usize| Vfloat8::splat(l_uncor.amod.lane(k));
        let sb = |k: usize| Vfloat8::splat(l_samec.bs.lane(k));
        let ew = |k: usize| Vfloat8::splat(blk.channel_weight.lane(k));

        let mut uncor_loparamv = Vfloat8::splat(1e10);
        let mut uncor_hiparamv = Vfloat8::splat(-1e10);

        let mut lane_ids = Vint8::lane_id();
        let mut i = 0;
        while i < texel_count {
            let mask = lane_ids.lt(Vint8::splat(texel_count as i32));
            let texel_idxs = &texel_indexes[i..];
            let d = [
                gather(blk, 0, texel_idxs),
                gather(blk, 1, texel_idxs),
                gather(blk, 2, texel_idxs),
            ];

            let uncor_param = (d[0] * ub(0)) + (d[1] * ub(1)) + (d[2] * ub(2));
            uncor_loparamv = uncor_param.min(uncor_loparamv);
            uncor_hiparamv = uncor_param.max(uncor_hiparamv);

            let ud = |k: usize| (ua(k) - d[k]) + (uncor_param * ub(k));
            let (u0, u1, u2) = (ud(0), ud(1), ud(2));
            let uncor_err = (ew(0) * u0 * u0) + (ew(1) * u1 * u1) + (ew(2) * u2 * u2);
            Vfloat8::haccumulate_masked(&mut uncor_errorsumv, uncor_err, mask);

            let samec_param = (d[0] * sb(0)) + (d[1] * sb(1)) + (d[2] * sb(2));
            let sd = |k: usize| samec_param * sb(k) - d[k];
            let (s0, s1, s2) = (sd(0), sd(1), sd(2));
            let samec_err = (ew(0) * s0 * s0) + (ew(1) * s1 * s1) + (ew(2) * s2 * s2);
            Vfloat8::haccumulate_masked(&mut samec_errorsumv, samec_err, mask);

            lane_ids += Vint8::splat(SIMD_WIDTH as i32);
            i += SIMD_WIDTH;
        }

        // Turn very small numbers and NaNs into a small number
        let uncor_linelen = uncor_hiparamv.hmax_s() - uncor_loparamv.hmin_s();
        pl.line_length = super::mathlib::max(uncor_linelen, 1e-7);
    }

    *uncor_error = uncor_errorsumv.hadd_s();
    *samec_error = samec_errorsumv.hadd_s();
}
