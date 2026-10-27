// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of tests/ktxdiff/ktxdiff_main.cpp: compares an expected and a received
//! KTX2 file allowing `tolerance` differences in the image data. Used by the
//! CTS runner for `outputTolerance` cases, like clitest.py on non-primary
//! platforms (zlib level 1 output differs between x86-64 and arm64 miniz).

#![allow(clippy::chunks_exact_to_as_chunks, clippy::too_many_arguments)]

use ktx::astcenc::*;
use ktx::dfd::khr_df::*;
use ktx::dfd::{
    BITLENGTH, MODEL, QUALIFIERS, SAMPLEUPPER, TEXELBLOCKDIMENSION0, TEXELBLOCKDIMENSION1,
    TEXELBLOCKDIMENSION2, TRANSFER, dfdsamplecount, dfdsval, dfdval,
};
use ktx::texture::KtxTexture;
use ktx::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, TranscodeFmt, VkFormat};

struct Texture {
    raw: Vec<u8>,
    handle: Texture2,
    transcoded: bool,
    dfd: Vec<u8>,
    kvd: Vec<u8>,
    sgd: Vec<u8>,
    level_count: u32,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from(rd32(b, o)) | (u64::from(rd32(b, o + 4)) << 32)
}

fn load(path: &std::path::Path) -> Result<Texture, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut handle = Texture2::create_from_memory(&raw, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
        .map_err(|e| format!("{}: ktxTexture2_CreateFromMemory: {e:?}", path.display()))?;
    let mut transcoded = false;
    if handle.is_transcodable() {
        let fmt = if handle.is_hdr() {
            TranscodeFmt::RGBA_HALF
        } else {
            TranscodeFmt::RGBA32
        };
        handle
            .transcode_basis(fmt, 0)
            .map_err(|e| format!("{}: ktxTexture2_TranscodeBasis: {e:?}", path.display()))?;
        transcoded = true;
    }
    let slice = |off: u64, len: u64| -> Vec<u8> {
        raw.get(off as usize..(off + len) as usize)
            .unwrap_or_default()
            .to_vec()
    };
    let (dfd_off, dfd_len) = (rd32(&raw, 48) as u64, rd32(&raw, 52) as u64);
    let (kvd_off, kvd_len) = (rd32(&raw, 56) as u64, rd32(&raw, 60) as u64);
    let sgd_len = rd64(&raw, 72);
    let dfd = slice(dfd_off, dfd_len);
    let kvd = slice(kvd_off, kvd_len);
    // Like ktxdiff, the "SGD" is read from the DFD offset / size.
    let sgd = if sgd_len != 0 {
        slice(dfd_off, dfd_len)
    } else {
        Vec::new()
    };
    let level_count = rd32(&raw, 40).max(1);
    Ok(Texture {
        raw,
        handle,
        transcoded,
        dfd,
        kvd,
        sgd,
        level_count,
    })
}

fn compare_unorm8(l: &[u8], r: &[u8], tolerance: f32) -> Option<(f32, usize)> {
    for (i, (a, b)) in l.iter().zip(r).enumerate() {
        let diff = (*a as f32 / 255.0 - *b as f32 / 255.0).abs();
        if diff > tolerance {
            return Some((diff, i));
        }
    }
    None
}

fn u16s(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn compare_unorm16(l: &[u8], r: &[u8], tolerance: f32) -> Option<(f32, usize)> {
    for (i, (a, b)) in u16s(l).iter().zip(u16s(r)).enumerate() {
        let diff = (*a as f32 / 65535.0 - b as f32 / 65535.0).abs();
        if diff > tolerance {
            return Some((diff, i));
        }
    }
    None
}

fn compare_sfloat32(l: &[u8], r: &[u8], tolerance: f32) -> Option<(f32, usize)> {
    let f = |b: &[u8]| -> Vec<f32> {
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    };
    let (l, r) = (f(l), f(r));
    // ktxdiff uses element [1] for the relative tolerance.
    let abs_min = l
        .get(1)
        .copied()
        .unwrap_or(0.0)
        .abs()
        .min(r.get(1).copied().unwrap_or(0.0).abs());
    for (i, (a, b)) in l.iter().zip(&r).enumerate() {
        let diff = (a - b).abs();
        if diff > tolerance * abs_min {
            return Some((diff, i));
        }
    }
    None
}

fn compare_sfloat16(l: &[u8], r: &[u8], tolerance: f32) -> Option<(f32, usize)> {
    for (i, (a, b)) in u16s(l).iter().zip(u16s(r)).enumerate() {
        let (a, b) = (
            ktx::tinyexr::half_to_float(*a),
            ktx::tinyexr::half_to_float(b),
        );
        let diff = (a - b).abs();
        let abs_min = a.abs().min(b.abs());
        if diff > tolerance * abs_min {
            return Some((diff, i));
        }
    }
    None
}

fn decode_astc(
    data: &[u8],
    w: u32,
    h: u32,
    is_float: bool,
    srgb: bool,
    bx: u32,
    by: u32,
    bz: u32,
) -> Result<Vec<u8>, String> {
    let profile = if is_float {
        AstcencProfile::Hdr
    } else if srgb {
        AstcencProfile::LdrSrgb
    } else {
        AstcencProfile::Ldr
    };
    let config = astcenc_config_init(
        profile,
        bx,
        by,
        bz,
        ASTCENC_PRE_MEDIUM,
        ASTCENC_FLG_DECOMPRESS_ONLY,
    )
    .map_err(|e| format!("astcenc_config_init: {}", astcenc_get_error_string(e)))?;
    let mut ctx = astcenc_context_alloc(&config, 1)
        .map_err(|e| format!("astcenc_context_alloc: {}", astcenc_get_error_string(e)))?;
    let n = w as usize * h as usize * 4;
    let mut image = AstcencImage {
        dim_x: w,
        dim_y: h,
        dim_z: 1,
        data: if is_float {
            AstcencImageData::F16(vec![vec![0; n]])
        } else {
            AstcencImageData::U8(vec![vec![0; n]])
        },
    };
    astcenc_decompress_image(&mut ctx, data, &mut image, &AstcencSwizzle::RGBA, 0)
        .map_err(|e| format!("astcenc_decompress_image: {}", astcenc_get_error_string(e)))?;
    Ok(match image.data {
        AstcencImageData::U8(mut p) => p.pop().unwrap_or_default(),
        AstcencImageData::F16(p) => p.iter().flatten().flat_map(|v| v.to_le_bytes()).collect(),
        AstcencImageData::F32(p) => p.iter().flatten().flat_map(|v| v.to_le_bytes()).collect(),
    })
}

/// `ktxdiff <expected> <received> <tolerance>`: `Ok(())` on match, `Err(reason)` otherwise.
pub fn ktxdiff(
    expected: &std::path::Path,
    received: &std::path::Path,
    tolerance: f32,
) -> Result<(), String> {
    let lhs = load(expected)?;
    let rhs = load(received)?;

    let bdfd: Vec<u32> = lhs
        .dfd
        .chunks_exact(4)
        .skip(1)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let component_count = dfdsamplecount(&bdfd).max(1) as usize;
    let bx = dfdval(&bdfd, TEXELBLOCKDIMENSION0) + 1;
    let by = dfdval(&bdfd, TEXELBLOCKDIMENSION1) + 1;
    let bz = dfdval(&bdfd, TEXELBLOCKDIMENSION2) + 1;
    let srgb = dfdval(&bdfd, TRANSFER) == KHR_DF_TRANSFER_SRGB;
    let is_astc = dfdval(&bdfd, MODEL) == KHR_DF_MODEL_ASTC;
    let q = dfdsval(&bdfd, 0, QUALIFIERS);
    let is_signed = q & KHR_DF_SAMPLE_DATATYPE_SIGNED != 0;
    let is_float = q & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0;
    let is_normalized =
        dfdsval(&bdfd, 0, SAMPLEUPPER) != if is_float { 1.0f32.to_bits() } else { 1 };
    let bits = dfdsval(&bdfd, 0, BITLENGTH) + 1;
    let vk = VkFormat(rd32(&lhs.raw, 12));
    let sfloat32 = is_signed && is_float && bits == 32 && vk != VkFormat::D32_SFLOAT_S8_UINT;
    let sfloat16 = is_signed && is_float && bits == 16;
    let unorm8 = !is_signed && !is_float && bits == 8 && is_normalized;
    let unorm16 = !is_signed && !is_float && bits == 16 && is_normalized;

    let header = |t: &Texture, n: usize| t.raw.get(..n).unwrap_or_default().to_vec();
    if lhs.transcoded {
        // For encoded images the compressed data sizes can differ.
        if header(&lhs, 64) != header(&rhs, 64) {
            return Err("Mismatching header".into());
        }
    } else {
        if header(&lhs, 80) != header(&rhs, 80) {
            return Err("Mismatching header".into());
        }
        if lhs.level_count != rhs.level_count {
            return Err("Mismatching levelIndices".into());
        }
        for i in 0..lhs.level_count as usize {
            // Offsets and (compressed) sizes can differ, but uncompressedByteLength must match
            let o = 80 + i * 24 + 16;
            if rd64(&lhs.raw, o) != rd64(&rhs.raw, o) {
                return Err(format!(
                    "Mismatching levelIndices[{i}].uncompressedByteLength"
                ));
            }
        }
    }
    if lhs.dfd != rhs.dfd {
        return Err("Mismatching DFD".into());
    }
    if lhs.kvd != rhs.kvd {
        return Err("Mismatching KVD".into());
    }
    if !lhs.transcoded && lhs.sgd != rhs.sgd {
        return Err("Mismatching SGD".into());
    }
    // If the tolerance is 1 or above and data is normalized, accept every image data as matching
    if is_normalized && tolerance >= 1.0 {
        return Ok(());
    }

    let (l, r) = (&lhs.handle, &rhs.handle);
    let ld = l.p_data.as_deref().unwrap_or_default();
    let rdat = r.p_data.as_deref().unwrap_or_default();
    for level in 0..l.num_levels {
        let size = l.get_image_size(level);
        let w = (l.base_width >> level).max(1);
        let h = (l.base_height >> level).max(1);
        let d = (l.base_depth >> level).max(1);
        for face in 0..l.num_faces {
            for layer in 0..l.num_layers {
                for depth in 0..d.div_ceil(bz) {
                    let off = l
                        .get_image_offset(level, layer, face + depth)
                        .map_err(|e| format!("{e:?}"))?;
                    let a = ld.get(off..off + size).unwrap_or_default();
                    let b = rdat.get(off..off + size).unwrap_or_default();
                    let result = if (lhs.transcoded && !is_float) || unorm8 {
                        compare_unorm8(a, b, tolerance)
                    } else if (lhs.transcoded && is_float) || sfloat16 {
                        compare_sfloat16(a, b, tolerance)
                    } else if unorm16 {
                        compare_unorm16(a, b, tolerance)
                    } else if is_astc {
                        let ua = decode_astc(a, w, h, is_float, srgb, bx, by, bz)?;
                        let ub = decode_astc(b, w, h, is_float, srgb, bx, by, bz)?;
                        if is_float {
                            compare_sfloat16(&ua, &ub, tolerance)
                        } else {
                            compare_unorm8(&ua, &ub, tolerance)
                        }
                    } else if sfloat32 {
                        compare_sfloat32(a, b, tolerance)
                    } else {
                        if let Some(i) = (0..size).find(|&i| a.get(i) != b.get(i)) {
                            return Err(format!(
                                "Mismatching image data: level {level}, face {face}, layer {layer}, depth {depth}, image byte {i}"
                            ));
                        }
                        None
                    };
                    if let Some((diff, i)) = result {
                        return Err(format!(
                            "Mismatching image data (diff: {diff}): level {level}, face {face}, layer {layer}, depth {depth}, pixel {}, component {}",
                            i / component_count,
                            i % component_count
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
