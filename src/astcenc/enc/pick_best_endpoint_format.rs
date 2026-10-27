//! Port of `astcenc_pick_best_endpoint_format.cpp`: estimates the error of each
//! endpoint format per quantization level and integer count, then picks the best
//! format combination for every candidate block mode.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::averages_and_directions::compute_avgs_and_dirs_3_comp_rgb;
use super::internal::*;
use super::mathlib;
use super::tables::QUANT_MODE_TABLE;
use super::vecmath::*;
use ktx_macros::isa;

#[isa]
/// `unit3()`.
#[allow(clippy::excessive_precision)]
fn unit3() -> Vfloat4 {
    let v = 0.577350258827209473f32;
    Vfloat4::new(v, v, v, 0.0)
}

#[isa]
/// `quant_mode_table[integer_count][bits]` (out of range reads -1).
fn qmt(integer_count: usize, bits: i32) -> i32 {
    usize::try_from(bits)
        .ok()
        .and_then(|b| QUANT_MODE_TABLE.get(integer_count)?.get(b))
        .map_or(-1, |&q| i32::from(q))
}

const Q6: usize = QUANT_6 as usize;
const Q16: usize = QUANT_16 as usize;
const Q192: usize = QUANT_192 as usize;
const Q256: usize = QUANT_256 as usize;

/// Port of `encoding_choice_errors`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EncodingChoiceErrors {
    pub rgb_scale_error: f32,
    pub rgb_luma_error: f32,
    pub luminance_error: f32,
    pub alpha_drop_error: f32,
    pub can_offset_encode: bool,
    pub can_blue_contract: bool,
}

#[isa]
/// `compute_error_squared_rgb_single_partition`: errors of the uncorrelated,
/// same chroma, rgb luma and luminance lines, plus the alpha drop error.
fn compute_error_squared_rgb_single_partition(
    pi: &PartitionInfo,
    partition_index: usize,
    blk: &ImageBlock,
    plines: [&ProcessedLine; 4],
) -> ([f32; 4], f32) {
    let ews = blk.channel_weight;
    let texel_count = pi.partition_texel_count[partition_index] as usize;
    let texel_indexes = &pi.texels_of_partition[partition_index];

    let mut a_drop_errv = Vfloat4::zero();
    let default_a = Vfloat8::splat(blk.get_default_alpha());
    let mut errv = [Vfloat4::zero(); 4];
    let bs = plines.map(|l| [0, 1, 2].map(|k| Vfloat8::splat(l.bs.lane(k))));
    let amod = plines.map(|l| [0, 1, 2].map(|k| Vfloat8::splat(l.amod.lane(k))));
    // The same chroma and luminance lines go through the origin
    let has_amod = [true, false, true, false];

    let mut lane_ids = Vint8::lane_id();
    let mut i = 0;
    while i < texel_count {
        let tix = texel_indexes.get(i..).unwrap_or_default();
        let mask = lane_ids.lt(Vint8::splat(texel_count as i32));
        lane_ids += Vint8::splat(SIMD_WIDTH as i32);

        let data_a = Vfloat8::gatherf_byte_inds(&blk.data_a, tix);
        let mut alpha_diff = data_a - default_a;
        alpha_diff = alpha_diff * alpha_diff;
        Vfloat8::haccumulate_masked(&mut a_drop_errv, alpha_diff, mask);

        let d = [
            Vfloat8::gatherf_byte_inds(&blk.data_r, tix),
            Vfloat8::gatherf_byte_inds(&blk.data_g, tix),
            Vfloat8::gatherf_byte_inds(&blk.data_b, tix),
        ];
        for l in 0..4 {
            let b = &bs[l];
            let param = d[0] * b[0] + d[1] * b[1] + d[2] * b[2];
            let dist = |k: usize| {
                if has_amod[l] {
                    (amod[l][k] + param * b[k]) - d[k]
                } else {
                    (param * b[k]) - d[k]
                }
            };
            let (d0, d1, d2) = (dist(0), dist(1), dist(2));
            let error = d0 * d0 * ews.lane(0) + d1 * d1 * ews.lane(1) + d2 * d2 * ews.lane(2);
            Vfloat8::haccumulate_masked(&mut errv[l], error, mask);
        }
        i += SIMD_WIDTH;
    }

    (
        errv.map(Vfloat4::hadd_s),
        a_drop_errv.hadd_s() * ews.lane(3),
    )
}

#[isa]
/// `compute_encoding_choice_errors`.
pub fn compute_encoding_choice_errors(
    blk: &ImageBlock,
    pi: &PartitionInfo,
    ep: &Endpoints,
    eci: &mut [EncodingChoiceErrors; BLOCK_MAX_PARTITIONS],
) {
    let partition_count = pi.partition_count as usize;
    let mut pms = [PartitionMetrics::default(); BLOCK_MAX_PARTITIONS];
    compute_avgs_and_dirs_3_comp_rgb(pi, blk, &mut pms);

    for i in 0..partition_count {
        let pm = &pms[i];
        let uncor_rgb_lines = Line {
            a: pm.avg,
            b: normalize_safe(pm.dir, unit3()),
        };
        let samec_rgb_lines = Line {
            a: Vfloat4::zero(),
            b: normalize_safe(pm.avg, unit3()),
        };
        let rgb_luma_lines = Line {
            a: pm.avg,
            b: unit3(),
        };

        let uncor_rgb_plines = ProcessedLine {
            amod: uncor_rgb_lines.a - uncor_rgb_lines.b * uncor_rgb_lines.a.dot3(uncor_rgb_lines.b),
            bs: uncor_rgb_lines.b,
        };
        let samec_rgb_plines = ProcessedLine {
            amod: Vfloat4::zero(),
            bs: samec_rgb_lines.b,
        };
        let rgb_luma_plines = ProcessedLine {
            amod: rgb_luma_lines.a - rgb_luma_lines.b * rgb_luma_lines.a.dot3(rgb_luma_lines.b),
            bs: rgb_luma_lines.b,
        };
        let luminance_plines = ProcessedLine {
            amod: Vfloat4::zero(),
            bs: unit3(),
        };

        let (
            [
                uncorr_rgb_error,
                samechroma_rgb_error,
                rgb_luma_error,
                luminance_rgb_error,
            ],
            alpha_drop_error,
        ) = compute_error_squared_rgb_single_partition(
            pi,
            i,
            blk,
            [
                &uncor_rgb_plines,
                &samec_rgb_plines,
                &rgb_luma_plines,
                &luminance_plines,
            ],
        );

        // Determine if we can offset encode RGB lanes
        let endpt_diff = (ep.endpt1[i] - ep.endpt0[i]).abs();
        let endpt_can_offset = endpt_diff.lt(Vfloat4::splat(0.12 * 65535.0));
        let can_offset_encode = (endpt_can_offset.mask() & 0x7) == 0x7;

        eci[i].rgb_scale_error = (samechroma_rgb_error - uncorr_rgb_error) * 0.7;
        eci[i].rgb_luma_error = (rgb_luma_error - uncorr_rgb_error) * 1.5;
        eci[i].luminance_error = (luminance_rgb_error - uncorr_rgb_error) * 3.0;
        eci[i].alpha_drop_error = alpha_drop_error * 3.0;
        eci[i].can_offset_encode = can_offset_encode;
        eci[i].can_blue_contract = !blk.is_luminance();
    }
}

const BQE: f32 = 65536.0 * 65536.0 / 18.0;

static BASELINE_QUANT_ERROR: [f32; 21 - Q6] = [
    BQE / (5 * 5) as f32,
    BQE / (7 * 7) as f32,
    BQE / (9 * 9) as f32,
    BQE / (11 * 11) as f32,
    BQE / (15 * 15) as f32,
    BQE / (19 * 19) as f32,
    BQE / (23 * 23) as f32,
    BQE / (31 * 31) as f32,
    BQE / (39 * 39) as f32,
    BQE / (47 * 47) as f32,
    BQE / (63 * 63) as f32,
    BQE / (79 * 79) as f32,
    BQE / (95 * 95) as f32,
    BQE / (127 * 127) as f32,
    BQE / (159 * 159) as f32,
    BQE / (191 * 191) as f32,
    BQE / (255 * 255) as f32,
];

#[isa]
/// `compute_color_error_for_every_integer_count_and_quant_level`.
pub fn compute_color_error_for_every_integer_count_and_quant_level(
    encode_hdr_rgb: bool,
    encode_hdr_alpha: bool,
    partition_index: usize,
    pi: &PartitionInfo,
    eci: &EncodingChoiceErrors,
    ep: &Endpoints,
    error_weight: Vfloat4,
    best_error: &mut [[f32; 4]; 21],
    format_of_choice: &mut [[u8; 4]; 21],
) {
    let partition_size = pi.partition_texel_count[partition_index] as f32;

    let ep0 = ep.endpt0[partition_index];
    let ep1 = ep.endpt1[partition_index];

    let ep1_min = mathlib::max(ep1.hmin_rgb_s(), 0.0);
    let error_weight_rgbsum = error_weight.hadd_rgb_s();

    let range_upper_limit_rgb = if encode_hdr_rgb { 61440.0 } else { 65535.0 };
    let range_upper_limit_alpha = if encode_hdr_alpha { 61440.0 } else { 65535.0 };

    // It is possible to get endpoint colors significantly outside [0,upper-limit] even if the
    // input data are safely contained in [0,upper-limit]; we need to add an error term for this
    let offset = Vfloat4::new(
        range_upper_limit_rgb,
        range_upper_limit_rgb,
        range_upper_limit_rgb,
        range_upper_limit_alpha,
    );
    let ep0_range_error_high = (ep0 - offset).max_s(0.0);
    let ep1_range_error_high = (ep1 - offset).max_s(0.0);
    let ep0_range_error_low = ep0.min_s(0.0);
    let ep1_range_error_low = ep1.min_s(0.0);

    let sum_range_error = (ep0_range_error_low * ep0_range_error_low)
        + (ep1_range_error_low * ep1_range_error_low)
        + (ep0_range_error_high * ep0_range_error_high)
        + (ep1_range_error_high * ep1_range_error_high);

    let rgb_range_error = sum_range_error.dot3_s(error_weight) * 0.5 * partition_size;
    let alpha_range_error = sum_range_error.lane(3) * error_weight.lane(3) * 0.5 * partition_size;

    if encode_hdr_rgb {
        // Collect some statistics
        let (af, cf) = if ep1.lane(0) > ep1.lane(1) && ep1.lane(0) > ep1.lane(2) {
            (ep1.lane(0), ep1.lane(0) - ep0.lane(0))
        } else if ep1.lane(1) > ep1.lane(2) {
            (ep1.lane(1), ep1.lane(1) - ep0.lane(1))
        } else {
            (ep1.lane(2), ep1.lane(2) - ep0.lane(2))
        };

        // Estimate of color-component spread in high endpoint color
        let bf = af - ep1_min;
        let prd = (ep1 - Vfloat4::splat(cf)).swz3(0, 1, 2);
        let pdif = prd - ep0.swz3(0, 1, 2);
        // Estimate of color-component spread in low endpoint color
        let df = pdif.abs().hmax_s();

        let b = mathlib::clamp(bf, 0.0, 65536.0) as i32;
        let c = mathlib::clamp(cf, 0.0, 65536.0) as i32;
        let d = mathlib::clamp(df, 0.0, 65536.0) as i32;

        // Determine which one of the 6 submodes is likely to be used in case of an RGBO-mode
        let mut rgbo_mode = 5; // 7 bits per component
        // mode 4: 8 7 6
        if b < 32768 && c < 16384 {
            rgbo_mode = 4;
        }
        // mode 3: 9 6 7
        if b < 8192 && c < 16384 {
            rgbo_mode = 3;
        }
        // mode 2: 10 5 8
        if b < 2048 && c < 16384 {
            rgbo_mode = 2;
        }
        // mode 1: 11 6 5
        if b < 2048 && c < 1024 {
            rgbo_mode = 1;
        }
        // mode 0: 11 5 7
        if b < 1024 && c < 4096 {
            rgbo_mode = 0;
        }

        // Determine which one of the 9 submodes is likely to be used in case of an RGB-mode.
        let mut rgb_mode = 8; // 8 bits per component, except 7 bits for blue
        // mode 0: 9 7 6 7
        if b < 16384 && c < 8192 && d < 8192 {
            rgb_mode = 0;
        }
        // mode 1: 9 8 6 6
        if b < 32768 && c < 8192 && d < 4096 {
            rgb_mode = 1;
        }
        // mode 2: 10 6 7 7
        if b < 4096 && c < 8192 && d < 4096 {
            rgb_mode = 2;
        }
        // mode 3: 10 7 7 6
        if b < 8192 && c < 8192 && d < 2048 {
            rgb_mode = 3;
        }
        // mode 4: 11 8 6 5
        if b < 8192 && c < 2048 && d < 512 {
            rgb_mode = 4;
        }
        // mode 5: 11 6 8 6
        if b < 2048 && c < 8192 && d < 1024 {
            rgb_mode = 5;
        }
        // mode 6: 12 7 7 5
        if b < 2048 && c < 2048 && d < 256 {
            rgb_mode = 6;
        }
        // mode 7: 12 6 7 6
        if b < 1024 && c < 2048 && d < 512 {
            rgb_mode = 7;
        }

        const RGBO_ERROR_SCALES: [f32; 6] = [4.0, 4.0, 16.0, 64.0, 256.0, 1024.0];
        const RGB_ERROR_SCALES: [f32; 9] = [64.0, 64.0, 16.0, 16.0, 4.0, 4.0, 1.0, 1.0, 384.0];

        let mode7mult = RGBO_ERROR_SCALES[rgbo_mode] * 0.0015; // Empirically determined ....
        let mode11mult = RGB_ERROR_SCALES[rgb_mode] * 0.010; // Empirically determined ....

        let lum_high = ep1.hadd_rgb_s() * (1.0 / 3.0);
        let lum_low = ep0.hadd_rgb_s() * (1.0 / 3.0);
        let lumdif = lum_high - lum_low;
        let mut mode23mult = if lumdif < 960.0 {
            4.0
        } else if lumdif < 3968.0 {
            16.0
        } else {
            128.0
        };
        mode23mult *= 0.0005; // Empirically determined ....

        let hdr_alpha_fmt = if encode_hdr_alpha {
            FMT_HDR_RGBA
        } else {
            FMT_HDR_RGB_LDR_ALPHA
        };

        // Pick among the available HDR endpoint modes
        for i in 0..Q16 {
            best_error[i] = [ERROR_CALC_DEFAULT; 4];
            format_of_choice[i] = [
                FMT_HDR_LUMINANCE_LARGE_RANGE,
                FMT_HDR_RGB_SCALE,
                FMT_HDR_RGB,
                hdr_alpha_fmt,
            ];
        }

        for i in Q16..=Q256 {
            let base_quant_error = BASELINE_QUANT_ERROR[i - Q6] * partition_size;
            let rgb_quantization_error = error_weight_rgbsum * base_quant_error * 2.0;
            let alpha_quantization_error = error_weight.lane(3) * base_quant_error * 2.0;
            let rgba_quantization_error = rgb_quantization_error + alpha_quantization_error;

            // For 8 integers, we have two encodings: one with HDR A and another one with LDR A
            let full_hdr_rgba_error = rgba_quantization_error + rgb_range_error + alpha_range_error;
            best_error[i][3] = full_hdr_rgba_error;
            format_of_choice[i][3] = hdr_alpha_fmt;

            // For 6 integers, we have one HDR-RGB encoding
            let full_hdr_rgb_error =
                (rgb_quantization_error * mode11mult) + rgb_range_error + eci.alpha_drop_error;
            best_error[i][2] = full_hdr_rgb_error;
            format_of_choice[i][2] = FMT_HDR_RGB;

            // For 4 integers, we have one HDR-RGB-Scale encoding
            let hdr_rgb_scale_error = (rgb_quantization_error * mode7mult)
                + rgb_range_error
                + eci.alpha_drop_error
                + eci.rgb_luma_error;
            best_error[i][1] = hdr_rgb_scale_error;
            format_of_choice[i][1] = FMT_HDR_RGB_SCALE;

            // For 2 integers, we assume luminance-with-large-range
            let hdr_luminance_error = (rgb_quantization_error * mode23mult)
                + rgb_range_error
                + eci.alpha_drop_error
                + eci.luminance_error;
            best_error[i][0] = hdr_luminance_error;
            format_of_choice[i][0] = FMT_HDR_LUMINANCE_LARGE_RANGE;
        }
    } else {
        for i in 0..Q6 {
            best_error[i] = [ERROR_CALC_DEFAULT; 4];
            format_of_choice[i] = [FMT_LUMINANCE, FMT_RGB_SCALE, FMT_RGB, FMT_RGBA];
        }

        let base_quant_error_rgb = error_weight_rgbsum * partition_size;
        let base_quant_error_a = error_weight.lane(3) * partition_size;
        let base_quant_error_rgba = base_quant_error_rgb + base_quant_error_a;

        let error_scale_bc_rgba = if eci.can_blue_contract { 0.625 } else { 1.0 };
        let mut error_scale_oe_rgba = if eci.can_offset_encode { 0.5 } else { 1.0 };

        let error_scale_bc_rgb = if eci.can_blue_contract { 0.5 } else { 1.0 };
        let mut error_scale_oe_rgb = if eci.can_offset_encode { 0.25 } else { 1.0 };

        // Pick among the available LDR endpoint modes
        for i in Q6..=Q256 {
            // Offset encoding not possible at higher quant levels
            if i >= Q192 {
                error_scale_oe_rgba = 1.0;
                error_scale_oe_rgb = 1.0;
            }

            let base_quant_error = BASELINE_QUANT_ERROR[i - Q6];
            let quant_error_rgb = base_quant_error_rgb * base_quant_error;
            let quant_error_rgba = base_quant_error_rgba * base_quant_error;

            // 8 integers can encode as RGBA+RGBA
            let full_ldr_rgba_error = quant_error_rgba * error_scale_bc_rgba * error_scale_oe_rgba
                + rgb_range_error
                + alpha_range_error;
            best_error[i][3] = full_ldr_rgba_error;
            format_of_choice[i][3] = FMT_RGBA;

            // 6 integers can encode as RGB+RGB or RGBS+AA
            let full_ldr_rgb_error = quant_error_rgb * error_scale_bc_rgb * error_scale_oe_rgb
                + rgb_range_error
                + eci.alpha_drop_error;
            let rgbs_alpha_error =
                quant_error_rgba + eci.rgb_scale_error + rgb_range_error + alpha_range_error;

            if rgbs_alpha_error < full_ldr_rgb_error {
                best_error[i][2] = rgbs_alpha_error;
                format_of_choice[i][2] = FMT_RGB_SCALE_ALPHA;
            } else {
                best_error[i][2] = full_ldr_rgb_error;
                format_of_choice[i][2] = FMT_RGB;
            }

            // 4 integers can encode as RGBS or LA+LA
            let ldr_rgbs_error =
                quant_error_rgb + rgb_range_error + eci.alpha_drop_error + eci.rgb_scale_error;
            let lum_alpha_error =
                quant_error_rgba + rgb_range_error + alpha_range_error + eci.luminance_error;

            if ldr_rgbs_error < lum_alpha_error {
                best_error[i][1] = ldr_rgbs_error;
                format_of_choice[i][1] = FMT_RGB_SCALE;
            } else {
                best_error[i][1] = lum_alpha_error;
                format_of_choice[i][1] = FMT_LUMINANCE_ALPHA;
            }

            // 2 integers can encode as L+L
            let luminance_error =
                quant_error_rgb + rgb_range_error + eci.alpha_drop_error + eci.luminance_error;
            best_error[i][0] = luminance_error;
            format_of_choice[i][0] = FMT_LUMINANCE;
        }
    }
}

#[isa]
/// `one_partition_find_best_combination_for_bitcount`.
fn one_partition_find_best_combination_for_bitcount(
    best_combined_error: &[[f32; 4]; 21],
    best_combined_format: &[[u8; 4]; 21],
    bits_available: i32,
    best_quant_level: &mut u8,
    best_format: &mut u8,
) -> f32 {
    let mut best_integer_count = 0usize;
    let mut best_integer_count_error = ERROR_CALC_DEFAULT;

    for integer_count in 1..=4usize {
        // Compute the quantization level for a given number of integers and a given number of bits
        let quant_level = qmt(integer_count, bits_available);

        // Don't have enough bits to represent a given endpoint format at all!
        if quant_level < QUANT_6 as i32 {
            continue;
        }

        let integer_count_error = best_combined_error[quant_level as usize][integer_count - 1];
        if integer_count_error < best_integer_count_error {
            best_integer_count_error = integer_count_error;
            best_integer_count = integer_count - 1;
        }
    }

    let ql = qmt(best_integer_count + 1, bits_available);

    *best_quant_level = ql as u8;
    *best_format = FMT_LUMINANCE;

    if ql >= QUANT_6 as i32 {
        *best_format = best_combined_format[ql as usize][best_integer_count];
    }

    best_integer_count_error
}

#[isa]
/// `{two,three,four}_partitions_find_best_combination_for_every_quantization_and_integer_count`:
/// `N` partitions, `C` = 3 * N + 1 combined integer counts.
fn n_partitions_find_best_combination<const N: usize, const C: usize>(
    best_error: &[[[f32; 4]; 21]; BLOCK_MAX_PARTITIONS],
    best_format: &[[[u8; 4]; 21]; BLOCK_MAX_PARTITIONS],
    best_combined_error: &mut [[f32; C]; 21],
    best_combined_format: &mut [[[u8; N]; C]; 21],
) {
    for row in best_combined_error.iter_mut() {
        *row = [ERROR_CALC_DEFAULT; C];
    }

    for quant in Q6..=Q256 {
        // Integer counts per endpoint pair; no two may differ by more than one
        let mut idx = [0usize; N];
        'combos: loop {
            let (lo, hi) = (
                idx.iter().copied().fold(usize::MAX, usize::min),
                idx.iter().copied().fold(0, usize::max),
            );
            if hi - lo <= 1 {
                let intcnt: usize = idx.iter().sum();
                let mut sum = 0.0f32;
                for p in 0..N {
                    sum += best_error[p][quant][idx[p]];
                }
                let errorterm = mathlib::min(sum, 1e10);
                if errorterm <= best_combined_error[quant][intcnt] {
                    best_combined_error[quant][intcnt] = errorterm;
                    for p in 0..N {
                        best_combined_format[quant][intcnt][p] = best_format[p][quant][idx[p]];
                    }
                }
            }
            // Advance the innermost (last) counter first
            let mut p = N;
            loop {
                if p == 0 {
                    break 'combos;
                }
                p -= 1;
                idx[p] += 1;
                if idx[p] < 4 {
                    break;
                }
                idx[p] = 0;
            }
        }
    }
}

#[isa]
/// `{two,three,four}_partitions_find_best_combination_for_bitcount`.
fn n_partitions_find_best_combination_for_bitcount<const N: usize, const C: usize>(
    best_combined_error: &[[f32; C]; 21],
    best_combined_format: &[[[u8; N]; C]; 21],
    bits_available: i32,
    best_quant_level: &mut u8,
    best_quant_level_mod: &mut u8,
    best_formats: &mut [u8; BLOCK_MAX_PARTITIONS],
) -> f32 {
    let mut best_integer_count = 0usize;
    let mut best_integer_count_error = ERROR_CALC_DEFAULT;

    for integer_count in N..=(2 * N + 4).min(9) {
        // Compute the quantization level for a given number of integers and a given number of bits
        let quant_level = qmt(integer_count, bits_available);

        // Don't have enough bits to represent a given endpoint format at all!
        if quant_level < QUANT_6 as i32 {
            break;
        }

        let integer_count_error = best_combined_error[quant_level as usize][integer_count - N];
        if integer_count_error < best_integer_count_error {
            best_integer_count_error = integer_count_error;
            best_integer_count = integer_count;
        }
    }

    // Extra bits for the partitioning-specific color endpoint mode bits
    let mod_bits = [0, 0, 2, 5, 8][N];
    let ql = qmt(best_integer_count, bits_available);
    let ql_mod = qmt(best_integer_count, bits_available + mod_bits);

    *best_quant_level = ql as u8;
    *best_quant_level_mod = ql_mod as u8;

    if ql >= QUANT_6 as i32 {
        best_formats[..N]
            .copy_from_slice(&best_combined_format[ql as usize][best_integer_count - N]);
    } else {
        for i in 0..N {
            best_formats[i] = FMT_LUMINANCE;
        }
    }

    best_integer_count_error
}

#[isa]
/// Shared mode loop of the 2, 3 and 4 partition cases.
fn n_partitions_modes<const N: usize, const C: usize>(
    best_error: &[[[f32; 4]; 21]; BLOCK_MAX_PARTITIONS],
    format_of_choice: &[[[u8; 4]; 21]; BLOCK_MAX_PARTITIONS],
    qwt_bitcounts: &[i8],
    qwt_errors: &[f32],
    end_block_mode: usize,
    tmpbuf: &mut CompressionWorkingBuffers,
    error_of_best_combination: &mut f32,
    index_of_best_combination: &mut i32,
) {
    let mut combined_best_error = [[0.0f32; C]; 21];
    let mut formats_of_choice = [[[0u8; N]; C]; 21];
    n_partitions_find_best_combination::<N, C>(
        best_error,
        format_of_choice,
        &mut combined_best_error,
        &mut formats_of_choice,
    );

    for i in 0..end_block_mode {
        if qwt_errors[i] >= ERROR_CALC_DEFAULT {
            tmpbuf.errors_of_best_combination[i] = ERROR_CALC_DEFAULT;
            continue;
        }

        let error_of_best = n_partitions_find_best_combination_for_bitcount::<N, C>(
            &combined_best_error,
            &formats_of_choice,
            i32::from(qwt_bitcounts[i]),
            &mut tmpbuf.best_quant_levels[i],
            &mut tmpbuf.best_quant_levels_mod[i],
            &mut tmpbuf.best_ep_formats[i],
        );

        let total_error = error_of_best + qwt_errors[i];
        tmpbuf.errors_of_best_combination[i] = total_error;
        if total_error < *error_of_best_combination {
            *error_of_best_combination = total_error;
            *index_of_best_combination = i as i32;
        }
    }
}

#[isa]
/// `round_down_to_simd_multiple_vla`.
fn round_down_simd(count: usize) -> usize {
    count & !(SIMD_WIDTH - 1)
}

#[isa]
/// `compute_ideal_endpoint_formats`: returns the number of candidates found.
pub fn compute_ideal_endpoint_formats(
    pi: &PartitionInfo,
    blk: &ImageBlock,
    ep: &Endpoints,
    qwt_bitcounts: &[i8],
    qwt_errors: &[f32],
    tune_candidate_limit: u32,
    start_block_mode: u32,
    end_block_mode: u32,
    partition_format_specifiers: &mut [[u8; BLOCK_MAX_PARTITIONS]; TUNE_MAX_TRIAL_CANDIDATES],
    block_mode: &mut [i32; TUNE_MAX_TRIAL_CANDIDATES],
    quant_level: &mut [u32; TUNE_MAX_TRIAL_CANDIDATES],
    quant_level_mod: &mut [u32; TUNE_MAX_TRIAL_CANDIDATES],
    tmpbuf: &mut CompressionWorkingBuffers,
) -> u32 {
    let partition_count = pi.partition_count as usize;
    let (start_block_mode, end_block_mode) = (start_block_mode as usize, end_block_mode as usize);

    let encode_hdr_rgb = blk.rgb_lns[0] != 0;
    let encode_hdr_alpha = blk.alpha_lns[0] != 0;

    // Compute the errors that result from various encoding choices (such as using luminance instead
    // of RGB, discarding Alpha, using RGB-scale in place of two separate RGB endpoints and so on)
    let mut eci = [EncodingChoiceErrors::default(); BLOCK_MAX_PARTITIONS];
    compute_encoding_choice_errors(blk, pi, ep, &mut eci);

    let mut best_error = [[[0.0f32; 4]; 21]; BLOCK_MAX_PARTITIONS];
    let mut format_of_choice = [[[0u8; 4]; 21]; BLOCK_MAX_PARTITIONS];

    for i in 0..partition_count {
        compute_color_error_for_every_integer_count_and_quant_level(
            encode_hdr_rgb,
            encode_hdr_alpha,
            i,
            pi,
            &eci[i],
            ep,
            blk.channel_weight,
            &mut best_error[i],
            &mut format_of_choice[i],
        );
    }

    // Ensure that the first iteration understep contains data that will never be picked
    let mut clear = |start: usize| {
        for j in start..start + SIMD_WIDTH {
            if let Some(e) = tmpbuf.errors_of_best_combination.get_mut(j) {
                *e = ERROR_CALC_DEFAULT;
            }
            if let Some(q) = tmpbuf.best_quant_levels.get_mut(j) {
                *q = 0;
            }
            if let Some(q) = tmpbuf.best_quant_levels_mod.get_mut(j) {
                *q = 0;
            }
        }
    };
    clear(round_down_simd(start_block_mode));
    // Ensure that last iteration overstep contains data that will never be picked
    clear(round_down_simd(end_block_mode.wrapping_sub(1)));

    let mut error_of_best_combination = ERROR_CALC_DEFAULT;
    let mut index_of_best_combination: i32 = -1;

    // The block contains 1 partition
    if partition_count == 1 {
        for i in start_block_mode..end_block_mode {
            if qwt_errors[i] >= ERROR_CALC_DEFAULT {
                tmpbuf.errors_of_best_combination[i] = ERROR_CALC_DEFAULT;
                continue;
            }

            let mut fmt = 0u8;
            let error_of_best = one_partition_find_best_combination_for_bitcount(
                &best_error[0],
                &format_of_choice[0],
                i32::from(qwt_bitcounts[i]),
                &mut tmpbuf.best_quant_levels[i],
                &mut fmt,
            );
            tmpbuf.best_ep_formats[i][0] = fmt;

            let total_error = error_of_best + qwt_errors[i];
            tmpbuf.errors_of_best_combination[i] = total_error;
            tmpbuf.best_quant_levels_mod[i] = tmpbuf.best_quant_levels[i];

            if total_error < error_of_best_combination {
                error_of_best_combination = total_error;
                index_of_best_combination = i as i32;
            }
        }
    } else {
        debug_assert!(start_block_mode == 0);
        let args = (
            &best_error,
            &format_of_choice,
            qwt_bitcounts,
            qwt_errors,
            end_block_mode,
        );
        match partition_count {
            2 => n_partitions_modes::<2, 7>(
                args.0,
                args.1,
                args.2,
                args.3,
                args.4,
                tmpbuf,
                &mut error_of_best_combination,
                &mut index_of_best_combination,
            ),
            3 => n_partitions_modes::<3, 10>(
                args.0,
                args.1,
                args.2,
                args.3,
                args.4,
                tmpbuf,
                &mut error_of_best_combination,
                &mut index_of_best_combination,
            ),
            _ => n_partitions_modes::<4, 13>(
                args.0,
                args.1,
                args.2,
                args.3,
                args.4,
                tmpbuf,
                &mut error_of_best_combination,
                &mut index_of_best_combination,
            ),
        }
    }

    let mut best_error_weights = [0i32; TUNE_MAX_TRIAL_CANDIDATES];

    // Write the best result into the output, and mark the result as consumed
    best_error_weights[0] = index_of_best_combination;
    if index_of_best_combination >= 0 {
        tmpbuf.errors_of_best_combination[index_of_best_combination as usize] = ERROR_CALC_DEFAULT;
    }

    // Search the remaining results and pick the best candidate modes for trial 1+
    for i in 1..tune_candidate_limit as usize {
        let mut vbest_error_index = Vint8::splat(-1);
        let mut vbest_ep_error = Vfloat8::splat(ERROR_CALC_DEFAULT);

        let start_mode = round_down_simd(start_block_mode);
        let mut lane_ids = Vint8::lane_id() + Vint8::splat(start_mode as i32);
        let mut j = start_mode;
        while j < end_block_mode {
            let err = Vfloat8::loada(
                tmpbuf
                    .errors_of_best_combination
                    .get(j..)
                    .unwrap_or_default(),
            );
            let mask = err.lt(vbest_ep_error);
            vbest_ep_error = Vfloat8::select(vbest_ep_error, err, mask);
            vbest_error_index = Vint8::select(vbest_error_index, lane_ids, mask);
            lane_ids += Vint8::splat(SIMD_WIDTH as i32);
            j += SIMD_WIDTH;
        }

        // Pick best mode from the SIMD result, using lowest matching index to ensure invariance
        let lanes_min_error = vbest_ep_error.eq(vbest_ep_error.hmin());
        vbest_error_index = Vint8::select(
            Vint8::splat(0x7FFF_FFFF),
            vbest_error_index,
            lanes_min_error,
        );

        let best_error_index = vbest_error_index.hmin_s();

        best_error_weights[i] = best_error_index;

        // Max the error for this candidate so we don't pick it again
        if best_error_index >= 0 {
            tmpbuf.errors_of_best_combination[best_error_index as usize] = ERROR_CALC_DEFAULT;
        } else {
            // Early-out if no more candidates are valid
            break;
        }
    }

    for i in 0..tune_candidate_limit as usize {
        if best_error_weights[i] < 0 {
            return i as u32;
        }

        let bw = best_error_weights[i] as usize;
        block_mode[i] = best_error_weights[i];

        quant_level[i] = u32::from(tmpbuf.best_quant_levels[bw]);
        quant_level_mod[i] = u32::from(tmpbuf.best_quant_levels_mod[bw]);

        debug_assert!(quant_level[i] >= QUANT_6 && quant_level[i] <= QUANT_256);
        debug_assert!(quant_level_mod[i] >= QUANT_6 && quant_level_mod[i] <= QUANT_256);

        for j in 0..partition_count {
            partition_format_specifiers[i][j] = tmpbuf.best_ep_formats[bw][j];
        }
    }

    tune_candidate_limit
}
