//! Port of `transcode_uastc_to_pvrtc1_4_rgb/rgba` and their modulation fix-up
//! passes from basisu_transcoder.cpp (shares the ETC1S PVRTC1 machinery).

use super::etc::Color32;
use super::pvrtc::{
    fixup_pvrtc1_4_modulation, get_endpoint_l8, get_opaque_endpoint_l0, get_opaque_endpoint_l1,
    set_endpoint_ceil, set_endpoint_floor, set_opaque_endpoint_ceil, set_opaque_endpoint_floor,
};
use super::uastc::unpack_uastc_pixels;

fn block(src: &[u8], i: usize) -> Option<[u8; 16]> {
    let mut b = [0u8; 16];
    b.copy_from_slice(src.get(i * 16..i * 16 + 16)?);
    Some(b)
}

/// Decodes block `i` (C ignores unpack failures in the fix-up pass).
fn pixels(src: &[u8], i: usize) -> [Color32; 16] {
    let mut px = [Color32::default(); 16];
    if let Some(b) = block(src, i) {
        unpack_uastc_pixels(&b, &mut px, false);
    }
    px
}

fn check_dims(num_blocks_x: u32, num_blocks_y: u32) -> bool {
    num_blocks_x != 0
        && num_blocks_y != 0
        && (num_blocks_x * 4).is_power_of_two()
        && (num_blocks_y * 4).is_power_of_two()
}

/// Port of `transcode_uastc_to_pvrtc1_4_rgb`.
pub fn transcode_uastc_to_pvrtc1_4_rgb(
    src: &[u8],
    dst: &mut [u8],
    num_blocks_x: u32,
    num_blocks_y: u32,
    from_alpha: bool,
) -> bool {
    if !check_dims(num_blocks_x, num_blocks_y) {
        return false;
    }
    let total = num_blocks_x as usize * num_blocks_y as usize;
    let mut endpoints = Vec::new();
    if endpoints.try_reserve_exact(total).is_err() {
        return false;
    }
    for i in 0..total {
        let Some(b) = block(src, i) else {
            return false;
        };
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_pixels(&b, &mut px, false) {
            return false;
        }
        let (mut low, mut high) = ([255u8; 4], [0u8; 4]);
        if from_alpha {
            let low_a = px.iter().map(|p| p.a()).min().unwrap_or(255);
            let high_a = px.iter().map(|p| p.a()).max().unwrap_or(0);
            low = [low_a, low_a, low_a, 255];
            high = [high_a, high_a, high_a, 255];
        } else {
            for p in &px {
                for c in 0..4 {
                    low[c] = low[c].min(p.c[c]);
                    high[c] = high[c].max(p.c[c]);
                }
            }
        }
        let mut e = 0u32;
        set_opaque_endpoint_floor(&mut e, 0, low);
        set_opaque_endpoint_ceil(&mut e, 1, high);
        endpoints.push(e);
    }
    let lum = |e: u32, idx: u32| {
        let l = if idx == 0 {
            get_opaque_endpoint_l0(e)
        } else {
            get_opaque_endpoint_l1(e)
        };
        (l * 255 / 31) as i32
    };
    let mut cache: Option<(usize, [Color32; 16])> = None;
    let pixel = |bi: usize, lx: u32, ly: u32| -> Option<i32> {
        if cache.as_ref().map(|c| c.0) != Some(bi) {
            let mut px = pixels(src, bi);
            if from_alpha {
                for p in px.iter_mut() {
                    let a = p.a();
                    p.c = [a, a, a, 255];
                }
            }
            cache = Some((bi, px));
        }
        let (_, px) = cache.as_ref()?;
        let p = &px[(ly * 4 + lx) as usize];
        Some((p.r() as i32 + p.g() as i32 + p.b() as i32) * 16)
    };
    fixup_pvrtc1_4_modulation(&endpoints, dst, num_blocks_x, num_blocks_y, lum, pixel)
}

/// Port of `transcode_uastc_to_pvrtc1_4_rgba`.
pub fn transcode_uastc_to_pvrtc1_4_rgba(
    src: &[u8],
    dst: &mut [u8],
    num_blocks_x: u32,
    num_blocks_y: u32,
) -> bool {
    if !check_dims(num_blocks_x, num_blocks_y) {
        return false;
    }
    let total = num_blocks_x as usize * num_blocks_y as usize;
    let mut endpoints = Vec::new();
    if endpoints.try_reserve_exact(total).is_err() {
        return false;
    }
    for i in 0..total {
        let Some(b) = block(src, i) else {
            return false;
        };
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_pixels(&b, &mut px, false) {
            return false;
        }
        let (mut low, mut high) = ([255u8; 4], [0u8; 4]);
        for p in &px {
            for c in 0..4 {
                low[c] = low[c].min(p.c[c]);
                high[c] = high[c].max(p.c[c]);
            }
        }
        let mut e = 0u32;
        set_endpoint_floor(&mut e, 0, low);
        set_endpoint_ceil(&mut e, 1, high);
        endpoints.push(e);
    }
    let lum = |e: u32, idx: u32| get_endpoint_l8(e, idx) as i32;
    let mut cache: Option<(usize, [Color32; 16])> = None;
    let pixel = |bi: usize, lx: u32, ly: u32| -> Option<i32> {
        if cache.as_ref().map(|c| c.0) != Some(bi) {
            cache = Some((bi, pixels(src, bi)));
        }
        let (_, px) = cache.as_ref()?;
        let p = &px[(ly * 4 + lx) as usize];
        Some(16 * (p.r() as i32 + p.g() as i32 + p.b() as i32 + p.a() as i32))
    };
    fixup_pvrtc1_4_modulation(&endpoints, dst, num_blocks_x, num_blocks_y, lum, pixel)
}
