//! Port of the ETC1 helpers used by the transcoder: `color32`, `endpoint`,
//! `selector` (basisu_transcoder_internal.h) and `decoder_etc_block`,
//! `g_etc1_inten_tables` (basisu_transcoder.cpp).

/// `g_etc1_inten_tables`.
pub const ETC1_INTEN_TABLES: [[i32; 4]; 8] = [
    [-8, -2, 2, 8],
    [-17, -5, 5, 17],
    [-29, -9, 9, 29],
    [-42, -13, 13, 42],
    [-60, -18, 18, 60],
    [-80, -24, 24, 80],
    [-106, -33, 33, 106],
    [-183, -47, 47, 183],
];

/// `g_selector_index_to_etc1`.
pub const SELECTOR_INDEX_TO_ETC1: [u8; 4] = [3, 2, 0, 1];

/// `g_etc_5_to_8`.
pub const ETC_5_TO_8: [u8; 32] = [
    0, 8, 16, 24, 33, 41, 49, 57, 66, 74, 82, 90, 99, 107, 115, 123, 132, 140, 148, 156, 165, 173,
    181, 189, 198, 206, 214, 222, 231, 239, 247, 255,
];

/// Port of `clamp255`.
#[inline]
pub fn clamp255(i: i32) -> u8 {
    if (i as u32) & 0xFFFF_FF00 != 0 {
        (!(i >> 31)) as u8
    } else {
        i as u8
    }
}

/// Port of `mul_8`.
#[inline]
pub fn mul_8(v: u32, q: u32) -> u32 {
    let v = v * q + 128;
    ((v + (v >> 8)) >> 8) & 0xFF
}

/// Port of `color32` (`r, g, b, a`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color32 {
    pub c: [u8; 4],
}

impl Color32 {
    pub const fn new(r: u32, g: u32, b: u32, a: u32) -> Color32 {
        Color32 {
            c: [r as u8, g as u8, b as u8, a as u8],
        }
    }
    #[inline]
    pub fn r(&self) -> u8 {
        self.c[0]
    }
    #[inline]
    pub fn g(&self) -> u8 {
        self.c[1]
    }
    #[inline]
    pub fn b(&self) -> u8 {
        self.c[2]
    }
    #[inline]
    pub fn a(&self) -> u8 {
        self.c[3]
    }
    pub fn set(&mut self, r: u32, g: u32, b: u32, a: u32) {
        self.c = [r as u8, g as u8, b as u8, a as u8];
    }
}

/// Port of `endpoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Endpoint {
    pub color5: Color32,
    pub inten5: u8,
}

/// Port of `selector`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selector {
    /// Plain selectors (2 bits per value).
    pub selectors: [u8; 4],
    /// ETC1 selectors.
    pub bytes: [u8; 4],
    pub lo_selector: u8,
    pub hi_selector: u8,
    pub num_unique_selectors: u8,
}

impl Selector {
    /// Port of `get_selector`.
    #[inline]
    pub fn get_selector(&self, x: u32, y: u32) -> u32 {
        ((self.selectors[(y & 3) as usize] >> ((x & 3) * 2)) & 3) as u32
    }

    /// Port of `set_selector`.
    pub fn set_selector(&mut self, x: u32, y: u32, val: u32) {
        let (x, y, val) = (x & 3, y & 3, val & 3);
        self.selectors[y as usize] &= !(3u8 << (x * 2));
        self.selectors[y as usize] |= (val << (x * 2)) as u8;
        let etc1_bit_index = x * 4 + y;
        let p = 3 - (etc1_bit_index >> 3) as usize;
        let byte_bit_ofs = etc1_bit_index & 7;
        let mask = 1u8 << byte_bit_ofs;
        let etc1_val = SELECTOR_INDEX_TO_ETC1[val as usize] as u32;
        let lsb = (etc1_val & 1) as u8;
        let msb = (etc1_val >> 1) as u8;
        self.bytes[p] &= !mask;
        self.bytes[p] |= lsb << byte_bit_ofs;
        self.bytes[p - 2] &= !mask;
        self.bytes[p - 2] |= msb << byte_bit_ofs;
    }

    /// Port of `init_flags`.
    pub fn init_flags(&mut self) {
        let mut hist = [0u32; 4];
        for y in 0..4 {
            for x in 0..4 {
                hist[self.get_selector(x, y) as usize] += 1;
            }
        }
        self.lo_selector = 3;
        self.hi_selector = 0;
        self.num_unique_selectors = 0;
        for (i, &h) in hist.iter().enumerate() {
            if h != 0 {
                self.num_unique_selectors += 1;
                self.lo_selector = self.lo_selector.min(i as u8);
                self.hi_selector = self.hi_selector.max(i as u8);
            }
        }
    }
}

const ETC1_BASE_COLOR5_R_BIT_OFFSET: u32 = 59;
const ETC1_BASE_COLOR5_G_BIT_OFFSET: u32 = 51;
const ETC1_BASE_COLOR5_B_BIT_OFFSET: u32 = 43;

/// Port of `decoder_etc_block` (8 bytes, big-endian bit layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecoderEtcBlock {
    pub bytes: [u8; 8],
}

impl DecoderEtcBlock {
    /// Port of `set_byte_bits`.
    pub fn set_byte_bits(&mut self, ofs: u32, num: u32, bits: u32) {
        let byte_ofs = 7 - (ofs >> 3) as usize;
        let byte_bit_ofs = ofs & 7;
        let mask = (1u32 << num) - 1;
        self.bytes[byte_ofs] &= !((mask << byte_bit_ofs) as u8);
        self.bytes[byte_ofs] |= (bits << byte_bit_ofs) as u8;
    }

    /// Port of `get_byte_bits`.
    pub fn get_byte_bits(&self, ofs: u32, num: u32) -> u32 {
        let byte_ofs = 7 - (ofs >> 3) as usize;
        let byte_bit_ofs = ofs & 7;
        ((self.bytes[byte_ofs] as u32) >> byte_bit_ofs) & ((1 << num) - 1)
    }

    pub fn set_flip_bit(&mut self, flip: bool) {
        self.bytes[3] &= !1;
        self.bytes[3] |= flip as u8;
    }

    pub fn set_diff_bit(&mut self, diff: bool) {
        self.bytes[3] &= !2;
        self.bytes[3] |= (diff as u8) << 1;
    }

    pub fn get_flip_bit(&self) -> bool {
        self.bytes[3] & 1 != 0
    }

    pub fn get_diff_bit(&self) -> bool {
        self.bytes[3] & 2 != 0
    }

    /// Port of `set_inten_table`.
    pub fn set_inten_table(&mut self, subblock_id: u32, t: u32) {
        let ofs = if subblock_id != 0 { 2 } else { 5 };
        self.bytes[3] &= !(7u8 << ofs);
        self.bytes[3] |= ((t & 7) << ofs) as u8;
    }

    /// Port of `get_inten_table`.
    pub fn get_inten_table(&self, subblock_id: u32) -> u32 {
        let ofs = if subblock_id != 0 { 2 } else { 5 };
        ((self.bytes[3] >> ofs) & 7) as u32
    }

    /// Port of `set_raw_selector_bits(byte0..byte3)`.
    pub fn set_raw_selector_bits(&mut self, b: [u8; 4]) {
        self.bytes[4..8].copy_from_slice(&b);
    }

    /// Port of `get_raw_selector_bits`.
    pub fn get_raw_selector_bits(&self) -> u32 {
        u32::from_le_bytes([self.bytes[4], self.bytes[5], self.bytes[6], self.bytes[7]])
    }

    /// Port of `get_selector` (returns the index into the inten table).
    pub fn get_selector(&self, x: u32, y: u32) -> u32 {
        const ETC1_TO_SELECTOR_INDEX: [u32; 4] = [2, 3, 1, 0];
        let bit_index = x * 4 + y;
        let p = 7 - (bit_index >> 3) as usize;
        let byte_bit_ofs = bit_index & 7;
        let lsb = ((self.bytes[p] >> byte_bit_ofs) & 1) as u32;
        let msb = ((self.bytes[p - 2] >> byte_bit_ofs) & 1) as u32;
        ETC1_TO_SELECTOR_INDEX[(lsb | (msb << 1)) as usize]
    }

    /// Port of `set_base5_color`.
    pub fn set_base5_color(&mut self, c: u16) {
        let c = c as u32;
        self.set_byte_bits(ETC1_BASE_COLOR5_R_BIT_OFFSET, 5, (c >> 10) & 31);
        self.set_byte_bits(ETC1_BASE_COLOR5_G_BIT_OFFSET, 5, (c >> 5) & 31);
        self.set_byte_bits(ETC1_BASE_COLOR5_B_BIT_OFFSET, 5, c & 31);
    }

    /// Port of `pack_color5`.
    pub fn pack_color5(color: &Color32, scaled: bool) -> u16 {
        Self::pack_color5_rgb(
            color.r() as u32,
            color.g() as u32,
            color.b() as u32,
            scaled,
            127,
        )
    }

    /// Port of `pack_color5(r, g, b, scaled, bias)`.
    pub fn pack_color5_rgb(mut r: u32, mut g: u32, mut b: u32, scaled: bool, bias: u32) -> u16 {
        if scaled {
            r = (r * 31 + bias) / 255;
            g = (g * 31 + bias) / 255;
            b = (b * 31 + bias) / 255;
        }
        (b.min(31) | (g.min(31) << 5) | (r.min(31) << 10)) as u16
    }

    /// Port of `unpack_color5`.
    pub fn unpack_color5(packed: u16, scaled: bool, alpha: u32) -> Color32 {
        let mut b = (packed & 31) as u32;
        let mut g = ((packed >> 5) & 31) as u32;
        let mut r = ((packed >> 10) & 31) as u32;
        if scaled {
            b = (b << 3) | (b >> 2);
            g = (g << 3) | (g >> 2);
            r = (r << 3) | (r >> 2);
        }
        Color32::new(r, g, b, alpha)
    }

    /// `(c << 3) | (c >> 2)` stored back into a `uint8_t` (as C does for color32).
    fn expand5(c: u8) -> i32 {
        ((((c as u32) << 3) | ((c as u32) >> 2)) as u8) as i32
    }

    /// Same expansion kept as `int` (C `get_block_colors5_g` style).
    fn expand5_int(c: u8) -> i32 {
        (((c as u32) << 3) | ((c as u32) >> 2)) as i32
    }

    /// Port of `get_block_colors5`.
    pub fn get_block_colors5(base_color5: &Color32, inten_table: u32) -> [Color32; 4] {
        let (r, g, b) = (
            Self::expand5(base_color5.r()),
            Self::expand5(base_color5.g()),
            Self::expand5(base_color5.b()),
        );
        let t = &ETC1_INTEN_TABLES[(inten_table & 7) as usize];
        let mut out = [Color32::default(); 4];
        for (i, o) in out.iter_mut().enumerate() {
            *o = Color32 {
                c: [
                    clamp255(r + t[i]),
                    clamp255(g + t[i]),
                    clamp255(b + t[i]),
                    255,
                ],
            };
        }
        out
    }

    /// Port of `get_block_color5`.
    pub fn get_block_color5(
        base_color5: &Color32,
        inten_table: u32,
        index: u32,
    ) -> (u32, u32, u32) {
        let t = ETC1_INTEN_TABLES[(inten_table & 7) as usize][(index & 3) as usize];
        (
            clamp255(Self::expand5(base_color5.r()) + t) as u32,
            clamp255(Self::expand5(base_color5.g()) + t) as u32,
            clamp255(Self::expand5(base_color5.b()) + t) as u32,
        )
    }

    /// Port of `get_block_colors5_g`.
    pub fn get_block_colors5_g(base_color5: &Color32, inten_table: u32) -> [i32; 4] {
        let g = Self::expand5_int(base_color5.g());
        let t = &ETC1_INTEN_TABLES[(inten_table & 7) as usize];
        [
            clamp255(g + t[0]) as i32,
            clamp255(g + t[1]) as i32,
            clamp255(g + t[2]) as i32,
            clamp255(g + t[3]) as i32,
        ]
    }

    /// Port of `get_block_colors5_bounds`.
    pub fn get_block_colors5_bounds(
        base_color5: &Color32,
        inten_table: u32,
        l: u32,
        h: u32,
    ) -> [Color32; 2] {
        let (r, g, b) = (
            Self::expand5(base_color5.r()),
            Self::expand5(base_color5.g()),
            Self::expand5(base_color5.b()),
        );
        let t = &ETC1_INTEN_TABLES[(inten_table & 7) as usize];
        let (tl, th) = (t[(l & 3) as usize], t[(h & 3) as usize]);
        [
            Color32 {
                c: [clamp255(r + tl), clamp255(g + tl), clamp255(b + tl), 255],
            },
            Color32 {
                c: [clamp255(r + th), clamp255(g + th), clamp255(b + th), 255],
            },
        ]
    }

    /// Port of `get_block_colors5_bounds_g`.
    pub fn get_block_colors5_bounds_g(
        base_color5: &Color32,
        inten_table: u32,
        l: u32,
        h: u32,
    ) -> [u32; 2] {
        let g = Self::expand5(base_color5.g());
        let t = &ETC1_INTEN_TABLES[(inten_table & 7) as usize];
        [
            clamp255(g + t[(l & 3) as usize]) as u32,
            clamp255(g + t[(h & 3) as usize]) as u32,
        ]
    }
}
