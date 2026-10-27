// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of `astcenc_color_quantize.cpp`: quantization of color endpoints for every LDR and
//! HDR endpoint format (`pack_color_endpoints`).
//!
//! As in the C++, the `try_*` encoders may partially overwrite their outputs before they fail.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use super::internal::*;
use super::mathlib::{clamp, clamp1f, clamp255f, flt2int_rtn};
use super::tables::COLOR_UNQUANT_TO_UQUANT;
use super::vecmath::{Vfloat4, Vint4, bit_transfer_signed};
use ktx_rs_macros::isa;

#[isa]
/// `get_rgba_encoding_error`.
fn get_rgba_encoding_error(
    uquant0: Vfloat4,
    uquant1: Vfloat4,
    quant0: Vint4,
    quant1: Vint4,
) -> f32 {
    let error0 = uquant0 - quant0.int_to_float();
    let error1 = uquant1 - quant1.int_to_float();
    (error0 * error0 + error1 * error1).hadd_s()
}

#[isa]
#[inline]
fn table(quant_level: u32, index: i32) -> u8 {
    COLOR_UNQUANT_TO_UQUANT
        .get((quant_level as usize).wrapping_sub(QUANT_6 as usize))
        .and_then(|t| t.get(index as usize))
        .copied()
        .unwrap_or(0)
}

#[isa]
/// `quant_color(quant_level, value)`.
#[inline]
fn quant_color(quant_level: u32, value: i32) -> u8 {
    table(quant_level, value.wrapping_mul(2).wrapping_add(1))
}

#[isa]
/// `quant_color(quant_level, value, valuef)`: rounds up unless the residual is below -0.1.
#[inline]
fn quant_color_f(quant_level: u32, value: i32, valuef: f32) -> u8 {
    let mut index = value.wrapping_mul(2);
    let residual = valuef - value as f32;
    if residual >= -0.1 {
        index += 1;
    }
    table(quant_level, index)
}

#[isa]
/// `quant_color3(quant_level, value)`.
fn quant_color3(quant_level: u32, value: Vint4) -> Vint4 {
    let index = value * 2 + 1;
    Vint4::new(
        table(quant_level, index.lane(0)) as i32,
        table(quant_level, index.lane(1)) as i32,
        table(quant_level, index.lane(2)) as i32,
        0,
    )
}

#[isa]
/// `quant_color3(quant_level, value, valuef)`.
fn quant_color3_f(quant_level: u32, value: Vint4, valuef: Vfloat4) -> Vint4 {
    let mut index = value * 2;
    let residual = valuef - value.int_to_float();
    let mask = residual.ge(Vfloat4::splat(-0.1));
    index = Vint4::select(index, index + 1, mask);
    Vint4::new(
        table(quant_level, index.lane(0)) as i32,
        table(quant_level, index.lane(1)) as i32,
        table(quant_level, index.lane(2)) as i32,
        0,
    )
}

#[isa]
/// `quantize_rgb`.
fn quantize_rgb(
    mut color0: Vfloat4,
    mut color1: Vfloat4,
    color0_out: &mut Vint4,
    color1_out: &mut Vint4,
    quant_level: u32,
) {
    let nudge = Vfloat4::splat(0.2);
    let (mut color0i, mut color1i);
    loop {
        let color0q = color0.float_to_int_rtn().max(Vint4::splat(0));
        color0i = quant_color3_f(quant_level, color0q, color0);
        color0 = color0 - nudge;
        let color1q = color1.float_to_int_rtn().min(Vint4::splat(255));
        color1i = quant_color3_f(quant_level, color1q, color1);
        color1 += nudge;
        if color0i.hadd_rgb_s() <= color1i.hadd_rgb_s() {
            break;
        }
    }
    *color0_out = color0i;
    *color1_out = color1i;
}

#[isa]
/// `quantize_rgba`.
fn quantize_rgba(
    color0: Vfloat4,
    color1: Vfloat4,
    color0_out: &mut Vint4,
    color1_out: &mut Vint4,
    quant_level: u32,
) {
    quantize_rgb(color0, color1, color0_out, color1_out, quant_level);
    let a0 = color0.lane(3);
    let a1 = color1.lane(3);
    color0_out.set_lane(3, quant_color_f(quant_level, flt2int_rtn(a0), a0) as i32);
    color1_out.set_lane(3, quant_color_f(quant_level, flt2int_rtn(a1), a1) as i32);
}

#[isa]
/// `color + (color - color.swz<2, 2, 2, 3>())` and the out of range test.
fn blue_contract(color0: &mut Vfloat4, color1: &mut Vfloat4) -> bool {
    *color0 += *color0 - color0.swz4(2, 2, 2, 3);
    *color1 += *color1 - color1.swz4(2, 2, 2, 3);
    let e0 = color0.lt(Vfloat4::splat(0.0)) | color0.gt(Vfloat4::splat(255.0));
    let e1 = color1.lt(Vfloat4::splat(0.0)) | color1.gt(Vfloat4::splat(255.0));
    !(e0 | e1).any()
}

#[isa]
/// `try_quantize_rgb_blue_contract`.
fn try_quantize_rgb_blue_contract(
    mut color0: Vfloat4,
    mut color1: Vfloat4,
    color0_out: &mut Vint4,
    color1_out: &mut Vint4,
    quant_level: u32,
) -> bool {
    if !blue_contract(&mut color0, &mut color1) {
        return false;
    }
    let color0i = quant_color3_f(quant_level, color0.float_to_int_rtn(), color0);
    let color1i = quant_color3_f(quant_level, color1.float_to_int_rtn(), color1);
    if color1i.hadd_rgb_s() <= color0i.hadd_rgb_s() {
        return false;
    }
    *color0_out = color1i;
    *color1_out = color0i;
    true
}

#[isa]
/// `try_quantize_rgba_blue_contract`.
fn try_quantize_rgba_blue_contract(
    color0: Vfloat4,
    color1: Vfloat4,
    color0_out: &mut Vint4,
    color1_out: &mut Vint4,
    quant_level: u32,
) -> bool {
    if try_quantize_rgb_blue_contract(color0, color1, color0_out, color1_out, quant_level) {
        let a0 = color0.lane(3);
        let a1 = color1.lane(3);
        color0_out.set_lane(3, quant_color_f(quant_level, flt2int_rtn(a1), a1) as i32);
        color1_out.set_lane(3, quant_color_f(quant_level, flt2int_rtn(a0), a0) as i32);
        return true;
    }
    false
}

#[isa]
/// Shared body of `try_quantize_rgb_delta` (`negative` false) and
/// `try_quantize_rgb_delta_blue_contract` (`negative` true, after contraction).
fn rgb_delta_core(
    color0: Vfloat4,
    color1: Vfloat4,
    color0_out: &mut Vint4,
    color1_out: &mut Vint4,
    quant_level: u32,
    negative: bool,
) -> bool {
    // Transform color0 to unorm9
    let mut color0a = color0.float_to_int_rtn();
    color0a = color0a.lsl(1);
    // Mask off the top bit
    let mut color0b = color0a & 0xFF;
    // Quantize then unquantize in order to get a value that we take differences against
    let color0be = quant_color3(quant_level, color0b);
    color0b = color0be | (color0a & 0x100);

    // Get hold of the second value
    let mut color1d = color1.float_to_int_rtn();
    color1d = color1d.lsl(1);
    // ... and take differences
    color1d = color1d - color0b;
    color1d.set_lane(3, 0);
    // Check if the difference is too large to be encodable
    if (color1d.gt(Vint4::splat(63)) | color1d.lt(Vint4::splat(-64))).any() {
        return false;
    }
    // Insert top bit of the base into the offset
    color1d = color1d & 0x7F;
    color1d = color1d | (color0b & 0x100).lsr(1);
    // Then quantize and unquantize; if this causes either top two bits to flip, then encoding fails
    let color1de = quant_color3(quant_level, color1d);
    let mut color_flips = (color1d ^ color1de) & 0xC0;
    color_flips.set_lane(3, 0);
    if color_flips.ne(Vint4::zero()).any() {
        return false;
    }
    // If the sum of offsets triggers blue-contraction then encoding fails
    let mut ep0 = color0be;
    let mut ep1 = color1de;
    bit_transfer_signed(&mut ep1, &mut ep0);
    if negative {
        if ep1.hadd_rgb_s() >= 0 {
            return false;
        }
    } else if ep1.hadd_rgb_s() < 0 {
        return false;
    }
    // Check that the offsets produce legitimate sums as well
    ep0 += ep1;
    if (ep0.lt(Vint4::splat(0)) | ep0.gt(Vint4::splat(0xFF))).any() {
        return false;
    }
    *color0_out = color0be;
    *color1_out = color1de;
    true
}

#[isa]
/// `try_quantize_rgb_delta`.
fn try_quantize_rgb_delta(
    color0: Vfloat4,
    color1: Vfloat4,
    c0: &mut Vint4,
    c1: &mut Vint4,
    quant_level: u32,
) -> bool {
    rgb_delta_core(color0, color1, c0, c1, quant_level, false)
}

#[isa]
/// `try_quantize_rgb_delta_blue_contract`.
fn try_quantize_rgb_delta_blue_contract(
    color0: Vfloat4,
    color1: Vfloat4,
    c0: &mut Vint4,
    c1: &mut Vint4,
    quant_level: u32,
) -> bool {
    // Note: Switch around endpoint colors already at start
    let (mut color0, mut color1) = (color1, color0);
    if !blue_contract(&mut color0, &mut color1) {
        return false;
    }
    rgb_delta_core(color0, color1, c0, c1, quant_level, true)
}

#[isa]
/// `try_quantize_alpha_delta`.
fn try_quantize_alpha_delta(
    color0: Vfloat4,
    color1: Vfloat4,
    c0: &mut Vint4,
    c1: &mut Vint4,
    quant_level: u32,
) -> bool {
    let a0 = color0.lane(3);
    let a1 = color1.lane(3);
    let mut a0a = flt2int_rtn(a0);
    a0a = a0a.wrapping_shl(1);
    let mut a0b = a0a & 0xFF;
    let a0be = quant_color(quant_level, a0b) as i32;
    a0b = a0be;
    a0b |= a0a & 0x100;
    let mut a1d = flt2int_rtn(a1);
    a1d = a1d.wrapping_shl(1);
    a1d = a1d.wrapping_sub(a0b);
    if !(-64..=63).contains(&a1d) {
        return false;
    }
    a1d &= 0x7F;
    a1d |= (a0b & 0x100) >> 1;
    let a1de = quant_color(quant_level, a1d) as i32;
    let mut a1du = a1de;
    if (a1d ^ a1du) & 0xC0 != 0 {
        return false;
    }
    a1du &= 0x7F;
    if a1du & 0x40 != 0 {
        a1du -= 0x80;
    }
    a1du += a0b;
    if !(0..=0x1FF).contains(&a1du) {
        return false;
    }
    c0.set_lane(3, a0be);
    c1.set_lane(3, a1de);
    true
}

#[isa]
/// `try_quantize_luminance_alpha_delta`.
fn try_quantize_luminance_alpha_delta(
    color0: Vfloat4,
    color1: Vfloat4,
    output: &mut [u8],
    quant_level: u32,
) -> bool {
    let l0 = color0.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let l1 = color1.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let a0 = color0.lane(3);
    let a1 = color1.lane(3);
    let mut l0a = flt2int_rtn(l0);
    let mut a0a = flt2int_rtn(a0);
    l0a = l0a.wrapping_shl(1);
    a0a = a0a.wrapping_shl(1);
    let mut l0b = l0a & 0xFF;
    let mut a0b = a0a & 0xFF;
    let l0be = quant_color(quant_level, l0b) as i32;
    let a0be = quant_color(quant_level, a0b) as i32;
    l0b = l0be;
    a0b = a0be;
    l0b |= l0a & 0x100;
    a0b |= a0a & 0x100;
    let mut l1d = flt2int_rtn(l1);
    let mut a1d = flt2int_rtn(a1);
    l1d = l1d.wrapping_shl(1);
    a1d = a1d.wrapping_shl(1);
    l1d = l1d.wrapping_sub(l0b);
    a1d = a1d.wrapping_sub(a0b);
    if !(-64..=63).contains(&l1d) || !(-64..=63).contains(&a1d) {
        return false;
    }
    l1d &= 0x7F;
    a1d &= 0x7F;
    l1d |= (l0b & 0x100) >> 1;
    a1d |= (a0b & 0x100) >> 1;
    let l1de = quant_color(quant_level, l1d) as i32;
    let a1de = quant_color(quant_level, a1d) as i32;
    let mut l1du = l1de;
    let mut a1du = a1de;
    if (l1d ^ l1du) & 0xC0 != 0 || (a1d ^ a1du) & 0xC0 != 0 {
        return false;
    }
    l1du &= 0x7F;
    a1du &= 0x7F;
    if l1du & 0x40 != 0 {
        l1du -= 0x80;
    }
    if a1du & 0x40 != 0 {
        a1du -= 0x80;
    }
    l1du += l0b;
    a1du += a0b;
    if !(0..=0x1FF).contains(&l1du) || !(0..=0x1FF).contains(&a1du) {
        return false;
    }
    output[0] = l0be as u8;
    output[1] = l1de as u8;
    output[2] = a0be as u8;
    output[3] = a1de as u8;
    true
}

#[isa]
/// `try_quantize_rgba_delta`.
fn try_quantize_rgba_delta(
    color0: Vfloat4,
    color1: Vfloat4,
    c0: &mut Vint4,
    c1: &mut Vint4,
    quant_level: u32,
) -> bool {
    try_quantize_rgb_delta(color0, color1, c0, c1, quant_level)
        && try_quantize_alpha_delta(color0, color1, c0, c1, quant_level)
}

#[isa]
/// `try_quantize_rgba_delta_blue_contract`.
fn try_quantize_rgba_delta_blue_contract(
    color0: Vfloat4,
    color1: Vfloat4,
    c0: &mut Vint4,
    c1: &mut Vint4,
    quant_level: u32,
) -> bool {
    // Note that we swap the color0 and color1 ordering for alpha because the RGB blue-contract
    // step swaps the endpoint order
    try_quantize_rgb_delta_blue_contract(color0, color1, c0, c1, quant_level)
        && try_quantize_alpha_delta(color1, color0, c0, c1, quant_level)
}

#[isa]
/// `quantize_rgbs`.
fn quantize_rgbs(color: Vfloat4, output: &mut [u8], quant_level: u32) {
    let scale = 1.0f32 / 257.0f32;
    let r = clamp255f(color.lane(0) * scale);
    let g = clamp255f(color.lane(1) * scale);
    let b = clamp255f(color.lane(2) * scale);
    let ri = quant_color_f(quant_level, flt2int_rtn(r), r) as i32;
    let gi = quant_color_f(quant_level, flt2int_rtn(g), g) as i32;
    let bi = quant_color_f(quant_level, flt2int_rtn(b), b) as i32;
    let oldcolorsum = color.hadd_rgb_s() * scale;
    let newcolorsum = (ri + gi + bi) as f32;
    let scalea = clamp1f(color.lane(3) * (oldcolorsum + 1e-10) / (newcolorsum + 1e-10));
    let scale_idx = flt2int_rtn(scalea * 256.0).clamp(0, 255);
    output[0] = ri as u8;
    output[1] = gi as u8;
    output[2] = bi as u8;
    output[3] = quant_color(quant_level, scale_idx);
}

#[isa]
/// `quantize_rgbs_alpha`.
fn quantize_rgbs_alpha(
    color0: Vfloat4,
    color1: Vfloat4,
    color: Vfloat4,
    output: &mut [u8],
    quant_level: u32,
) {
    let a0 = color0.lane(3);
    let a1 = color1.lane(3);
    output[4] = quant_color_f(quant_level, flt2int_rtn(a0), a0);
    output[5] = quant_color_f(quant_level, flt2int_rtn(a1), a1);
    quantize_rgbs(color, output, quant_level);
}

#[isa]
/// `quantize_luminance`.
fn quantize_luminance(color0: Vfloat4, color1: Vfloat4, output: &mut [u8], quant_level: u32) {
    let mut lum0 = color0.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let mut lum1 = color1.hadd_rgb_s() * (1.0f32 / 3.0f32);
    if lum0 > lum1 {
        let avg = (lum0 + lum1) * 0.5;
        lum0 = avg;
        lum1 = avg;
    }
    output[0] = quant_color_f(quant_level, flt2int_rtn(lum0), lum0);
    output[1] = quant_color_f(quant_level, flt2int_rtn(lum1), lum1);
}

#[isa]
/// `quantize_luminance_alpha`.
fn quantize_luminance_alpha(color0: Vfloat4, color1: Vfloat4, output: &mut [u8], quant_level: u32) {
    let lum0 = color0.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let lum1 = color1.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let a0 = color0.lane(3);
    let a1 = color1.lane(3);
    output[0] = quant_color_f(quant_level, flt2int_rtn(lum0), lum0);
    output[1] = quant_color_f(quant_level, flt2int_rtn(lum1), lum1);
    output[2] = quant_color_f(quant_level, flt2int_rtn(a0), a0);
    output[3] = quant_color_f(quant_level, flt2int_rtn(a1), a1);
}

#[isa]
/// `quantize_and_unquantize_retain_top_two_bits` / `_four_bits` (`mask` 0xC0 / 0xF0).
fn quantize_retain_top_bits(quant_level: u32, mut value: u8, mask: u8) -> u8 {
    loop {
        let quantval = quant_color(quant_level, value as i32);
        let perform_loop = (value & mask) != (quantval & mask);
        if (quantval & mask) != (value & mask) {
            value = value.wrapping_sub(1);
        }
        if !perform_loop {
            return quantval;
        }
    }
}

#[isa]
fn top2(quant_level: u32, value: u8) -> u8 {
    quantize_retain_top_bits(quant_level, value, 0xC0)
}

#[isa]
fn top4(quant_level: u32, value: u8) -> u8 {
    quantize_retain_top_bits(quant_level, value, 0xF0)
}

#[isa]
/// `quantize_hdr_rgbo`.
fn quantize_hdr_rgbo(mut color: Vfloat4, output: &mut [u8], quant_level: u32) {
    color.set_lane(0, color.lane(0) + color.lane(3));
    color.set_lane(1, color.lane(1) + color.lane(3));
    color.set_lane(2, color.lane(2) + color.lane(3));
    color = Vfloat4::clamp(0.0, 65535.0, color);
    let color_bak = color;

    let majcomp = if color.lane(0) > color.lane(1) && color.lane(0) > color.lane(2) {
        0 // red is largest component
    } else if color.lane(1) > color.lane(2) {
        1 // green is largest component
    } else {
        2 // blue is largest component
    };
    // swap around the red component and the largest component.
    match majcomp {
        1 => color = color.swz4(1, 0, 2, 3),
        2 => color = color.swz4(2, 1, 0, 3),
        _ => {}
    }

    const MODE_BITS: [[i32; 3]; 5] = [[11, 5, 7], [11, 6, 5], [10, 5, 8], [9, 6, 7], [8, 7, 6]];
    const MODE_CUTOFFS: [[f32; 2]; 5] = [
        [1024.0, 4096.0],
        [2048.0, 1024.0],
        [2048.0, 16384.0],
        [8192.0, 16384.0],
        [32768.0, 16384.0],
    ];
    const MODE_RSCALES: [f32; 5] = [32.0, 32.0, 64.0, 128.0, 256.0];
    const MODE_SCALES: [f32; 5] = [1.0 / 32.0, 1.0 / 32.0, 1.0 / 64.0, 1.0 / 128.0, 1.0 / 256.0];

    let r_base = color.lane(0);
    let g_base = color.lane(0) - color.lane(1);
    let b_base = color.lane(0) - color.lane(2);
    let s_base = color.lane(3);

    for mode in 0..5usize {
        if g_base > MODE_CUTOFFS[mode][0]
            || b_base > MODE_CUTOFFS[mode][0]
            || s_base > MODE_CUTOFFS[mode][1]
        {
            continue;
        }
        // Encode the mode into a 4-bit vector
        let mode_enc: i32 = if mode < 4 {
            mode as i32 | (majcomp << 2)
        } else {
            majcomp | 0xC
        };
        let mode_scale = MODE_SCALES[mode];
        let mode_rscale = MODE_RSCALES[mode];
        let gb_intcutoff = 1 << MODE_BITS[mode][1];
        let s_intcutoff = 1 << MODE_BITS[mode][2];

        // Quantize and unquantize R
        let mut r_intval = flt2int_rtn(r_base * mode_scale);
        let mut r_lowbits = r_intval & 0x3f;
        r_lowbits |= (mode_enc & 3) << 6;
        let r_quantval = top2(quant_level, r_lowbits as u8);
        r_intval = (r_intval & !0x3f) | (r_quantval as i32 & 0x3f);
        let r_fval = r_intval as f32 * mode_rscale;

        // Recompute G and B, then quantize and unquantize them
        let mut g_fval = r_fval - color.lane(1);
        let mut b_fval = r_fval - color.lane(2);
        g_fval = clamp(g_fval, 0.0, 65535.0);
        b_fval = clamp(b_fval, 0.0, 65535.0);
        let mut g_intval = flt2int_rtn(g_fval * mode_scale);
        let mut b_intval = flt2int_rtn(b_fval * mode_scale);
        if g_intval >= gb_intcutoff || b_intval >= gb_intcutoff {
            continue;
        }
        let mut g_lowbits = g_intval & 0x1f;
        let mut b_lowbits = b_intval & 0x1f;
        let bit0 = match mode {
            0 | 2 => (r_intval >> 9) & 1,
            1 | 3 => (r_intval >> 8) & 1,
            _ => (g_intval >> 6) & 1,
        };
        let bit2 = match mode {
            0..=3 => (r_intval >> 7) & 1,
            _ => (b_intval >> 6) & 1,
        };
        let bit1 = match mode {
            0 | 2 => (r_intval >> 8) & 1,
            _ => (g_intval >> 5) & 1,
        };
        let bit3 = match mode {
            0 => (r_intval >> 10) & 1,
            2 => (r_intval >> 6) & 1,
            _ => (b_intval >> 5) & 1,
        };
        g_lowbits |= (mode_enc & 0x4) << 5;
        b_lowbits |= (mode_enc & 0x8) << 4;
        g_lowbits |= bit0 << 6;
        g_lowbits |= bit1 << 5;
        b_lowbits |= bit2 << 6;
        b_lowbits |= bit3 << 5;
        let g_quantval = top4(quant_level, g_lowbits as u8);
        let b_quantval = top4(quant_level, b_lowbits as u8);
        g_intval = (g_intval & !0x1f) | (g_quantval as i32 & 0x1f);
        b_intval = (b_intval & !0x1f) | (b_quantval as i32 & 0x1f);
        g_fval = g_intval as f32 * mode_rscale;
        b_fval = b_intval as f32 * mode_rscale;

        // Compute error and quantize S
        let rgb_errorsum = (r_fval - color.lane(0))
            + (r_fval - g_fval - color.lane(1))
            + (r_fval - b_fval - color.lane(2));
        let mut s_fval = s_base + rgb_errorsum * (1.0f32 / 3.0f32);
        s_fval = clamp(s_fval, 0.0, 1e9);
        let s_intval = flt2int_rtn(s_fval * mode_scale);
        if s_intval >= s_intcutoff {
            continue;
        }
        let mut s_lowbits = s_intval & 0x1f;
        let bit6 = if mode == 1 {
            (r_intval >> 9) & 1
        } else {
            (s_intval >> 5) & 1
        };
        let bit5 = match mode {
            4 => (r_intval >> 7) & 1,
            1 => (r_intval >> 10) & 1,
            _ => (s_intval >> 6) & 1,
        };
        let bit4 = if mode == 2 {
            (s_intval >> 7) & 1
        } else {
            (r_intval >> 6) & 1
        };
        s_lowbits |= bit6 << 5;
        s_lowbits |= bit5 << 6;
        s_lowbits |= bit4 << 7;
        let s_quantval = top4(quant_level, s_lowbits as u8);
        output[0] = r_quantval;
        output[1] = g_quantval;
        output[2] = b_quantval;
        output[3] = s_quantval;
        return;
    }

    // Failed to encode any of the modes above? In that case encode using mode #5
    let mut vals = [
        color_bak.lane(0),
        color_bak.lane(1),
        color_bak.lane(2),
        color_bak.lane(3),
    ];
    let mut ivals = [0i32; 4];
    let mut cvals = [0.0f32; 3];
    for i in 0..3 {
        vals[i] = clamp(vals[i], 0.0, 65020.0);
        ivals[i] = flt2int_rtn(vals[i] * (1.0f32 / 512.0f32));
        cvals[i] = ivals[i] as f32 * 512.0;
    }
    let rgb_errorsum = (cvals[0] - vals[0]) + (cvals[1] - vals[1]) + (cvals[2] - vals[2]);
    vals[3] += rgb_errorsum * (1.0f32 / 3.0f32);
    vals[3] = clamp(vals[3], 0.0, 65020.0);
    ivals[3] = flt2int_rtn(vals[3] * (1.0f32 / 512.0f32));
    let encvals = [
        (ivals[0] & 0x3f) | 0xC0,
        (ivals[1] & 0x7f) | 0x80,
        (ivals[2] & 0x7f) | 0x80,
        (ivals[3] & 0x7f) | ((ivals[0] & 0x40) << 1),
    ];
    for i in 0..4 {
        output[i] = top4(quant_level, encvals[i] as u8);
    }
}

#[isa]
/// `quantize_hdr_rgb`.
fn quantize_hdr_rgb(color0: Vfloat4, color1: Vfloat4, output: &mut [u8], quant_level: u32) {
    // Note: color*.lane<3> is not used so we can ignore it
    let mut color0 = Vfloat4::clamp(0.0, 65535.0, color0);
    let mut color1 = Vfloat4::clamp(0.0, 65535.0, color1);
    let color0_bak = color0;
    let color1_bak = color1;

    let majcomp: i32 = if color1.lane(0) > color1.lane(1) && color1.lane(0) > color1.lane(2) {
        0
    } else if color1.lane(1) > color1.lane(2) {
        1
    } else {
        2
    };
    // Swizzle the components
    match majcomp {
        1 => {
            // red-green swap
            color0 = color0.swz4(1, 0, 2, 3);
            color1 = color1.swz4(1, 0, 2, 3);
        }
        2 => {
            // red-blue swap
            color0 = color0.swz4(2, 1, 0, 3);
            color1 = color1.swz4(2, 1, 0, 3);
        }
        _ => {}
    }

    let a_base = clamp(color1.lane(0), 0.0, 65535.0);
    let b0_base = a_base - color1.lane(1);
    let b1_base = a_base - color1.lane(2);
    let c_base = a_base - color0.lane(0);
    let d0_base = a_base - b0_base - c_base - color0.lane(1);
    let d1_base = a_base - b1_base - c_base - color0.lane(2);

    // Number of bits in the various fields in the various modes
    const MODE_BITS: [[i32; 4]; 8] = [
        [9, 7, 6, 7],
        [9, 8, 6, 6],
        [10, 6, 7, 7],
        [10, 7, 7, 6],
        [11, 8, 6, 5],
        [11, 6, 8, 6],
        [12, 7, 7, 5],
        [12, 6, 7, 6],
    ];
    // Cutoffs to use for the computed values of a,b,c,d, assuming the range 0..65535 are LNS values
    const MODE_CUTOFFS: [[f32; 4]; 8] = [
        [16384.0, 8192.0, 8192.0, 8.0],
        [32768.0, 8192.0, 4096.0, 8.0],
        [4096.0, 8192.0, 4096.0, 4.0],
        [8192.0, 8192.0, 2048.0, 4.0],
        [8192.0, 2048.0, 512.0, 2.0],
        [2048.0, 8192.0, 1024.0, 2.0],
        [2048.0, 2048.0, 256.0, 1.0],
        [1024.0, 2048.0, 512.0, 1.0],
    ];
    const MODE_SCALES: [f32; 8] = [
        1.0 / 128.0,
        1.0 / 128.0,
        1.0 / 64.0,
        1.0 / 64.0,
        1.0 / 32.0,
        1.0 / 32.0,
        1.0 / 16.0,
        1.0 / 16.0,
    ];
    // Scaling factors when going from what was encoded in the mode to 16 bits.
    const MODE_RSCALES: [f32; 8] = [128.0, 128.0, 64.0, 64.0, 32.0, 32.0, 16.0, 16.0];

    // Try modes one by one, with the highest-precision mode first.
    for mode in (0..8usize).rev() {
        let b_cutoff = MODE_CUTOFFS[mode][0];
        let c_cutoff = MODE_CUTOFFS[mode][1];
        let d_cutoff = MODE_CUTOFFS[mode][2];
        if b0_base > b_cutoff
            || b1_base > b_cutoff
            || c_base > c_cutoff
            || d0_base.abs() > d_cutoff
            || d1_base.abs() > d_cutoff
        {
            continue;
        }
        let mode_scale = MODE_SCALES[mode];
        let mode_rscale = MODE_RSCALES[mode];
        let b_intcutoff = 1 << MODE_BITS[mode][1];
        let c_intcutoff = 1 << MODE_BITS[mode][2];
        let d_intcutoff = 1 << (MODE_BITS[mode][3] - 1);

        // Quantize and unquantize A, with the assumption that its high bits can be handled safely.
        let mut a_intval = flt2int_rtn(a_base * mode_scale);
        let a_lowbits = a_intval & 0xFF;
        let a_quantval = quant_color(quant_level, a_lowbits) as i32;
        let a_uquantval = a_quantval;
        a_intval = (a_intval & !0xFF) | a_uquantval;
        let a_fval = a_intval as f32 * mode_rscale;

        // Recompute C, then quantize and unquantize it
        let mut c_fval = a_fval - color0.lane(0);
        c_fval = clamp(c_fval, 0.0, 65535.0);
        let mut c_intval = flt2int_rtn(c_fval * mode_scale);
        if c_intval >= c_intcutoff {
            continue;
        }
        let mut c_lowbits = c_intval & 0x3f;
        c_lowbits |= (mode as i32 & 1) << 7;
        c_lowbits |= (a_intval & 0x100) >> 2;
        let c_quantval = top2(quant_level, c_lowbits as u8);
        c_intval = (c_intval & !0x3F) | (c_quantval as i32 & 0x3F);
        c_fval = c_intval as f32 * mode_rscale;

        // Recompute B0 and B1, then quantize and unquantize them
        let mut b0_fval = a_fval - color1.lane(1);
        let mut b1_fval = a_fval - color1.lane(2);
        b0_fval = clamp(b0_fval, 0.0, 65535.0);
        b1_fval = clamp(b1_fval, 0.0, 65535.0);
        let mut b0_intval = flt2int_rtn(b0_fval * mode_scale);
        let mut b1_intval = flt2int_rtn(b1_fval * mode_scale);
        if b0_intval >= b_intcutoff || b1_intval >= b_intcutoff {
            continue;
        }
        let mut b0_lowbits = b0_intval & 0x3f;
        let mut b1_lowbits = b1_intval & 0x3f;
        let bit0 = match mode {
            0 | 1 | 3 | 4 | 6 => (b0_intval >> 6) & 1,
            _ => (a_intval >> 9) & 1,
        };
        let bit1 = match mode {
            0 | 1 | 3 | 4 | 6 => (b1_intval >> 6) & 1,
            2 => (c_intval >> 6) & 1,
            _ => (a_intval >> 10) & 1,
        };
        b0_lowbits |= bit0 << 6;
        b1_lowbits |= bit1 << 6;
        b0_lowbits |= ((mode as i32 >> 1) & 1) << 7;
        b1_lowbits |= ((mode as i32 >> 2) & 1) << 7;
        let b0_quantval = top2(quant_level, b0_lowbits as u8);
        let b1_quantval = top2(quant_level, b1_lowbits as u8);
        b0_intval = (b0_intval & !0x3f) | (b0_quantval as i32 & 0x3f);
        b1_intval = (b1_intval & !0x3f) | (b1_quantval as i32 & 0x3f);
        b0_fval = b0_intval as f32 * mode_rscale;
        b1_fval = b1_intval as f32 * mode_rscale;

        // Recompute D0 and D1, then quantize and unquantize them
        let mut d0_fval = a_fval - b0_fval - c_fval - color0.lane(1);
        let mut d1_fval = a_fval - b1_fval - c_fval - color0.lane(2);
        d0_fval = clamp(d0_fval, -65535.0, 65535.0);
        d1_fval = clamp(d1_fval, -65535.0, 65535.0);
        let d0_intval = flt2int_rtn(d0_fval * mode_scale);
        let d1_intval = flt2int_rtn(d1_fval * mode_scale);
        if d0_intval.wrapping_abs() >= d_intcutoff || d1_intval.wrapping_abs() >= d_intcutoff {
            continue;
        }
        let mut d0_lowbits = d0_intval & 0x1f;
        let mut d1_lowbits = d1_intval & 0x1f;
        let bit2 = match mode {
            0 | 2 => (d0_intval >> 6) & 1,
            1 | 4 => (b0_intval >> 7) & 1,
            3 => (a_intval >> 9) & 1,
            5 => (c_intval >> 7) & 1,
            _ => (a_intval >> 11) & 1,
        };
        let bit3 = match mode {
            0 | 2 => (d1_intval >> 6) & 1,
            1 | 4 => (b1_intval >> 7) & 1,
            _ => (c_intval >> 6) & 1,
        };
        let (bit4, bit5) = match mode {
            4 | 6 => ((a_intval >> 9) & 1, (a_intval >> 10) & 1),
            _ => ((d0_intval >> 5) & 1, (d1_intval >> 5) & 1),
        };
        d0_lowbits |= bit2 << 6;
        d1_lowbits |= bit3 << 6;
        d0_lowbits |= bit4 << 5;
        d1_lowbits |= bit5 << 5;
        d0_lowbits |= (majcomp & 1) << 7;
        d1_lowbits |= ((majcomp >> 1) & 1) << 7;
        let d0_quantval = top4(quant_level, d0_lowbits as u8);
        let d1_quantval = top4(quant_level, d1_lowbits as u8);

        // Output bits
        output[0] = a_quantval as u8;
        output[1] = c_quantval;
        output[2] = b0_quantval;
        output[3] = b1_quantval;
        output[4] = d0_quantval;
        output[5] = d1_quantval;
        return;
    }

    // If neither of the modes fit we will use a flat representation for storing data, using
    // 8 bits for red and green, and 7 bits for blue. This gives color accuracy roughly similar
    // to LDR 4:4:3 which is not at all great but usable. This representation is used if the
    // light color is more than 4x the color value of the dark color.
    let mut vals = [
        color0_bak.lane(0),
        color1_bak.lane(0),
        color0_bak.lane(1),
        color1_bak.lane(1),
        color0_bak.lane(2),
        color1_bak.lane(2),
    ];
    for v in vals.iter_mut() {
        *v = clamp(*v, 0.0, 65020.0);
    }
    for i in 0..4 {
        let idx = flt2int_rtn(vals[i] * 1.0 / 256.0);
        output[i] = quant_color(quant_level, idx);
    }
    for i in 4..6 {
        let idx = flt2int_rtn(vals[i] * 1.0 / 512.0) + 128;
        output[i] = top2(quant_level, idx as u8);
    }
}

#[isa]
/// `quantize_hdr_rgb_ldr_alpha`.
fn quantize_hdr_rgb_ldr_alpha(
    color0: Vfloat4,
    color1: Vfloat4,
    output: &mut [u8],
    quant_level: u32,
) {
    let scale = 1.0f32 / 257.0f32;
    let a0 = clamp255f(color0.lane(3) * scale);
    let a1 = clamp255f(color1.lane(3) * scale);
    output[6] = quant_color_f(quant_level, flt2int_rtn(a0), a0);
    output[7] = quant_color_f(quant_level, flt2int_rtn(a1), a1);
    quantize_hdr_rgb(color0, color1, output, quant_level);
}

#[isa]
/// `quantize_hdr_luminance_large_range`.
fn quantize_hdr_luminance_large_range(
    color0: Vfloat4,
    color1: Vfloat4,
    output: &mut [u8],
    quant_level: u32,
) {
    let mut lum0 = color0.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let mut lum1 = color1.hadd_rgb_s() * (1.0f32 / 3.0f32);
    if lum1 < lum0 {
        let avg = (lum0 + lum1) * 0.5;
        lum0 = avg;
        lum1 = avg;
    }
    let ilum1 = flt2int_rtn(lum1);
    let ilum0 = flt2int_rtn(lum0);

    // Find the closest encodable point in the upper half of the code-point space
    let upper_v0 = (ilum0.wrapping_add(128) >> 8).clamp(0, 255);
    let upper_v1 = (ilum1.wrapping_add(128) >> 8).clamp(0, 255);
    // Find the closest encodable point in the lower half of the code-point space
    let lower_v0 = (ilum1.wrapping_add(256) >> 8).clamp(0, 255);
    let lower_v1 = (ilum0 >> 8).clamp(0, 255);

    // Determine the distance between the point in code-point space and the input value
    let upper0_dec = upper_v0 << 8;
    let upper1_dec = upper_v1 << 8;
    let lower0_dec = (lower_v1 << 8) + 128;
    let lower1_dec = (lower_v0 << 8) - 128;
    let upper0_diff = upper0_dec.wrapping_sub(ilum0);
    let upper1_diff = upper1_dec.wrapping_sub(ilum1);
    let lower0_diff = lower0_dec.wrapping_sub(ilum0);
    let lower1_diff = lower1_dec.wrapping_sub(ilum1);
    let upper_error = upper0_diff
        .wrapping_mul(upper0_diff)
        .wrapping_add(upper1_diff.wrapping_mul(upper1_diff));
    let lower_error = lower0_diff
        .wrapping_mul(lower0_diff)
        .wrapping_add(lower1_diff.wrapping_mul(lower1_diff));
    let (v0, v1) = if upper_error < lower_error {
        (upper_v0, upper_v1)
    } else {
        (lower_v0, lower_v1)
    };

    // OK; encode
    output[0] = quant_color(quant_level, v0);
    output[1] = quant_color(quant_level, v1);
}

#[isa]
/// `try_quantize_hdr_luminance_small_range`.
fn try_quantize_hdr_luminance_small_range(
    color0: Vfloat4,
    color1: Vfloat4,
    output: &mut [u8],
    quant_level: u32,
) -> bool {
    let mut lum0 = color0.hadd_rgb_s() * (1.0f32 / 3.0f32);
    let mut lum1 = color1.hadd_rgb_s() * (1.0f32 / 3.0f32);
    if lum1 < lum0 {
        let avg = (lum0 + lum1) * 0.5;
        lum0 = avg;
        lum1 = avg;
    }
    let ilum1 = flt2int_rtn(lum1);
    let ilum0 = flt2int_rtn(lum0);

    // Difference of more than a factor-of-2 results in immediate failure
    if ilum1.wrapping_sub(ilum0) > 2048 {
        return false;
    }

    // First, try to encode the high-precision submode
    let mut lowval = (ilum0.wrapping_add(16) >> 5).clamp(0, 2047);
    let highval = (ilum1.wrapping_add(16) >> 5).clamp(0, 2047);
    let v0 = lowval & 0x7F;
    let v0e = quant_color(quant_level, v0) as i32;
    let v0d = v0e;
    if v0d < 0x80 {
        lowval = (lowval & !0x7F) | v0d;
        let diffval = highval - lowval;
        if (0..=15).contains(&diffval) {
            let v1 = ((lowval >> 3) & 0xF0) | diffval;
            let v1e = quant_color(quant_level, v1) as i32;
            let v1d = v1e;
            if (v1d & 0xF0) == (v1 & 0xF0) {
                output[0] = v0e as u8;
                output[1] = v1e as u8;
                return true;
            }
        }
    }

    // Failed to encode the high-precision submode; well, then try to encode the
    // low-precision submode.
    let mut lowval = (ilum0.wrapping_add(32) >> 6).clamp(0, 1023);
    let highval = (ilum1.wrapping_add(32) >> 6).clamp(0, 1023);
    let v0 = (lowval & 0x7F) | 0x80;
    let v0e = quant_color(quant_level, v0) as i32;
    let v0d = v0e;
    if (v0d & 0x80) == 0 {
        return false;
    }
    lowval = (lowval & !0x7F) | (v0d & 0x7F);
    let diffval = highval - lowval;
    if !(0..=31).contains(&diffval) {
        return false;
    }
    let v1 = ((lowval >> 2) & 0xE0) | diffval;
    let v1e = quant_color(quant_level, v1) as i32;
    let v1d = v1e;
    if (v1d & 0xE0) != (v1 & 0xE0) {
        return false;
    }
    output[0] = v0e as u8;
    output[1] = v1e as u8;
    true
}

#[isa]
/// `quantize_hdr_alpha`.
fn quantize_hdr_alpha(alpha0: f32, alpha1: f32, output: &mut [u8], quant_level: u32) {
    let alpha0 = clamp(alpha0, 0.0, 65280.0);
    let alpha1 = clamp(alpha1, 0.0, 65280.0);
    let ialpha0 = flt2int_rtn(alpha0);
    let ialpha1 = flt2int_rtn(alpha1);

    // Try to encode one of the delta submodes, in decreasing-precision order
    for i in (0..=2i32).rev() {
        let mut val0 = (ialpha0 + (128 >> i)) >> (8 - i);
        let val1 = (ialpha1 + (128 >> i)) >> (8 - i);
        let v6 = (val0 & 0x7F) | ((i & 1) << 7);
        let v6e = quant_color(quant_level, v6) as i32;
        let v6d = v6e;
        if (v6 ^ v6d) & 0x80 != 0 {
            continue;
        }
        val0 = (val0 & !0x7f) | (v6d & 0x7f);
        let diffval = val1 - val0;
        let cutoff = 32 >> i;
        let mask = 2 * cutoff - 1;
        if diffval < -cutoff || diffval >= cutoff {
            continue;
        }
        let v7 = ((i & 2) << 6) | ((val0 >> 7) << (6 - i)) | (diffval & mask);
        let v7e = quant_color(quant_level, v7) as i32;
        let v7d = v7e;
        const TESTBITS: [i32; 3] = [0xE0, 0xF0, 0xF8];
        if (v7 ^ v7d) & TESTBITS[i as usize] != 0 {
            continue;
        }
        output[0] = v6e as u8;
        output[1] = v7e as u8;
        return;
    }

    // Could not encode any of the delta modes; instead encode a flat value
    let val0 = (ialpha0 + 256) >> 9;
    let val1 = (ialpha1 + 256) >> 9;
    let v6 = val0 | 0x80;
    let v7 = val1 | 0x80;
    output[0] = quant_color(quant_level, v6);
    output[1] = quant_color(quant_level, v7);
}

#[isa]
/// `quantize_hdr_rgb_alpha`.
fn quantize_hdr_rgb_alpha(color0: Vfloat4, color1: Vfloat4, output: &mut [u8], quant_level: u32) {
    quantize_hdr_rgb(color0, color1, output, quant_level);
    quantize_hdr_alpha(
        color0.lane(3),
        color1.lane(3),
        &mut output[6..],
        quant_level,
    );
}

#[isa]
fn to_arr(v: Vint4) -> [i32; 4] {
    v.m
}

#[isa]
fn from_arr(a: [i32; 4]) -> Vint4 {
    Vint4 { m: a }
}

#[isa]
/// `rgba_delta_unpack` on vector types.
fn delta_unpack(c0: Vint4, c1: Vint4) -> (Vint4, Vint4) {
    let (a, b) = super::rgba_delta_unpack(to_arr(c0), to_arr(c1));
    (from_arr(a), from_arr(b))
}

#[isa]
/// `rgba_unpack` on vector types.
fn unpack(c0: Vint4, c1: Vint4) -> (Vint4, Vint4) {
    let (a, b) = super::rgba_unpack(to_arr(c0), to_arr(c1));
    (from_arr(a), from_arr(b))
}

#[isa]
/// `pack_color_endpoints`: quantizes the endpoints into `output` and returns the actual
/// endpoint format used.
pub fn pack_color_endpoints(
    color0: Vfloat4,
    color1: Vfloat4,
    rgbs_color: Vfloat4,
    rgbo_color: Vfloat4,
    format: u8,
    output: &mut [u8; 8],
    quant_level: u32,
) -> u8 {
    // We do not support negative colors
    let color0 = Vfloat4::clamp(0.0, 65535.0, color0);
    let color1 = Vfloat4::clamp(0.0, 65535.0, color1);
    let color0_ldr = color0 * (1.0f32 / 257.0f32);
    let color1_ldr = color1 * (1.0f32 / 257.0f32);

    let mut retval = 0u8;
    let mut best_error = ERROR_CALC_DEFAULT;
    let mut color0_out = Vint4::zero();
    let mut color1_out = Vint4::zero();
    let mut color0_out2 = Vint4::zero();
    let mut color1_out2 = Vint4::zero();

    let rgba = format == FMT_RGBA;
    match format {
        FMT_RGB | FMT_RGBA => {
            let (delta_bc, delta): (fn(Vfloat4, Vfloat4, &mut Vint4, &mut Vint4, u32) -> bool, _) =
                if rgba {
                    (
                        try_quantize_rgba_delta_blue_contract,
                        try_quantize_rgba_delta as fn(_, _, &mut _, &mut _, _) -> bool,
                    )
                } else {
                    (
                        try_quantize_rgb_delta_blue_contract,
                        try_quantize_rgb_delta as fn(_, _, &mut _, &mut _, _) -> bool,
                    )
                };
            let (fmt_delta, fmt_plain) = if rgba {
                (FMT_RGBA_DELTA, FMT_RGBA)
            } else {
                (FMT_RGB_DELTA, FMT_RGB)
            };
            if quant_level <= QUANT_160 {
                if delta_bc(
                    color0_ldr,
                    color1_ldr,
                    &mut color0_out,
                    &mut color1_out,
                    quant_level,
                ) {
                    let (u0, u1) = delta_unpack(color0_out, color1_out);
                    retval = fmt_delta;
                    best_error = get_rgba_encoding_error(color0_ldr, color1_ldr, u0, u1);
                }
                if delta(
                    color0_ldr,
                    color1_ldr,
                    &mut color0_out2,
                    &mut color1_out2,
                    quant_level,
                ) {
                    let (u0, u1) = delta_unpack(color0_out2, color1_out2);
                    let error = get_rgba_encoding_error(color0_ldr, color1_ldr, u0, u1);
                    if error < best_error {
                        retval = fmt_delta;
                        best_error = error;
                        color0_out = color0_out2;
                        color1_out = color1_out2;
                    }
                }
            }
            if quant_level < QUANT_256 {
                let ok = if rgba {
                    try_quantize_rgba_blue_contract(
                        color0_ldr,
                        color1_ldr,
                        &mut color0_out2,
                        &mut color1_out2,
                        quant_level,
                    )
                } else {
                    try_quantize_rgb_blue_contract(
                        color0_ldr,
                        color1_ldr,
                        &mut color0_out2,
                        &mut color1_out2,
                        quant_level,
                    )
                };
                if ok {
                    let (u0, u1) = unpack(color0_out2, color1_out2);
                    let error = get_rgba_encoding_error(color0_ldr, color1_ldr, u0, u1);
                    if error < best_error {
                        retval = fmt_plain;
                        best_error = error;
                        color0_out = color0_out2;
                        color1_out = color1_out2;
                    }
                }
            }
            if rgba {
                quantize_rgba(
                    color0_ldr,
                    color1_ldr,
                    &mut color0_out2,
                    &mut color1_out2,
                    quant_level,
                );
            } else {
                quantize_rgb(
                    color0_ldr,
                    color1_ldr,
                    &mut color0_out2,
                    &mut color1_out2,
                    quant_level,
                );
            }
            let (u0, u1) = unpack(color0_out2, color1_out2);
            let error = get_rgba_encoding_error(color0_ldr, color1_ldr, u0, u1);
            if error < best_error {
                retval = fmt_plain;
                color0_out = color0_out2;
                color1_out = color1_out2;
            }
            output[0] = color0_out.lane(0) as u8;
            output[1] = color1_out.lane(0) as u8;
            output[2] = color0_out.lane(1) as u8;
            output[3] = color1_out.lane(1) as u8;
            output[4] = color0_out.lane(2) as u8;
            output[5] = color1_out.lane(2) as u8;
            if rgba {
                output[6] = color0_out.lane(3) as u8;
                output[7] = color1_out.lane(3) as u8;
            }
        }
        FMT_RGB_SCALE => {
            quantize_rgbs(rgbs_color, output, quant_level);
            retval = FMT_RGB_SCALE;
        }
        FMT_HDR_RGB_SCALE => {
            quantize_hdr_rgbo(rgbo_color, output, quant_level);
            retval = FMT_HDR_RGB_SCALE;
        }
        FMT_HDR_RGB => {
            quantize_hdr_rgb(color0, color1, output, quant_level);
            retval = FMT_HDR_RGB;
        }
        FMT_RGB_SCALE_ALPHA => {
            quantize_rgbs_alpha(color0_ldr, color1_ldr, rgbs_color, output, quant_level);
            retval = FMT_RGB_SCALE_ALPHA;
        }
        FMT_HDR_LUMINANCE_SMALL_RANGE | FMT_HDR_LUMINANCE_LARGE_RANGE => {
            if try_quantize_hdr_luminance_small_range(color0, color1, output, quant_level) {
                retval = FMT_HDR_LUMINANCE_SMALL_RANGE;
            } else {
                quantize_hdr_luminance_large_range(color0, color1, output, quant_level);
                retval = FMT_HDR_LUMINANCE_LARGE_RANGE;
            }
        }
        FMT_LUMINANCE => {
            quantize_luminance(color0_ldr, color1_ldr, output, quant_level);
            retval = FMT_LUMINANCE;
        }
        FMT_LUMINANCE_ALPHA => {
            if quant_level <= 18
                && try_quantize_luminance_alpha_delta(color0_ldr, color1_ldr, output, quant_level)
            {
                retval = FMT_LUMINANCE_ALPHA_DELTA;
            } else {
                quantize_luminance_alpha(color0_ldr, color1_ldr, output, quant_level);
                retval = FMT_LUMINANCE_ALPHA;
            }
        }
        FMT_HDR_RGB_LDR_ALPHA => {
            quantize_hdr_rgb_ldr_alpha(color0, color1, output, quant_level);
            retval = FMT_HDR_RGB_LDR_ALPHA;
        }
        FMT_HDR_RGBA => {
            quantize_hdr_rgb_alpha(color0, color1, output, quant_level);
            retval = FMT_HDR_RGBA;
        }
        _ => {}
    }
    retval
}
