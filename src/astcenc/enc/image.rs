// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of the compressor block loaders of `astcenc_image.cpp`
//! (`load_image_block`, `load_image_block_fast_ldr`).

use super::internal::{BlockSizeDescriptor, ImageBlock};
use super::vecmath::{Vfloat4, Vint4, Vmask4, float_to_lns, float16_to_float_v, lns_to_sf16};
use super::{AstcencImage, AstcencImageData, AstcencProfile, AstcencSwizzle, AstcencSwz};
use ktx_rs_macros::isa;

#[isa]
/// The loader of one texel: RGBA at texel `index` of plane `z`.
fn load_texel(img: &AstcencImage, z: usize, base_offset: usize) -> Vfloat4 {
    match &img.data {
        AstcencImageData::U8(p) => {
            let d = p.get(z).map(Vec::as_slice).unwrap_or_default();
            Vint4::from_u8(d.get(base_offset..).unwrap_or_default()).int_to_float() / 255.0
        }
        AstcencImageData::F16(p) => {
            let d = p.get(z).map(Vec::as_slice).unwrap_or_default();
            let v = |k: usize| d.get(base_offset + k).copied().unwrap_or(0) as i32;
            float16_to_float_v(Vint4::new(v(0), v(1), v(2), v(3)))
        }
        AstcencImageData::F32(p) => {
            let d = p.get(z).map(Vec::as_slice).unwrap_or_default();
            Vfloat4::load(d.get(base_offset..).unwrap_or_default())
        }
    }
}

#[isa]
/// `swz_texel`.
fn swz_texel(data: Vfloat4, swz: &AstcencSwizzle) -> Vfloat4 {
    let datas = [data.m[0], data.m[1], data.m[2], data.m[3], 0.0, 1.0];
    let pick = |s: AstcencSwz| datas.get(s as usize).copied().unwrap_or(0.0);
    Vfloat4::new(pick(swz.r), pick(swz.g), pick(swz.b), pick(swz.a))
}

#[isa]
/// `encode_texel_unorm`.
fn encode_texel_unorm(data: Vfloat4) -> Vfloat4 {
    Vfloat4::clamp(0.0, 65535.0, data * 65535.0)
}

#[isa]
/// `encode_texel_lns`.
fn encode_texel_lns(data: Vfloat4, lns_mask: Vmask4) -> Vfloat4 {
    let datav_unorm = encode_texel_unorm(data);
    let datav_lns = float_to_lns(data);
    Vfloat4::select(datav_unorm, datav_lns, lns_mask)
}

#[isa]
/// `grayscalev & (datav.swz<0,0,0,0>() == datav.swz<1,1,2,2>())`.
fn gray_mask(datav: Vfloat4) -> Vmask4 {
    datav.swz4(0, 0, 0, 0).eq(datav.swz4(1, 1, 2, 2))
}

#[isa]
/// `load_image_block`.
#[allow(clippy::too_many_arguments)]
pub fn load_image_block(
    decode_mode: AstcencProfile,
    img: &AstcencImage,
    blk: &mut ImageBlock,
    bsd: &BlockSizeDescriptor,
    pos_x: usize,
    pos_y: usize,
    pos_z: usize,
    swz: &AstcencSwizzle,
) {
    let size_x = img.dim_x as usize;
    let size_y = img.dim_y as usize;
    let size_z = img.dim_z as usize;
    blk.pos_x = pos_x;
    blk.pos_y = pos_y;
    blk.pos_z = pos_z;

    // True if any non-identity swizzle
    let needs_swz = *swz != AstcencSwizzle::RGBA;

    let mut data_min = Vfloat4::splat(1e38);
    let mut data_mean = Vfloat4::splat(0.0);
    let data_mean_scale = Vfloat4::splat(1.0 / bsd.texel_count as f32);
    let mut data_max = Vfloat4::splat(-1e38);
    let mut grayscalev = Vmask4::splat(true);
    let mut idx = 0usize;

    let rgb_lns: u8 = matches!(
        decode_mode,
        AstcencProfile::Hdr | AstcencProfile::HdrRgbLdrA
    ) as u8;
    let a_lns: u8 = (decode_mode == AstcencProfile::Hdr) as u8;
    let use_lns = Vint4::new(rgb_lns as i32, rgb_lns as i32, rgb_lns as i32, a_lns as i32);
    let lns_mask = use_lns.ne(Vint4::zero());
    let lns = lns_mask.any();

    for z in 0..bsd.dim_z as usize {
        let zi = (pos_z + z).min(size_z.wrapping_sub(1));
        for y in 0..bsd.dim_y as usize {
            let yi = (pos_y + y).min(size_y.wrapping_sub(1));
            for x in 0..bsd.dim_x as usize {
                let xi = (pos_x + x).min(size_x.wrapping_sub(1));
                let mut datav = load_texel(img, zi, (4 * size_x * yi) + (4 * xi));
                if needs_swz {
                    datav = swz_texel(datav, swz);
                }
                datav = if lns {
                    encode_texel_lns(datav, lns_mask)
                } else {
                    encode_texel_unorm(datav)
                };

                // Compute block metadata
                data_min = data_min.min(datav);
                data_mean += datav * data_mean_scale;
                data_max = data_max.max(datav);
                grayscalev = grayscalev & gray_mask(datav);

                blk.data_r[idx] = datav.m[0];
                blk.data_g[idx] = datav.m[1];
                blk.data_b[idx] = datav.m[2];
                blk.data_a[idx] = datav.m[3];
                blk.rgb_lns[idx] = rgb_lns;
                blk.alpha_lns[idx] = a_lns;
                idx += 1;
            }
        }
    }

    // Reverse the encoding so we store origin block in the original format
    let data_enc = blk.texel(0);
    let data_enc_unorm = data_enc / 65535.0;
    let mut data_enc_lns = Vfloat4::zero();
    if rgb_lns != 0 || a_lns != 0 {
        data_enc_lns = float16_to_float_v(lns_to_sf16(data_enc.float_to_int()));
    }
    blk.origin_texel = Vfloat4::select(data_enc_unorm, data_enc_lns, lns_mask);

    // Store block metadata
    blk.data_min = data_min;
    blk.data_mean = data_mean;
    blk.data_max = data_max;
    blk.grayscale = grayscalev.all();
}

#[isa]
/// `load_image_block_fast_ldr` (U8 input, LDR, no swizzle, 2D).
pub fn load_image_block_fast_ldr(
    img: &AstcencImage,
    blk: &mut ImageBlock,
    bsd: &BlockSizeDescriptor,
    pos_x: usize,
    pos_y: usize,
) {
    let size_x = img.dim_x as usize;
    let size_y = img.dim_y as usize;
    blk.pos_x = pos_x;
    blk.pos_y = pos_y;
    blk.pos_z = 0;

    let mut data_min = Vfloat4::splat(1e38);
    let mut data_mean = Vfloat4::zero();
    let mut data_max = Vfloat4::splat(-1e38);
    let mut grayscalev = Vmask4::splat(true);
    let mut idx = 0usize;

    let plane: &[u8] = match &img.data {
        AstcencImageData::U8(p) => p.first().map(Vec::as_slice).unwrap_or_default(),
        _ => &[],
    };
    for y in pos_y..pos_y + bsd.dim_y as usize {
        let yi = y.min(size_y.wrapping_sub(1));
        for x in pos_x..pos_x + bsd.dim_x as usize {
            let xi = x.min(size_x.wrapping_sub(1));
            let datavi = Vint4::from_u8(
                plane
                    .get((4 * size_x * yi) + (4 * xi)..)
                    .unwrap_or_default(),
            );
            let datav = datavi.int_to_float() * (65535.0f32 / 255.0f32);

            // Compute block metadata
            data_min = data_min.min(datav);
            data_mean += datav;
            data_max = data_max.max(datav);
            grayscalev = grayscalev & gray_mask(datav);

            blk.data_r[idx] = datav.m[0];
            blk.data_g[idx] = datav.m[1];
            blk.data_b[idx] = datav.m[2];
            blk.data_a[idx] = datav.m[3];
            idx += 1;
        }
    }

    // Reverse the encoding so we store origin block in the original format
    blk.origin_texel = blk.texel(0) / 65535.0;

    // Store block metadata
    blk.rgb_lns[0] = 0;
    blk.alpha_lns[0] = 0;
    blk.data_min = data_min;
    blk.data_mean = data_mean / bsd.texel_count as f32;
    blk.data_max = data_max;
    blk.grayscale = grayscalev.all();
}
