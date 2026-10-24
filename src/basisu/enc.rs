//! Subset of `encoder/basisu_enc.h` / `basisu_enc.cpp` shared by the Basis Universal encoders:
//! `color_rgba` helpers, `color_distance` and `hash_hsieh`.

/// `basisu::color_rgba` (r, g, b, a).
pub type ColorRgba = [u8; 4];

/// `color_rgba(sr, sg, sb, sa)`: clamps each component to 0..=255.
#[inline]
pub fn color_rgba(r: i32, g: i32, b: i32, a: i32) -> ColorRgba {
    [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
        a.clamp(0, 255) as u8,
    ]
}

/// `color_rgba(cNoClamp, sr, sg, sb, sa)`: truncates each component to 8 bits.
#[inline]
pub fn color_rgba_noclamp(r: i32, g: i32, b: i32, a: i32) -> ColorRgba {
    [r as u8, g as u8, b as u8, a as u8]
}

/// `color_distance(c0, c1, alpha)` (squared RGB(A) distance).
#[inline]
pub fn color_distance_linear(c0: &ColorRgba, c1: &ColorRgba, alpha: bool) -> i32 {
    let dr = c0[0] as i32 - c1[0] as i32;
    let dg = c0[1] as i32 - c1[1] as i32;
    let db = c0[2] as i32 - c1[2] as i32;
    let mut d = dr * dr + dg * dg + db * db;
    if alpha {
        let da = c0[3] as i32 - c1[3] as i32;
        d += da * da;
    }
    d
}

/// `color_distance(perceptual, e1, e2, alpha)`.
#[inline]
pub fn color_distance(perceptual: bool, e1: &ColorRgba, e2: &ColorRgba, alpha: bool) -> u32 {
    if perceptual {
        let dr = e1[0] as i32 - e2[0] as i32;
        let dg = e1[1] as i32 - e2[1] as i32;
        let db = e1[2] as i32 - e2[2] as i32;
        let delta_l = dr * 14 + dg * 45 + db * 5;
        let delta_cr = dr * 64 - delta_l;
        let delta_cb = db * 64 - delta_l;
        let mut id = ((delta_l * delta_l) as u32 >> 5)
            .wrapping_add(((((delta_cr * delta_cr) as u32) >> 5).wrapping_mul(26)) >> 7)
            .wrapping_add(((((delta_cb * delta_cb) as u32) >> 5).wrapping_mul(3)) >> 7);
        if alpha {
            let da = (e1[3] as i32 - e2[3] as i32) << 7;
            id = id.wrapping_add((da * da) as u32 >> 7);
        }
        id
    } else {
        color_distance_linear(e1, e2, alpha) as u32
    }
}

/// `basist::hash_hsieh`.
pub fn hash_hsieh(buf: &[u8]) -> u32 {
    let len = buf.len();
    if len == 0 {
        return 0;
    }
    let mut h = len as u32;
    let bytes_left = len & 3;
    let mut p = 0;
    for _ in 0..len >> 2 {
        let w0 = u16::from_le_bytes([buf[p], buf[p + 1]]) as u32;
        let w1 = u16::from_le_bytes([buf[p + 2], buf[p + 3]]) as u32;
        h = h.wrapping_add(w0);
        let t = (w1 << 11) ^ h;
        h = (h << 16) ^ t;
        p += 4;
        h = h.wrapping_add(h >> 11);
    }
    match bytes_left {
        1 => {
            h = h.wrapping_add(buf[p] as i8 as i32 as u32);
            h ^= h << 10;
            h = h.wrapping_add(h >> 1);
        }
        2 => {
            h = h.wrapping_add(u16::from_le_bytes([buf[p], buf[p + 1]]) as u32);
            h ^= h << 11;
            h = h.wrapping_add(h >> 17);
        }
        3 => {
            h = h.wrapping_add(u16::from_le_bytes([buf[p], buf[p + 1]]) as u32);
            h ^= h << 16;
            h ^= ((buf[p + 2] as i8 as i32) << 18) as u32;
            h = h.wrapping_add(h >> 11);
        }
        _ => {}
    }
    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);
    h
}

/// `indirect_sort`: fills `indices` with 0..n ordered by `keys`.
///
/// `std::sort` is not stable; callers only rely on the key order (ties are equivalent).
pub fn indirect_sort<T: PartialOrd>(indices: &mut [u32], keys: &[T]) {
    for (i, v) in indices.iter_mut().enumerate() {
        *v = i as u32;
    }
    indices.sort_by(|&a, &b| {
        keys[a as usize]
            .partial_cmp(&keys[b as usize])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
