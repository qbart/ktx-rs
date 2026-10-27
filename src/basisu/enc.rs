// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
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

/// `indirect_sort`: fills `indices` with 0..n ordered by `keys` (`std::sort`, so ties keep
/// the libstdc++ introsort order).
pub fn indirect_sort<T: PartialOrd>(indices: &mut [u32], keys: &[T]) {
    for (i, v) in indices.iter_mut().enumerate() {
        *v = i as u32;
    }
    std_sort_by(indices, |&a, &b| keys[a as usize] < keys[b as usize]);
}

/// libstdc++ `std::sort(first, last, comp)` (introsort: median of three quicksort with a
/// `2 * log2(n)` depth limit and heapsort fallback, then a final insertion sort with a 16
/// element threshold). Reproduces the exact order of equivalent elements.
pub fn std_sort_by<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut less: F) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let depth = 2 * (usize::BITS - 1 - n.leading_zeros()) as usize;
    introsort_loop(v, 0, n, depth, &mut less);
    final_insertion_sort(v, 0, n, &mut less);
}

const STL_THRESHOLD: usize = 16;

fn introsort_loop<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    mut last: usize,
    mut depth_limit: usize,
    less: &mut F,
) {
    while last - first > STL_THRESHOLD {
        if depth_limit == 0 {
            // __partial_sort(first, last, last): make_heap + sort_heap
            heap_sort(&mut v[first..last], less);
            return;
        }
        depth_limit -= 1;
        let cut = unguarded_partition_pivot(v, first, last, less);
        introsort_loop(v, cut, last, depth_limit, less);
        last = cut;
    }
}

fn unguarded_partition_pivot<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &mut F,
) -> usize {
    let mid = first + (last - first) / 2;
    move_median_to_first(v, first, first + 1, mid, last - 1, less);
    // __unguarded_partition(first + 1, last, first)
    let (mut lo, mut hi) = (first + 1, last);
    loop {
        while less(&v[lo], &v[first]) {
            lo += 1;
        }
        hi -= 1;
        while less(&v[first], &v[hi]) {
            hi -= 1;
        }
        if lo >= hi {
            return lo;
        }
        v.swap(lo, hi);
        lo += 1;
    }
}

fn move_median_to_first<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    result: usize,
    a: usize,
    b: usize,
    c: usize,
    less: &mut F,
) {
    let m = if less(&v[a], &v[b]) {
        if less(&v[b], &v[c]) {
            b
        } else if less(&v[a], &v[c]) {
            c
        } else {
            a
        }
    } else if less(&v[a], &v[c]) {
        a
    } else if less(&v[b], &v[c]) {
        c
    } else {
        b
    };
    v.swap(result, m);
}

fn final_insertion_sort<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &mut F,
) {
    if last - first > STL_THRESHOLD {
        insertion_sort(v, first, first + STL_THRESHOLD, less);
        for i in first + STL_THRESHOLD..last {
            unguarded_linear_insert(v, i, less);
        }
    } else {
        insertion_sort(v, first, last, less);
    }
}

fn insertion_sort<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &mut F,
) {
    if first == last {
        return;
    }
    for i in first + 1..last {
        if less(&v[i], &v[first]) {
            let val = v[i];
            v.copy_within(first..i, first + 1);
            v[first] = val;
        } else {
            unguarded_linear_insert(v, i, less);
        }
    }
}

fn unguarded_linear_insert<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut last: usize,
    less: &mut F,
) {
    let val = v[last];
    let mut next = last - 1;
    while less(&val, &v[next]) {
        v[last] = v[next];
        last = next;
        next -= 1;
    }
    v[last] = val;
}

fn heap_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], less: &mut F) {
    let len = v.len();
    if len < 2 {
        return;
    }
    // make_heap
    let mut parent = (len - 2) / 2;
    loop {
        let value = v[parent];
        adjust_heap(v, parent, len, value, less);
        if parent == 0 {
            break;
        }
        parent -= 1;
    }
    // sort_heap
    let mut last = len;
    while last > 1 {
        last -= 1;
        // __pop_heap(first, last, last)
        let value = v[last];
        v[last] = v[0];
        adjust_heap(v, 0, last, value, less);
    }
}

fn adjust_heap<T: Copy, F: FnMut(&T, &T) -> bool>(
    v: &mut [T],
    mut hole: usize,
    len: usize,
    value: T,
    less: &mut F,
) {
    let top = hole;
    let mut second = hole;
    while second < (len - 1) / 2 {
        second = 2 * (second + 1);
        if less(&v[second], &v[second - 1]) {
            second -= 1;
        }
        v[hole] = v[second];
        hole = second;
    }
    if len & 1 == 0 && second == (len - 2) / 2 {
        second = 2 * (second + 1);
        v[hole] = v[second - 1];
        hole = second - 1;
    }
    // __push_heap(first, hole, top, value)
    let mut parent = hole.wrapping_sub(1) / 2;
    while hole > top && less(&v[parent], &value) {
        v[hole] = v[parent];
        hole = parent;
        parent = hole.wrapping_sub(1) / 2;
    }
    v[hole] = value;
}

/// `basist::crc16`.
pub fn crc16(buf: &[u8], crc: u16) -> u16 {
    let mut crc = !crc;
    for &b in buf {
        let q = b as u16 ^ (crc >> 8);
        let k = (q >> 4) ^ q;
        crc = (((crc << 8) ^ k) ^ (k << 5)) ^ (k << 12);
    }
    !crc
}
