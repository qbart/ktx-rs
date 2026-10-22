//! Port of the deflate half of basisu_miniz.h (miniz 1.x `tdefl` and the
//! `mz_compress2` / `mz_compressBound` zlib-style wrappers) that libktx uses
//! for ZLIB supercompression (lib/src/miniz_wrapper.cpp).
//!
//! The x86-64 configuration is ported (`MINIZ_USE_UNALIGNED_LOADS_AND_STORES`,
//! `MINIZ_LITTLE_ENDIAN`, `MINIZ_HAS_64BIT_REGISTERS`): `tdefl_find_match`
//! compares 16-bit words and level 1 uses `tdefl_compress_fast`, so output is
//! byte-identical to libktx on x86-64. Output always goes to a growable
//! buffer, which yields the same bytes as miniz writing into a caller buffer
//! of sufficient size.

/// `TDEFL_HUFFMAN_ONLY`.
pub const TDEFL_HUFFMAN_ONLY: u32 = 0;
/// `TDEFL_DEFAULT_MAX_PROBES`.
pub const TDEFL_DEFAULT_MAX_PROBES: u32 = 128;
/// `TDEFL_MAX_PROBES_MASK`.
pub const TDEFL_MAX_PROBES_MASK: u32 = 0xFFF;
/// `TDEFL_WRITE_ZLIB_HEADER`.
pub const TDEFL_WRITE_ZLIB_HEADER: u32 = 0x01000;
/// `TDEFL_COMPUTE_ADLER32`.
pub const TDEFL_COMPUTE_ADLER32: u32 = 0x02000;
/// `TDEFL_GREEDY_PARSING_FLAG`.
pub const TDEFL_GREEDY_PARSING_FLAG: u32 = 0x04000;
/// `TDEFL_NONDETERMINISTIC_PARSING_FLAG`.
pub const TDEFL_NONDETERMINISTIC_PARSING_FLAG: u32 = 0x08000;
/// `TDEFL_RLE_MATCHES`.
pub const TDEFL_RLE_MATCHES: u32 = 0x10000;
/// `TDEFL_FILTER_MATCHES`.
pub const TDEFL_FILTER_MATCHES: u32 = 0x20000;
/// `TDEFL_FORCE_ALL_STATIC_BLOCKS`.
pub const TDEFL_FORCE_ALL_STATIC_BLOCKS: u32 = 0x40000;
/// `TDEFL_FORCE_ALL_RAW_BLOCKS`.
pub const TDEFL_FORCE_ALL_RAW_BLOCKS: u32 = 0x80000;

/// zlib-style status codes (`MZ_OK`, ...).
pub const MZ_OK: i32 = 0;
pub const MZ_STREAM_END: i32 = 1;
pub const MZ_STREAM_ERROR: i32 = -2;
pub const MZ_BUF_ERROR: i32 = -5;
pub const MZ_PARAM_ERROR: i32 = -10000;

/// `MZ_DEFAULT_LEVEL`.
pub const MZ_DEFAULT_LEVEL: i32 = 6;
/// `MZ_DEFAULT_COMPRESSION`.
pub const MZ_DEFAULT_COMPRESSION: i32 = -1;
/// Strategies (`MZ_DEFAULT_STRATEGY`, ...).
pub const MZ_DEFAULT_STRATEGY: i32 = 0;
pub const MZ_FILTERED: i32 = 1;
pub const MZ_HUFFMAN_ONLY: i32 = 2;
pub const MZ_RLE: i32 = 3;
pub const MZ_FIXED: i32 = 4;
/// `MZ_DEFAULT_WINDOW_BITS`.
pub const MZ_DEFAULT_WINDOW_BITS: i32 = 15;

const TDEFL_MAX_HUFF_SYMBOLS_0: usize = 288;
const TDEFL_MAX_HUFF_SYMBOLS_1: usize = 32;
const TDEFL_MAX_HUFF_SYMBOLS_2: usize = 19;
const TDEFL_LZ_DICT_SIZE: u32 = 32768;
const TDEFL_LZ_DICT_SIZE_MASK: u32 = TDEFL_LZ_DICT_SIZE - 1;
const TDEFL_MIN_MATCH_LEN: u32 = 3;
const TDEFL_MAX_MATCH_LEN: u32 = 258;
const TDEFL_LZ_CODE_BUF_SIZE: usize = 64 * 1024;
const TDEFL_OUT_BUF_SIZE: usize = (TDEFL_LZ_CODE_BUF_SIZE * 13) / 10;
const TDEFL_MAX_HUFF_SYMBOLS: usize = 288;
const TDEFL_LZ_HASH_BITS: u32 = 15;
const TDEFL_LEVEL1_HASH_SIZE_MASK: u32 = 4095;
const TDEFL_LZ_HASH_SHIFT: u32 = TDEFL_LZ_HASH_BITS.div_ceil(3);
const TDEFL_LZ_HASH_SIZE: u32 = 1 << TDEFL_LZ_HASH_BITS;
const TDEFL_MAX_SUPPORTED_HUFF_CODESIZE: usize = 32;
const DICT_LEN: usize = (TDEFL_LZ_DICT_SIZE + TDEFL_MAX_MATCH_LEN - 1) as usize;

/// `tdefl_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TdeflStatus {
    BadParam = -2,
    PutBufFailed = -1,
    Okay = 0,
    Done = 1,
}

/// `tdefl_flush`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TdeflFlush {
    NoFlush = 0,
    SyncFlush = 2,
    FullFlush = 3,
    Finish = 4,
}

const TDEFL_LEN_SYM: [u16; 256] = [
    257, 258, 259, 260, 261, 262, 263, 264, 265, 265, 266, 266, 267, 267, 268, 268, 269, 269, 269,
    269, 270, 270, 270, 270, 271, 271, 271, 271, 272, 272, 272, 272, 273, 273, 273, 273, 273, 273,
    273, 273, 274, 274, 274, 274, 274, 274, 274, 274, 275, 275, 275, 275, 275, 275, 275, 275, 276,
    276, 276, 276, 276, 276, 276, 276, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277,
    277, 277, 277, 277, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278,
    278, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 280, 280,
    280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 281, 281, 281, 281, 281,
    281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281,
    281, 281, 281, 281, 281, 281, 281, 281, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    282, 282, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283,
    283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 284, 284, 284, 284,
    284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284,
    284, 284, 284, 284, 284, 284, 284, 284, 285,
];

const TDEFL_LEN_EXTRA: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 0,
];

const TDEFL_SMALL_DIST_SYM: [u8; 512] = [
    0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

const TDEFL_SMALL_DIST_EXTRA: [u8; 512] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
];

const TDEFL_LARGE_DIST_SYM: [u8; 128] = [
    0, 0, 18, 19, 20, 20, 21, 21, 22, 22, 22, 22, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24,
    25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
    29, 29, 29, 29, 29, 29, 29, 29,
];

const TDEFL_LARGE_DIST_EXTRA: [u8; 128] = [
    0, 0, 8, 8, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13,
];

const S_TDEFL_NUM_PROBES: [u32; 11] = [0, 1, 6, 32, 16, 32, 128, 256, 512, 768, 1500];
const S_TDEFL_PACKED_CODE_SIZE_SYMS_SWIZZLE: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
const MZ_BITMASKS: [u32; 17] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000F, 0x001F, 0x003F, 0x007F, 0x00FF, 0x01FF, 0x03FF, 0x07FF,
    0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF, 0xFFFF,
];

/// Port of `mz_adler32`.
pub fn mz_adler32(adler: u32, data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (adler & 0xffff, adler >> 16);
    let mut rest = data;
    let mut block_len = data.len() % 5552;
    while !rest.is_empty() {
        let (block, tail) = rest.split_at(block_len.min(rest.len()));
        for &b in block {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
        rest = tail;
        block_len = 5552;
    }
    (s2 << 16).wrapping_add(s1)
}

/// `tdefl_sym_freq`.
#[derive(Debug, Clone, Copy, Default)]
struct SymFreq {
    key: u16,
    sym_index: u16,
}

/// Port of `tdefl_radix_sort_syms`: returns the sorted symbols.
fn radix_sort_syms(syms0: &[SymFreq]) -> Vec<SymFreq> {
    let num_syms = syms0.len();
    let mut hist = [0u32; 256 * 2];
    for s in syms0 {
        let freq = u32::from(s.key);
        hist[(freq & 0xFF) as usize] += 1;
        hist[256 + ((freq >> 8) & 0xFF) as usize] += 1;
    }
    let mut total_passes = 2usize;
    while total_passes > 1 && num_syms as u32 == hist[(total_passes - 1) * 256] {
        total_passes -= 1;
    }
    let mut cur = syms0.to_vec();
    let mut new = vec![SymFreq::default(); num_syms];
    for pass in 0..total_passes {
        let pass_shift = pass * 8;
        let hist = &hist[pass << 8..(pass << 8) + 256];
        let mut offsets = [0u32; 256];
        let mut cur_ofs = 0u32;
        for (o, h) in offsets.iter_mut().zip(hist) {
            *o = cur_ofs;
            cur_ofs += h;
        }
        for s in &cur {
            let o = &mut offsets[((u32::from(s.key) >> pass_shift) & 0xFF) as usize];
            new[*o as usize] = *s;
            *o += 1;
        }
        std::mem::swap(&mut cur, &mut new);
    }
    cur
}

/// Port of `tdefl_calculate_minimum_redundancy` (Moffat & Katajainen).
fn calculate_minimum_redundancy(a: &mut [SymFreq]) {
    let n = a.len() as i32;
    let k = |i: i32| i as usize;
    if n == 0 {
        return;
    } else if n == 1 {
        a[0].key = 1;
        return;
    }
    a[0].key = a[0].key.wrapping_add(a[1].key);
    let mut root = 0i32;
    let mut leaf = 2i32;
    for next in 1..n - 1 {
        if leaf >= n || a[k(root)].key < a[k(leaf)].key {
            a[k(next)].key = a[k(root)].key;
            a[k(root)].key = next as u16;
            root += 1;
        } else {
            a[k(next)].key = a[k(leaf)].key;
            leaf += 1;
        }
        if leaf >= n || (root < next && a[k(root)].key < a[k(leaf)].key) {
            a[k(next)].key = a[k(next)].key.wrapping_add(a[k(root)].key);
            a[k(root)].key = next as u16;
            root += 1;
        } else {
            a[k(next)].key = a[k(next)].key.wrapping_add(a[k(leaf)].key);
            leaf += 1;
        }
    }
    a[k(n - 2)].key = 0;
    let mut next = n - 3;
    while next >= 0 {
        a[k(next)].key = a[a[k(next)].key as usize].key + 1;
        next -= 1;
    }
    let mut avbl = 1i32;
    let mut used = 0i32;
    let mut dpth = 0i32;
    let mut root = n - 2;
    let mut next = n - 1;
    while avbl > 0 {
        while root >= 0 && i32::from(a[k(root)].key) == dpth {
            used += 1;
            root -= 1;
        }
        while avbl > used {
            a[k(next)].key = dpth as u16;
            next -= 1;
            avbl -= 1;
        }
        avbl = 2 * used;
        dpth += 1;
        used = 0;
    }
}

/// Port of `tdefl_huffman_enforce_max_code_size`.
fn huffman_enforce_max_code_size(
    num_codes: &mut [i32],
    code_list_len: usize,
    max_code_size: usize,
) {
    if code_list_len <= 1 {
        return;
    }
    for i in max_code_size + 1..=TDEFL_MAX_SUPPORTED_HUFF_CODESIZE {
        num_codes[max_code_size] += num_codes[i];
    }
    let mut total = 0u32;
    for i in (1..=max_code_size).rev() {
        total = total.wrapping_add((num_codes[i] as u32) << (max_code_size - i));
    }
    while total != 1u32 << max_code_size {
        num_codes[max_code_size] -= 1;
        for i in (1..max_code_size).rev() {
            if num_codes[i] != 0 {
                num_codes[i] -= 1;
                num_codes[i + 1] += 2;
                break;
            }
        }
        total = total.wrapping_sub(1);
    }
}

/// Port of `tdefl_compressor` (output collected in `output`).
pub struct TdeflCompressor {
    flags: u32,
    max_probes: [u32; 2],
    greedy_parsing: bool,
    adler32: u32,
    lookahead_pos: u32,
    lookahead_size: u32,
    dict_size: u32,
    /// `m_pLZ_code_buf` / `m_pLZ_flags` as offsets into `lz_code_buf`.
    lz_code_pos: usize,
    lz_flags_pos: usize,
    num_flags_left: u32,
    total_lz_bytes: u32,
    lz_code_buf_dict_pos: u32,
    bits_in: u32,
    bit_buffer: u32,
    saved_match_dist: u32,
    saved_match_len: u32,
    saved_lit: u32,
    finished: bool,
    block_index: u32,
    wants_to_finish: bool,
    prev_return_status: TdeflStatus,
    flush: TdeflFlush,
    src_pos: usize,
    src_buf_left: usize,
    dict: Vec<u8>,
    huff_count: [[u16; TDEFL_MAX_HUFF_SYMBOLS]; 3],
    huff_codes: [[u16; TDEFL_MAX_HUFF_SYMBOLS]; 3],
    huff_code_sizes: [[u8; TDEFL_MAX_HUFF_SYMBOLS]; 3],
    lz_code_buf: Vec<u8>,
    next: Vec<u16>,
    hash: Vec<u16>,
    /// `m_output_buf` with `m_pOutput_buf` / `m_pOutput_buf_end` offsets.
    output_buf: Vec<u8>,
    output_pos: usize,
    output_end: usize,
    /// Everything flushed so far (the caller's output buffer).
    output: Vec<u8>,
}

impl TdeflCompressor {
    /// Port of `tdefl_init` (no put-buffer callback).
    pub fn new(flags: u32) -> TdeflCompressor {
        TdeflCompressor {
            flags,
            max_probes: [
                1 + (flags & 0xFFF).div_ceil(3),
                1 + ((flags & 0xFFF) >> 2).div_ceil(3),
            ],
            greedy_parsing: (flags & TDEFL_GREEDY_PARSING_FLAG) != 0,
            adler32: 1,
            lookahead_pos: 0,
            lookahead_size: 0,
            dict_size: 0,
            lz_code_pos: 1,
            lz_flags_pos: 0,
            num_flags_left: 8,
            total_lz_bytes: 0,
            lz_code_buf_dict_pos: 0,
            bits_in: 0,
            bit_buffer: 0,
            saved_match_dist: 0,
            saved_match_len: 0,
            saved_lit: 0,
            finished: false,
            block_index: 0,
            wants_to_finish: false,
            prev_return_status: TdeflStatus::Okay,
            flush: TdeflFlush::NoFlush,
            src_pos: 0,
            src_buf_left: 0,
            // miniz leaves the dictionary and chains uninitialised (malloc);
            // only bytes it has written can influence the output.
            dict: vec![0; DICT_LEN],
            huff_count: [[0; TDEFL_MAX_HUFF_SYMBOLS]; 3],
            huff_codes: [[0; TDEFL_MAX_HUFF_SYMBOLS]; 3],
            huff_code_sizes: [[0; TDEFL_MAX_HUFF_SYMBOLS]; 3],
            lz_code_buf: vec![0; TDEFL_LZ_CODE_BUF_SIZE],
            next: vec![0; TDEFL_LZ_DICT_SIZE as usize],
            hash: vec![0; TDEFL_LZ_HASH_SIZE as usize],
            output_buf: vec![0; TDEFL_OUT_BUF_SIZE],
            output_pos: 0,
            output_end: 0,
            output: Vec::new(),
        }
    }

    /// `tdefl_get_adler32`.
    pub fn adler32(&self) -> u32 {
        self.adler32
    }

    /// `tdefl_get_prev_return_status`.
    pub fn prev_return_status(&self) -> TdeflStatus {
        self.prev_return_status
    }

    /// The compressed bytes produced so far.
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Takes the compressed bytes.
    pub fn into_output(self) -> Vec<u8> {
        self.output
    }

    /// `TDEFL_PUT_BITS`: bytes past the block buffer end are dropped.
    fn put_bits(&mut self, bits: u32, len: u32) {
        self.bit_buffer |= bits << self.bits_in;
        self.bits_in += len;
        while self.bits_in >= 8 {
            if self.output_pos < self.output_end {
                self.output_buf[self.output_pos] = self.bit_buffer as u8;
                self.output_pos += 1;
            }
            self.bit_buffer >>= 8;
            self.bits_in -= 8;
        }
    }

    /// Port of `tdefl_optimize_huffman_table`.
    fn optimize_huffman_table(
        &mut self,
        table_num: usize,
        table_len: usize,
        code_size_limit: usize,
        static_table: bool,
    ) {
        let mut num_codes = [0i32; 1 + TDEFL_MAX_SUPPORTED_HUFF_CODESIZE];
        if static_table {
            for i in 0..table_len {
                num_codes[self.huff_code_sizes[table_num][i] as usize] += 1;
            }
        } else {
            let syms0: Vec<SymFreq> = self.huff_count[table_num][..table_len]
                .iter()
                .enumerate()
                .filter(|(_, c)| **c != 0)
                .map(|(i, c)| SymFreq {
                    key: *c,
                    sym_index: i as u16,
                })
                .collect();
            let num_used_syms = syms0.len();
            let mut syms = radix_sort_syms(&syms0);
            calculate_minimum_redundancy(&mut syms);
            for s in &syms {
                num_codes[s.key as usize] += 1;
            }
            huffman_enforce_max_code_size(&mut num_codes, num_used_syms, code_size_limit);

            self.huff_code_sizes[table_num] = [0; TDEFL_MAX_HUFF_SYMBOLS];
            self.huff_codes[table_num] = [0; TDEFL_MAX_HUFF_SYMBOLS];
            let mut j = num_used_syms;
            for (i, &n) in num_codes
                .iter()
                .enumerate()
                .take(code_size_limit + 1)
                .skip(1)
            {
                for _ in 0..n.max(0) {
                    j -= 1;
                    self.huff_code_sizes[table_num][syms[j].sym_index as usize] = i as u8;
                }
            }
        }

        let mut next_code = [0u32; TDEFL_MAX_SUPPORTED_HUFF_CODESIZE + 1];
        let mut j = 0i32;
        for i in 2..=code_size_limit {
            j = (j + num_codes[i - 1]) << 1;
            next_code[i] = j as u32;
        }
        for i in 0..table_len {
            let code_size = u32::from(self.huff_code_sizes[table_num][i]);
            if code_size == 0 {
                continue;
            }
            let mut code = next_code[code_size as usize];
            next_code[code_size as usize] += 1;
            let mut rev_code = 0u32;
            for _ in 0..code_size {
                rev_code = (rev_code << 1) | (code & 1);
                code >>= 1;
            }
            self.huff_codes[table_num][i] = rev_code as u16;
        }
    }

    /// Port of `tdefl_start_dynamic_block`.
    fn start_dynamic_block(&mut self) {
        let mut code_sizes_to_pack = [0u8; TDEFL_MAX_HUFF_SYMBOLS_0 + TDEFL_MAX_HUFF_SYMBOLS_1];
        let mut packed_code_sizes = [0u8; TDEFL_MAX_HUFF_SYMBOLS_0 + TDEFL_MAX_HUFF_SYMBOLS_1];
        let mut prev_code_size = 0xFFu8;

        self.huff_count[0][256] = 1;
        self.optimize_huffman_table(0, TDEFL_MAX_HUFF_SYMBOLS_0, 15, false);
        self.optimize_huffman_table(1, TDEFL_MAX_HUFF_SYMBOLS_1, 15, false);

        let mut num_lit_codes = 286usize;
        while num_lit_codes > 257 && self.huff_code_sizes[0][num_lit_codes - 1] == 0 {
            num_lit_codes -= 1;
        }
        let mut num_dist_codes = 30usize;
        while num_dist_codes > 1 && self.huff_code_sizes[1][num_dist_codes - 1] == 0 {
            num_dist_codes -= 1;
        }

        code_sizes_to_pack[..num_lit_codes]
            .copy_from_slice(&self.huff_code_sizes[0][..num_lit_codes]);
        code_sizes_to_pack[num_lit_codes..num_lit_codes + num_dist_codes]
            .copy_from_slice(&self.huff_code_sizes[1][..num_dist_codes]);
        let total_code_sizes_to_pack = num_lit_codes + num_dist_codes;
        let mut num_packed = 0usize;
        let mut rle_z_count = 0u32;
        let mut rle_repeat_count = 0u32;

        self.huff_count[2][..TDEFL_MAX_HUFF_SYMBOLS_2].fill(0);

        let counts = &mut self.huff_count[2];
        let rle_prev_code_size =
            |rle_repeat_count: &mut u32,
             num_packed: &mut usize,
             packed: &mut [u8],
             prev: u8,
             counts: &mut [u16; TDEFL_MAX_HUFF_SYMBOLS]| {
                if *rle_repeat_count != 0 {
                    if *rle_repeat_count < 3 {
                        counts[prev as usize] =
                            counts[prev as usize].wrapping_add(*rle_repeat_count as u16);
                        while *rle_repeat_count != 0 {
                            packed[*num_packed] = prev;
                            *num_packed += 1;
                            *rle_repeat_count -= 1;
                        }
                    } else {
                        counts[16] = counts[16].wrapping_add(1);
                        packed[*num_packed] = 16;
                        packed[*num_packed + 1] = (*rle_repeat_count - 3) as u8;
                        *num_packed += 2;
                    }
                    *rle_repeat_count = 0;
                }
            };
        let rle_zero_code_size =
            |rle_z_count: &mut u32,
             num_packed: &mut usize,
             packed: &mut [u8],
             counts: &mut [u16; TDEFL_MAX_HUFF_SYMBOLS]| {
                if *rle_z_count != 0 {
                    if *rle_z_count < 3 {
                        counts[0] = counts[0].wrapping_add(*rle_z_count as u16);
                        while *rle_z_count != 0 {
                            packed[*num_packed] = 0;
                            *num_packed += 1;
                            *rle_z_count -= 1;
                        }
                    } else if *rle_z_count <= 10 {
                        counts[17] = counts[17].wrapping_add(1);
                        packed[*num_packed] = 17;
                        packed[*num_packed + 1] = (*rle_z_count - 3) as u8;
                        *num_packed += 2;
                    } else {
                        counts[18] = counts[18].wrapping_add(1);
                        packed[*num_packed] = 18;
                        packed[*num_packed + 1] = (*rle_z_count - 11) as u8;
                        *num_packed += 2;
                    }
                    *rle_z_count = 0;
                }
            };

        for &code_size in &code_sizes_to_pack[..total_code_sizes_to_pack] {
            if code_size == 0 {
                rle_prev_code_size(
                    &mut rle_repeat_count,
                    &mut num_packed,
                    &mut packed_code_sizes,
                    prev_code_size,
                    counts,
                );
                rle_z_count += 1;
                if rle_z_count == 138 {
                    rle_zero_code_size(
                        &mut rle_z_count,
                        &mut num_packed,
                        &mut packed_code_sizes,
                        counts,
                    );
                }
            } else {
                rle_zero_code_size(
                    &mut rle_z_count,
                    &mut num_packed,
                    &mut packed_code_sizes,
                    counts,
                );
                if code_size != prev_code_size {
                    rle_prev_code_size(
                        &mut rle_repeat_count,
                        &mut num_packed,
                        &mut packed_code_sizes,
                        prev_code_size,
                        counts,
                    );
                    counts[code_size as usize] = counts[code_size as usize].wrapping_add(1);
                    packed_code_sizes[num_packed] = code_size;
                    num_packed += 1;
                } else {
                    rle_repeat_count += 1;
                    if rle_repeat_count == 6 {
                        rle_prev_code_size(
                            &mut rle_repeat_count,
                            &mut num_packed,
                            &mut packed_code_sizes,
                            prev_code_size,
                            counts,
                        );
                    }
                }
            }
            prev_code_size = code_size;
        }
        if rle_repeat_count != 0 {
            rle_prev_code_size(
                &mut rle_repeat_count,
                &mut num_packed,
                &mut packed_code_sizes,
                prev_code_size,
                counts,
            );
        } else {
            rle_zero_code_size(
                &mut rle_z_count,
                &mut num_packed,
                &mut packed_code_sizes,
                counts,
            );
        }

        self.optimize_huffman_table(2, TDEFL_MAX_HUFF_SYMBOLS_2, 7, false);

        self.put_bits(2, 2);
        self.put_bits(num_lit_codes as u32 - 257, 5);
        self.put_bits(num_dist_codes as u32 - 1, 5);

        let mut num_bit_lengths = 18i32;
        while num_bit_lengths >= 0
            && self.huff_code_sizes[2]
                [S_TDEFL_PACKED_CODE_SIZE_SYMS_SWIZZLE[num_bit_lengths as usize] as usize]
                == 0
        {
            num_bit_lengths -= 1;
        }
        let num_bit_lengths = 4.max(num_bit_lengths + 1) as usize;
        self.put_bits(num_bit_lengths as u32 - 4, 4);
        for &s in &S_TDEFL_PACKED_CODE_SIZE_SYMS_SWIZZLE[..num_bit_lengths] {
            self.put_bits(u32::from(self.huff_code_sizes[2][s as usize]), 3);
        }

        let mut i = 0usize;
        while i < num_packed {
            let code = packed_code_sizes[i] as usize;
            i += 1;
            self.put_bits(
                u32::from(self.huff_codes[2][code]),
                u32::from(self.huff_code_sizes[2][code]),
            );
            if code >= 16 {
                self.put_bits(u32::from(packed_code_sizes[i]), [2u32, 3, 7][code - 16]);
                i += 1;
            }
        }
    }

    /// Port of `tdefl_start_static_block`.
    fn start_static_block(&mut self) {
        let sizes = &mut self.huff_code_sizes[0];
        sizes[..=143].fill(8);
        sizes[144..=255].fill(9);
        sizes[256..=279].fill(7);
        sizes[280..=287].fill(8);
        self.huff_code_sizes[1][..32].fill(5);
        self.optimize_huffman_table(0, 288, 15, true);
        self.optimize_huffman_table(1, 32, 15, true);
        self.put_bits(1, 2);
    }

    /// Port of `tdefl_compress_lz_codes` (the portable variant emits the same
    /// bit stream as the 64-bit one).
    fn compress_lz_codes(&mut self) -> bool {
        let mut flags = 1u32;
        let mut p = 0usize;
        let end = self.lz_code_pos;
        while p < end {
            if flags == 1 {
                flags = u32::from(self.lz_code_buf[p]) | 0x100;
                p += 1;
            }
            if flags & 1 != 0 {
                let match_len = self.lz_code_buf[p] as usize;
                let match_dist = usize::from(self.lz_code_buf[p + 1])
                    | (usize::from(self.lz_code_buf[p + 2]) << 8);
                p += 3;

                let len_sym = TDEFL_LEN_SYM[match_len] as usize;
                self.put_bits(
                    u32::from(self.huff_codes[0][len_sym]),
                    u32::from(self.huff_code_sizes[0][len_sym]),
                );
                let extra = TDEFL_LEN_EXTRA[match_len] as usize;
                self.put_bits(match_len as u32 & MZ_BITMASKS[extra], extra as u32);

                let (sym, num_extra_bits) = if match_dist < 512 {
                    (
                        TDEFL_SMALL_DIST_SYM[match_dist] as usize,
                        TDEFL_SMALL_DIST_EXTRA[match_dist] as usize,
                    )
                } else {
                    (
                        TDEFL_LARGE_DIST_SYM[match_dist >> 8] as usize,
                        TDEFL_LARGE_DIST_EXTRA[match_dist >> 8] as usize,
                    )
                };
                self.put_bits(
                    u32::from(self.huff_codes[1][sym]),
                    u32::from(self.huff_code_sizes[1][sym]),
                );
                self.put_bits(
                    match_dist as u32 & MZ_BITMASKS[num_extra_bits],
                    num_extra_bits as u32,
                );
            } else {
                let lit = self.lz_code_buf[p] as usize;
                p += 1;
                self.put_bits(
                    u32::from(self.huff_codes[0][lit]),
                    u32::from(self.huff_code_sizes[0][lit]),
                );
            }
            flags >>= 1;
        }
        self.put_bits(
            u32::from(self.huff_codes[0][256]),
            u32::from(self.huff_code_sizes[0][256]),
        );
        self.output_pos < self.output_end
    }

    /// Port of `tdefl_compress_block`.
    fn compress_block(&mut self, static_block: bool) -> bool {
        if static_block {
            self.start_static_block();
        } else {
            self.start_dynamic_block();
        }
        self.compress_lz_codes()
    }

    /// Port of `tdefl_flush_block`. Always returns 0 (no output pending)
    /// because the output buffer grows.
    fn flush_block(&mut self, flush: TdeflFlush) -> i32 {
        let in_dict = self.lookahead_pos.wrapping_sub(self.lz_code_buf_dict_pos) <= self.dict_size;
        let use_raw_block = (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) != 0 && in_dict;

        self.output_pos = 0;
        self.output_end = TDEFL_OUT_BUF_SIZE - 16;

        let shift = self.num_flags_left;
        let f = &mut self.lz_code_buf[self.lz_flags_pos];
        *f = (u32::from(*f) >> shift) as u8;
        if self.num_flags_left == 8 {
            self.lz_code_pos -= 1;
        }

        if (self.flags & TDEFL_WRITE_ZLIB_HEADER) != 0 && self.block_index == 0 {
            self.put_bits(0x78, 8);
            self.put_bits(0x01, 8);
        }

        self.put_bits(u32::from(flush == TdeflFlush::Finish), 1);

        let saved = (self.output_pos, self.bit_buffer, self.bits_in);
        let mut comp_block_succeeded = false;
        if !use_raw_block {
            comp_block_succeeded = self.compress_block(
                (self.flags & TDEFL_FORCE_ALL_STATIC_BLOCKS) != 0 || self.total_lz_bytes < 48,
            );
        }

        // If the block gets expanded, forget the current contents of the output buffer and send a raw block instead.
        let expanded = self.total_lz_bytes != 0
            && (self.output_pos - saved.0 + 1) as u64 >= u64::from(self.total_lz_bytes);
        if (use_raw_block || expanded) && in_dict {
            (self.output_pos, self.bit_buffer, self.bits_in) = saved;
            self.put_bits(0, 2);
            if self.bits_in != 0 {
                self.put_bits(0, 8 - self.bits_in);
            }
            for _ in 0..2 {
                self.put_bits(self.total_lz_bytes & 0xFFFF, 16);
                self.total_lz_bytes ^= 0xFFFF;
            }
            for i in 0..self.total_lz_bytes {
                let b = self.dict[(self.lz_code_buf_dict_pos.wrapping_add(i)
                    & TDEFL_LZ_DICT_SIZE_MASK) as usize];
                self.put_bits(u32::from(b), 8);
            }
        } else if !comp_block_succeeded {
            // Check for the extremely unlikely (if not impossible) case of the compressed block not fitting into the output buffer when using dynamic codes.
            (self.output_pos, self.bit_buffer, self.bits_in) = saved;
            self.compress_block(true);
        }

        if flush != TdeflFlush::NoFlush {
            if flush == TdeflFlush::Finish {
                if self.bits_in != 0 {
                    self.put_bits(0, 8 - self.bits_in);
                }
                if (self.flags & TDEFL_WRITE_ZLIB_HEADER) != 0 {
                    let mut a = self.adler32;
                    for _ in 0..4 {
                        self.put_bits((a >> 24) & 0xFF, 8);
                        a <<= 8;
                    }
                }
            } else {
                self.put_bits(0, 3);
                if self.bits_in != 0 {
                    self.put_bits(0, 8 - self.bits_in);
                }
                let mut z = 0u32;
                for _ in 0..2 {
                    self.put_bits(z & 0xFFFF, 16);
                    z ^= 0xFFFF;
                }
            }
        }

        self.huff_count[0][..TDEFL_MAX_HUFF_SYMBOLS_0].fill(0);
        self.huff_count[1][..TDEFL_MAX_HUFF_SYMBOLS_1].fill(0);

        self.lz_code_pos = 1;
        self.lz_flags_pos = 0;
        self.num_flags_left = 8;
        self.lz_code_buf_dict_pos = self.lz_code_buf_dict_pos.wrapping_add(self.total_lz_bytes);
        self.total_lz_bytes = 0;
        self.block_index += 1;

        let n = self.output_pos;
        self.output.extend_from_slice(&self.output_buf[..n]);
        0
    }

    #[inline]
    fn dict_u16(&self, i: u32) -> u16 {
        let i = i as usize;
        u16::from_le_bytes([self.dict[i], self.dict[i + 1]])
    }

    #[inline]
    fn dict_u32(&self, i: u32) -> u32 {
        let i = i as usize;
        u32::from_le_bytes([
            self.dict[i],
            self.dict[i + 1],
            self.dict[i + 2],
            self.dict[i + 3],
        ])
    }

    /// The unrolled 16-bit word compare of `tdefl_find_match` /
    /// `tdefl_compress_fast`: starting after the first word, compares up to
    /// 128 more words. Returns `(words advanced, probe_len left)`.
    fn compare_words(&self, s: u32, q: u32) -> (u32, u32) {
        let mut pi = 0u32;
        let mut probe_len = 32u32;
        loop {
            let mut all = true;
            for _ in 0..4 {
                pi += 1;
                if self.dict_u16(s + 2 * pi) != self.dict_u16(q + 2 * pi) {
                    all = false;
                    break;
                }
            }
            if !all {
                return (pi, probe_len);
            }
            probe_len -= 1;
            if probe_len == 0 {
                return (pi, 0);
            }
        }
    }

    /// Port of `tdefl_find_match` (unaligned-loads variant).
    fn find_match(
        &self,
        lookahead_pos: u32,
        max_dist: u32,
        max_match_len: u32,
        match_dist: &mut u32,
        match_len_io: &mut u32,
    ) {
        let pos = lookahead_pos & TDEFL_LZ_DICT_SIZE_MASK;
        let mut match_len = *match_len_io;
        let mut probe_pos = pos;
        let mut num_probes_left = self.max_probes[usize::from(match_len >= 32)];
        let mut c01 = self.dict_u16(pos + match_len - 1);
        let s01 = self.dict_u16(pos);
        if max_match_len <= match_len {
            return;
        }
        let mut dist;
        loop {
            'probe: loop {
                num_probes_left = num_probes_left.wrapping_sub(1);
                if num_probes_left == 0 {
                    return;
                }
                for _ in 0..3 {
                    let next_probe_pos = u32::from(self.next[probe_pos as usize]);
                    if next_probe_pos == 0 {
                        return;
                    }
                    dist = u32::from(lookahead_pos.wrapping_sub(next_probe_pos) as u16);
                    if dist > max_dist {
                        return;
                    }
                    probe_pos = next_probe_pos & TDEFL_LZ_DICT_SIZE_MASK;
                    if self.dict_u16(probe_pos + match_len - 1) == c01 {
                        break 'probe;
                    }
                }
            }
            if dist == 0 {
                break;
            }
            if self.dict_u16(probe_pos) != s01 {
                continue;
            }
            let (pi, probe_len) = self.compare_words(pos, probe_pos);
            if probe_len == 0 {
                *match_dist = dist;
                *match_len_io = max_match_len.min(TDEFL_MAX_MATCH_LEN);
                break;
            }
            let probe_len = pi * 2
                + u32::from(
                    self.dict[(pos + 2 * pi) as usize] == self.dict[(probe_pos + 2 * pi) as usize],
                );
            if probe_len > match_len {
                *match_dist = dist;
                match_len = max_match_len.min(probe_len);
                *match_len_io = match_len;
                if match_len == max_match_len {
                    break;
                }
                c01 = self.dict_u16(pos + match_len - 1);
            }
        }
    }

    /// Records one literal into the LZ code buffer (shared by
    /// `tdefl_record_literal` and `tdefl_compress_fast`).
    #[inline]
    fn push_literal_code(&mut self, lit: u8) {
        self.lz_code_buf[self.lz_code_pos] = lit;
        self.lz_code_pos += 1;
        let f = &mut self.lz_code_buf[self.lz_flags_pos];
        *f >>= 1;
        self.num_flags_left -= 1;
        if self.num_flags_left == 0 {
            self.num_flags_left = 8;
            self.lz_flags_pos = self.lz_code_pos;
            self.lz_code_pos += 1;
        }
        self.huff_count[0][lit as usize] = self.huff_count[0][lit as usize].wrapping_add(1);
    }

    /// Port of `tdefl_record_literal`.
    fn record_literal(&mut self, lit: u8) {
        self.total_lz_bytes = self.total_lz_bytes.wrapping_add(1);
        self.push_literal_code(lit);
    }

    /// Port of `tdefl_record_match`.
    fn record_match(&mut self, match_len: u32, match_dist: u32) {
        self.total_lz_bytes = self.total_lz_bytes.wrapping_add(match_len);
        let p = self.lz_code_pos;
        self.lz_code_buf[p] = (match_len - TDEFL_MIN_MATCH_LEN) as u8;
        let match_dist = match_dist - 1;
        self.lz_code_buf[p + 1] = (match_dist & 0xFF) as u8;
        self.lz_code_buf[p + 2] = (match_dist >> 8) as u8;
        self.lz_code_pos += 3;

        let f = &mut self.lz_code_buf[self.lz_flags_pos];
        *f = (*f >> 1) | 0x80;
        self.num_flags_left -= 1;
        if self.num_flags_left == 0 {
            self.num_flags_left = 8;
            self.lz_flags_pos = self.lz_code_pos;
            self.lz_code_pos += 1;
        }

        let s0 = TDEFL_SMALL_DIST_SYM[(match_dist & 511) as usize];
        let s1 = TDEFL_LARGE_DIST_SYM[((match_dist >> 8) & 127) as usize];
        let sym = usize::from(if match_dist < 512 { s0 } else { s1 });
        self.huff_count[1][sym] = self.huff_count[1][sym].wrapping_add(1);
        if match_len >= TDEFL_MIN_MATCH_LEN {
            let ls = TDEFL_LEN_SYM[(match_len - TDEFL_MIN_MATCH_LEN) as usize] as usize;
            self.huff_count[0][ls] = self.huff_count[0][ls].wrapping_add(1);
        }
    }

    /// Port of `tdefl_compress_fast` (level 1 on x86-64).
    fn compress_fast(&mut self, input: &[u8]) -> bool {
        let mut lookahead_pos = self.lookahead_pos;
        let mut lookahead_size = self.lookahead_size;
        let mut dict_size = self.dict_size;
        let mut cur_pos = lookahead_pos & TDEFL_LZ_DICT_SIZE_MASK;

        while self.src_buf_left != 0 || (self.flush != TdeflFlush::NoFlush && lookahead_size != 0) {
            const TDEFL_COMP_FAST_LOOKAHEAD_SIZE: u32 = 4096;
            let mut dst_pos = lookahead_pos.wrapping_add(lookahead_size) & TDEFL_LZ_DICT_SIZE_MASK;
            let mut num_bytes_to_process = self
                .src_buf_left
                .min((TDEFL_COMP_FAST_LOOKAHEAD_SIZE - lookahead_size) as usize)
                as u32;
            self.src_buf_left -= num_bytes_to_process as usize;
            lookahead_size += num_bytes_to_process;

            while num_bytes_to_process != 0 {
                let n = (TDEFL_LZ_DICT_SIZE - dst_pos).min(num_bytes_to_process);
                let src = &input[self.src_pos..self.src_pos + n as usize];
                let d = dst_pos as usize;
                self.dict[d..d + n as usize].copy_from_slice(src);
                if dst_pos < TDEFL_MAX_MATCH_LEN - 1 {
                    let m = n.min((TDEFL_MAX_MATCH_LEN - 1) - dst_pos) as usize;
                    let base = TDEFL_LZ_DICT_SIZE as usize + d;
                    self.dict[base..base + m].copy_from_slice(&src[..m]);
                }
                self.src_pos += n as usize;
                dst_pos = (dst_pos + n) & TDEFL_LZ_DICT_SIZE_MASK;
                num_bytes_to_process -= n;
            }

            dict_size = (TDEFL_LZ_DICT_SIZE - lookahead_size).min(dict_size);
            if self.flush == TdeflFlush::NoFlush && lookahead_size < TDEFL_COMP_FAST_LOOKAHEAD_SIZE
            {
                break;
            }

            while lookahead_size >= 4 {
                let mut cur_match_len = 1u32;
                let first_trigram = self.dict_u32(cur_pos) & 0xFFFFFF;
                let hash = ((first_trigram ^ (first_trigram >> (24 - (TDEFL_LZ_HASH_BITS - 8))))
                    & TDEFL_LEVEL1_HASH_SIZE_MASK) as usize;
                let mut probe_pos = u32::from(self.hash[hash]);
                self.hash[hash] = lookahead_pos as u16;

                let mut cur_match_dist = u32::from(lookahead_pos.wrapping_sub(probe_pos) as u16);
                let matched = cur_match_dist <= dict_size && {
                    probe_pos &= TDEFL_LZ_DICT_SIZE_MASK;
                    (self.dict_u32(probe_pos) & 0xFFFFFF) == first_trigram
                };
                if matched {
                    let (pi, probe_len) = self.compare_words(cur_pos, probe_pos);
                    cur_match_len = pi * 2
                        + u32::from(
                            self.dict[(cur_pos + 2 * pi) as usize]
                                == self.dict[(probe_pos + 2 * pi) as usize],
                        );
                    if probe_len == 0 {
                        cur_match_len = if cur_match_dist != 0 {
                            TDEFL_MAX_MATCH_LEN
                        } else {
                            0
                        };
                    }

                    if cur_match_len < TDEFL_MIN_MATCH_LEN
                        || (cur_match_len == TDEFL_MIN_MATCH_LEN && cur_match_dist >= 8 * 1024)
                    {
                        cur_match_len = 1;
                        self.push_literal_code(first_trigram as u8);
                    } else {
                        cur_match_len = cur_match_len.min(lookahead_size);
                        cur_match_dist -= 1;
                        let p = self.lz_code_pos;
                        self.lz_code_buf[p] = (cur_match_len - TDEFL_MIN_MATCH_LEN) as u8;
                        self.lz_code_buf[p + 1..p + 3]
                            .copy_from_slice(&(cur_match_dist as u16).to_le_bytes());
                        self.lz_code_pos += 3;
                        let f = &mut self.lz_code_buf[self.lz_flags_pos];
                        *f = (*f >> 1) | 0x80;
                        self.num_flags_left -= 1;
                        if self.num_flags_left == 0 {
                            self.num_flags_left = 8;
                            self.lz_flags_pos = self.lz_code_pos;
                            self.lz_code_pos += 1;
                        }
                        let s0 = TDEFL_SMALL_DIST_SYM[(cur_match_dist & 511) as usize];
                        let s1 = TDEFL_LARGE_DIST_SYM[(cur_match_dist >> 8) as usize & 127];
                        let sym = usize::from(if cur_match_dist < 512 { s0 } else { s1 });
                        self.huff_count[1][sym] = self.huff_count[1][sym].wrapping_add(1);
                        let ls =
                            TDEFL_LEN_SYM[(cur_match_len - TDEFL_MIN_MATCH_LEN) as usize] as usize;
                        self.huff_count[0][ls] = self.huff_count[0][ls].wrapping_add(1);
                    }
                } else {
                    self.push_literal_code(first_trigram as u8);
                }

                self.total_lz_bytes = self.total_lz_bytes.wrapping_add(cur_match_len);
                lookahead_pos = lookahead_pos.wrapping_add(cur_match_len);
                dict_size = (dict_size + cur_match_len).min(TDEFL_LZ_DICT_SIZE);
                cur_pos = (cur_pos + cur_match_len) & TDEFL_LZ_DICT_SIZE_MASK;
                lookahead_size -= cur_match_len;

                if self.lz_code_pos > TDEFL_LZ_CODE_BUF_SIZE - 8 {
                    self.lookahead_pos = lookahead_pos;
                    self.lookahead_size = lookahead_size;
                    self.dict_size = dict_size;
                    let n = self.flush_block(TdeflFlush::NoFlush);
                    if n != 0 {
                        return n >= 0;
                    }
                }
            }

            while lookahead_size != 0 {
                let lit = self.dict[cur_pos as usize];
                self.total_lz_bytes = self.total_lz_bytes.wrapping_add(1);
                self.push_literal_code(lit);
                lookahead_pos = lookahead_pos.wrapping_add(1);
                dict_size = (dict_size + 1).min(TDEFL_LZ_DICT_SIZE);
                cur_pos = (cur_pos + 1) & TDEFL_LZ_DICT_SIZE_MASK;
                lookahead_size -= 1;

                if self.lz_code_pos > TDEFL_LZ_CODE_BUF_SIZE - 8 {
                    self.lookahead_pos = lookahead_pos;
                    self.lookahead_size = lookahead_size;
                    self.dict_size = dict_size;
                    let n = self.flush_block(TdeflFlush::NoFlush);
                    if n != 0 {
                        return n >= 0;
                    }
                }
            }
        }

        self.lookahead_pos = lookahead_pos;
        self.lookahead_size = lookahead_size;
        self.dict_size = dict_size;
        true
    }

    /// Port of `tdefl_compress_normal`.
    fn compress_normal(&mut self, input: &[u8]) -> bool {
        let mut src_pos = self.src_pos;
        let mut src_buf_left = self.src_buf_left;
        let flush = self.flush;
        let mask = TDEFL_LZ_DICT_SIZE_MASK;

        while src_buf_left != 0 || (flush != TdeflFlush::NoFlush && self.lookahead_size != 0) {
            // Update dictionary and hash chains. Keeps the lookahead size equal to TDEFL_MAX_MATCH_LEN.
            if self.lookahead_size + self.dict_size >= TDEFL_MIN_MATCH_LEN - 1 {
                let mut dst_pos = self.lookahead_pos.wrapping_add(self.lookahead_size) & mask;
                let mut ins_pos = self
                    .lookahead_pos
                    .wrapping_add(self.lookahead_size)
                    .wrapping_sub(2);
                let mut hash = (u32::from(self.dict[(ins_pos & mask) as usize])
                    << TDEFL_LZ_HASH_SHIFT)
                    ^ u32::from(self.dict[(ins_pos.wrapping_add(1) & mask) as usize]);
                let num_bytes_to_process =
                    src_buf_left.min((TDEFL_MAX_MATCH_LEN - self.lookahead_size) as usize);
                src_buf_left -= num_bytes_to_process;
                self.lookahead_size += num_bytes_to_process as u32;
                for &c in &input[src_pos..src_pos + num_bytes_to_process] {
                    self.dict[dst_pos as usize] = c;
                    if dst_pos < TDEFL_MAX_MATCH_LEN - 1 {
                        self.dict[(TDEFL_LZ_DICT_SIZE + dst_pos) as usize] = c;
                    }
                    hash =
                        ((hash << TDEFL_LZ_HASH_SHIFT) ^ u32::from(c)) & (TDEFL_LZ_HASH_SIZE - 1);
                    self.next[(ins_pos & mask) as usize] = self.hash[hash as usize];
                    self.hash[hash as usize] = ins_pos as u16;
                    dst_pos = (dst_pos + 1) & mask;
                    ins_pos = ins_pos.wrapping_add(1);
                }
                src_pos += num_bytes_to_process;
            } else {
                while src_buf_left != 0 && self.lookahead_size < TDEFL_MAX_MATCH_LEN {
                    let c = input[src_pos];
                    src_pos += 1;
                    let dst_pos = self.lookahead_pos.wrapping_add(self.lookahead_size) & mask;
                    src_buf_left -= 1;
                    self.dict[dst_pos as usize] = c;
                    if dst_pos < TDEFL_MAX_MATCH_LEN - 1 {
                        self.dict[(TDEFL_LZ_DICT_SIZE + dst_pos) as usize] = c;
                    }
                    self.lookahead_size += 1;
                    if self.lookahead_size + self.dict_size >= TDEFL_MIN_MATCH_LEN {
                        let ins_pos = self
                            .lookahead_pos
                            .wrapping_add(self.lookahead_size - 1)
                            .wrapping_sub(2);
                        let hash = ((u32::from(self.dict[(ins_pos & mask) as usize])
                            << (TDEFL_LZ_HASH_SHIFT * 2))
                            ^ (u32::from(self.dict[(ins_pos.wrapping_add(1) & mask) as usize])
                                << TDEFL_LZ_HASH_SHIFT)
                            ^ u32::from(c))
                            & (TDEFL_LZ_HASH_SIZE - 1);
                        self.next[(ins_pos & mask) as usize] = self.hash[hash as usize];
                        self.hash[hash as usize] = ins_pos as u16;
                    }
                }
            }
            self.dict_size = (TDEFL_LZ_DICT_SIZE - self.lookahead_size).min(self.dict_size);
            if flush == TdeflFlush::NoFlush && self.lookahead_size < TDEFL_MAX_MATCH_LEN {
                break;
            }

            // Simple lazy/greedy parsing state machine.
            let mut len_to_move = 1u32;
            let mut cur_match_dist = 0u32;
            let mut cur_match_len = if self.saved_match_len != 0 {
                self.saved_match_len
            } else {
                TDEFL_MIN_MATCH_LEN - 1
            };
            let cur_pos = self.lookahead_pos & mask;
            if self.flags & (TDEFL_RLE_MATCHES | TDEFL_FORCE_ALL_RAW_BLOCKS) != 0 {
                if self.dict_size != 0 && (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) == 0 {
                    let c = self.dict[(cur_pos.wrapping_sub(1) & mask) as usize];
                    cur_match_len = 0;
                    while cur_match_len < self.lookahead_size {
                        if self.dict[(cur_pos + cur_match_len) as usize] != c {
                            break;
                        }
                        cur_match_len += 1;
                    }
                    if cur_match_len < TDEFL_MIN_MATCH_LEN {
                        cur_match_len = 0;
                    } else {
                        cur_match_dist = 1;
                    }
                }
            } else {
                self.find_match(
                    self.lookahead_pos,
                    self.dict_size,
                    self.lookahead_size,
                    &mut cur_match_dist,
                    &mut cur_match_len,
                );
            }
            if (cur_match_len == TDEFL_MIN_MATCH_LEN && cur_match_dist >= 8 * 1024)
                || cur_pos == cur_match_dist
                || ((self.flags & TDEFL_FILTER_MATCHES) != 0 && cur_match_len <= 5)
            {
                cur_match_dist = 0;
                cur_match_len = 0;
            }
            let lit_at = |d: &Self, p: u32| d.dict[(p as usize).min(DICT_LEN - 1)];
            if self.saved_match_len != 0 {
                if cur_match_len > self.saved_match_len {
                    self.record_literal(self.saved_lit as u8);
                    if cur_match_len >= 128 {
                        self.record_match(cur_match_len, cur_match_dist);
                        self.saved_match_len = 0;
                        len_to_move = cur_match_len;
                    } else {
                        self.saved_lit = u32::from(self.dict[cur_pos as usize]);
                        self.saved_match_dist = cur_match_dist;
                        self.saved_match_len = cur_match_len;
                    }
                } else {
                    self.record_match(self.saved_match_len, self.saved_match_dist);
                    len_to_move = self.saved_match_len - 1;
                    self.saved_match_len = 0;
                }
            } else if cur_match_dist == 0 {
                self.record_literal(lit_at(self, cur_pos));
            } else if self.greedy_parsing
                || (self.flags & TDEFL_RLE_MATCHES) != 0
                || cur_match_len >= 128
            {
                self.record_match(cur_match_len, cur_match_dist);
                len_to_move = cur_match_len;
            } else {
                self.saved_lit = u32::from(lit_at(self, cur_pos));
                self.saved_match_dist = cur_match_dist;
                self.saved_match_len = cur_match_len;
            }
            // Move the lookahead forward by len_to_move bytes.
            self.lookahead_pos = self.lookahead_pos.wrapping_add(len_to_move);
            self.lookahead_size -= len_to_move;
            self.dict_size = (self.dict_size + len_to_move).min(TDEFL_LZ_DICT_SIZE);
            // Check if it's time to flush the current LZ codes to the internal output buffer.
            if self.lz_code_pos > TDEFL_LZ_CODE_BUF_SIZE - 8
                || (self.total_lz_bytes > 31 * 1024
                    && ((((self.lz_code_pos as u32).wrapping_mul(115)) >> 7)
                        >= self.total_lz_bytes
                        || (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) != 0))
            {
                self.src_pos = src_pos;
                self.src_buf_left = src_buf_left;
                let n = self.flush_block(TdeflFlush::NoFlush);
                if n != 0 {
                    return n >= 0;
                }
            }
        }

        self.src_pos = src_pos;
        self.src_buf_left = src_buf_left;
        true
    }

    /// Port of `tdefl_compress` for one input buffer: consumes all of `input`
    /// and appends the compressed bytes to the output.
    pub fn compress(&mut self, input: &[u8], flush: TdeflFlush) -> TdeflStatus {
        self.src_pos = 0;
        self.src_buf_left = input.len();
        self.flush = flush;

        if self.prev_return_status != TdeflStatus::Okay
            || (self.wants_to_finish && flush != TdeflFlush::Finish)
        {
            self.prev_return_status = TdeflStatus::BadParam;
            return self.prev_return_status;
        }
        self.wants_to_finish |= flush == TdeflFlush::Finish;

        if self.finished {
            self.prev_return_status = TdeflStatus::Done;
            return self.prev_return_status;
        }

        let ok = if (self.flags & TDEFL_MAX_PROBES_MASK) == 1
            && (self.flags & TDEFL_GREEDY_PARSING_FLAG) != 0
            && (self.flags
                & (TDEFL_FILTER_MATCHES | TDEFL_FORCE_ALL_RAW_BLOCKS | TDEFL_RLE_MATCHES))
                == 0
        {
            self.compress_fast(input)
        } else {
            self.compress_normal(input)
        };
        if !ok {
            return self.prev_return_status;
        }

        if (self.flags & (TDEFL_WRITE_ZLIB_HEADER | TDEFL_COMPUTE_ADLER32)) != 0 {
            self.adler32 = mz_adler32(self.adler32, &input[..self.src_pos]);
        }

        if flush != TdeflFlush::NoFlush && self.lookahead_size == 0 && self.src_buf_left == 0 {
            if self.flush_block(flush) < 0 {
                return self.prev_return_status;
            }
            self.finished = flush == TdeflFlush::Finish;
            if flush == TdeflFlush::FullFlush {
                self.hash.fill(0);
                self.next.fill(0);
                self.dict_size = 0;
            }
        }

        self.prev_return_status = if self.finished {
            TdeflStatus::Done
        } else {
            TdeflStatus::Okay
        };
        self.prev_return_status
    }
}

/// Port of `tdefl_create_comp_flags_from_zip_params`.
pub fn tdefl_create_comp_flags_from_zip_params(level: i32, window_bits: i32, strategy: i32) -> u32 {
    let idx = if level >= 0 {
        level.min(10)
    } else {
        MZ_DEFAULT_LEVEL
    };
    let mut comp_flags = S_TDEFL_NUM_PROBES[idx as usize]
        | if level <= 3 {
            TDEFL_GREEDY_PARSING_FLAG
        } else {
            0
        };
    if window_bits > 0 {
        comp_flags |= TDEFL_WRITE_ZLIB_HEADER;
    }
    if level == 0 {
        comp_flags |= TDEFL_FORCE_ALL_RAW_BLOCKS;
    } else if strategy == MZ_FILTERED {
        comp_flags |= TDEFL_FILTER_MATCHES;
    } else if strategy == MZ_HUFFMAN_ONLY {
        comp_flags &= !TDEFL_MAX_PROBES_MASK;
    } else if strategy == MZ_FIXED {
        comp_flags |= TDEFL_FORCE_ALL_STATIC_BLOCKS;
    } else if strategy == MZ_RLE {
        comp_flags |= TDEFL_RLE_MATCHES;
    }
    comp_flags
}

/// Port of `mz_compressBound` (`mz_deflateBound`).
pub fn mz_compress_bound(source_len: u64) -> u64 {
    let a = 128u64.wrapping_add(source_len.wrapping_mul(110) / 100);
    let b = 128u64
        .wrapping_add(source_len)
        .wrapping_add((source_len / (31 * 1024) + 1).wrapping_mul(5));
    a.max(b)
}

/// Port of `mz_compress2`: zlib-wraps `source` at `level` into at most
/// `dest_len` bytes. `Err` holds the `MZ_*` status.
pub fn mz_compress2(source: &[u8], dest_len: u64, level: i32) -> Result<Vec<u8>, i32> {
    // In case mz_ulong is 64-bits (argh I hate longs).
    if (source.len() as u64 | dest_len) > 0xFFFF_FFFF {
        return Err(MZ_PARAM_ERROR);
    }
    if dest_len == 0 {
        return Err(MZ_BUF_ERROR);
    }
    let comp_flags = TDEFL_COMPUTE_ADLER32
        | tdefl_create_comp_flags_from_zip_params(
            level,
            MZ_DEFAULT_WINDOW_BITS,
            MZ_DEFAULT_STRATEGY,
        );
    let mut comp = TdeflCompressor::new(comp_flags);
    match comp.compress(source, TdeflFlush::Finish) {
        TdeflStatus::Done => {}
        TdeflStatus::Okay => return Err(MZ_BUF_ERROR),
        _ => return Err(MZ_STREAM_ERROR),
    }
    let out = comp.into_output();
    if out.len() as u64 > dest_len {
        return Err(MZ_BUF_ERROR);
    }
    Ok(out)
}
