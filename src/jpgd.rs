// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal (jpgd) and modified; see NOTICE.md.
//! Port of basis_universal's jpgd (encoder/jpgd.cpp / jpgd.h, Rich
//! Geldreich's baseline + progressive JPEG decoder) as used by the `ktx
//! create` JPEG input.
//!
//! `setjmp` / `longjmp` error handling is `Result` propagation: a stop sets
//! the error code like `stop_decoding`. Memory-pool allocations are `Vec`s
//! (zeroed; the C++ only reads bytes it wrote). Integer arithmetic wraps and
//! shift counts are masked like x86. Two C++ memory layouts are modeled
//! because corrupted streams write through them: `huff_tables` is one word
//! array (a tree index before the tree lands in `code_size` / `look_up2`),
//! and `m_mcu_org` overflows into the six `int` members that follow it.

pub type JpgdStatus = i32;
pub const JPGD_SUCCESS: JpgdStatus = 0;
pub const JPGD_FAILED: JpgdStatus = -1;
pub const JPGD_DONE: JpgdStatus = 1;
pub const JPGD_BAD_DHT_COUNTS: JpgdStatus = -256;
pub const JPGD_BAD_DHT_INDEX: JpgdStatus = -255;
pub const JPGD_BAD_DHT_MARKER: JpgdStatus = -254;
pub const JPGD_BAD_DQT_MARKER: JpgdStatus = -253;
pub const JPGD_BAD_DQT_TABLE: JpgdStatus = -252;
pub const JPGD_BAD_PRECISION: JpgdStatus = -251;
pub const JPGD_BAD_HEIGHT: JpgdStatus = -250;
pub const JPGD_BAD_WIDTH: JpgdStatus = -249;
pub const JPGD_TOO_MANY_COMPONENTS: JpgdStatus = -248;
pub const JPGD_BAD_SOF_LENGTH: JpgdStatus = -247;
pub const JPGD_BAD_VARIABLE_MARKER: JpgdStatus = -246;
pub const JPGD_BAD_DRI_LENGTH: JpgdStatus = -245;
pub const JPGD_BAD_SOS_LENGTH: JpgdStatus = -244;
pub const JPGD_BAD_SOS_COMP_ID: JpgdStatus = -243;
pub const JPGD_W_EXTRA_BYTES_BEFORE_MARKER: JpgdStatus = -242;
pub const JPGD_NO_ARITHMITIC_SUPPORT: JpgdStatus = -241;
pub const JPGD_UNEXPECTED_MARKER: JpgdStatus = -240;
pub const JPGD_NOT_JPEG: JpgdStatus = -239;
pub const JPGD_UNSUPPORTED_MARKER: JpgdStatus = -238;
pub const JPGD_BAD_DQT_LENGTH: JpgdStatus = -237;
pub const JPGD_TOO_MANY_BLOCKS: JpgdStatus = -236;
pub const JPGD_UNDEFINED_QUANT_TABLE: JpgdStatus = -235;
pub const JPGD_UNDEFINED_HUFF_TABLE: JpgdStatus = -234;
pub const JPGD_NOT_SINGLE_SCAN: JpgdStatus = -233;
pub const JPGD_UNSUPPORTED_COLORSPACE: JpgdStatus = -232;
pub const JPGD_UNSUPPORTED_SAMP_FACTORS: JpgdStatus = -231;
pub const JPGD_DECODE_ERROR: JpgdStatus = -230;
pub const JPGD_BAD_RESTART_MARKER: JpgdStatus = -229;
pub const JPGD_BAD_SOS_SPECTRAL: JpgdStatus = -228;
pub const JPGD_BAD_SOS_SUCCESSIVE: JpgdStatus = -227;
pub const JPGD_STREAM_READ: JpgdStatus = -226;
pub const JPGD_NOTENOUGHMEM: JpgdStatus = -225;
pub const JPGD_TOO_MANY_SCANS: JpgdStatus = -224;

/// Name of a status (the `fmt::formatter<jpgd_status>` of jpginput.cc).
pub fn jpgd_status_name(s: JpgdStatus) -> &'static str {
    const NAMES: [&str; 33] = [
        "JPGD_BAD_DHT_COUNTS",
        "JPGD_BAD_DHT_INDEX",
        "JPGD_BAD_DHT_MARKER",
        "JPGD_BAD_DQT_MARKER",
        "JPGD_BAD_DQT_TABLE",
        "JPGD_BAD_PRECISION",
        "JPGD_BAD_HEIGHT",
        "JPGD_BAD_WIDTH",
        "JPGD_TOO_MANY_COMPONENTS",
        "JPGD_BAD_SOF_LENGTH",
        "JPGD_BAD_VARIABLE_MARKER",
        "JPGD_BAD_DRI_LENGTH",
        "JPGD_BAD_SOS_LENGTH",
        "JPGD_BAD_SOS_COMP_ID",
        "JPGD_W_EXTRA_BYTES_BEFORE_MARKER",
        "JPGD_NO_ARITHMITIC_SUPPORT",
        "JPGD_UNEXPECTED_MARKER",
        "JPGD_NOT_JPEG",
        "JPGD_UNSUPPORTED_MARKER",
        "JPGD_BAD_DQT_LENGTH",
        "JPGD_TOO_MANY_BLOCKS",
        "JPGD_UNDEFINED_QUANT_TABLE",
        "JPGD_UNDEFINED_HUFF_TABLE",
        "JPGD_NOT_SINGLE_SCAN",
        "JPGD_UNSUPPORTED_COLORSPACE",
        "JPGD_UNSUPPORTED_SAMP_FACTORS",
        "JPGD_DECODE_ERROR",
        "JPGD_BAD_RESTART_MARKER",
        "JPGD_BAD_SOS_SPECTRAL",
        "JPGD_BAD_SOS_SUCCESSIVE",
        "JPGD_STREAM_READ",
        "JPGD_NOTENOUGHMEM",
        "JPGD_TOO_MANY_SCANS",
    ];
    match s {
        JPGD_SUCCESS => "JPGD_SUCCESS",
        JPGD_FAILED => "JPGD_FAILED",
        JPGD_DONE => "JPGD_DONE",
        _ => usize::try_from(s - JPGD_BAD_DHT_COUNTS)
            .ok()
            .and_then(|i| NAMES.get(i))
            .copied()
            .unwrap_or(""),
    }
}

/// `jpeg_decoder_stream`.
pub trait JpegDecoderStream {
    /// Reads up to `buf.len()` bytes; -1 on error, sets `eof_flag` at the end.
    fn read(&mut self, buf: &mut [u8], eof_flag: &mut bool) -> i32;
}

/// `jpeg_decoder_mem_stream`.
pub struct JpegDecoderMemStream {
    src: Option<Vec<u8>>,
    ofs: usize,
}

impl JpegDecoderMemStream {
    pub fn new(src: Vec<u8>) -> JpegDecoderMemStream {
        JpegDecoderMemStream {
            src: Some(src),
            ofs: 0,
        }
    }

    /// `close`: later reads fail.
    pub fn close(&mut self) {
        self.src = None;
        self.ofs = 0;
    }
}

impl JpegDecoderStream for JpegDecoderMemStream {
    fn read(&mut self, buf: &mut [u8], eof_flag: &mut bool) -> i32 {
        *eof_flag = false;
        let Some(src) = &self.src else {
            return -1;
        };
        let remaining = src.len() - self.ofs;
        let mut n = buf.len();
        if n > remaining {
            n = remaining;
            *eof_flag = true;
        }
        buf[..n].copy_from_slice(&src[self.ofs..self.ofs + n]);
        self.ofs += n;
        n as i32
    }
}

const JPGD_IN_BUF_SIZE: usize = 8192;
const JPGD_MAX_BLOCKS_PER_MCU: usize = 10;
const JPGD_MAX_HUFF_TABLES: usize = 8;
const JPGD_MAX_QUANT_TABLES: usize = 4;
const JPGD_MAX_COMPONENTS: usize = 4;
const JPGD_MAX_COMPS_IN_SCAN: usize = 4;
const JPGD_MAX_BLOCKS_PER_ROW: i32 = 16384;
const JPGD_MAX_HEIGHT: i32 = 32768;
const JPGD_MAX_WIDTH: i32 = 32768;
const JPGD_HUFF_TREE_MAX_LENGTH: usize = 512;

/// DCT coefficients are stored in this sequence.
const G_ZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const M_SOF0: i32 = 0xC0;
const M_SOF1: i32 = 0xC1;
const M_SOF2: i32 = 0xC2;
const M_SOF3: i32 = 0xC3;
const M_SOF5: i32 = 0xC5;
const M_SOF6: i32 = 0xC6;
const M_SOF7: i32 = 0xC7;
const M_JPG: i32 = 0xC8;
const M_SOF9: i32 = 0xC9;
const M_SOF10: i32 = 0xCA;
const M_SOF11: i32 = 0xCB;
const M_SOF13: i32 = 0xCD;
const M_SOF14: i32 = 0xCE;
const M_SOF15: i32 = 0xCF;
const M_DHT: i32 = 0xC4;
const M_DAC: i32 = 0xCC;
const M_RST0: i32 = 0xD0;
const M_RST7: i32 = 0xD7;
const M_SOI: i32 = 0xD8;
const M_EOI: i32 = 0xD9;
const M_SOS: i32 = 0xDA;
const M_DQT: i32 = 0xDB;
const M_DRI: i32 = 0xDD;
const M_TEM: i32 = 0x01;

const JPGD_GRAYSCALE: i32 = 0;
const JPGD_YH1V1: i32 = 1;
const JPGD_YH2V1: i32 = 2;
const JPGD_YH1V2: i32 = 3;
const JPGD_YH2V2: i32 = 4;

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;

fn descale(x: i32, n: i32) -> i32 {
    x.wrapping_add(1 << (n - 1)) >> n
}

fn descale_zeroshift(x: i32, n: i32) -> i32 {
    x.wrapping_add(128 << n).wrapping_add(1 << (n - 1)) >> n
}

/// `CLAMP` / `clamp`.
fn clamp(i: i32) -> u8 {
    if i as u32 > 255 {
        ((!i) >> 31) as u8
    } else {
        i as u8
    }
}

fn left_shifti(val: i32, bits: i32) -> i32 {
    (val as u32).wrapping_shl(bits as u32) as i32
}

/// The 1D IDCT of `Row<N>` / `Col<N>` on 8 inputs (`n` non-zero ones).
fn idct_1d(v: [i32; 8], n: usize) -> [i32; 8] {
    let a = |x: usize| if x < n { v[x] } else { 0 };
    let z2 = a(2);
    let z3 = a(6);
    let z1 = z2.wrapping_add(z3).wrapping_mul(FIX_0_541196100);
    let tmp2 = z1.wrapping_add(z3.wrapping_mul(-FIX_1_847759065));
    let tmp3 = z1.wrapping_add(z2.wrapping_mul(FIX_0_765366865));
    let tmp0 = left_shifti(a(0).wrapping_add(a(4)), CONST_BITS);
    let tmp1 = left_shifti(a(0).wrapping_sub(a(4)), CONST_BITS);
    let tmp10 = tmp0.wrapping_add(tmp3);
    let tmp13 = tmp0.wrapping_sub(tmp3);
    let tmp11 = tmp1.wrapping_add(tmp2);
    let tmp12 = tmp1.wrapping_sub(tmp2);
    let (atmp0, atmp1, atmp2, atmp3) = (a(7), a(5), a(3), a(1));
    let bz1 = atmp0.wrapping_add(atmp3);
    let bz2 = atmp1.wrapping_add(atmp2);
    let bz3 = atmp0.wrapping_add(atmp2);
    let bz4 = atmp1.wrapping_add(atmp3);
    let bz5 = bz3.wrapping_add(bz4).wrapping_mul(FIX_1_175875602);
    let az1 = bz1.wrapping_mul(-FIX_0_899976223);
    let az2 = bz2.wrapping_mul(-FIX_2_562915447);
    let az3 = bz3.wrapping_mul(-FIX_1_961570560).wrapping_add(bz5);
    let az4 = bz4.wrapping_mul(-FIX_0_390180644).wrapping_add(bz5);
    let btmp0 = atmp0
        .wrapping_mul(FIX_0_298631336)
        .wrapping_add(az1)
        .wrapping_add(az3);
    let btmp1 = atmp1
        .wrapping_mul(FIX_2_053119869)
        .wrapping_add(az2)
        .wrapping_add(az4);
    let btmp2 = atmp2
        .wrapping_mul(FIX_3_072711026)
        .wrapping_add(az2)
        .wrapping_add(az3);
    let btmp3 = atmp3
        .wrapping_mul(FIX_1_501321110)
        .wrapping_add(az1)
        .wrapping_add(az4);
    [
        tmp10.wrapping_add(btmp3),
        tmp11.wrapping_add(btmp2),
        tmp12.wrapping_add(btmp1),
        tmp13.wrapping_add(btmp0),
        tmp13.wrapping_sub(btmp0),
        tmp12.wrapping_sub(btmp1),
        tmp11.wrapping_sub(btmp2),
        tmp10.wrapping_sub(btmp3),
    ]
}

const S_IDCT_ROW_TABLE: [u8; 512] = [
    1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 2, 1, 1, 0, 0, 0, 0, 0,
    2, 2, 1, 0, 0, 0, 0, 0, 3, 2, 1, 0, 0, 0, 0, 0, 4, 2, 1, 0, 0, 0, 0, 0, 4, 3, 1, 0, 0, 0, 0, 0,
    4, 3, 2, 0, 0, 0, 0, 0, 4, 3, 2, 1, 0, 0, 0, 0, 4, 3, 2, 1, 1, 0, 0, 0, 4, 3, 2, 2, 1, 0, 0, 0,
    4, 3, 3, 2, 1, 0, 0, 0, 4, 4, 3, 2, 1, 0, 0, 0, 5, 4, 3, 2, 1, 0, 0, 0, 6, 4, 3, 2, 1, 0, 0, 0,
    6, 5, 3, 2, 1, 0, 0, 0, 6, 5, 4, 2, 1, 0, 0, 0, 6, 5, 4, 3, 1, 0, 0, 0, 6, 5, 4, 3, 2, 0, 0, 0,
    6, 5, 4, 3, 2, 1, 0, 0, 6, 5, 4, 3, 2, 1, 1, 0, 6, 5, 4, 3, 2, 2, 1, 0, 6, 5, 4, 3, 3, 2, 1, 0,
    6, 5, 4, 4, 3, 2, 1, 0, 6, 5, 5, 4, 3, 2, 1, 0, 6, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0,
    8, 6, 5, 4, 3, 2, 1, 0, 8, 7, 5, 4, 3, 2, 1, 0, 8, 7, 6, 4, 3, 2, 1, 0, 8, 7, 6, 5, 3, 2, 1, 0,
    8, 7, 6, 5, 4, 2, 1, 0, 8, 7, 6, 5, 4, 3, 1, 0, 8, 7, 6, 5, 4, 3, 2, 0, 8, 7, 6, 5, 4, 3, 2, 1,
    8, 7, 6, 5, 4, 3, 2, 2, 8, 7, 6, 5, 4, 3, 3, 2, 8, 7, 6, 5, 4, 4, 3, 2, 8, 7, 6, 5, 5, 4, 3, 2,
    8, 7, 6, 6, 5, 4, 3, 2, 8, 7, 7, 6, 5, 4, 3, 2, 8, 8, 7, 6, 5, 4, 3, 2, 8, 8, 8, 6, 5, 4, 3, 2,
    8, 8, 8, 7, 5, 4, 3, 2, 8, 8, 8, 7, 6, 4, 3, 2, 8, 8, 8, 7, 6, 5, 3, 2, 8, 8, 8, 7, 6, 5, 4, 2,
    8, 8, 8, 7, 6, 5, 4, 3, 8, 8, 8, 7, 6, 5, 4, 4, 8, 8, 8, 7, 6, 5, 5, 4, 8, 8, 8, 7, 6, 6, 5, 4,
    8, 8, 8, 7, 7, 6, 5, 4, 8, 8, 8, 8, 7, 6, 5, 4, 8, 8, 8, 8, 8, 6, 5, 4, 8, 8, 8, 8, 8, 7, 5, 4,
    8, 8, 8, 8, 8, 7, 6, 4, 8, 8, 8, 8, 8, 7, 6, 5, 8, 8, 8, 8, 8, 7, 6, 6, 8, 8, 8, 8, 8, 7, 7, 6,
    8, 8, 8, 8, 8, 8, 7, 6, 8, 8, 8, 8, 8, 8, 8, 6, 8, 8, 8, 8, 8, 8, 8, 7, 8, 8, 8, 8, 8, 8, 8, 8,
];

const S_IDCT_COL_TABLE: [u8; 64] = [
    1, 1, 2, 3, 3, 3, 3, 3, 3, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
];

/// `idct`: scalar fast path IDCT of one block into 8x8 bytes at `dst`
/// (stride 8).
fn idct(src: &[i16], dst: &mut [u8], block_max_zag: i32) {
    if block_max_zag <= 1 {
        let k = clamp(((i32::from(src[0]) + 4) >> 3) + 128);
        dst[..64].fill(k);
        return;
    }
    let mut temp = [0i32; 64];
    let row_tab = &S_IDCT_ROW_TABLE[((block_max_zag - 1) * 8) as usize..];
    for r in 0..8 {
        let n = row_tab[r] as usize;
        let t = &mut temp[r * 8..r * 8 + 8];
        let s = &src[r * 8..r * 8 + 8];
        match n {
            0 => {}
            1 => {
                let dcval = left_shifti(i32::from(s[0]), PASS1_BITS);
                t.fill(dcval);
            }
            _ => {
                let v = std::array::from_fn(|i| i32::from(s[i]));
                let o = idct_1d(v, n);
                for i in 0..8 {
                    t[i] = descale(o[i], CONST_BITS - PASS1_BITS);
                }
            }
        }
    }
    let nonzero_rows = S_IDCT_COL_TABLE[(block_max_zag - 1) as usize] as usize;
    for c in 0..8 {
        if nonzero_rows == 1 {
            let dcval = clamp(descale_zeroshift(temp[c], PASS1_BITS + 3));
            for r in 0..8 {
                dst[r * 8 + c] = dcval;
            }
        } else {
            let v = std::array::from_fn(|r| temp[r * 8 + c]);
            let o = idct_1d(v, nonzero_rows);
            for r in 0..8 {
                dst[r * 8 + c] = clamp(descale_zeroshift(o[r], CONST_BITS + PASS1_BITS + 3));
            }
        }
    }
}

const S_EXTEND_TEST: [i32; 16] = [
    0, 0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0020, 0x0040, 0x0080, 0x0100, 0x0200, 0x0400,
    0x0800, 0x1000, 0x2000, 0x4000,
];
const S_EXTEND_OFFSET: [i32; 16] = [
    0, -1, -3, -7, -15, -31, -63, -127, -255, -511, -1023, -2047, -4095, -8191, -16383, -32767,
];

/// `JPGD_HUFF_EXTEND`.
fn huff_extend(x: i32, s: i32) -> i32 {
    let s = (s & 15) as usize;
    if x < S_EXTEND_TEST[s] {
        x.wrapping_add(S_EXTEND_OFFSET[s])
    } else {
        x
    }
}

/// `huff_tables` as the words of the C struct (`ac_table` + padding,
/// `look_up[256]`, `look_up2[256]`, `code_size[256]` bytes, `tree[512]`).
#[derive(Clone)]
struct HuffTables {
    w: Vec<u32>,
}

const HT_LOOK_UP: usize = 1;
const HT_LOOK_UP2: usize = 257;
const HT_CODE_SIZE: usize = 513;
const HT_TREE: usize = 577;
const HT_WORDS: usize = HT_TREE + JPGD_HUFF_TREE_MAX_LENGTH;

impl HuffTables {
    fn new() -> HuffTables {
        HuffTables {
            w: vec![0; HT_WORDS],
        }
    }
    fn look_up(&self, i: usize) -> i32 {
        self.w[HT_LOOK_UP + i] as i32
    }
    fn look_up2(&self, i: usize) -> i32 {
        self.w[HT_LOOK_UP2 + i] as i32
    }
    fn code_size(&self, i: usize) -> u32 {
        (self.w[HT_CODE_SIZE + i / 4] >> (8 * (i % 4))) & 0xff
    }
    fn set_code_size(&mut self, i: usize, v: u8) {
        let word = &mut self.w[HT_CODE_SIZE + i / 4];
        let sh = 8 * (i % 4);
        *word = (*word & !(0xff << sh)) | (u32::from(v) << sh);
    }
    fn tree(&self, i: usize) -> i32 {
        self.w[HT_TREE + i] as i32
    }
    /// `tree[i]` for a possibly negative index inside the struct.
    fn tree_slot(&mut self, i: i64) -> Option<&mut u32> {
        usize::try_from(HT_TREE as i64 + i)
            .ok()
            .and_then(|k| self.w.get_mut(k))
    }
}

/// `coeff_buf`.
struct CoeffBuf {
    data: Vec<i16>,
    block_num_x: i32,
    block_num_y: i32,
    /// `block_size` in coefficients.
    block_len: i32,
}

/// Indices of the `int` members that `m_mcu_org[10]` overflows into.
const ORG_TOTAL_LINES_LEFT: usize = 10;
const ORG_MCU_LINES_LEFT: usize = 11;
const ORG_NUM_BUFFERED_SCANLINES: usize = 12;
const ORG_REAL_DEST_BYTES_PER_SCAN_LINE: usize = 13;
const ORG_DEST_BYTES_PER_SCAN_LINE: usize = 14;
const ORG_DEST_BYTES_PER_PIXEL: usize = 15;

/// Progressive block decoders (`pDecode_block_func`).
#[derive(Clone, Copy)]
enum DecodeBlock {
    DcFirst,
    DcRefine,
    AcFirst,
    AcRefine,
}

type R<T> = Result<T, JpgdStatus>;

/// `jpeg_decoder`.
pub struct JpegDecoder {
    flags: u32,
    image_x_size: i32,
    image_y_size: i32,
    stream: Option<Box<dyn JpegDecoderStream>>,
    progressive_flag: bool,
    huff_ac: [u8; JPGD_MAX_HUFF_TABLES],
    huff_num: [Option<[u8; 17]>; JPGD_MAX_HUFF_TABLES],
    huff_val: [Option<[u8; 256]>; JPGD_MAX_HUFF_TABLES],
    quant: [Option<[i16; 64]>; JPGD_MAX_QUANT_TABLES],
    scan_type: i32,
    comps_in_frame: i32,
    comp_h_samp: [i32; JPGD_MAX_COMPONENTS],
    comp_v_samp: [i32; JPGD_MAX_COMPONENTS],
    comp_quant: [i32; JPGD_MAX_COMPONENTS],
    comp_ident: [i32; JPGD_MAX_COMPONENTS],
    comp_h_blocks: [i32; JPGD_MAX_COMPONENTS],
    comp_v_blocks: [i32; JPGD_MAX_COMPONENTS],
    comps_in_scan: i32,
    comp_list: [i32; JPGD_MAX_COMPS_IN_SCAN],
    comp_dc_tab: [i32; JPGD_MAX_COMPONENTS],
    comp_ac_tab: [i32; JPGD_MAX_COMPONENTS],
    spectral_start: i32,
    spectral_end: i32,
    successive_low: i32,
    successive_high: i32,
    max_mcu_x_size: i32,
    max_mcu_y_size: i32,
    blocks_per_mcu: i32,
    max_blocks_per_row: i32,
    mcus_per_row: i32,
    mcus_per_col: i32,
    /// `m_mcu_org[10]` followed by `m_total_lines_left`,
    /// `m_mcu_lines_left`, `m_num_buffered_scanlines`,
    /// `m_real_dest_bytes_per_scan_line`, `m_dest_bytes_per_scan_line` and
    /// `m_dest_bytes_per_pixel`.
    mcu_org: [i32; 16],
    huff_tabs: [Option<HuffTables>; JPGD_MAX_HUFF_TABLES],
    dc_coeffs: [Option<CoeffBuf>; JPGD_MAX_COMPONENTS],
    ac_coeffs: [Option<CoeffBuf>; JPGD_MAX_COMPONENTS],
    eob_run: i32,
    block_y_mcu: [i32; JPGD_MAX_COMPONENTS],
    /// `m_in_buf_pad_start[64]`, `m_in_buf[JPGD_IN_BUF_SIZE + 128]`,
    /// `m_in_buf_pad_end[64]`.
    in_buf: Vec<u8>,
    in_buf_ofs: usize,
    in_buf_left: i32,
    tem_flag: i32,
    bits_left: i32,
    bit_buf: u32,
    restart_interval: i32,
    restarts_left: i32,
    next_restart_num: i32,
    max_mcus_per_row: i32,
    max_blocks_per_mcu: i32,
    max_mcus_per_col: i32,
    last_dc_val: [u32; JPGD_MAX_COMPONENTS],
    mcu_coefficients: Vec<i16>,
    mcu_block_max_zag: [i32; JPGD_MAX_BLOCKS_PER_MCU],
    sample_buf: Vec<u8>,
    sample_buf_prev: Vec<u8>,
    crr: [i32; 256],
    cbb: [i32; 256],
    crg: [i32; 256],
    cbg: [i32; 256],
    scan_line_0: Vec<u8>,
    scan_line_1: Vec<u8>,
    error_code: JpgdStatus,
    total_bytes_read: i32,
    ready_flag: bool,
    eof_flag: bool,
    sample_buf_prev_valid: bool,
}

/// Start of `m_in_buf` inside `in_buf`.
const IN_BUF_START: usize = 64;

impl JpegDecoder {
    pub const C_FLAG_LINEAR_CHROMA_FILTERING: u32 = 1;

    /// `jpeg_decoder(pStream, flags)`: reads the headers up to the frame
    /// (check `get_error_code`).
    pub fn new(stream: Box<dyn JpegDecoderStream>, flags: u32) -> JpegDecoder {
        let mut d = JpegDecoder {
            flags,
            image_x_size: 0,
            image_y_size: 0,
            stream: Some(stream),
            progressive_flag: false,
            huff_ac: [0; JPGD_MAX_HUFF_TABLES],
            huff_num: [None; JPGD_MAX_HUFF_TABLES],
            huff_val: [None; JPGD_MAX_HUFF_TABLES],
            quant: [None; JPGD_MAX_QUANT_TABLES],
            scan_type: 0,
            comps_in_frame: 0,
            comp_h_samp: [0; 4],
            comp_v_samp: [0; 4],
            comp_quant: [0; 4],
            comp_ident: [0; 4],
            comp_h_blocks: [0; 4],
            comp_v_blocks: [0; 4],
            comps_in_scan: 0,
            comp_list: [0; 4],
            comp_dc_tab: [0; 4],
            comp_ac_tab: [0; 4],
            spectral_start: 0,
            spectral_end: 0,
            successive_low: 0,
            successive_high: 0,
            max_mcu_x_size: 0,
            max_mcu_y_size: 0,
            blocks_per_mcu: 0,
            max_blocks_per_row: 0,
            mcus_per_row: 0,
            mcus_per_col: 0,
            mcu_org: [0; 16],
            huff_tabs: Default::default(),
            dc_coeffs: Default::default(),
            ac_coeffs: Default::default(),
            eob_run: 0,
            block_y_mcu: [0; 4],
            in_buf: vec![0; 64 + JPGD_IN_BUF_SIZE + 128 + 64],
            in_buf_ofs: IN_BUF_START,
            in_buf_left: 0,
            tem_flag: 0,
            bits_left: 0,
            bit_buf: 0,
            restart_interval: 0,
            restarts_left: 0,
            next_restart_num: 0,
            max_mcus_per_row: 0,
            max_blocks_per_mcu: 0,
            max_mcus_per_col: 0,
            last_dc_val: [0; 4],
            mcu_coefficients: Vec::new(),
            mcu_block_max_zag: [64; JPGD_MAX_BLOCKS_PER_MCU],
            sample_buf: Vec::new(),
            sample_buf_prev: Vec::new(),
            crr: [0; 256],
            cbb: [0; 256],
            crg: [0; 256],
            cbg: [0; 256],
            scan_line_0: Vec::new(),
            scan_line_1: Vec::new(),
            error_code: JPGD_SUCCESS,
            total_bytes_read: 0,
            ready_flag: false,
            eof_flag: false,
            sample_buf_prev_valid: false,
        };
        let r = d.decode_init();
        d.catch(r);
        d
    }

    /// Records a `stop_decoding` status (the `setjmp` landing).
    fn catch<T>(&mut self, r: R<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(status) => {
                self.error_code = status;
                // free_all_blocks
                self.stream = None;
                None
            }
        }
    }

    pub fn get_error_code(&self) -> JpgdStatus {
        self.error_code
    }
    pub fn get_width(&self) -> i32 {
        self.image_x_size
    }
    pub fn get_height(&self) -> i32 {
        self.image_y_size
    }
    pub fn get_num_components(&self) -> i32 {
        self.comps_in_frame
    }
    pub fn get_bytes_per_pixel(&self) -> i32 {
        self.mcu_org[ORG_DEST_BYTES_PER_PIXEL]
    }
    pub fn get_bytes_per_scan_line(&self) -> i32 {
        self.image_x_size.wrapping_mul(self.get_bytes_per_pixel())
    }
    pub fn get_total_bytes_read(&self) -> i32 {
        self.total_bytes_read
    }

    fn total_lines_left(&self) -> i32 {
        self.mcu_org[ORG_TOTAL_LINES_LEFT]
    }

    // ---------------------------------------------------------------- input

    /// `prep_in_buffer`: refills the input buffer.
    fn prep_in_buffer(&mut self) -> R<()> {
        self.in_buf_left = 0;
        self.in_buf_ofs = IN_BUF_START;
        if self.eof_flag {
            return Ok(());
        }
        loop {
            let start = IN_BUF_START + self.in_buf_left as usize;
            let end = IN_BUF_START + JPGD_IN_BUF_SIZE;
            let mut eof = self.eof_flag;
            let bytes_read = match &mut self.stream {
                Some(s) => s.read(&mut self.in_buf[start..end], &mut eof),
                None => -1,
            };
            self.eof_flag = eof;
            if bytes_read == -1 {
                return Err(JPGD_STREAM_READ);
            }
            self.in_buf_left += bytes_read;
            if !((self.in_buf_left as usize) < JPGD_IN_BUF_SIZE && !self.eof_flag) {
                break;
            }
        }
        self.total_bytes_read = self.total_bytes_read.wrapping_add(self.in_buf_left);
        // Pad the end of the block with M_EOI.
        let p = self.in_buf_ofs + self.in_buf_left as usize;
        for i in 0..64 {
            self.in_buf[p + 2 * i] = 0xFF;
            self.in_buf[p + 2 * i + 1] = 0xD9;
        }
        Ok(())
    }

    /// `get_char`.
    fn get_char(&mut self) -> R<u32> {
        let mut pad = false;
        self.get_char_pad(&mut pad)
    }

    /// `get_char(bool* pPadding_flag)`.
    fn get_char_pad(&mut self, padding_flag: &mut bool) -> R<u32> {
        if self.in_buf_left == 0 {
            self.prep_in_buffer()?;
            if self.in_buf_left == 0 {
                *padding_flag = true;
                let t = self.tem_flag;
                self.tem_flag ^= 1;
                return Ok(if t != 0 { 0xD9 } else { 0xFF });
            }
        }
        *padding_flag = false;
        let c = u32::from(self.in_buf[self.in_buf_ofs]);
        self.in_buf_ofs += 1;
        self.in_buf_left -= 1;
        Ok(c)
    }

    /// `stuff_char`.
    fn stuff_char(&mut self, q: u8) {
        if self.in_buf_ofs > 0 {
            self.in_buf_ofs -= 1;
        }
        self.in_buf[self.in_buf_ofs] = q;
        self.in_buf_left += 1;
    }

    /// `get_octet`: does not read past markers.
    fn get_octet(&mut self) -> R<u8> {
        let mut padding_flag = false;
        let mut c = self.get_char_pad(&mut padding_flag)?;
        if c == 0xFF {
            if padding_flag {
                return Ok(0xFF);
            }
            c = self.get_char_pad(&mut padding_flag)?;
            if padding_flag {
                self.stuff_char(0xFF);
                return Ok(0xFF);
            }
            if c == 0x00 {
                return Ok(0xFF);
            }
            self.stuff_char(c as u8);
            self.stuff_char(0xFF);
            return Ok(0xFF);
        }
        Ok(c as u8)
    }

    /// `get_bits`: does not recognize markers.
    fn get_bits(&mut self, num_bits: i32) -> R<u32> {
        if num_bits == 0 {
            return Ok(0);
        }
        let i = self.bit_buf.wrapping_shr((32 - num_bits) as u32);
        self.bits_left = self.bits_left.wrapping_sub(num_bits);
        if self.bits_left <= 0 {
            let n = num_bits.wrapping_add(self.bits_left);
            self.bit_buf = self.bit_buf.wrapping_shl(n as u32);
            let c1 = self.get_char()?;
            let c2 = self.get_char()?;
            self.bit_buf = (self.bit_buf & 0xFFFF_0000) | (c1 << 8) | c2;
            self.bit_buf = self
                .bit_buf
                .wrapping_shl(self.bits_left.wrapping_neg() as u32);
            self.bits_left = self.bits_left.wrapping_add(16);
        } else {
            self.bit_buf = self.bit_buf.wrapping_shl(num_bits as u32);
        }
        Ok(i)
    }

    /// `get_bits_no_markers`: returns 1s at a marker.
    fn get_bits_no_markers(&mut self, num_bits: i32) -> R<u32> {
        if num_bits == 0 {
            return Ok(0);
        }
        let i = self.bit_buf.wrapping_shr((32 - num_bits) as u32);
        self.bits_left = self.bits_left.wrapping_sub(num_bits);
        if self.bits_left <= 0 {
            let n = num_bits.wrapping_add(self.bits_left);
            self.bit_buf = self.bit_buf.wrapping_shl(n as u32);
            let p = self.in_buf_ofs;
            if self.in_buf_left < 2 || self.in_buf[p] == 0xFF || self.in_buf[p + 1] == 0xFF {
                let c1 = u32::from(self.get_octet()?);
                let c2 = u32::from(self.get_octet()?);
                self.bit_buf |= (c1 << 8) | c2;
            } else {
                self.bit_buf |= (u32::from(self.in_buf[p]) << 8) | u32::from(self.in_buf[p + 1]);
                self.in_buf_left -= 2;
                self.in_buf_ofs += 2;
            }
            self.bit_buf = self
                .bit_buf
                .wrapping_shl(self.bits_left.wrapping_neg() as u32);
            self.bits_left = self.bits_left.wrapping_add(16);
        } else {
            self.bit_buf = self.bit_buf.wrapping_shl(num_bits as u32);
        }
        Ok(i)
    }

    /// Walks the Huffman tree for a symbol longer than 8 bits.
    fn huff_tree_walk(&self, t: usize, mut symbol: i32) -> R<(i32, i32)> {
        let Some(h) = &self.huff_tabs[t] else {
            return Err(JPGD_DECODE_ERROR);
        };
        let mut ofs: i32 = 23;
        loop {
            let bit = (self.bit_buf.wrapping_shr(ofs as u32) & 1) as i32;
            let idx = symbol.wrapping_add(bit).wrapping_neg() as u32;
            if idx as usize >= JPGD_HUFF_TREE_MAX_LENGTH || ofs < 0 {
                return Err(JPGD_DECODE_ERROR);
            }
            symbol = h.tree(idx as usize);
            ofs -= 1;
            if symbol >= 0 {
                return Ok((symbol, ofs));
            }
        }
    }

    /// `huff_decode(pH)` for table `t`.
    fn huff_decode(&mut self, t: Option<usize>) -> R<i32> {
        let Some(t) = t.filter(|&t| self.huff_tabs.get(t).is_some_and(Option::is_some)) else {
            return Err(JPGD_DECODE_ERROR);
        };
        let symbol = self.huff_tabs[t]
            .as_ref()
            .map_or(0, |h| h.look_up((self.bit_buf >> 24) as usize));
        if symbol < 0 {
            let (symbol, ofs) = self.huff_tree_walk(t, symbol)?;
            self.get_bits_no_markers(8 + (23 - ofs))?;
            Ok(symbol)
        } else {
            let cs = self.huff_tabs[t]
                .as_ref()
                .map_or(0, |h| h.code_size(symbol as usize & 0xff));
            self.get_bits_no_markers(cs as i32)?;
            Ok(symbol)
        }
    }

    /// `huff_decode(pH, extra_bits)` for table `t`.
    fn huff_decode_extra(&mut self, t: Option<usize>, extra_bits: &mut i32) -> R<i32> {
        let Some(t) = t.filter(|&t| self.huff_tabs.get(t).is_some_and(Option::is_some)) else {
            return Err(JPGD_DECODE_ERROR);
        };
        let mut symbol = self.huff_tabs[t]
            .as_ref()
            .map_or(0, |h| h.look_up2((self.bit_buf >> 24) as usize));
        if symbol < 0 {
            let (s, ofs) = self.huff_tree_walk(t, symbol)?;
            symbol = s;
            self.get_bits_no_markers(8 + (23 - ofs))?;
            *extra_bits = self.get_bits_no_markers(symbol & 0xF)? as i32;
        } else {
            if symbol & 0x8000 != 0 {
                self.get_bits_no_markers((symbol >> 8) & 15)?;
                *extra_bits = symbol >> 16;
            } else {
                let code_size = (symbol >> 8) & 31;
                let num_extra_bits = symbol & 0xF;
                let bits = code_size + num_extra_bits;
                if bits <= 16 {
                    *extra_bits =
                        (self.get_bits_no_markers(bits)? & ((1u32 << num_extra_bits) - 1)) as i32;
                } else {
                    self.get_bits_no_markers(code_size)?;
                    *extra_bits = self.get_bits_no_markers(num_extra_bits)? as i32;
                }
            }
            symbol &= 0xFF;
        }
        Ok(symbol)
    }

    // -------------------------------------------------------------- markers

    /// `read_dht_marker`.
    #[allow(clippy::needless_range_loop)]
    fn read_dht_marker(&mut self) -> R<()> {
        let mut num_left = self.get_bits(16)?;
        if num_left < 2 {
            return Err(JPGD_BAD_DHT_MARKER);
        }
        num_left -= 2;
        while num_left != 0 {
            let mut index = self.get_bits(8)? as i32;
            let mut huff_num = [0u8; 17];
            let mut huff_val = [0u8; 256];
            let mut count = 0i32;
            for i in 1..=16 {
                huff_num[i] = self.get_bits(8)? as u8;
                count += i32::from(huff_num[i]);
            }
            if count > 255 {
                return Err(JPGD_BAD_DHT_COUNTS);
            }
            let mut symbol_present = [false; 256];
            for v in huff_val.iter_mut().take(count as usize) {
                let s = self.get_bits(8)? as usize;
                // Check for obviously bogus tables.
                if symbol_present[s] {
                    return Err(JPGD_BAD_DHT_COUNTS);
                }
                *v = s as u8;
                symbol_present[s] = true;
            }
            let i = 1 + 16 + count;
            if num_left < i as u32 {
                return Err(JPGD_BAD_DHT_MARKER);
            }
            num_left -= i as u32;
            index = (index & 0x0F) + ((index & 0x10) >> 4) * (JPGD_MAX_HUFF_TABLES as i32 >> 1);
            if index >= JPGD_MAX_HUFF_TABLES as i32 {
                return Err(JPGD_BAD_DHT_INDEX);
            }
            let index = index as usize;
            // (index & 0x10) of the remapped index: always 0
            self.huff_ac[index] = u8::from(index & 0x10 != 0);
            self.huff_num[index] = Some(huff_num);
            self.huff_val[index] = Some(huff_val);
        }
        Ok(())
    }

    /// `read_dqt_marker`.
    fn read_dqt_marker(&mut self) -> R<()> {
        let mut num_left = self.get_bits(16)?;
        if num_left < 2 {
            return Err(JPGD_BAD_DQT_MARKER);
        }
        num_left -= 2;
        while num_left != 0 {
            let mut n = self.get_bits(8)? as i32;
            let prec = n >> 4;
            n &= 0x0F;
            if n >= JPGD_MAX_QUANT_TABLES as i32 {
                return Err(JPGD_BAD_DQT_TABLE);
            }
            let mut q = self.quant[n as usize].unwrap_or([0; 64]);
            // read quantization entries, in zag order
            for v in q.iter_mut() {
                let mut temp = self.get_bits(8)?;
                if prec != 0 {
                    temp = (temp << 8).wrapping_add(self.get_bits(8)?);
                }
                *v = temp as i16;
            }
            self.quant[n as usize] = Some(q);
            let mut i = 64 + 1;
            if prec != 0 {
                i += 64;
            }
            if num_left < i {
                return Err(JPGD_BAD_DQT_LENGTH);
            }
            num_left -= i;
        }
        Ok(())
    }

    /// `read_sof_marker`.
    fn read_sof_marker(&mut self) -> R<()> {
        let num_left = self.get_bits(16)?;
        // precision: only 8-bit precision is supported
        if self.get_bits(8)? != 8 {
            return Err(JPGD_BAD_PRECISION);
        }
        self.image_y_size = self.get_bits(16)? as i32;
        if self.image_y_size < 1 || self.image_y_size > JPGD_MAX_HEIGHT {
            return Err(JPGD_BAD_HEIGHT);
        }
        self.image_x_size = self.get_bits(16)? as i32;
        if self.image_x_size < 1 || self.image_x_size > JPGD_MAX_WIDTH {
            return Err(JPGD_BAD_WIDTH);
        }
        self.comps_in_frame = self.get_bits(8)? as i32;
        if self.comps_in_frame > JPGD_MAX_COMPONENTS as i32 {
            return Err(JPGD_TOO_MANY_COMPONENTS);
        }
        if num_left != (self.comps_in_frame * 3 + 8) as u32 {
            return Err(JPGD_BAD_SOF_LENGTH);
        }
        for i in 0..self.comps_in_frame as usize {
            self.comp_ident[i] = self.get_bits(8)? as i32;
            self.comp_h_samp[i] = self.get_bits(4)? as i32;
            self.comp_v_samp[i] = self.get_bits(4)? as i32;
            let (h, v) = (self.comp_h_samp[i], self.comp_v_samp[i]);
            if h == 0 || v == 0 || h > 2 || v > 2 {
                return Err(JPGD_UNSUPPORTED_SAMP_FACTORS);
            }
            self.comp_quant[i] = self.get_bits(8)? as i32;
            if self.comp_quant[i] >= JPGD_MAX_QUANT_TABLES as i32 {
                return Err(JPGD_DECODE_ERROR);
            }
        }
        Ok(())
    }

    /// `skip_variable_marker`.
    fn skip_variable_marker(&mut self) -> R<()> {
        let mut num_left = self.get_bits(16)?;
        if num_left < 2 {
            return Err(JPGD_BAD_VARIABLE_MARKER);
        }
        num_left -= 2;
        while num_left != 0 {
            self.get_bits(8)?;
            num_left -= 1;
        }
        Ok(())
    }

    /// `read_dri_marker`.
    fn read_dri_marker(&mut self) -> R<()> {
        if self.get_bits(16)? != 4 {
            return Err(JPGD_BAD_DRI_LENGTH);
        }
        self.restart_interval = self.get_bits(16)? as i32;
        Ok(())
    }

    /// `read_sos_marker`.
    fn read_sos_marker(&mut self) -> R<()> {
        let mut num_left = self.get_bits(16)?;
        let n = self.get_bits(8)? as i32;
        self.comps_in_scan = n;
        num_left = num_left.wrapping_sub(3);
        if num_left != (n * 2 + 3) as u32 || n < 1 || n > JPGD_MAX_COMPS_IN_SCAN as i32 {
            return Err(JPGD_BAD_SOS_LENGTH);
        }
        for i in 0..n as usize {
            let cc = self.get_bits(8)? as i32;
            let c = self.get_bits(8)? as i32;
            num_left = num_left.wrapping_sub(2);
            let mut ci = 0;
            while ci < self.comps_in_frame {
                if cc == self.comp_ident[ci as usize] {
                    break;
                }
                ci += 1;
            }
            if ci >= self.comps_in_frame {
                return Err(JPGD_BAD_SOS_COMP_ID);
            }
            if ci >= JPGD_MAX_COMPONENTS as i32 {
                return Err(JPGD_DECODE_ERROR);
            }
            self.comp_list[i] = ci;
            let ci = ci as usize;
            self.comp_dc_tab[ci] = (c >> 4) & 15;
            self.comp_ac_tab[ci] = (c & 15) + (JPGD_MAX_HUFF_TABLES as i32 >> 1);
            if self.comp_dc_tab[ci] >= JPGD_MAX_HUFF_TABLES as i32
                || self.comp_ac_tab[ci] >= JPGD_MAX_HUFF_TABLES as i32
            {
                return Err(JPGD_DECODE_ERROR);
            }
        }
        self.spectral_start = self.get_bits(8)? as i32;
        self.spectral_end = self.get_bits(8)? as i32;
        self.successive_high = self.get_bits(4)? as i32;
        self.successive_low = self.get_bits(4)? as i32;
        if !self.progressive_flag {
            self.spectral_start = 0;
            self.spectral_end = 63;
        }
        num_left = num_left.wrapping_sub(3);
        // read past whatever is num_left
        while num_left != 0 {
            self.get_bits(8)?;
            num_left = num_left.wrapping_sub(1);
        }
        Ok(())
    }

    /// `next_marker`.
    fn next_marker(&mut self) -> R<i32> {
        let mut c;
        loop {
            loop {
                c = self.get_bits(8)?;
                if c == 0xFF {
                    break;
                }
            }
            loop {
                c = self.get_bits(8)?;
                if c != 0xFF {
                    break;
                }
            }
            if c != 0 {
                break;
            }
        }
        Ok(c as i32)
    }

    /// `process_markers`: returns at SOFx, SOI, EOI or SOS.
    fn process_markers(&mut self) -> R<i32> {
        loop {
            let c = self.next_marker()?;
            match c {
                M_SOF0 | M_SOF1 | M_SOF2 | M_SOF3 | M_SOF5 | M_SOF6 | M_SOF7 | M_SOF9 | M_SOF10
                | M_SOF11 | M_SOF13 | M_SOF14 | M_SOF15 | M_SOI | M_EOI | M_SOS => return Ok(c),
                M_DHT => self.read_dht_marker()?,
                // No arithmetic support
                M_DAC => return Err(JPGD_NO_ARITHMITIC_SUPPORT),
                M_DQT => self.read_dqt_marker()?,
                M_DRI => self.read_dri_marker()?,
                M_JPG | M_RST0..=M_RST7 | M_TEM => return Err(JPGD_UNEXPECTED_MARKER),
                // DNL, DHP, EXP, APPn, JPGn, COM, RESn
                _ => self.skip_variable_marker()?,
            }
        }
    }

    /// `locate_soi_marker`.
    fn locate_soi_marker(&mut self) -> R<()> {
        let mut lastchar = self.get_bits(8)?;
        let mut thischar = self.get_bits(8)?;
        // ok if it's a normal JPEG file without a special header
        if lastchar == 0xFF && thischar == M_SOI as u32 {
            return Ok(());
        }
        let mut bytesleft: u32 = 4096;
        loop {
            bytesleft -= 1;
            if bytesleft == 0 {
                return Err(JPGD_NOT_JPEG);
            }
            lastchar = thischar;
            thischar = self.get_bits(8)?;
            if lastchar == 0xFF {
                if thischar == M_SOI as u32 {
                    break;
                } else if thischar == M_EOI as u32 {
                    return Err(JPGD_NOT_JPEG);
                }
            }
        }
        // The next character after the marker must start a marker.
        thischar = (self.bit_buf >> 24) & 0xFF;
        if thischar != 0xFF {
            return Err(JPGD_NOT_JPEG);
        }
        Ok(())
    }

    /// `locate_sof_marker`.
    fn locate_sof_marker(&mut self) -> R<()> {
        self.locate_soi_marker()?;
        match self.process_markers()? {
            M_SOF2 => {
                self.progressive_flag = true;
                self.read_sof_marker()
            }
            // baseline DCT, extended sequential DCT
            M_SOF0 | M_SOF1 => self.read_sof_marker(),
            // Arithmetic coding
            M_SOF9 => Err(JPGD_NO_ARITHMITIC_SUPPORT),
            _ => Err(JPGD_UNSUPPORTED_MARKER),
        }
    }

    /// `locate_sos_marker`.
    fn locate_sos_marker(&mut self) -> R<bool> {
        let c = self.process_markers()?;
        if c == M_EOI {
            return Ok(false);
        } else if c != M_SOS {
            return Err(JPGD_UNEXPECTED_MARKER);
        }
        self.read_sos_marker()?;
        Ok(true)
    }

    /// `init` (the state is reset by `new`) + `locate_sof_marker`.
    fn decode_init(&mut self) -> R<()> {
        // Ready the input buffer.
        self.prep_in_buffer()?;
        // Prime the bit buffer.
        self.bits_left = 16;
        self.bit_buf = 0;
        self.get_bits(16)?;
        self.get_bits(16)?;
        self.locate_sof_marker()
    }

    /// `create_look_ups`: YCbCr to RGB tables.
    fn create_look_ups(&mut self) {
        const SCALEBITS: i32 = 16;
        const ONE_HALF: i32 = 1 << (SCALEBITS - 1);
        let fix = |x: f32| (x * (1i64 << SCALEBITS) as f32 + 0.5) as i32;
        for i in 0..256 {
            let k = i as i32 - 128;
            self.crr[i] = (fix(1.40200) * k + ONE_HALF) >> SCALEBITS;
            self.cbb[i] = (fix(1.77200) * k + ONE_HALF) >> SCALEBITS;
            self.crg[i] = (-fix(0.71414)) * k;
            self.cbg[i] = (-fix(0.34414)) * k + ONE_HALF;
        }
    }

    /// `fix_in_buffer`: throws back the bytes read into the bit buffer while
    /// scanning markers.
    fn fix_in_buffer(&mut self) -> R<()> {
        if self.bits_left == 16 {
            self.stuff_char((self.bit_buf & 0xFF) as u8);
        }
        if self.bits_left >= 8 {
            self.stuff_char(((self.bit_buf >> 8) & 0xFF) as u8);
        }
        self.stuff_char(((self.bit_buf >> 16) & 0xFF) as u8);
        self.stuff_char(((self.bit_buf >> 24) & 0xFF) as u8);
        self.bits_left = 16;
        self.get_bits_no_markers(16)?;
        self.get_bits_no_markers(16)?;
        Ok(())
    }

    /// `transform_mcu`: IDCT of the MCU's blocks into the sample buffer.
    fn transform_mcu(&mut self, mcu_row: i32) -> R<()> {
        if mcu_row.wrapping_mul(self.blocks_per_mcu) >= self.max_blocks_per_row {
            return Err(JPGD_DECODE_ERROR);
        }
        let base = (mcu_row * self.blocks_per_mcu * 64) as usize;
        for mcu_block in 0..self.blocks_per_mcu as usize {
            let src = &self.mcu_coefficients[mcu_block * 64..mcu_block * 64 + 64];
            let d = base + mcu_block * 64;
            if let Some(dst) = self.sample_buf.get_mut(d..d + 64) {
                idct(src, dst, self.mcu_block_max_zag[mcu_block]);
            }
        }
        Ok(())
    }

    /// `coeff_buf_getp`: index of a block's first coefficient.
    fn coeff_buf_getp(cb: &CoeffBuf, block_x: i32, block_y: i32) -> R<usize> {
        if block_x >= cb.block_num_x || block_y >= cb.block_num_y {
            return Err(JPGD_DECODE_ERROR);
        }
        Ok((block_x * cb.block_len + block_y * (cb.block_len * cb.block_num_x)) as usize)
    }

    /// `load_next_row`: dequantizes the next row of a progressive image.
    fn load_next_row(&mut self) -> R<()> {
        let mut block_x_mcu = [0i32; JPGD_MAX_COMPONENTS];
        for mcu_row in 0..self.mcus_per_row {
            let mut block_x_mcu_ofs = 0;
            let mut block_y_mcu_ofs = 0;
            for mcu_block in 0..self.blocks_per_mcu as usize {
                let component_id = self.mcu_org[mcu_block] as usize;
                let cq = self.comp_quant.get(component_id).copied().unwrap_or(0);
                if cq >= JPGD_MAX_QUANT_TABLES as i32 {
                    return Err(JPGD_DECODE_ERROR);
                }
                let q = self.quant[cq as usize].unwrap_or([0; 64]);
                let bx = block_x_mcu[component_id] + block_x_mcu_ofs;
                let by = self.block_y_mcu[component_id] + block_y_mcu_ofs;
                let (Some(ac), Some(dc)) =
                    (&self.ac_coeffs[component_id], &self.dc_coeffs[component_id])
                else {
                    return Err(JPGD_DECODE_ERROR);
                };
                let pac = Self::coeff_buf_getp(ac, bx, by)?;
                let pdc = Self::coeff_buf_getp(dc, bx, by)?;
                let p = &mut self.mcu_coefficients[64 * mcu_block..64 * mcu_block + 64];
                p[0] = dc.data[pdc];
                p[1..64].copy_from_slice(&ac.data[pac + 1..pac + 64]);
                let mut i = 63i32;
                while i > 0 {
                    if p[G_ZAG[i as usize]] != 0 {
                        break;
                    }
                    i -= 1;
                }
                self.mcu_block_max_zag[mcu_block] = i + 1;
                while i >= 0 {
                    let z = G_ZAG[i as usize];
                    if p[z] != 0 {
                        p[z] = i32::from(p[z]).wrapping_mul(i32::from(q[i as usize])) as i16;
                    }
                    i -= 1;
                }
                if self.comps_in_scan == 1 {
                    block_x_mcu[component_id] += 1;
                } else {
                    block_x_mcu_ofs += 1;
                    if block_x_mcu_ofs == self.comp_h_samp[component_id] {
                        block_x_mcu_ofs = 0;
                        block_y_mcu_ofs += 1;
                        if block_y_mcu_ofs == self.comp_v_samp[component_id] {
                            block_y_mcu_ofs = 0;
                            block_x_mcu[component_id] += self.comp_h_samp[component_id];
                        }
                    }
                }
            }
            self.transform_mcu(mcu_row)?;
        }
        if self.comps_in_scan == 1 {
            self.block_y_mcu[self.comp_list[0] as usize] += 1;
        } else {
            for component_num in 0..self.comps_in_scan as usize {
                let component_id = self.comp_list[component_num] as usize;
                self.block_y_mcu[component_id] += self.comp_v_samp[component_id];
            }
        }
        Ok(())
    }

    /// `process_restart`.
    fn process_restart(&mut self) -> R<()> {
        let mut c = 0u32;
        let mut i = 1536;
        // Scan a little bit to find the marker, but not too far.
        while i > 0 {
            if self.get_char()? == 0xFF {
                break;
            }
            i -= 1;
        }
        if i == 0 {
            return Err(JPGD_BAD_RESTART_MARKER);
        }
        while i > 0 {
            c = self.get_char()?;
            if c != 0xFF {
                break;
            }
            i -= 1;
        }
        if i == 0 {
            return Err(JPGD_BAD_RESTART_MARKER);
        }
        // Is it the expected marker?
        if c as i32 != self.next_restart_num + M_RST0 {
            return Err(JPGD_BAD_RESTART_MARKER);
        }
        // Reset each component's DC prediction values.
        for v in self
            .last_dc_val
            .iter_mut()
            .take(self.comps_in_frame as usize)
        {
            *v = 0;
        }
        self.eob_run = 0;
        self.restarts_left = self.restart_interval;
        self.next_restart_num = (self.next_restart_num + 1) & 7;
        // Get the bit buffer going again...
        self.bits_left = 16;
        self.get_bits_no_markers(16)?;
        self.get_bits_no_markers(16)?;
        Ok(())
    }

    /// `decode_next_row`: decodes and dequantizes the next row (sequential).
    fn decode_next_row(&mut self) -> R<()> {
        for mcu_row in 0..self.mcus_per_row {
            if self.restart_interval != 0 && self.restarts_left == 0 {
                self.process_restart()?;
            }
            for mcu_block in 0..self.blocks_per_mcu as usize {
                let component_id = self.mcu_org[mcu_block] as usize;
                let cq = self.comp_quant.get(component_id).copied().unwrap_or(0);
                if cq >= JPGD_MAX_QUANT_TABLES as i32 {
                    return Err(JPGD_DECODE_ERROR);
                }
                let q = self.quant[cq as usize].unwrap_or([0; 64]);
                let mut r = 0i32;
                let dc_tab = self.comp_dc_tab[component_id] as usize;
                let mut s = self.huff_decode_extra(Some(dc_tab), &mut r)?;
                if s >= 16 {
                    return Err(JPGD_DECODE_ERROR);
                }
                s = huff_extend(r, s);
                let dc = (s as u32).wrapping_add(self.last_dc_val[component_id]);
                self.last_dc_val[component_id] = dc;
                s = dc as i32;
                let pbase = mcu_block * 64;
                self.mcu_coefficients[pbase] = s.wrapping_mul(i32::from(q[0])) as i16;

                let prev_num_set = self.mcu_block_max_zag[mcu_block];
                let ac_tab = self.comp_ac_tab[component_id] as usize;
                let mut k = 1i32;
                while k < 64 {
                    let mut extra_bits = 0i32;
                    s = self.huff_decode_extra(Some(ac_tab), &mut extra_bits)?;
                    r = s >> 4;
                    s &= 15;
                    let p = &mut self.mcu_coefficients[pbase..pbase + 64];
                    if s != 0 {
                        if r != 0 {
                            if k + r > 63 {
                                return Err(JPGD_DECODE_ERROR);
                            }
                            if k < prev_num_set {
                                let n = r.min(prev_num_set - k);
                                for kt in k..k + n {
                                    p[G_ZAG[kt as usize]] = 0;
                                }
                            }
                            k += r;
                        }
                        s = huff_extend(extra_bits, s);
                        if k >= 64 {
                            return Err(JPGD_DECODE_ERROR);
                        }
                        p[G_ZAG[k as usize]] = s.wrapping_mul(i32::from(q[k as usize])) as i16;
                    } else if r == 15 {
                        if k + 16 > 64 {
                            return Err(JPGD_DECODE_ERROR);
                        }
                        if k < prev_num_set {
                            let n = 16.min(prev_num_set - k);
                            for kt in k..k + n {
                                if kt > 63 {
                                    return Err(JPGD_DECODE_ERROR);
                                }
                                p[G_ZAG[kt as usize]] = 0;
                            }
                        }
                        k += 16 - 1; // - 1 because the loop counter is k
                        if p[G_ZAG[(k & 63) as usize]] != 0 {
                            return Err(JPGD_DECODE_ERROR);
                        }
                    } else {
                        break;
                    }
                    k += 1;
                }
                if k < prev_num_set {
                    let p = &mut self.mcu_coefficients[pbase..pbase + 64];
                    for kt in k..prev_num_set {
                        p[G_ZAG[kt as usize]] = 0;
                    }
                }
                self.mcu_block_max_zag[mcu_block] = k;
            }
            self.transform_mcu(mcu_row)?;
            self.restarts_left = self.restarts_left.wrapping_sub(1);
        }
        Ok(())
    }

    fn sb(&self, i: i32) -> i32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.sample_buf.get(i))
            .map_or(0, |&v| i32::from(v))
    }

    fn sbp(&self, prev: bool, i: i32) -> i32 {
        let b = if prev {
            &self.sample_buf_prev
        } else {
            &self.sample_buf
        };
        usize::try_from(i)
            .ok()
            .and_then(|i| b.get(i))
            .map_or(0, |&v| i32::from(v))
    }

    /// RGBA of one pixel from Y, Cb, Cr.
    fn ycc(&self, y: i32, cb: i32, cr: i32) -> [u8; 4] {
        let cb = (cb & 0xff) as usize;
        let cr = (cr & 0xff) as usize;
        let rc = self.crr[cr];
        let gc = self.crg[cr].wrapping_add(self.cbg[cb]) >> 16;
        let bc = self.cbb[cb];
        [clamp(y + rc), clamp(y + gc), clamp(y + bc), 255]
    }

    fn put4(line: &mut [u8], at: usize, px: [u8; 4]) {
        if let Some(d) = line.get_mut(at..at + 4) {
            d.copy_from_slice(&px);
        }
    }

    /// `H1V1Convert`.
    fn h1v1_convert(&mut self) {
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut line = std::mem::take(&mut self.scan_line_0);
        let mut d = 0usize;
        let mut s = row * 8;
        for _ in 0..self.max_mcus_per_row {
            for j in 0..8 {
                let px = self.ycc(self.sb(s + j), self.sb(s + 64 + j), self.sb(s + 128 + j));
                Self::put4(&mut line, d, px);
                d += 4;
            }
            s += 64 * 3;
        }
        self.scan_line_0 = line;
    }

    /// `H2V1Convert`.
    fn h2v1_convert(&mut self) {
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut line = std::mem::take(&mut self.scan_line_0);
        let mut d0 = 0usize;
        let mut y = row * 8;
        let mut c = 2 * 64 + row * 8;
        for _ in 0..self.max_mcus_per_row {
            for _ in 0..2 {
                for j in 0..4 {
                    let cb = self.sb(c);
                    let cr = self.sb(c + 64);
                    Self::put4(&mut line, d0, self.ycc(self.sb(y + (j << 1)), cb, cr));
                    Self::put4(
                        &mut line,
                        d0 + 4,
                        self.ycc(self.sb(y + (j << 1) + 1), cb, cr),
                    );
                    d0 += 8;
                    c += 1;
                }
                y += 64;
            }
            y += 64 * 4 - 64 * 2;
            c += 64 * 4 - 8;
        }
        self.scan_line_0 = line;
    }

    /// `H2V1ConvertFiltered`.
    fn h2v1_convert_filtered(&mut self) {
        const BPM: i32 = 4;
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut line = std::mem::take(&mut self.scan_line_0);
        let half_image_x_size = (self.image_x_size >> 1) - 1;
        let row_x8 = row * 8;
        for x in 0..self.image_x_size {
            let y =
                self.sb((x >> 4) * BPM * 64 + if x & 8 != 0 { 64 } else { 0 } + (x & 7) + row_x8);
            let mut c_x0 = (x - 1) >> 1;
            let c_x1 = (c_x0 + 1).min(half_image_x_size);
            c_x0 = c_x0.max(0);
            let a = (c_x0 >> 3) * BPM * 64 + (c_x0 & 7) + row_x8 + 128;
            let (cb0, cr0) = (self.sb(a), self.sb(a + 64));
            let b = (c_x1 >> 3) * BPM * 64 + (c_x1 & 7) + row_x8 + 128;
            let (cb1, cr1) = (self.sb(b), self.sb(b + 64));
            let w0 = if x & 1 != 0 { 3 } else { 1 };
            let w1 = if x & 1 != 0 { 1 } else { 3 };
            let cb = (cb0 * w0 + cb1 * w1 + 2) >> 2;
            let cr = (cr0 * w0 + cr1 * w1 + 2) >> 2;
            Self::put4(&mut line, (x * 4) as usize, self.ycc(y, cb, cr));
        }
        self.scan_line_0 = line;
    }

    /// `H1V2Convert`.
    fn h1v2_convert(&mut self) {
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut l0 = std::mem::take(&mut self.scan_line_0);
        let mut l1 = std::mem::take(&mut self.scan_line_1);
        let mut d = 0usize;
        let mut y = if row < 8 { row * 8 } else { 64 + (row & 7) * 8 };
        let mut c = 64 * 2 + (row >> 1) * 8;
        for _ in 0..self.max_mcus_per_row {
            for j in 0..8 {
                let cb = self.sb(c + j);
                let cr = self.sb(c + 64 + j);
                Self::put4(&mut l0, d, self.ycc(self.sb(y + j), cb, cr));
                Self::put4(&mut l1, d, self.ycc(self.sb(y + 8 + j), cb, cr));
                d += 4;
            }
            y += 64 * 4;
            c += 64 * 4;
        }
        self.scan_line_0 = l0;
        self.scan_line_1 = l1;
    }

    /// `H1V2ConvertFiltered`.
    fn h1v2_convert_filtered(&mut self) {
        const BPM: i32 = 4;
        let y = self.image_y_size - self.total_lines_left();
        let row = y & 15;
        let half_image_y_size = (self.image_y_size >> 1) - 1;
        let mut line = std::mem::take(&mut self.scan_line_0);
        let w0 = if row & 1 != 0 { 3 } else { 1 };
        let w1 = if row & 1 != 0 { 1 } else { 3 };
        let c_y0 = (y - 1) >> 1;
        let c_y1 = (c_y0 + 1).min(half_image_y_size);
        let mut y_prev = false;
        let mut c0_prev = false;
        if c_y0 >= 0 && ((row & 15) == 0 || (row & 15) == 15) && self.total_lines_left() > 1 {
            if (row & 15) == 15 {
                y_prev = true;
            }
            c0_prev = true;
        }
        let y_sample_base_ofs = if row & 8 != 0 { 64 } else { 0 } + (row & 7) * 8;
        let y0_base = (c_y0 & 7) * 8 + 128;
        let y1_base = (c_y1 & 7) * 8 + 128;
        for x in 0..self.image_x_size {
            let base_ofs = (x >> 3) * BPM * 64 + (x & 7);
            let y_sample = self.sbp(y_prev, base_ofs + y_sample_base_ofs);
            let a = base_ofs + y0_base;
            let (cb0, cr0) = (self.sbp(c0_prev, a), self.sbp(c0_prev, a + 64));
            let b = base_ofs + y1_base;
            let (cb1, cr1) = (self.sb(b), self.sb(b + 64));
            let cb = (cb0 * w0 + cb1 * w1 + 2) >> 2;
            let cr = (cr0 * w0 + cr1 * w1 + 2) >> 2;
            Self::put4(&mut line, (x * 4) as usize, self.ycc(y_sample, cb, cr));
        }
        self.scan_line_0 = line;
    }

    /// `H2V2Convert`.
    fn h2v2_convert(&mut self) {
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut l0 = std::mem::take(&mut self.scan_line_0);
        let mut l1 = std::mem::take(&mut self.scan_line_1);
        let mut d = 0usize;
        let mut y = if row < 8 {
            row * 8
        } else {
            64 * 2 + (row & 7) * 8
        };
        let mut c = 64 * 4 + (row >> 1) * 8;
        for _ in 0..self.max_mcus_per_row {
            for _ in 0..2 {
                for j in (0..8).step_by(2) {
                    let cb = self.sb(c);
                    let cr = self.sb(c + 64);
                    Self::put4(&mut l0, d, self.ycc(self.sb(y + j), cb, cr));
                    Self::put4(&mut l0, d + 4, self.ycc(self.sb(y + j + 1), cb, cr));
                    Self::put4(&mut l1, d, self.ycc(self.sb(y + j + 8), cb, cr));
                    Self::put4(&mut l1, d + 4, self.ycc(self.sb(y + j + 8 + 1), cb, cr));
                    d += 8;
                    c += 1;
                }
                y += 64;
            }
            y += 64 * 6 - 64 * 2;
            c += 64 * 6 - 8;
        }
        self.scan_line_0 = l0;
        self.scan_line_1 = l1;
    }

    /// `H2V2ConvertFiltered`: returns the number of scan lines produced.
    fn h2v2_convert_filtered(&mut self) -> i32 {
        const BPM: i32 = 6;
        const S_MULS: [[[i32; 4]; 2]; 2] =
            [[[1, 3, 3, 9], [3, 9, 1, 3]], [[3, 1, 9, 3], [9, 3, 3, 1]]];
        let y = self.image_y_size - self.total_lines_left();
        let row = y & 15;
        let half_image_y_size = (self.image_y_size >> 1) - 1;
        let c_y0 = (y - 1) >> 1;
        let c_y1 = (c_y0 + 1).min(half_image_y_size);
        let mut y_prev = false;
        let mut c0_prev = false;
        if c_y0 >= 0 && ((row & 15) == 0 || (row & 15) == 15) && self.total_lines_left() > 1 {
            if (row & 15) == 15 {
                y_prev = true;
            }
            c0_prev = true;
        }
        let y_sample_base_ofs = if row & 8 != 0 { 128 } else { 0 } + (row & 7) * 8;
        let y0_base = (c_y0 & 7) * 8 + 256;
        let y1_base = (c_y1 & 7) * 8 + 256;
        let half_image_x_size = (self.image_x_size >> 1) - 1;

        // Chroma samples around column x: (cb00, cr00, cb01, cr01, cb10, cr10, cb11, cr11).
        let chroma = |d: &Self, x: i32| -> [i32; 8] {
            let mut c_x0 = (x - 1) >> 1;
            let c_x1 = (c_x0 + 1).min(half_image_x_size);
            c_x0 = c_x0.max(0);
            let a = (c_x0 >> 3) * BPM * 64 + (c_x0 & 7);
            let b = (c_x1 >> 3) * BPM * 64 + (c_x1 & 7);
            [
                d.sbp(c0_prev, a + y0_base),
                d.sbp(c0_prev, a + y0_base + 64),
                d.sb(a + y1_base),
                d.sb(a + y1_base + 64),
                d.sbp(c0_prev, b + y0_base),
                d.sbp(c0_prev, b + y0_base + 64),
                d.sb(b + y1_base),
                d.sb(b + y1_base + 64),
            ]
        };
        let pixel = |d: &Self, ys: i32, c: &[i32; 8], m: &[i32; 4]| -> [u8; 4] {
            let cb = (c[0] * m[0] + c[2] * m[1] + c[4] * m[2] + c[6] * m[3] + 8) >> 4;
            let cr = (c[1] * m[0] + c[3] * m[1] + c[5] * m[2] + c[7] * m[3] + 8) >> 4;
            d.ycc(ys, cb, cr)
        };
        let ykey = |x: i32| (x >> 4) * BPM * 64 + if x & 8 != 0 { 64 } else { 0 } + (x & 7);

        let mut l0 = std::mem::take(&mut self.scan_line_0);
        let mut l1 = std::mem::take(&mut self.scan_line_1);

        let result = if (1..=14).contains(&(row & 15)) {
            let y_sample_base_ofs1 = if (row + 1) & 8 != 0 { 128 } else { 0 } + ((row + 1) & 7) * 8;
            let mut d0 = 0usize;
            let mut d1 = 0usize;
            let mut x = 0;
            while x < self.image_x_size {
                let k = ykey(x);
                let y0 = self.sbp(y_prev, k + y_sample_base_ofs);
                let y1 = self.sbp(y_prev, k + y_sample_base_ofs1);
                let c = chroma(self, x);
                Self::put4(
                    &mut l0,
                    d0,
                    pixel(self, y0, &c, &S_MULS[(row & 1) as usize][(x & 1) as usize]),
                );
                d0 += 4;
                Self::put4(
                    &mut l1,
                    d1,
                    pixel(
                        self,
                        y1,
                        &c,
                        &S_MULS[((row + 1) & 1) as usize][(x & 1) as usize],
                    ),
                );
                d1 += 4;
                if (x & 1) == 1 && x < self.image_x_size - 1 {
                    let nx = x + 1;
                    let k = ykey(nx);
                    let y0 = self.sbp(y_prev, k + y_sample_base_ofs);
                    let y1 = self.sbp(y_prev, k + y_sample_base_ofs1);
                    Self::put4(
                        &mut l0,
                        d0,
                        pixel(self, y0, &c, &S_MULS[(row & 1) as usize][(nx & 1) as usize]),
                    );
                    d0 += 4;
                    Self::put4(
                        &mut l1,
                        d1,
                        pixel(
                            self,
                            y1,
                            &c,
                            &S_MULS[((row + 1) & 1) as usize][(nx & 1) as usize],
                        ),
                    );
                    d1 += 4;
                    x += 1;
                }
                x += 1;
            }
            2
        } else {
            for x in 0..self.image_x_size {
                let ys = self.sbp(y_prev, ykey(x) + y_sample_base_ofs);
                let c = chroma(self, x);
                Self::put4(
                    &mut l0,
                    (x * 4) as usize,
                    pixel(self, ys, &c, &S_MULS[(row & 1) as usize][(x & 1) as usize]),
                );
            }
            1
        };
        self.scan_line_0 = l0;
        self.scan_line_1 = l1;
        result
    }

    /// `gray_convert`.
    fn gray_convert(&mut self) {
        let row = self.max_mcu_y_size - self.mcu_org[ORG_MCU_LINES_LEFT];
        let mut line = std::mem::take(&mut self.scan_line_0);
        let mut s = row * 8;
        let mut d = 0usize;
        for _ in 0..self.max_mcus_per_row {
            for j in 0..8 {
                if let Some(v) = line.get_mut(d + j) {
                    *v = self.sb(s + j as i32) as u8;
                }
            }
            s += 64;
            d += 8;
        }
        self.scan_line_0 = line;
    }

    /// `find_eoi`.
    fn find_eoi(&mut self) -> R<()> {
        if !self.progressive_flag {
            // Prime the bit buffer
            self.bits_left = 16;
            self.get_bits(16)?;
            self.get_bits(16)?;
            // The next marker _should_ be EOI
            self.process_markers()?;
        }
        self.total_bytes_read = self.total_bytes_read.wrapping_sub(self.in_buf_left);
        Ok(())
    }

    fn chroma_y_filtering(&self) -> bool {
        (self.flags & Self::C_FLAG_LINEAR_CHROMA_FILTERING) != 0
            && (self.scan_type == JPGD_YH2V2 || self.scan_type == JPGD_YH1V2)
            && self.image_x_size >= 2
            && self.image_y_size >= 2
    }

    /// `decode_next_mcu_row`.
    fn decode_next_mcu_row(&mut self) -> R<()> {
        if self.chroma_y_filtering() {
            std::mem::swap(&mut self.sample_buf, &mut self.sample_buf_prev);
            self.sample_buf_prev_valid = true;
        }
        if self.progressive_flag {
            self.load_next_row()?;
        } else {
            self.decode_next_row()?;
        }
        // Find the EOI marker if that was the last row.
        if self.total_lines_left() <= self.max_mcu_y_size {
            self.find_eoi()?;
        }
        self.mcu_org[ORG_MCU_LINES_LEFT] = self.max_mcu_y_size;
        Ok(())
    }

    /// `decode`: the next scan line (RGBA or Y). Returns the status and the
    /// line (`m_real_dest_bytes_per_scan_line` bytes) on success.
    pub fn decode(&mut self) -> (i32, &[u8]) {
        if self.error_code != 0 || !self.ready_flag {
            return (JPGD_FAILED, &[]);
        }
        if self.total_lines_left() == 0 {
            return (JPGD_DONE, &[]);
        }
        let chroma_y_filtering = self.chroma_y_filtering();
        let mut get_another_mcu_row = false;
        let mut got_mcu_early = false;
        if chroma_y_filtering {
            if self.total_lines_left() == self.image_y_size {
                get_another_mcu_row = true;
            } else if self.mcu_org[ORG_MCU_LINES_LEFT] == 1 && self.total_lines_left() > 1 {
                get_another_mcu_row = true;
                got_mcu_early = true;
            }
        } else {
            get_another_mcu_row = self.mcu_org[ORG_MCU_LINES_LEFT] == 0;
        }
        if get_another_mcu_row {
            let r = self.decode_next_mcu_row();
            if self.catch(r).is_none() {
                return (JPGD_FAILED, &[]);
            }
        }

        let filtering = (self.flags & Self::C_FLAG_LINEAR_CHROMA_FILTERING) != 0
            && self.image_x_size >= 2
            && self.image_y_size >= 2;
        let mut line1 = false;
        match self.scan_type {
            JPGD_YH2V2 => {
                if filtering {
                    let nb = self.mcu_org[ORG_NUM_BUFFERED_SCANLINES];
                    if nb == 1 {
                        line1 = true;
                    } else if nb == 0 {
                        self.mcu_org[ORG_NUM_BUFFERED_SCANLINES] = self.h2v2_convert_filtered();
                    }
                    self.mcu_org[ORG_NUM_BUFFERED_SCANLINES] -= 1;
                } else if (self.mcu_org[ORG_MCU_LINES_LEFT] & 1) == 0 {
                    self.h2v2_convert();
                } else {
                    line1 = true;
                }
            }
            JPGD_YH2V1 => {
                if filtering {
                    self.h2v1_convert_filtered();
                } else {
                    self.h2v1_convert();
                }
            }
            JPGD_YH1V2 => {
                if chroma_y_filtering {
                    self.h1v2_convert_filtered();
                } else if (self.mcu_org[ORG_MCU_LINES_LEFT] & 1) == 0 {
                    self.h1v2_convert();
                } else {
                    line1 = true;
                }
            }
            JPGD_YH1V1 => self.h1v1_convert(),
            JPGD_GRAYSCALE => self.gray_convert(),
            _ => {}
        }

        if !got_mcu_early {
            self.mcu_org[ORG_MCU_LINES_LEFT] -= 1;
        }
        self.mcu_org[ORG_TOTAL_LINES_LEFT] -= 1;

        let len = usize::try_from(self.mcu_org[ORG_REAL_DEST_BYTES_PER_SCAN_LINE]).unwrap_or(0);
        let line = if line1 {
            &self.scan_line_1
        } else {
            &self.scan_line_0
        };
        (JPGD_SUCCESS, line.get(..len).unwrap_or(line))
    }

    /// `make_huff_table`.
    #[allow(clippy::needless_range_loop)]
    fn make_huff_table(&mut self, index: usize) -> R<()> {
        let huff_num = self.huff_num[index].unwrap_or([0; 17]);
        let huff_val = self.huff_val[index].unwrap_or([0; 256]);
        let ac = self.huff_ac[index] != 0;
        let mut huffsize = [0u8; 258];
        let mut huffcode = [0u32; 258];
        let mut p = 0usize;
        for l in 1..=16 {
            for _ in 1..=huff_num[l] {
                if p >= 257 {
                    return Err(JPGD_DECODE_ERROR);
                }
                huffsize[p] = l as u8;
                p += 1;
            }
        }
        huffsize[p] = 0;
        let lastp = p;
        let mut code: u32 = 0;
        let mut si = u32::from(huffsize[0]);
        p = 0;
        while huffsize[p] != 0 {
            while u32::from(huffsize[p]) == si {
                if p >= 257 {
                    return Err(JPGD_DECODE_ERROR);
                }
                huffcode[p] = code;
                p += 1;
                code = code.wrapping_add(1);
            }
            code = code.wrapping_shl(1);
            si += 1;
        }

        let mut h = self.huff_tabs[index].take().unwrap_or_else(HuffTables::new);
        h.w[0] = u32::from(ac);
        h.w[HT_LOOK_UP..].fill(0);
        let r = (|| -> R<()> {
            let mut nextfreeentry: i32 = -1;
            for p in 0..lastp {
                let i = i32::from(huff_val[p]);
                let mut code = huffcode[p];
                let code_size = i32::from(huffsize[p]);
                h.set_code_size(i as usize, code_size as u8);
                if code_size <= 8 {
                    code <<= 8 - code_size;
                    for _ in 0..(1 << (8 - code_size)) {
                        if code >= 256 {
                            return Err(JPGD_DECODE_ERROR);
                        }
                        h.w[HT_LOOK_UP + code as usize] = i as u32;
                        let mut has_extrabits = false;
                        let mut extra_bits = 0i32;
                        let num_extra_bits = i & 15;
                        let mut bits_to_fetch = code_size;
                        if num_extra_bits != 0 {
                            let total_codesize = code_size + num_extra_bits;
                            if total_codesize <= 8 {
                                has_extrabits = true;
                                extra_bits = ((1 << num_extra_bits) - 1)
                                    & (code >> (8 - total_codesize)) as i32;
                                if extra_bits > 0x7FFF {
                                    return Err(JPGD_DECODE_ERROR);
                                }
                                bits_to_fetch += num_extra_bits;
                            }
                        }
                        h.w[HT_LOOK_UP2 + code as usize] = if !has_extrabits {
                            (i | (bits_to_fetch << 8)) as u32
                        } else {
                            (i | 0x8000 | (extra_bits << 16) | (bits_to_fetch << 8)) as u32
                        };
                        code += 1;
                    }
                } else {
                    let subtree = ((code >> (code_size - 8)) & 0xFF) as usize;
                    let mut currententry = h.look_up(subtree);
                    if currententry == 0 {
                        h.w[HT_LOOK_UP + subtree] = nextfreeentry as u32;
                        h.w[HT_LOOK_UP2 + subtree] = nextfreeentry as u32;
                        currententry = nextfreeentry;
                        nextfreeentry -= 2;
                    }
                    code = code.wrapping_shl((16 - (code_size - 8)) as u32);
                    let mut l = code_size;
                    while l > 9 {
                        if (code & 0x8000) == 0 {
                            currententry = currententry.wrapping_sub(1);
                        }
                        let idx = currententry.wrapping_neg().wrapping_sub(1) as u32 as usize;
                        if idx >= JPGD_HUFF_TREE_MAX_LENGTH {
                            return Err(JPGD_DECODE_ERROR);
                        }
                        if h.tree(idx) == 0 {
                            h.w[HT_TREE + idx] = nextfreeentry as u32;
                            currententry = nextfreeentry;
                            nextfreeentry -= 2;
                        } else {
                            currententry = h.tree(idx);
                        }
                        code = code.wrapping_shl(1);
                        l -= 1;
                    }
                    if (code & 0x8000) == 0 {
                        currententry = currententry.wrapping_sub(1);
                    }
                    let t = i64::from(currententry.wrapping_neg().wrapping_sub(1));
                    if t >= JPGD_HUFF_TREE_MAX_LENGTH as i64 {
                        return Err(JPGD_DECODE_ERROR);
                    }
                    // A negative index writes before the tree inside the
                    // struct (undefined behavior outside it).
                    match h.tree_slot(t) {
                        Some(slot) => *slot = i as u32,
                        None => return Err(JPGD_DECODE_ERROR),
                    }
                }
            }
            Ok(())
        })();
        self.huff_tabs[index] = Some(h);
        r
    }

    /// `check_quant_tables`.
    fn check_quant_tables(&self) -> R<()> {
        for i in 0..self.comps_in_scan as usize {
            let q = self.comp_quant[self.comp_list[i] as usize];
            if self.quant.get(q as usize).is_none_or(Option::is_none) {
                return Err(JPGD_UNDEFINED_QUANT_TABLE);
            }
        }
        Ok(())
    }

    /// `check_huff_tables`.
    fn check_huff_tables(&mut self) -> R<()> {
        for i in 0..self.comps_in_scan as usize {
            let c = self.comp_list[i] as usize;
            if self.spectral_start == 0 && self.huff_num[self.comp_dc_tab[c] as usize].is_none() {
                return Err(JPGD_UNDEFINED_HUFF_TABLE);
            }
            if self.spectral_end > 0 && self.huff_num[self.comp_ac_tab[c] as usize].is_none() {
                return Err(JPGD_UNDEFINED_HUFF_TABLE);
            }
        }
        for i in 0..JPGD_MAX_HUFF_TABLES {
            if self.huff_num[i].is_some() {
                self.make_huff_table(i)?;
            }
        }
        Ok(())
    }

    /// `calc_mcu_block_order`: component order inside each MCU.
    fn calc_mcu_block_order(&mut self) -> bool {
        let mut max_h_samp = 0;
        let mut max_v_samp = 0;
        let nf = self.comps_in_frame.clamp(0, 4) as usize;
        for c in 0..nf {
            max_h_samp = max_h_samp.max(self.comp_h_samp[c]);
            max_v_samp = max_v_samp.max(self.comp_v_samp[c]);
        }
        for c in 0..nf {
            self.comp_h_blocks[c] = ((self.image_x_size * self.comp_h_samp[c] + (max_h_samp - 1))
                / max_h_samp.max(1)
                + 7)
                / 8;
            self.comp_v_blocks[c] = ((self.image_y_size * self.comp_v_samp[c] + (max_v_samp - 1))
                / max_v_samp.max(1)
                + 7)
                / 8;
        }
        if self.comps_in_scan == 1 {
            self.mcus_per_row = self.comp_h_blocks[self.comp_list[0] as usize];
            self.mcus_per_col = self.comp_v_blocks[self.comp_list[0] as usize];
        } else {
            self.mcus_per_row =
                ((self.image_x_size + 7) / 8 + (max_h_samp - 1)) / max_h_samp.max(1);
            self.mcus_per_col =
                ((self.image_y_size + 7) / 8 + (max_v_samp - 1)) / max_v_samp.max(1);
        }
        if self.comps_in_scan == 1 {
            self.mcu_org[0] = self.comp_list[0];
            self.blocks_per_mcu = 1;
        } else {
            self.blocks_per_mcu = 0;
            for component_num in 0..self.comps_in_scan as usize {
                let component_id = self.comp_list[component_num];
                let c = component_id as usize;
                let num_blocks = self.comp_h_samp[c] * self.comp_v_samp[c];
                for _ in 0..num_blocks {
                    // m_mcu_org[10] overflows into the members after it
                    if let Some(v) = self.mcu_org.get_mut(self.blocks_per_mcu as usize) {
                        *v = component_id;
                    }
                    self.blocks_per_mcu += 1;
                }
            }
        }
        if self.blocks_per_mcu > self.max_blocks_per_mcu {
            return false;
        }
        for mcu_block in 0..self.blocks_per_mcu as usize {
            if self.mcu_org[mcu_block] >= JPGD_MAX_QUANT_TABLES as i32 {
                return false;
            }
        }
        true
    }

    /// `init_scan`.
    fn init_scan(&mut self) -> R<bool> {
        if !self.locate_sos_marker()? {
            return Ok(false);
        }
        if !self.calc_mcu_block_order() {
            return Ok(false);
        }
        self.check_huff_tables()?;
        self.check_quant_tables()?;
        for v in self
            .last_dc_val
            .iter_mut()
            .take(self.comps_in_frame as usize)
        {
            *v = 0;
        }
        self.eob_run = 0;
        if self.restart_interval != 0 {
            self.restarts_left = self.restart_interval;
            self.next_restart_num = 0;
        }
        self.fix_in_buffer()?;
        Ok(true)
    }

    /// `init_frame`: checks the components / sampling factors and allocates
    /// the buffers.
    fn init_frame(&mut self) -> R<()> {
        if self.comps_in_frame == 1 {
            if self.comp_h_samp[0] != 1 || self.comp_v_samp[0] != 1 {
                return Err(JPGD_UNSUPPORTED_SAMP_FACTORS);
            }
            self.scan_type = JPGD_GRAYSCALE;
            self.max_blocks_per_mcu = 1;
            self.max_mcu_x_size = 8;
            self.max_mcu_y_size = 8;
        } else if self.comps_in_frame == 3 {
            if self.comp_h_samp[1] != 1
                || self.comp_v_samp[1] != 1
                || self.comp_h_samp[2] != 1
                || self.comp_v_samp[2] != 1
            {
                return Err(JPGD_UNSUPPORTED_SAMP_FACTORS);
            }
            let (st, mb, mx, my) = match (self.comp_h_samp[0], self.comp_v_samp[0]) {
                (1, 1) => (JPGD_YH1V1, 3, 8, 8),
                (2, 1) => (JPGD_YH2V1, 4, 16, 8),
                (1, 2) => (JPGD_YH1V2, 4, 8, 16),
                (2, 2) => (JPGD_YH2V2, 6, 16, 16),
                _ => return Err(JPGD_UNSUPPORTED_SAMP_FACTORS),
            };
            self.scan_type = st;
            self.max_blocks_per_mcu = mb;
            self.max_mcu_x_size = mx;
            self.max_mcu_y_size = my;
        } else {
            return Err(JPGD_UNSUPPORTED_COLORSPACE);
        }

        self.max_mcus_per_row =
            (self.image_x_size + (self.max_mcu_x_size - 1)) / self.max_mcu_x_size;
        self.max_mcus_per_col =
            (self.image_y_size + (self.max_mcu_y_size - 1)) / self.max_mcu_y_size;

        // These values are for the destination pixels: after conversion.
        let bpp = if self.scan_type == JPGD_GRAYSCALE {
            1
        } else {
            4
        };
        self.mcu_org[ORG_DEST_BYTES_PER_PIXEL] = bpp;
        self.mcu_org[ORG_DEST_BYTES_PER_SCAN_LINE] = ((self.image_x_size + 15) & 0xFFF0) * bpp;
        self.mcu_org[ORG_REAL_DEST_BYTES_PER_SCAN_LINE] = self.image_x_size * bpp;

        let line_len = self.mcu_org[ORG_DEST_BYTES_PER_SCAN_LINE].max(0) as usize;
        self.scan_line_0 = vec![0; line_len];
        if self.scan_type == JPGD_YH1V2 || self.scan_type == JPGD_YH2V2 {
            self.scan_line_1 = vec![0; line_len];
        }

        self.max_blocks_per_row = self.max_mcus_per_row * self.max_blocks_per_mcu;
        if self.max_blocks_per_row > JPGD_MAX_BLOCKS_PER_ROW {
            return Err(JPGD_DECODE_ERROR);
        }

        self.mcu_coefficients = vec![0; self.max_blocks_per_mcu as usize * 64];
        self.mcu_block_max_zag[..self.max_blocks_per_mcu as usize].fill(64);

        self.sample_buf = vec![0; self.max_blocks_per_row as usize * 64];
        self.sample_buf_prev = vec![0; self.max_blocks_per_row as usize * 64];

        self.mcu_org[ORG_TOTAL_LINES_LEFT] = self.image_y_size;
        self.mcu_org[ORG_MCU_LINES_LEFT] = 0;

        self.create_look_ups();
        Ok(())
    }

    /// `coeff_buf_open`.
    fn coeff_buf_open(
        block_num_x: i32,
        block_num_y: i32,
        block_len_x: i32,
        block_len_y: i32,
    ) -> CoeffBuf {
        let block_len = block_len_x * block_len_y;
        let n = (block_len as usize)
            .saturating_mul(block_num_x.max(0) as usize)
            .saturating_mul(block_num_y.max(0) as usize);
        CoeffBuf {
            data: vec![0; n],
            block_num_x,
            block_num_y,
            block_len,
        }
    }

    /// `decode_block_dc_first`.
    fn decode_block_dc_first(&mut self, component_id: usize, block_x: i32, block_y: i32) -> R<()> {
        let Some(cb) = &self.dc_coeffs[component_id] else {
            return Err(JPGD_DECODE_ERROR);
        };
        let p = Self::coeff_buf_getp(cb, block_x, block_y)?;
        let mut s = self.huff_decode(Some(self.comp_dc_tab[component_id] as usize))?;
        if s != 0 {
            if s >= 16 {
                return Err(JPGD_DECODE_ERROR);
            }
            let r = self.get_bits_no_markers(s)? as i32;
            s = huff_extend(r, s);
        }
        let dc = (s as u32).wrapping_add(self.last_dc_val[component_id]);
        self.last_dc_val[component_id] = dc;
        let v = (dc as i32).wrapping_shl(self.successive_low as u32) as i16;
        if let Some(cb) = &mut self.dc_coeffs[component_id] {
            cb.data[p] = v;
        }
        Ok(())
    }

    /// `decode_block_dc_refine`.
    fn decode_block_dc_refine(&mut self, component_id: usize, block_x: i32, block_y: i32) -> R<()> {
        if self.get_bits_no_markers(1)? != 0 {
            let sl = self.successive_low;
            let Some(cb) = &mut self.dc_coeffs[component_id] else {
                return Err(JPGD_DECODE_ERROR);
            };
            let p = Self::coeff_buf_getp(cb, block_x, block_y)?;
            cb.data[p] |= 1i32.wrapping_shl(sl as u32) as i16;
        }
        Ok(())
    }

    /// `decode_block_ac_first`.
    fn decode_block_ac_first(&mut self, component_id: usize, block_x: i32, block_y: i32) -> R<()> {
        if self.eob_run != 0 {
            self.eob_run -= 1;
            return Ok(());
        }
        let Some(cb) = &self.ac_coeffs[component_id] else {
            return Err(JPGD_DECODE_ERROR);
        };
        let p = Self::coeff_buf_getp(cb, block_x, block_y)?;
        let mut k = self.spectral_start;
        while k <= self.spectral_end {
            let idx = self.comp_ac_tab[component_id] as u32;
            if idx as usize >= JPGD_MAX_HUFF_TABLES {
                return Err(JPGD_DECODE_ERROR);
            }
            let mut s = self.huff_decode(Some(idx as usize))?;
            let mut r = s >> 4;
            s &= 15;
            if s != 0 {
                k += r;
                if k > 63 {
                    return Err(JPGD_DECODE_ERROR);
                }
                r = self.get_bits_no_markers(s)? as i32;
                s = huff_extend(r, s);
                let v = s.wrapping_shl(self.successive_low as u32) as i16;
                if let Some(cb) = &mut self.ac_coeffs[component_id] {
                    cb.data[p + G_ZAG[k as usize]] = v;
                }
            } else if r == 15 {
                k += 15;
                if k > 63 {
                    return Err(JPGD_DECODE_ERROR);
                }
            } else {
                self.eob_run = 1 << r;
                if r != 0 {
                    self.eob_run = self
                        .eob_run
                        .wrapping_add(self.get_bits_no_markers(r)? as i32);
                }
                self.eob_run -= 1;
                break;
            }
            k += 1;
        }
        Ok(())
    }

    /// One correction bit for a non-zero coefficient (AC refinement).
    fn refine_coef(&mut self, component_id: usize, at: usize, p1: i32, m1: i32) -> R<bool> {
        let coef = self.ac_coeffs[component_id]
            .as_ref()
            .map_or(0, |cb| cb.data[at]);
        if coef != 0 {
            if self.get_bits_no_markers(1)? != 0 && (i32::from(coef) & p1) == 0 {
                let v = if coef >= 0 {
                    i32::from(coef).wrapping_add(p1)
                } else {
                    i32::from(coef).wrapping_add(m1)
                };
                if let Some(cb) = &mut self.ac_coeffs[component_id] {
                    cb.data[at] = v as i16;
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// `decode_block_ac_refine`.
    fn decode_block_ac_refine(&mut self, component_id: usize, block_x: i32, block_y: i32) -> R<()> {
        let p1 = 1i32.wrapping_shl(self.successive_low as u32);
        let m1 = u32::MAX.wrapping_shl(self.successive_low as u32) as i32;
        let Some(cb) = &self.ac_coeffs[component_id] else {
            return Err(JPGD_DECODE_ERROR);
        };
        let p = Self::coeff_buf_getp(cb, block_x, block_y)?;
        if self.spectral_end > 63 {
            return Err(JPGD_DECODE_ERROR);
        }
        let mut k = self.spectral_start;
        if self.eob_run == 0 {
            while k <= self.spectral_end {
                let idx = self.comp_ac_tab[component_id] as u32;
                if idx as usize >= JPGD_MAX_HUFF_TABLES {
                    return Err(JPGD_DECODE_ERROR);
                }
                let mut s = self.huff_decode(Some(idx as usize))?;
                let mut r = s >> 4;
                s &= 15;
                if s != 0 {
                    if s != 1 {
                        return Err(JPGD_DECODE_ERROR);
                    }
                    s = if self.get_bits_no_markers(1)? != 0 {
                        p1
                    } else {
                        m1
                    };
                } else if r != 15 {
                    self.eob_run = 1 << r;
                    if r != 0 {
                        self.eob_run = self
                            .eob_run
                            .wrapping_add(self.get_bits_no_markers(r)? as i32);
                    }
                    break;
                }
                loop {
                    let at = p + G_ZAG[(k & 63) as usize];
                    if !self.refine_coef(component_id, at, p1, m1)? {
                        r -= 1;
                        if r < 0 {
                            break;
                        }
                    }
                    k += 1;
                    if k > self.spectral_end {
                        break;
                    }
                }
                if s != 0
                    && k < 64
                    && let Some(cb) = &mut self.ac_coeffs[component_id]
                {
                    cb.data[p + G_ZAG[k as usize]] = s as i16;
                }
                k += 1;
            }
        }
        if self.eob_run > 0 {
            while k <= self.spectral_end {
                let at = p + G_ZAG[(k & 63) as usize];
                self.refine_coef(component_id, at, p1, m1)?;
                k += 1;
            }
            self.eob_run -= 1;
        }
        Ok(())
    }

    /// `decode_scan`: one scan of a progressive image.
    fn decode_scan(&mut self, func: DecodeBlock) -> R<()> {
        let mut block_y_mcu = [0i32; JPGD_MAX_COMPONENTS];
        for _mcu_col in 0..self.mcus_per_col {
            let mut block_x_mcu = [0i32; JPGD_MAX_COMPONENTS];
            for _mcu_row in 0..self.mcus_per_row {
                let mut block_x_mcu_ofs = 0;
                let mut block_y_mcu_ofs = 0;
                if self.restart_interval != 0 && self.restarts_left == 0 {
                    self.process_restart()?;
                }
                for mcu_block in 0..self.blocks_per_mcu as usize {
                    let component_id = self.mcu_org[mcu_block] as usize;
                    let bx = block_x_mcu[component_id] + block_x_mcu_ofs;
                    let by = block_y_mcu[component_id] + block_y_mcu_ofs;
                    match func {
                        DecodeBlock::DcFirst => self.decode_block_dc_first(component_id, bx, by)?,
                        DecodeBlock::DcRefine => {
                            self.decode_block_dc_refine(component_id, bx, by)?
                        }
                        DecodeBlock::AcFirst => self.decode_block_ac_first(component_id, bx, by)?,
                        DecodeBlock::AcRefine => {
                            self.decode_block_ac_refine(component_id, bx, by)?
                        }
                    }
                    if self.comps_in_scan == 1 {
                        block_x_mcu[component_id] += 1;
                    } else {
                        block_x_mcu_ofs += 1;
                        if block_x_mcu_ofs == self.comp_h_samp[component_id] {
                            block_x_mcu_ofs = 0;
                            block_y_mcu_ofs += 1;
                            if block_y_mcu_ofs == self.comp_v_samp[component_id] {
                                block_y_mcu_ofs = 0;
                                block_x_mcu[component_id] += self.comp_h_samp[component_id];
                            }
                        }
                    }
                }
                self.restarts_left = self.restarts_left.wrapping_sub(1);
            }
            if self.comps_in_scan == 1 {
                block_y_mcu[self.comp_list[0] as usize] += 1;
            } else {
                for component_num in 0..self.comps_in_scan as usize {
                    let component_id = self.comp_list[component_num] as usize;
                    block_y_mcu[component_id] += self.comp_v_samp[component_id];
                }
            }
        }
        Ok(())
    }

    /// `init_progressive`: decodes all scans of a progressive image.
    fn init_progressive(&mut self) -> R<()> {
        if self.comps_in_frame == 4 {
            return Err(JPGD_UNSUPPORTED_COLORSPACE);
        }
        // Allocate the coefficient buffers.
        for i in 0..self.comps_in_frame as usize {
            let nx = self.max_mcus_per_row * self.comp_h_samp[i];
            let ny = self.max_mcus_per_col * self.comp_v_samp[i];
            self.dc_coeffs[i] = Some(Self::coeff_buf_open(nx, ny, 1, 1));
            self.ac_coeffs[i] = Some(Self::coeff_buf_open(nx, ny, 8, 8));
        }
        let mut total_scans: u32 = 0;
        const MAX_SCANS_TO_PROCESS: u32 = 1000;
        loop {
            if !self.init_scan()? {
                break;
            }
            let dc_only_scan = self.spectral_start == 0;
            let refinement_scan = self.successive_high != 0;
            if self.spectral_start > self.spectral_end || self.spectral_end > 63 {
                return Err(JPGD_BAD_SOS_SPECTRAL);
            }
            if dc_only_scan {
                if self.spectral_end != 0 {
                    return Err(JPGD_BAD_SOS_SPECTRAL);
                }
            } else if self.comps_in_scan != 1 {
                // AC scans can only contain one component
                return Err(JPGD_BAD_SOS_SPECTRAL);
            }
            if refinement_scan && self.successive_low != self.successive_high - 1 {
                return Err(JPGD_BAD_SOS_SUCCESSIVE);
            }
            let func = match (dc_only_scan, refinement_scan) {
                (true, true) => DecodeBlock::DcRefine,
                (true, false) => DecodeBlock::DcFirst,
                (false, true) => DecodeBlock::AcRefine,
                (false, false) => DecodeBlock::AcFirst,
            };
            self.decode_scan(func)?;
            self.bits_left = 16;
            self.get_bits(16)?;
            self.get_bits(16)?;
            total_scans += 1;
            if total_scans > MAX_SCANS_TO_PROCESS {
                return Err(JPGD_TOO_MANY_SCANS);
            }
        }
        self.comps_in_scan = self.comps_in_frame;
        for i in 0..(self.comps_in_frame as usize).min(JPGD_MAX_COMPS_IN_SCAN) {
            self.comp_list[i] = i as i32;
        }
        if !self.calc_mcu_block_order() {
            return Err(JPGD_DECODE_ERROR);
        }
        Ok(())
    }

    /// `init_sequential`.
    fn init_sequential(&mut self) -> R<()> {
        if !self.init_scan()? {
            return Err(JPGD_UNEXPECTED_MARKER);
        }
        Ok(())
    }

    /// `decode_start`.
    fn decode_start(&mut self) -> R<()> {
        self.init_frame()?;
        if self.progressive_flag {
            self.init_progressive()
        } else {
            self.init_sequential()
        }
    }

    /// `begin_decoding`.
    pub fn begin_decoding(&mut self) -> i32 {
        if self.ready_flag {
            return JPGD_SUCCESS;
        }
        if self.error_code != 0 {
            return JPGD_FAILED;
        }
        let r = self.decode_start();
        if self.catch(r).is_none() {
            return JPGD_FAILED;
        }
        self.ready_flag = true;
        JPGD_SUCCESS
    }
}

/// `decompress_jpeg_image_from_stream`: the whole image with `req_comps`
/// (1, 3 or 4) channels, or `None`. Returns (pixels, width, height,
/// actual components).
pub fn decompress_jpeg_image_from_stream(
    stream: Box<dyn JpegDecoderStream>,
    req_comps: i32,
    flags: u32,
) -> Option<(Vec<u8>, i32, i32, i32)> {
    if req_comps != 1 && req_comps != 3 && req_comps != 4 {
        return None;
    }
    let mut decoder = JpegDecoder::new(stream, flags);
    if decoder.get_error_code() != JPGD_SUCCESS {
        return None;
    }
    let image_width = decoder.get_width();
    let image_height = decoder.get_height();
    let actual_comps = decoder.get_num_components();
    if decoder.begin_decoding() != JPGD_SUCCESS {
        return None;
    }
    let dst_bpl = (image_width * req_comps) as usize;
    let mut image = vec![0u8; dst_bpl * image_height as usize];
    for y in 0..image_height as usize {
        let (status, scan_line) = decoder.decode();
        if status != JPGD_SUCCESS {
            return None;
        }
        let dst = &mut image[y * dst_bpl..(y + 1) * dst_bpl];
        let px = |x: usize, c: usize| scan_line.get(x * 4 + c).copied().unwrap_or(0);
        if (req_comps == 1 && actual_comps == 1) || (req_comps == 4 && actual_comps == 3) {
            let n = dst_bpl.min(scan_line.len());
            dst[..n].copy_from_slice(&scan_line[..n]);
        } else if actual_comps == 1 {
            let rc = req_comps as usize;
            for x in 0..image_width as usize {
                let luma = scan_line.get(x).copied().unwrap_or(0);
                dst[x * rc..x * rc + 3].fill(luma);
                if rc == 4 {
                    dst[x * 4 + 3] = 255;
                }
            }
        } else if actual_comps == 3 {
            if req_comps == 1 {
                const YR: i32 = 19595;
                const YG: i32 = 38470;
                const YB: i32 = 7471;
                for (x, d) in dst.iter_mut().enumerate() {
                    let (r, g, b) = (
                        i32::from(px(x, 0)),
                        i32::from(px(x, 1)),
                        i32::from(px(x, 2)),
                    );
                    *d = ((r * YR + g * YG + b * YB + 32768) >> 16) as u8;
                }
            } else {
                for x in 0..image_width as usize {
                    for c in 0..3 {
                        dst[x * 3 + c] = px(x, c);
                    }
                }
            }
        }
    }
    Some((image, image_width, image_height, actual_comps))
}

/// `decompress_jpeg_image_from_memory`.
pub fn decompress_jpeg_image_from_memory(
    src: &[u8],
    req_comps: i32,
    flags: u32,
) -> Option<(Vec<u8>, i32, i32, i32)> {
    decompress_jpeg_image_from_stream(
        Box::new(JpegDecoderMemStream::new(src.to_vec())),
        req_comps,
        flags,
    )
}
