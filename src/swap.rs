//! Port of lib/src/swap.c.
//!
//! Divergence: the C `_ktxSwapEndian64` mis-shifts byte 6 (`<< 40` instead of `>> 40`);
//! this port performs a correct 64-bit byte swap.

/// Port of `_ktxSwapEndian16`.
pub fn ktx_swap_endian16(data: &mut [u16]) {
    data.iter_mut().for_each(|x| *x = x.swap_bytes());
}

/// Port of `_ktxSwapEndian32`.
pub fn ktx_swap_endian32(data: &mut [u32]) {
    data.iter_mut().for_each(|x| *x = x.swap_bytes());
}

/// Port of `_ktxSwapEndian64`.
pub fn ktx_swap_endian64(data: &mut [u64]) {
    data.iter_mut().for_each(|x| *x = x.swap_bytes());
}

/// Swaps byte order of every `width`-byte group in a raw byte buffer.
/// `width` must be 2, 4 or 8; trailing bytes that do not form a full group are left as is.
pub fn ktx_swap_endian_bytes(data: &mut [u8], width: usize) {
    if width < 2 {
        return;
    }
    data.chunks_exact_mut(width).for_each(|c| c.reverse());
}
