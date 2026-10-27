// SPDX-FileCopyrightText: 2005-2025 Lode Vandevenne
// SPDX-License-Identifier: Zlib
// Translated to Rust from LodePNG and modified; see NOTICE.md.
//! Port of LodePNG (external/lodepng, version 20250506) as used by the ktx
//! tools: the encoder (`lodepng::encode` with a `State`, `ktx extract`) and the
//! decoder with KTX-Software's split `lodepng_decode_chunks` /
//! `lodepng_finish_decode` API (`decode`, the inflater in `zlib`).
//!
//! Encoder: color modes and conversion, color statistics and `auto_convert`
//! (palette / grey / bit depth / color key selection), scanline filters (all
//! strategies), Adam7 interlacing, the zlib encoder (`zlib`) and the chunks
//! IHDR, sRGB, gAMA, cHRM, sBIT, PLTE, tRNS, IDAT, IEND. Not ported (never set
//! by the tools): iCCP, cICP, mDCV, cLLI, eXIf, bKGD, pHYs, tIME, text chunks,
//! unknown chunks. Errors are LodePNG's numeric codes.

mod decode;
pub mod zlib;

use std::collections::HashMap;

pub use decode::*;
pub use zlib::{
    CompressSettings, DecompressSettings, lodepng_inflate, lodepng_zlib_compress,
    lodepng_zlib_decompress,
};

/// `LodePNGColorType`.
pub type ColorType = u32;
/// Greyscale: 1,2,4,8,16 bit.
pub const LCT_GREY: ColorType = 0;
/// RGB: 8,16 bit.
pub const LCT_RGB: ColorType = 2;
/// Palette: 1,2,4,8 bit.
pub const LCT_PALETTE: ColorType = 3;
/// Greyscale with alpha: 8,16 bit.
pub const LCT_GREY_ALPHA: ColorType = 4;
/// RGB with alpha: 8,16 bit.
pub const LCT_RGBA: ColorType = 6;

pub const LODEPNG_VERSION_STRING: &str = "20250506";

/// Port of `LodePNGColorMode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorMode {
    pub colortype: ColorType,
    pub bitdepth: u32,
    /// Empty or 1024 bytes (256 RGBA colors), like the C allocation.
    pub palette: Vec<u8>,
    pub palettesize: usize,
    pub key_defined: bool,
    pub key_r: u32,
    pub key_g: u32,
    pub key_b: u32,
}

/// `lodepng_color_mode_init`.
impl Default for ColorMode {
    fn default() -> ColorMode {
        ColorMode {
            colortype: LCT_RGBA,
            bitdepth: 8,
            palette: Vec::new(),
            palettesize: 0,
            key_defined: false,
            key_r: 0,
            key_g: 0,
            key_b: 0,
        }
    }
}

impl ColorMode {
    /// `lodepng_color_mode_make`.
    pub fn make(colortype: ColorType, bitdepth: u32) -> ColorMode {
        ColorMode {
            colortype,
            bitdepth,
            ..ColorMode::default()
        }
    }

    /// `lodepng_color_mode_alloc_palette`: room for 256 colors, all opaque black.
    fn alloc_palette(&mut self) {
        if self.palette.is_empty() {
            self.palette = [0, 0, 0, 255].repeat(256);
        }
    }

    /// `lodepng_palette_clear`.
    pub fn palette_clear(&mut self) {
        self.palette = Vec::new();
        self.palettesize = 0;
    }

    /// `lodepng_palette_add`.
    pub fn palette_add(&mut self, r: u8, g: u8, b: u8, a: u8) -> Result<(), u32> {
        self.alloc_palette();
        if self.palettesize >= 256 {
            return Err(108); // too many palette values
        }
        let p = 4 * self.palettesize;
        self.palette[p..p + 4].copy_from_slice(&[r, g, b, a]);
        self.palettesize += 1;
        Ok(())
    }

    /// `lodepng_color_mode_copy`.
    fn copy_from(&mut self, source: &ColorMode) {
        *self = source.clone();
        if !source.palette.is_empty() {
            let mut palette = [0, 0, 0, 255].repeat(256);
            let n = source.palettesize * 4;
            palette[..n].copy_from_slice(&source.palette[..n]);
            self.palette = palette;
        }
    }

    /// `lodepng_get_bpp`.
    pub fn bpp(&self) -> u32 {
        bpp_lct(self.colortype, self.bitdepth)
    }

    /// `lodepng_is_greyscale_type`.
    pub fn is_greyscale_type(&self) -> bool {
        self.colortype == LCT_GREY || self.colortype == LCT_GREY_ALPHA
    }

    /// `lodepng_is_alpha_type`.
    pub fn is_alpha_type(&self) -> bool {
        self.colortype & 4 != 0
    }

    /// `lodepng_has_palette_alpha`.
    pub fn has_palette_alpha(&self) -> bool {
        (0..self.palettesize).any(|i| self.palette[i * 4 + 3] < 255)
    }

    /// `lodepng_can_have_alpha`.
    pub fn can_have_alpha(&self) -> bool {
        self.key_defined || self.is_alpha_type() || self.has_palette_alpha()
    }

    /// `lodepng_color_mode_equal`.
    fn equal(&self, b: &ColorMode) -> bool {
        if self.colortype != b.colortype
            || self.bitdepth != b.bitdepth
            || self.key_defined != b.key_defined
        {
            return false;
        }
        if self.key_defined
            && (self.key_r != b.key_r || self.key_g != b.key_g || self.key_b != b.key_b)
        {
            return false;
        }
        if self.palettesize != b.palettesize {
            return false;
        }
        let n = self.palettesize * 4;
        self.palette[..n] == b.palette[..n]
    }
}

/// `checkColorValidity`.
fn check_color_validity(colortype: ColorType, bd: u32) -> Result<(), u32> {
    let ok = match colortype {
        LCT_GREY => matches!(bd, 1 | 2 | 4 | 8 | 16),
        LCT_RGB | LCT_GREY_ALPHA | LCT_RGBA => matches!(bd, 8 | 16),
        LCT_PALETTE => matches!(bd, 1 | 2 | 4 | 8),
        _ => return Err(31), // invalid color type
    };
    if ok { Ok(()) } else { Err(37) }
}

/// `getNumColorChannels`.
fn num_color_channels(colortype: ColorType) -> u32 {
    match colortype {
        LCT_GREY | LCT_PALETTE => 1,
        LCT_RGB => 3,
        LCT_GREY_ALPHA => 2,
        LCT_RGBA => 4,
        _ => 0,
    }
}

/// `lodepng_get_bpp_lct`.
fn bpp_lct(colortype: ColorType, bitdepth: u32) -> u32 {
    num_color_channels(colortype) * bitdepth
}

/// `lodepng_get_raw_size_lct`.
pub fn get_raw_size_lct(w: u32, h: u32, colortype: ColorType, bitdepth: u32) -> usize {
    let bpp = bpp_lct(colortype, bitdepth) as usize;
    let n = w as usize * h as usize;
    (n / 8) * bpp + ((n & 7) * bpp).div_ceil(8)
}

/// `lodepng_get_raw_size`.
pub fn get_raw_size(w: u32, h: u32, color: &ColorMode) -> usize {
    get_raw_size_lct(w, h, color.colortype, color.bitdepth)
}

/// `lodepng_get_raw_size_idat`: one extra filter byte per line plus padding bits.
fn get_raw_size_idat(w: u32, h: u32, bpp: u32) -> usize {
    let bpp = bpp as usize;
    let line = (w as usize / 8) * bpp + 1 + ((w as usize & 7) * bpp).div_ceil(8);
    h as usize * line
}

fn read_bit_from_reversed_stream(bitpointer: &mut usize, bitstream: &[u8]) -> u8 {
    let result = (bitstream[*bitpointer >> 3] >> (7 - (*bitpointer & 7))) & 1;
    *bitpointer += 1;
    result
}

fn read_bits_from_reversed_stream(bitpointer: &mut usize, bitstream: &[u8], nbits: u32) -> u32 {
    let mut result = 0u32;
    for _ in 0..nbits {
        result <<= 1;
        result |= u32::from(read_bit_from_reversed_stream(bitpointer, bitstream));
    }
    result
}

fn set_bit_of_reversed_stream(bitpointer: &mut usize, bitstream: &mut [u8], bit: u8) {
    let mask = 1u8 << (7 - (*bitpointer & 7));
    if bit == 0 {
        bitstream[*bitpointer >> 3] &= !mask;
    } else {
        bitstream[*bitpointer >> 3] |= mask;
    }
    *bitpointer += 1;
}

/// `addColorBits`: index is the bitgroup index, bits the bitgroup size (1, 2 or 4).
fn add_color_bits(out: &mut [u8], index: usize, bits: u32, input: u32) {
    let m: usize = match bits {
        1 => 7,
        2 => 3,
        _ => 1,
    };
    let p = index & m;
    let v = ((input & ((1 << bits) - 1)) << (bits as usize * (m - p))) as u8;
    let o = index * bits as usize / 8;
    if p == 0 {
        out[o] = v;
    } else {
        out[o] |= v;
    }
}

/// `ColorTree`: RGBA color -> palette index.
type ColorTree = HashMap<[u8; 4], i32>;

/// `rgba8ToPixel`: puts a pixel, given its RGBA color, into an image of any color type.
#[allow(clippy::too_many_arguments)]
fn rgba8_to_pixel(
    out: &mut [u8],
    i: usize,
    mode: &ColorMode,
    tree: &ColorTree,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<(), u32> {
    match mode.colortype {
        LCT_GREY => {
            let gray = r;
            if mode.bitdepth == 8 {
                out[i] = gray;
            } else if mode.bitdepth == 16 {
                out[i * 2] = gray;
                out[i * 2 + 1] = gray;
            } else {
                // take the most significant bits of gray
                let gray = (u32::from(gray) >> (8 - mode.bitdepth)) & ((1 << mode.bitdepth) - 1);
                add_color_bits(out, i, mode.bitdepth, gray);
            }
        }
        LCT_RGB => {
            if mode.bitdepth == 8 {
                out[i * 3..i * 3 + 3].copy_from_slice(&[r, g, b]);
            } else {
                out[i * 6..i * 6 + 6].copy_from_slice(&[r, r, g, g, b, b]);
            }
        }
        LCT_PALETTE => {
            let index = tree.get(&[r, g, b, a]).copied().unwrap_or(-1);
            if index < 0 {
                return Err(82); // color not in palette
            }
            if mode.bitdepth == 8 {
                out[i] = index as u8;
            } else {
                add_color_bits(out, i, mode.bitdepth, index as u32);
            }
        }
        LCT_GREY_ALPHA => {
            let gray = r;
            if mode.bitdepth == 8 {
                out[i * 2..i * 2 + 2].copy_from_slice(&[gray, a]);
            } else if mode.bitdepth == 16 {
                out[i * 4..i * 4 + 4].copy_from_slice(&[gray, gray, a, a]);
            }
        }
        LCT_RGBA => {
            if mode.bitdepth == 8 {
                out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
            } else {
                out[i * 8..i * 8 + 8].copy_from_slice(&[r, r, g, g, b, b, a, a]);
            }
        }
        _ => {}
    }
    Ok(())
}

/// `rgba16ToPixel`: into an image of any 16-bit color type.
fn rgba16_to_pixel(out: &mut [u8], i: usize, mode: &ColorMode, r: u16, g: u16, b: u16, a: u16) {
    let [rh, rl] = r.to_be_bytes();
    let [gh, gl] = g.to_be_bytes();
    let [bh, bl] = b.to_be_bytes();
    let [ah, al] = a.to_be_bytes();
    match mode.colortype {
        LCT_GREY => out[i * 2..i * 2 + 2].copy_from_slice(&[rh, rl]),
        LCT_RGB => out[i * 6..i * 6 + 6].copy_from_slice(&[rh, rl, gh, gl, bh, bl]),
        LCT_GREY_ALPHA => out[i * 4..i * 4 + 4].copy_from_slice(&[rh, rl, ah, al]),
        LCT_RGBA => out[i * 8..i * 8 + 8].copy_from_slice(&[rh, rl, gh, gl, bh, bl, ah, al]),
        _ => {}
    }
}

/// `getPixelColorRGBA8` (also `getPixelColorsRGBA8` / `getPixelColorsRGB8`,
/// which give the same values): RGBA8 color of pixel `i`.
fn get_pixel_color_rgba8(input: &[u8], i: usize, mode: &ColorMode) -> [u8; 4] {
    let be16 = |o: usize| 256 * u32::from(input[o]) + u32::from(input[o + 1]);
    match mode.colortype {
        LCT_GREY => {
            let (v, key) = if mode.bitdepth == 8 {
                (input[i], u32::from(input[i]))
            } else if mode.bitdepth == 16 {
                (input[i * 2], be16(i * 2))
            } else {
                let highest = (1u32 << mode.bitdepth) - 1;
                let mut j = i * mode.bitdepth as usize;
                let value = read_bits_from_reversed_stream(&mut j, input, mode.bitdepth);
                ((value * 255 / highest) as u8, value)
            };
            let a = if mode.key_defined && key == mode.key_r {
                0
            } else {
                255
            };
            [v, v, v, a]
        }
        LCT_RGB => {
            let (rgb, key) = if mode.bitdepth == 8 {
                let p = &input[i * 3..i * 3 + 3];
                ([p[0], p[1], p[2]], [p[0], p[1], p[2]].map(u32::from))
            } else {
                (
                    [input[i * 6], input[i * 6 + 2], input[i * 6 + 4]],
                    [be16(i * 6), be16(i * 6 + 2), be16(i * 6 + 4)],
                )
            };
            let a = if mode.key_defined
                && key[0] == mode.key_r
                && key[1] == mode.key_g
                && key[2] == mode.key_b
            {
                0
            } else {
                255
            };
            [rgb[0], rgb[1], rgb[2], a]
        }
        LCT_PALETTE => {
            let index = if mode.bitdepth == 8 {
                usize::from(input[i])
            } else {
                let mut j = i * mode.bitdepth as usize;
                read_bits_from_reversed_stream(&mut j, input, mode.bitdepth) as usize
            };
            // out of bounds of palette not checked: see lodepng_color_mode_alloc_palette.
            let p = mode
                .palette
                .get(index * 4..index * 4 + 4)
                .unwrap_or(&[0, 0, 0, 255]);
            [p[0], p[1], p[2], p[3]]
        }
        LCT_GREY_ALPHA => {
            if mode.bitdepth == 8 {
                let v = input[i * 2];
                [v, v, v, input[i * 2 + 1]]
            } else {
                let v = input[i * 4];
                [v, v, v, input[i * 4 + 2]]
            }
        }
        LCT_RGBA => {
            if mode.bitdepth == 8 {
                let p = &input[i * 4..i * 4 + 4];
                [p[0], p[1], p[2], p[3]]
            } else {
                [
                    input[i * 8],
                    input[i * 8 + 2],
                    input[i * 8 + 4],
                    input[i * 8 + 6],
                ]
            }
        }
        _ => [0, 0, 0, 0],
    }
}

/// `getPixelColorRGBA16`: the given color type must be 16-bit itself.
fn get_pixel_color_rgba16(input: &[u8], i: usize, mode: &ColorMode) -> [u16; 4] {
    let be16 = |o: usize| u16::from_be_bytes([input[o], input[o + 1]]);
    match mode.colortype {
        LCT_GREY => {
            let v = be16(i * 2);
            let a = if mode.key_defined && u32::from(v) == mode.key_r {
                0
            } else {
                65535
            };
            [v, v, v, a]
        }
        LCT_RGB => {
            let (r, g, b) = (be16(i * 6), be16(i * 6 + 2), be16(i * 6 + 4));
            let a = if mode.key_defined
                && u32::from(r) == mode.key_r
                && u32::from(g) == mode.key_g
                && u32::from(b) == mode.key_b
            {
                0
            } else {
                65535
            };
            [r, g, b, a]
        }
        LCT_GREY_ALPHA => {
            let v = be16(i * 4);
            [v, v, v, be16(i * 4 + 2)]
        }
        LCT_RGBA => [
            be16(i * 8),
            be16(i * 8 + 2),
            be16(i * 8 + 4),
            be16(i * 8 + 6),
        ],
        _ => [0, 0, 0, 0],
    }
}

/// Port of `lodepng_convert`: converts `input` from `mode_in` to `mode_out`
/// into `out` (`lodepng_get_raw_size(w, h, mode_out)` bytes).
pub fn lodepng_convert(
    out: &mut [u8],
    input: &[u8],
    mode_out: &ColorMode,
    mode_in: &ColorMode,
    w: u32,
    h: u32,
) -> Result<(), u32> {
    let numpixels = w as usize * h as usize;
    if mode_in.colortype == LCT_PALETTE && mode_in.palette.is_empty() {
        return Err(107); // must provide palette if input mode is palette
    }
    if mode_out.equal(mode_in) {
        let numbytes = get_raw_size(w, h, mode_in);
        out[..numbytes].copy_from_slice(&input[..numbytes]);
        return Ok(());
    }

    let mut tree = ColorTree::new();
    if mode_out.colortype == LCT_PALETTE {
        let mut palettesize = mode_out.palettesize;
        let mut palette = &mode_out.palette;
        let mut palsize = 1usize << mode_out.bitdepth;
        // if the user specified output palette but did not give the values, assume
        // they want the values of the input color type (assuming that one is palette).
        if palettesize == 0 {
            palettesize = mode_in.palettesize;
            palette = &mode_in.palette;
            // if the input was also palette with same bitdepth, copy literally.
            if mode_in.colortype == LCT_PALETTE && mode_in.bitdepth == mode_out.bitdepth {
                let numbytes = get_raw_size(w, h, mode_in);
                out[..numbytes].copy_from_slice(&input[..numbytes]);
                return Ok(());
            }
        }
        palsize = palsize.min(palettesize);
        for i in 0..palsize {
            let p = &palette[i * 4..i * 4 + 4];
            tree.insert([p[0], p[1], p[2], p[3]], i as i32);
        }
    }

    if mode_in.bitdepth == 16 && mode_out.bitdepth == 16 {
        for i in 0..numpixels {
            let [r, g, b, a] = get_pixel_color_rgba16(input, i, mode_in);
            rgba16_to_pixel(out, i, mode_out, r, g, b, a);
        }
    } else if mode_out.bitdepth == 8 && mode_out.colortype == LCT_RGBA {
        for i in 0..numpixels {
            out[i * 4..i * 4 + 4].copy_from_slice(&get_pixel_color_rgba8(input, i, mode_in));
        }
    } else if mode_out.bitdepth == 8 && mode_out.colortype == LCT_RGB {
        for i in 0..numpixels {
            let c = get_pixel_color_rgba8(input, i, mode_in);
            out[i * 3..i * 3 + 3].copy_from_slice(&c[..3]);
        }
    } else {
        for i in 0..numpixels {
            let [r, g, b, a] = get_pixel_color_rgba8(input, i, mode_in);
            rgba8_to_pixel(out, i, mode_out, &tree, r, g, b, a)?;
        }
    }
    Ok(())
}

/// Port of `LodePNGColorStats`.
#[derive(Debug, Clone)]
pub struct ColorStats {
    /// Not grayscale.
    pub colored: bool,
    /// Image is not opaque and color key is possible instead of full alpha.
    pub key: bool,
    /// Key values, always as 16-bit, in 8-bit case the byte is duplicated.
    pub key_r: u32,
    pub key_g: u32,
    pub key_b: u32,
    /// Image is not opaque and alpha channel or alpha palette required.
    pub alpha: bool,
    /// Amount of colors, up to 257. Not valid if bits == 16 or allow_palette is disabled.
    pub numcolors: u32,
    /// Remembers up to the first 256 RGBA colors, in no particular order.
    pub palette: [u8; 1024],
    /// Bits per channel (not for palette). 1,2 or 4 for grayscale only. 16 if 16-bit per channel required.
    pub bits: u32,
    pub numpixels: usize,
    /// If 0, disallow choosing palette colortype in auto_choose_color.
    pub allow_palette: bool,
    /// If 0, disallow choosing grayscale colortype in auto_choose_color.
    pub allow_greyscale: bool,
}

/// `lodepng_color_stats_init`.
impl Default for ColorStats {
    fn default() -> ColorStats {
        ColorStats {
            colored: false,
            key: false,
            key_r: 0,
            key_g: 0,
            key_b: 0,
            alpha: false,
            numcolors: 0,
            palette: [0; 1024],
            bits: 1,
            numpixels: 0,
            allow_palette: true,
            allow_greyscale: true,
        }
    }
}

/// `getValueRequiredBits`: bits needed to represent the value (max 8 bit).
fn get_value_required_bits(value: u8) -> u32 {
    if value == 0 || value == 255 {
        return 1;
    }
    // The scaling of 2-bit and 4-bit values uses multiples of 85 and 17
    if value.is_multiple_of(17) {
        return if value.is_multiple_of(85) { 2 } else { 4 };
    }
    8
}

/// Port of `lodepng_compute_color_stats`.
pub fn lodepng_compute_color_stats(
    stats: &mut ColorStats,
    input: &[u8],
    w: u32,
    h: u32,
    mode_in: &ColorMode,
) -> Result<(), u32> {
    let numpixels = w as usize * h as usize;
    // mark things as done already if it would be impossible to have a more expensive case
    let mut colored_done = mode_in.is_greyscale_type();
    let mut alpha_done = !mode_in.can_have_alpha();
    let mut numcolors_done = false;
    let bpp = mode_in.bpp();
    let mut bits_done = stats.bits == 1 && bpp == 1;
    let mut sixteen = false; // whether the input image is 16 bit
    let mut maxnumcolors: u32 = 257;
    if bpp <= 8 {
        maxnumcolors = 257.min(stats.numcolors + (1 << bpp));
    }

    stats.numpixels += numpixels;

    // if palette not allowed, no need to compute numcolors
    if !stats.allow_palette {
        numcolors_done = true;
    }

    let mut tree = ColorTree::new();

    // If the stats was already filled in from previous data, fill its palette in tree
    // and mark things as done already if we know they are the most expensive case already
    if stats.alpha {
        alpha_done = true;
    }
    if stats.colored {
        colored_done = true;
    }
    if stats.bits == 16 {
        numcolors_done = true;
    }
    if stats.bits >= bpp {
        bits_done = true;
    }
    if stats.numcolors >= maxnumcolors {
        numcolors_done = true;
    }

    if !numcolors_done {
        for i in 0..stats.numcolors as usize {
            let c = &stats.palette[i * 4..i * 4 + 4];
            tree.insert([c[0], c[1], c[2], c[3]], i as i32);
        }
    }

    // Check if the 16-bit input is truly 16-bit
    if mode_in.bitdepth == 16 && !sixteen {
        for i in 0..numpixels {
            let c = get_pixel_color_rgba16(input, i, mode_in);
            if c.iter().any(|&v| (v & 255) != ((v >> 8) & 255)) {
                // first and second byte differ
                stats.bits = 16;
                sixteen = true;
                bits_done = true;
                numcolors_done = true; // counting colors no longer useful, palette doesn't support 16-bit
                break;
            }
        }
    }

    if sixteen {
        for i in 0..numpixels {
            let [r, g, b, a] = get_pixel_color_rgba16(input, i, mode_in);
            let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));

            if !colored_done && (r != g || r != b) {
                stats.colored = true;
                colored_done = true;
            }

            if !alpha_done {
                let matchkey = r == stats.key_r && g == stats.key_g && b == stats.key_b;
                if a != 65535 && (a != 0 || (stats.key && !matchkey)) {
                    stats.alpha = true;
                    stats.key = false;
                    alpha_done = true;
                } else if a == 0 && !stats.alpha && !stats.key {
                    stats.key = true;
                    stats.key_r = r;
                    stats.key_g = g;
                    stats.key_b = b;
                } else if a == 65535 && stats.key && matchkey {
                    // Color key cannot be used if an opaque pixel also has that RGB color.
                    stats.alpha = true;
                    stats.key = false;
                    alpha_done = true;
                }
            }
            if alpha_done && numcolors_done && colored_done && bits_done {
                break;
            }
        }

        if stats.key && !stats.alpha {
            for i in 0..numpixels {
                let [r, g, b, a] = get_pixel_color_rgba16(input, i, mode_in);
                if a != 0
                    && u32::from(r) == stats.key_r
                    && u32::from(g) == stats.key_g
                    && u32::from(b) == stats.key_b
                {
                    // Color key cannot be used if an opaque pixel also has that RGB color.
                    stats.alpha = true;
                    stats.key = false;
                }
            }
        }
    } else {
        // < 16-bit
        let mut prev = [0u8; 4];
        for i in 0..numpixels {
            let c = get_pixel_color_rgba8(input, i, mode_in);
            let [r, g, b, a] = c;

            // skip if color same as before, this speeds up large non-photographic
            // images with many same colors by avoiding 'color_tree_has' below
            if i != 0 && c == prev {
                continue;
            }
            prev = c;

            if !bits_done && stats.bits < 8 {
                // only r is checked, < 8 bits is only relevant for grayscale
                let bits = get_value_required_bits(r);
                if bits > stats.bits {
                    stats.bits = bits;
                }
            }
            bits_done = stats.bits >= bpp;

            if !colored_done && (r != g || r != b) {
                stats.colored = true;
                colored_done = true;
                if stats.bits < 8 {
                    stats.bits = 8; // PNG has no colored modes with less than 8-bit per channel
                }
            }

            if !alpha_done {
                let (r32, g32, b32) = (u32::from(r), u32::from(g), u32::from(b));
                let matchkey = r32 == stats.key_r && g32 == stats.key_g && b32 == stats.key_b;
                if a != 255 && (a != 0 || (stats.key && !matchkey)) {
                    stats.alpha = true;
                    stats.key = false;
                    alpha_done = true;
                    if stats.bits < 8 {
                        stats.bits = 8; // PNG has no alphachannel modes with less than 8-bit per channel
                    }
                } else if a == 0 && !stats.alpha && !stats.key {
                    stats.key = true;
                    stats.key_r = r32;
                    stats.key_g = g32;
                    stats.key_b = b32;
                } else if a == 255 && stats.key && matchkey {
                    // Color key cannot be used if an opaque pixel also has that RGB color.
                    stats.alpha = true;
                    stats.key = false;
                    alpha_done = true;
                    if stats.bits < 8 {
                        stats.bits = 8;
                    }
                }
            }

            if !numcolors_done && !tree.contains_key(&c) {
                tree.insert(c, stats.numcolors as i32);
                if stats.numcolors < 256 {
                    let n = stats.numcolors as usize * 4;
                    stats.palette[n..n + 4].copy_from_slice(&c);
                }
                stats.numcolors += 1;
                numcolors_done = stats.numcolors >= maxnumcolors;
            }

            if alpha_done && numcolors_done && colored_done && bits_done {
                break;
            }
        }

        if stats.key && !stats.alpha {
            for i in 0..numpixels {
                let [r, g, b, a] = get_pixel_color_rgba8(input, i, mode_in);
                if a != 0
                    && u32::from(r) == stats.key_r
                    && u32::from(g) == stats.key_g
                    && u32::from(b) == stats.key_b
                {
                    // Color key cannot be used if an opaque pixel also has that RGB color.
                    stats.alpha = true;
                    stats.key = false;
                    if stats.bits < 8 {
                        stats.bits = 8;
                    }
                }
            }
        }

        // make the stats's key always 16-bit for consistency - repeat each byte twice
        stats.key_r += stats.key_r << 8;
        stats.key_g += stats.key_g << 8;
        stats.key_b += stats.key_b << 8;
    }
    Ok(())
}

/// `auto_choose_color`: a minimal PNG color model that can contain all colors
/// indicated by the stats.
fn auto_choose_color(
    mode_out: &mut ColorMode,
    mode_in: &ColorMode,
    stats: &ColorStats,
) -> Result<(), u32> {
    let numpixels = stats.numpixels;
    let mut alpha = stats.alpha;
    let mut key = stats.key;
    let mut bits = stats.bits;

    mode_out.key_defined = false;

    if key && numpixels <= 16 {
        alpha = true; // too few pixels to justify tRNS chunk overhead
        key = false;
        if bits < 8 {
            bits = 8; // PNG has no alphachannel modes with less than 8-bit per channel
        }
    }

    let gray_ok = !stats.colored && stats.allow_greyscale;
    if !gray_ok && bits < 8 {
        bits = 8;
    }

    let n = stats.numcolors as usize;
    let palettebits: u32 = if n <= 2 {
        1
    } else if n <= 4 {
        2
    } else if n <= 16 {
        4
    } else {
        8
    };
    // n==0 means likely numcolors wasn't computed
    let mut palette_ok = n <= 256 && bits <= 8 && n != 0;
    if numpixels < n * 2 {
        palette_ok = false; // don't add palette overhead if image has only a few pixels
    }
    if gray_ok && !alpha && bits <= palettebits {
        palette_ok = false; // gray is less overhead
    }
    if !stats.allow_palette {
        palette_ok = false;
    }

    if palette_ok {
        let p = &stats.palette;
        mode_out.palette_clear(); // remove potential earlier palette
        for i in 0..n {
            mode_out.palette_add(p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3])?;
        }
        mode_out.colortype = LCT_PALETTE;
        mode_out.bitdepth = palettebits;

        if mode_in.colortype == LCT_PALETTE
            && mode_in.palettesize >= mode_out.palettesize
            && mode_in.bitdepth == mode_out.bitdepth
        {
            // If input should have same palette colors, keep original to preserve its order and prevent conversion
            mode_out.copy_from(mode_in);
        }
    } else {
        // 8-bit or 16-bit per channel
        mode_out.bitdepth = bits;
        mode_out.colortype = match (alpha, gray_ok) {
            (true, true) => LCT_GREY_ALPHA,
            (true, false) => LCT_RGBA,
            (false, true) => LCT_GREY,
            (false, false) => LCT_RGB,
        };
        if key {
            // stats always uses 16-bit, mask converts it
            let mask = (1u32 << mode_out.bitdepth) - 1;
            mode_out.key_r = stats.key_r & mask;
            mode_out.key_g = stats.key_g & mask;
            mode_out.key_b = stats.key_b & mask;
            mode_out.key_defined = true;
        }
    }
    Ok(())
}

/// `paethPredictor`.
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let (ai, bi, ci) = (i16::from(a), i16::from(b), i16::from(c));
    let mut pa = (bi - ci).abs();
    let pb = (ai - ci).abs();
    let pc = (ai + bi - ci - ci).abs();
    let mut r = a;
    if pb < pa {
        r = b;
        pa = pb;
    }
    if pc < pa { c } else { r }
}

const ADAM7_IX: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
const ADAM7_IY: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
const ADAM7_DX: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
const ADAM7_DY: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];

/// `Adam7_getpassvalues`.
struct Adam7Passes {
    passw: [u32; 7],
    passh: [u32; 7],
    filter_passstart: [usize; 8],
    padded_passstart: [usize; 8],
    passstart: [usize; 8],
}

fn adam7_getpassvalues(w: u32, h: u32, bpp: u32) -> Adam7Passes {
    let mut p = Adam7Passes {
        passw: [0; 7],
        passh: [0; 7],
        filter_passstart: [0; 8],
        padded_passstart: [0; 8],
        passstart: [0; 8],
    };
    // calculate width and height in pixels of each pass
    for i in 0..7 {
        p.passw[i] = w.wrapping_add(ADAM7_DX[i] - ADAM7_IX[i] - 1) / ADAM7_DX[i];
        p.passh[i] = h.wrapping_add(ADAM7_DY[i] - ADAM7_IY[i] - 1) / ADAM7_DY[i];
        if p.passw[i] == 0 {
            p.passh[i] = 0;
        }
        if p.passh[i] == 0 {
            p.passw[i] = 0;
        }
    }
    let bpp = bpp as usize;
    for i in 0..7 {
        let (pw, ph) = (p.passw[i] as usize, p.passh[i] as usize);
        // if passw[i] is 0, it's 0 bytes, not 1 (no filtertype-byte)
        p.filter_passstart[i + 1] = p.filter_passstart[i]
            + if pw != 0 && ph != 0 {
                ph * (1 + (pw * bpp).div_ceil(8))
            } else {
                0
            };
        // bits padded if needed to fill full byte at end of each scanline
        p.padded_passstart[i + 1] = p.padded_passstart[i] + ph * (pw * bpp).div_ceil(8);
        // only padded at end of reduced image
        p.passstart[i + 1] = p.passstart[i] + (ph * pw * bpp).div_ceil(8);
    }
    p
}

/// `LodePNGFilterStrategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStrategy {
    /// Every filter at zero.
    Zero = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    /// Use filter that gives minimum sum, as described in the official PNG filter heuristic.
    MinSum = 5,
    /// Use the filter type that gives smallest Shannon entropy for this scanline.
    Entropy = 6,
    /// Brute-force-search PNG filters by compressing each filter for each scanline.
    BruteForce = 7,
    /// Use predefined_filters buffer.
    Predefined = 8,
}

/// Port of `LodePNGEncoderSettings`.
#[derive(Debug, Clone)]
pub struct EncoderSettings {
    pub zlibsettings: CompressSettings,
    /// Automatically choose output PNG color type. Default: true.
    pub auto_convert: bool,
    /// If true, follows the official PNG heuristic: if the PNG uses a palette
    /// or lower than 8 bit depth, set all filters to zero.
    pub filter_palette_zero: bool,
    pub filter_strategy: FilterStrategy,
    /// Used if filter_strategy is Predefined: one filter type per scanline.
    pub predefined_filters: Vec<u8>,
    /// Force creating a PLTE chunk if colortype is 2 or 6.
    pub force_palette: bool,
}

/// `lodepng_encoder_settings_init`.
impl Default for EncoderSettings {
    fn default() -> EncoderSettings {
        EncoderSettings {
            zlibsettings: CompressSettings::default(),
            filter_palette_zero: true,
            filter_strategy: FilterStrategy::MinSum,
            auto_convert: true,
            force_palette: false,
            predefined_filters: Vec::new(),
        }
    }
}

/// `filterScanline`.
fn filter_scanline(
    out: &mut [u8],
    scanline: &[u8],
    prevline: Option<&[u8]>,
    length: usize,
    bytewidth: usize,
    filter_type: u8,
) {
    let s = scanline;
    match filter_type {
        0 => out[..length].copy_from_slice(&s[..length]),
        1 => {
            for i in 0..length {
                out[i] = if i < bytewidth {
                    s[i]
                } else {
                    s[i].wrapping_sub(s[i - bytewidth])
                };
            }
        }
        2 => match prevline {
            Some(p) => {
                for i in 0..length {
                    out[i] = s[i].wrapping_sub(p[i]);
                }
            }
            None => out[..length].copy_from_slice(&s[..length]),
        },
        3 => {
            for i in 0..length {
                let left = if i < bytewidth {
                    0
                } else {
                    u32::from(s[i - bytewidth])
                };
                let up = prevline.map_or(0, |p| u32::from(p[i]));
                out[i] = s[i].wrapping_sub(((left + up) >> 1) as u8);
            }
        }
        4 => match prevline {
            Some(p) => {
                for i in 0..length {
                    out[i] = if i < bytewidth {
                        s[i].wrapping_sub(p[i])
                    } else {
                        s[i].wrapping_sub(paeth_predictor(s[i - bytewidth], p[i], p[i - bytewidth]))
                    };
                }
            }
            None => {
                for i in 0..length {
                    out[i] = if i < bytewidth {
                        s[i]
                    } else {
                        s[i].wrapping_sub(s[i - bytewidth])
                    };
                }
            }
        },
        _ => {} // invalid filter type given
    }
}

/// `ilog2`: integer binary logarithm, max return value is 31.
fn ilog2(mut i: usize) -> usize {
    let mut result = 0;
    for (limit, shift) in [(65536, 16), (256, 8), (16, 4), (4, 2)] {
        if i >= limit {
            result += shift;
            i >>= shift;
        }
    }
    if i >= 2 {
        result += 1;
    }
    result
}

/// `ilog2i`: integer approximation for i * log2(i).
fn ilog2i(i: usize) -> usize {
    if i == 0 {
        return 0;
    }
    let l = ilog2(i);
    i * l + ((i - (1usize << l)) << 1)
}

/// `filter`: `out` gets `h + (w * h * bpp + 7) / 8` bytes of filtered scanlines.
fn filter(
    out: &mut [u8],
    input: &[u8],
    w: u32,
    h: u32,
    color: &ColorMode,
    settings: &EncoderSettings,
) -> Result<(), u32> {
    let bpp = color.bpp();
    // the width of a scanline in bytes, not including the filter type
    let linebytes = get_raw_size_idat(w, 1, bpp) - 1;
    // bytewidth is used for filtering, is 1 when bpp < 8, number of bytes per pixel otherwise
    let bytewidth = (bpp as usize).div_ceil(8);
    let mut strategy = settings.filter_strategy;

    if settings.filter_palette_zero && (color.colortype == LCT_PALETTE || color.bitdepth < 8) {
        // override the filter strategy with zero for palette and less-than-8-bitdepth images
        strategy = FilterStrategy::Zero;
    }

    if bpp == 0 {
        return Err(31); // invalid color type
    }

    let line = |y: usize| &input[y * linebytes..(y + 1) * linebytes];
    match strategy {
        FilterStrategy::Zero
        | FilterStrategy::One
        | FilterStrategy::Two
        | FilterStrategy::Three
        | FilterStrategy::Four
        | FilterStrategy::Predefined => {
            for y in 0..h as usize {
                let filter_type = if strategy == FilterStrategy::Predefined {
                    settings.predefined_filters.get(y).copied().unwrap_or(0)
                } else {
                    strategy as u8
                };
                let outindex = (1 + linebytes) * y; // the extra filterbyte added to each row
                out[outindex] = filter_type;
                let prevline = if y == 0 { None } else { Some(line(y - 1)) };
                filter_scanline(
                    &mut out[outindex + 1..],
                    line(y),
                    prevline,
                    linebytes,
                    bytewidth,
                    filter_type,
                );
            }
        }
        FilterStrategy::MinSum | FilterStrategy::Entropy | FilterStrategy::BruteForce => {
            // adaptive filtering: independently for each row, try all five filter types
            let mut attempt = vec![vec![0u8; linebytes]; 5];
            let mut zlibsettings = settings.zlibsettings;
            // use fixed tree on the attempts so that the tree is not adapted to the filtertype on purpose
            zlibsettings.btype = 1;
            for y in 0..h as usize {
                let prevline = if y == 0 { None } else { Some(line(y - 1)) };
                let mut best_type = 0usize;
                let mut best = 0usize;
                for (t, a) in attempt.iter_mut().enumerate() {
                    filter_scanline(a, line(y), prevline, linebytes, bytewidth, t as u8);
                    match strategy {
                        FilterStrategy::MinSum => {
                            // For differences, each byte should be treated as signed, values above
                            // 127 are negative. Filtertype 0 isn't a difference though, so use
                            // unsigned there.
                            let sum: usize = if t == 0 {
                                a.iter().map(|&s| usize::from(s)).sum()
                            } else {
                                a.iter()
                                    .map(|&s| {
                                        if s < 128 {
                                            usize::from(s)
                                        } else {
                                            255 - usize::from(s)
                                        }
                                    })
                                    .sum()
                            };
                            if t == 0 || sum < best {
                                best_type = t;
                                best = sum;
                            }
                        }
                        FilterStrategy::Entropy => {
                            let mut count = [0usize; 256];
                            for &s in a.iter() {
                                count[usize::from(s)] += 1;
                            }
                            count[t] += 1; // the filter type itself is part of the scanline
                            let sum: usize = count.iter().map(|&c| ilog2i(c)).sum();
                            if t == 0 || sum > best {
                                best_type = t;
                                best = sum;
                            }
                        }
                        _ => {
                            let size = lodepng_zlib_compress(a, &zlibsettings)
                                .map(|v| v.len())
                                .unwrap_or(0);
                            if t == 0 || size < best {
                                best_type = t;
                                best = size;
                            }
                        }
                    }
                }
                let o = y * (linebytes + 1);
                out[o] = best_type as u8; // the first byte of a scanline will be the filter type
                out[o + 1..o + 1 + linebytes].copy_from_slice(&attempt[best_type]);
            }
        }
    }
    Ok(())
}

/// `addPaddingBits`: olinebits must be >= ilinebits.
fn add_padding_bits(out: &mut [u8], input: &[u8], olinebits: usize, ilinebits: usize, h: u32) {
    let diff = olinebits - ilinebits;
    let (mut obp, mut ibp) = (0usize, 0usize);
    for _ in 0..h {
        for _ in 0..ilinebits {
            let bit = read_bit_from_reversed_stream(&mut ibp, input);
            set_bit_of_reversed_stream(&mut obp, out, bit);
        }
        for _ in 0..diff {
            set_bit_of_reversed_stream(&mut obp, out, 0);
        }
    }
}

/// `Adam7_interlace`.
fn adam7_interlace(out: &mut [u8], input: &[u8], w: u32, h: u32, bpp: u32) {
    let p = adam7_getpassvalues(w, h, bpp);
    for i in 0..7 {
        for y in 0..p.passh[i] as usize {
            for x in 0..p.passw[i] as usize {
                let iy = ADAM7_IY[i] as usize + y * ADAM7_DY[i] as usize;
                let ix = ADAM7_IX[i] as usize + x * ADAM7_DX[i] as usize;
                if bpp >= 8 {
                    let bytewidth = bpp as usize / 8;
                    let pixelinstart = (iy * w as usize + ix) * bytewidth;
                    let pixeloutstart = p.passstart[i] + (y * p.passw[i] as usize + x) * bytewidth;
                    out[pixeloutstart..pixeloutstart + bytewidth]
                        .copy_from_slice(&input[pixelinstart..pixelinstart + bytewidth]);
                } else {
                    let ilinebits = (bpp * p.passw[i]) as usize;
                    let olinebits = (bpp * w) as usize;
                    let mut ibp = iy * olinebits + ix * bpp as usize;
                    let mut obp = 8 * p.passstart[i] + (y * ilinebits + x * bpp as usize);
                    for _ in 0..bpp {
                        let bit = read_bit_from_reversed_stream(&mut ibp, input);
                        set_bit_of_reversed_stream(&mut obp, out, bit);
                    }
                }
            }
        }
    }
}

/// `preProcessScanlines`: the PNG colortype image into filtered, padded,
/// interlaced IDAT data.
fn pre_process_scanlines(
    input: &[u8],
    w: u32,
    h: u32,
    info_png: &Info,
    settings: &EncoderSettings,
) -> Result<Vec<u8>, u32> {
    let bpp = info_png.color.bpp() as usize;
    let (wz, hz) = (w as usize, h as usize);
    if info_png.interlace_method == 0 {
        // image size plus an extra byte per scanline + possible padding bits
        let mut out = vec![0u8; hz + hz * (wz * bpp).div_ceil(8)];
        // non multiple of 8 bits per scanline, padding bits needed per scanline
        if bpp < 8 && wz * bpp != (wz * bpp).div_ceil(8) * 8 {
            let mut padded = vec![0u8; hz * (wz * bpp).div_ceil(8)];
            add_padding_bits(&mut padded, input, (wz * bpp).div_ceil(8) * 8, wz * bpp, h);
            filter(&mut out, &padded, w, h, &info_png.color, settings)?;
        } else {
            // we can immediately filter into the out buffer, no other steps needed
            filter(&mut out, input, w, h, &info_png.color, settings)?;
        }
        Ok(out)
    } else {
        // interlace_method is 1 (Adam7)
        let p = adam7_getpassvalues(w, h, bpp as u32);
        let mut out = vec![0u8; p.filter_passstart[7]];
        let mut adam7 = vec![0u8; p.passstart[7]];
        adam7_interlace(&mut adam7, input, w, h, bpp as u32);
        for i in 0..7 {
            let pw = p.passw[i] as usize;
            let dst = &mut out[p.filter_passstart[i]..];
            if bpp < 8 {
                let mut padded = vec![0u8; p.padded_passstart[i + 1] - p.padded_passstart[i]];
                add_padding_bits(
                    &mut padded,
                    &adam7[p.passstart[i]..],
                    (pw * bpp).div_ceil(8) * 8,
                    pw * bpp,
                    p.passh[i],
                );
                filter(
                    dst,
                    &padded,
                    p.passw[i],
                    p.passh[i],
                    &info_png.color,
                    settings,
                )?;
            } else {
                filter(
                    dst,
                    &adam7[p.padded_passstart[i]..],
                    p.passw[i],
                    p.passh[i],
                    &info_png.color,
                    settings,
                )?;
            }
        }
        Ok(out)
    }
}

/// Port of `LodePNGInfo` (the encoder-supported fields).
#[derive(Debug, Clone, Default)]
pub struct Info {
    /// Interlace method of the original file: 0 = none, 1 = Adam7.
    pub interlace_method: u32,
    /// Color type and bits, palette and transparency of the PNG file.
    pub color: ColorMode,
    /// gAMA chunk: gamma times 100000.
    pub gama_defined: bool,
    pub gama_gamma: u32,
    /// cHRM chunk: chromaticities times 100000.
    pub chrm_defined: bool,
    pub chrm_white_x: u32,
    pub chrm_white_y: u32,
    pub chrm_red_x: u32,
    pub chrm_red_y: u32,
    pub chrm_green_x: u32,
    pub chrm_green_y: u32,
    pub chrm_blue_x: u32,
    pub chrm_blue_y: u32,
    /// sRGB chunk: rendering intent.
    pub srgb_defined: bool,
    pub srgb_intent: u32,
    /// sBIT chunk: significant bits.
    pub sbit_defined: bool,
    pub sbit_r: u32,
    pub sbit_g: u32,
    pub sbit_b: u32,
    pub sbit_a: u32,
    /// Compression method of the original file (always 0).
    pub compression_method: u32,
    /// Filter method of the original file (always 0).
    pub filter_method: u32,
    /// bKGD chunk.
    pub background_defined: bool,
    pub background_r: u32,
    pub background_g: u32,
    pub background_b: u32,
    /// tEXt / zTXt chunks: (key, string) pairs.
    pub texts: Vec<(Vec<u8>, Vec<u8>)>,
    /// iTXt chunks: (key, langtag, transkey, string).
    pub itexts: Vec<[Vec<u8>; 4]>,
    /// tIME chunk: year, month, day, hour, minute, second.
    pub time_defined: bool,
    pub time: [u32; 6],
    /// pHYs chunk.
    pub phys_defined: bool,
    pub phys_x: u32,
    pub phys_y: u32,
    pub phys_unit: u32,
    /// iCCP chunk.
    pub iccp_defined: bool,
    pub iccp_name: Vec<u8>,
    pub iccp_profile: Vec<u8>,
    /// cICP chunk: primaries, transfer, matrix coefficients, full range flag.
    pub cicp_defined: bool,
    pub cicp: [u32; 4],
    /// mDCV chunk: red, green, blue, white x/y, max / min luminance.
    pub mdcv_defined: bool,
    pub mdcv: [u32; 10],
    /// cLLI chunk: max CLL, max FALL.
    pub clli_defined: bool,
    pub clli: [u32; 2],
    /// eXIf chunk.
    pub exif_defined: bool,
    pub exif: Vec<u8>,
    /// Unknown chunks before PLTE, between PLTE and IDAT and after IDAT.
    pub unknown_chunks_data: [Vec<u8>; 3],
}

/// Port of `LodePNGState`.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub decoder: DecoderSettings,
    pub encoder: EncoderSettings,
    /// Specifies the format in which you would like to get the raw pixel buffer.
    pub info_raw: ColorMode,
    /// Info of the PNG image obtained after decoding / to encode.
    pub info_png: Info,
    pub error: u32,
}

impl State {
    /// `lodepng_state_init`.
    pub fn new() -> State {
        State::default()
    }
}

/// `lodepng_crc32` table (standard reflected CRC-32).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// Port of `lodepng_crc32`.
pub fn lodepng_crc32(data: &[u8]) -> u32 {
    let mut r = 0xffff_ffffu32;
    for &b in data {
        r = CRC32_TABLE[((r ^ u32::from(b)) & 0xff) as usize] ^ (r >> 8);
    }
    r ^ 0xffff_ffff
}

/// `lodepng_chunk_createv`: length, type, data and CRC.
fn add_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) -> Result<(), u32> {
    let length = u32::try_from(data.len()).map_err(|_| 77u32)?;
    out.extend_from_slice(&length.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = lodepng_crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
    Ok(())
}

/// `addChunk_IHDR`.
fn add_chunk_ihdr(
    out: &mut Vec<u8>,
    w: u32,
    h: u32,
    colortype: ColorType,
    bitdepth: u32,
    interlace_method: u32,
) -> Result<(), u32> {
    let mut data = Vec::with_capacity(13);
    data.extend_from_slice(&w.to_be_bytes());
    data.extend_from_slice(&h.to_be_bytes());
    data.extend_from_slice(&[
        bitdepth as u8,
        colortype as u8,
        0,
        0,
        interlace_method as u8,
    ]);
    add_chunk(out, b"IHDR", &data)
}

/// `addChunk_PLTE`.
fn add_chunk_plte(out: &mut Vec<u8>, info: &ColorMode) -> Result<(), u32> {
    if info.palettesize == 0 || info.palettesize > 256 {
        return Err(68); // invalid palette size, it is only allowed to be 1-256
    }
    let data: Vec<u8> = (0..info.palettesize)
        .flat_map(|i| info.palette[i * 4..i * 4 + 3].iter().copied())
        .collect();
    add_chunk(out, b"PLTE", &data)
}

/// `addChunk_tRNS`: only added when needed (a key or palette with alpha).
fn add_chunk_trns(out: &mut Vec<u8>, info: &ColorMode) -> Result<(), u32> {
    let be16 = |v: u32| [(v >> 8) as u8, (v & 255) as u8];
    match info.colortype {
        LCT_PALETTE => {
            // the tail of palette values that all have 255 as alpha, does not have to be encoded
            let mut amount = info.palettesize;
            while amount != 0 && info.palette[4 * (amount - 1) + 3] == 255 {
                amount -= 1;
            }
            if amount != 0 {
                let data: Vec<u8> = (0..amount).map(|i| info.palette[4 * i + 3]).collect();
                add_chunk(out, b"tRNS", &data)?;
            }
        }
        LCT_GREY if info.key_defined => add_chunk(out, b"tRNS", &be16(info.key_r))?,
        LCT_RGB if info.key_defined => {
            let mut data = Vec::with_capacity(6);
            for k in [info.key_r, info.key_g, info.key_b] {
                data.extend_from_slice(&be16(k));
            }
            add_chunk(out, b"tRNS", &data)?;
        }
        _ => {}
    }
    Ok(())
}

/// `addChunk_IDAT`.
fn add_chunk_idat(
    out: &mut Vec<u8>,
    data: &[u8],
    zlibsettings: &CompressSettings,
) -> Result<(), u32> {
    // max chunk length allowed by the specification is 2147483647 bytes
    const MAX_CHUNK_LENGTH: usize = 2147483647;
    let zlib = lodepng_zlib_compress(data, zlibsettings)?;
    let mut pos = 0;
    loop {
        if zlib.len() - pos > MAX_CHUNK_LENGTH {
            add_chunk(out, b"IDAT", &zlib[pos..pos + MAX_CHUNK_LENGTH])?;
            pos += MAX_CHUNK_LENGTH;
        } else {
            return add_chunk(out, b"IDAT", &zlib[pos..]);
        }
    }
}

/// `addChunk_gAMA`.
fn add_chunk_gama(out: &mut Vec<u8>, info: &Info) -> Result<(), u32> {
    add_chunk(out, b"gAMA", &info.gama_gamma.to_be_bytes())
}

/// `addChunk_cHRM`.
fn add_chunk_chrm(out: &mut Vec<u8>, info: &Info) -> Result<(), u32> {
    let mut data = Vec::with_capacity(32);
    for v in [
        info.chrm_white_x,
        info.chrm_white_y,
        info.chrm_red_x,
        info.chrm_red_y,
        info.chrm_green_x,
        info.chrm_green_y,
        info.chrm_blue_x,
        info.chrm_blue_y,
    ] {
        data.extend_from_slice(&v.to_be_bytes());
    }
    add_chunk(out, b"cHRM", &data)
}

/// `addChunk_sRGB`.
fn add_chunk_srgb(out: &mut Vec<u8>, info: &Info) -> Result<(), u32> {
    add_chunk(out, b"sRGB", &[info.srgb_intent as u8])
}

/// `addChunk_sBIT`.
fn add_chunk_sbit(out: &mut Vec<u8>, info: &Info) -> Result<(), u32> {
    let bitdepth = if info.color.colortype == LCT_PALETTE {
        8
    } else {
        info.color.bitdepth
    };
    let invalid = |vals: &[u32]| vals.iter().any(|&v| v == 0 || v > bitdepth);
    let data: Vec<u32> = match info.color.colortype {
        LCT_GREY => vec![info.sbit_r],
        LCT_RGB | LCT_PALETTE => vec![info.sbit_r, info.sbit_g, info.sbit_b],
        LCT_GREY_ALPHA => vec![info.sbit_r, info.sbit_a],
        LCT_RGBA => vec![info.sbit_r, info.sbit_g, info.sbit_b, info.sbit_a],
        _ => return Ok(()),
    };
    if invalid(&data) {
        return Err(115);
    }
    let bytes: Vec<u8> = data.iter().map(|&v| v as u8).collect();
    add_chunk(out, b"sBIT", &bytes)
}

/// Port of `lodepng_encode`: encodes `image` (`state.info_raw` format) as PNG.
/// Sets `state.error` like C and returns it as the error.
pub fn lodepng_encode(image: &[u8], w: u32, h: u32, state: &mut State) -> Result<Vec<u8>, u32> {
    let result = encode_impl(image, w, h, state);
    state.error = result.as_ref().err().copied().unwrap_or(0);
    result
}

fn encode_impl(image: &[u8], w: u32, h: u32, state: &State) -> Result<Vec<u8>, u32> {
    let info_png = &state.info_png;

    // check input values validity
    if (info_png.color.colortype == LCT_PALETTE || state.encoder.force_palette)
        && (info_png.color.palettesize == 0 || info_png.color.palettesize > 256)
    {
        return Err(68); // invalid palette size, it is only allowed to be 1-256
    }
    if state.encoder.zlibsettings.btype > 2 {
        return Err(61); // invalid btype
    }
    if info_png.interlace_method > 1 {
        return Err(71); // invalid interlace mode
    }
    check_color_validity(info_png.color.colortype, info_png.color.bitdepth)?;
    check_color_validity(state.info_raw.colortype, state.info_raw.bitdepth)?;

    // color convert and compute scanline filter types
    let mut info = info_png.clone();
    if state.encoder.auto_convert {
        let mut stats = ColorStats::default();
        let mut allow_convert = true;
        lodepng_compute_color_stats(&mut stats, image, w, h, &state.info_raw)?;
        let mut auto_color = ColorMode::default();
        auto_choose_color(&mut auto_color, &state.info_raw, &stats)?;
        if info_png.sbit_defined {
            // if sbit is defined, due to strict requirements of which sbit values can be
            // present for which color modes, auto_convert can't be done in many cases.
            let sbit_max = info_png
                .sbit_r
                .max(info_png.sbit_g)
                .max(info_png.sbit_b)
                .max(info_png.sbit_a);
            let equal = (info_png.sbit_g == 0 || info_png.sbit_g == info_png.sbit_r)
                && (info_png.sbit_b == 0 || info_png.sbit_b == info_png.sbit_r)
                && (info_png.sbit_a == 0 || info_png.sbit_a == info_png.sbit_r);
            let (ct, act) = (info.color.colortype, auto_color.colortype);
            allow_convert = false;
            if ct == LCT_PALETTE && act == LCT_PALETTE {
                // input and output are palette
                allow_convert = true;
            }
            // going from 8-bit RGB to palette (or 16-bit as long as sbit_max <= 8) is possible
            if ct == LCT_RGB && act == LCT_PALETTE && sbit_max <= 8 {
                allow_convert = true;
            }
            // going from 8-bit RGBA to palette is also ok but only if sbit_a is exactly 8
            if ct == LCT_RGBA && act == LCT_PALETTE && info_png.sbit_a == 8 && sbit_max <= 8 {
                allow_convert = true;
            }
            // going from 16-bit RGB(A) to 8-bit RGB(A) is ok if all sbit values are <= 8
            if (ct == LCT_RGB || ct == LCT_RGBA)
                && info.color.bitdepth == 16
                && act == ct
                && auto_color.bitdepth == 8
                && sbit_max <= 8
            {
                allow_convert = true;
            }
            // going to less channels is ok if all bit values are equal
            if ct != LCT_PALETTE
                && act != LCT_PALETTE
                && equal
                && info_png.sbit_r == auto_color.bitdepth
            {
                allow_convert = true;
            }
        }
        if state.encoder.force_palette
            && info.color.colortype != LCT_GREY
            && info.color.colortype != LCT_GREY_ALPHA
            && (auto_color.colortype == LCT_GREY || auto_color.colortype == LCT_GREY_ALPHA)
        {
            // user speficially forced a PLTE palette, so cannot convert to grayscale types
            allow_convert = false;
        }
        if allow_convert {
            info.color.copy_from(&auto_color);
        }
    }

    let data = if !state.info_raw.equal(&info.color) {
        let size = (w as usize * h as usize * info.color.bpp() as usize).div_ceil(8);
        let mut converted = vec![0u8; size];
        lodepng_convert(&mut converted, image, &info.color, &state.info_raw, w, h)?;
        pre_process_scanlines(&converted, w, h, &info, &state.encoder)?
    } else {
        pre_process_scanlines(image, w, h, &info, &state.encoder)?
    };

    // output all PNG chunks
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    add_chunk_ihdr(
        &mut out,
        w,
        h,
        info.color.colortype,
        info.color.bitdepth,
        info.interlace_method,
    )?;
    // color profile chunks must come before PLTE
    if info.srgb_defined {
        add_chunk_srgb(&mut out, &info)?;
    }
    if info.gama_defined {
        add_chunk_gama(&mut out, &info)?;
    }
    if info.chrm_defined {
        add_chunk_chrm(&mut out, &info)?;
    }
    if info_png.sbit_defined {
        add_chunk_sbit(&mut out, &info)?;
    }
    if info.color.colortype == LCT_PALETTE {
        add_chunk_plte(&mut out, &info.color)?;
    }
    if state.encoder.force_palette
        && (info.color.colortype == LCT_RGB || info.color.colortype == LCT_RGBA)
    {
        // force_palette means: write suggested palette for truecolor in PLTE chunk
        add_chunk_plte(&mut out, &info.color)?;
    }
    // tRNS (this will only add if when necessary)
    add_chunk_trns(&mut out, &info.color)?;
    // IDAT (multiple IDAT chunks must be consecutive)
    add_chunk_idat(&mut out, &data, &state.encoder.zlibsettings)?;
    add_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

/// Port of the C++ `lodepng::encode(out, in, w, h, state)` (vector input):
/// checks the input size, then `lodepng_encode`.
pub fn encode(image: &[u8], w: u32, h: u32, state: &mut State) -> Result<Vec<u8>, u32> {
    if get_raw_size(w, h, &state.info_raw) > image.len() {
        return Err(84);
    }
    lodepng_encode(image, w, h, state)
}

/// Port of `lodepng_error_text` (generated from lodepng.cpp).
pub fn lodepng_error_text(code: u32) -> &'static str {
    match code {
        0 => "no error, everything went ok",
        1 => "nothing done yet",
        10 => "end of input memory reached without huffman end code",
        11 => "error in code tree made it jump outside of huffman tree",
        13 => "problem while processing dynamic deflate block",
        14 => "problem while processing dynamic deflate block",
        15 => "problem while processing dynamic deflate block",
        16 => "invalid code while processing dynamic deflate block",
        17 => "end of out buffer memory reached while inflating",
        18 => "invalid distance code while inflating",
        19 => "end of out buffer memory reached while inflating",
        20 => "invalid deflate block BTYPE encountered while decoding",
        21 => "NLEN is not ones complement of LEN in a deflate block",
        22 => "end of out buffer memory reached while inflating",
        23 => "end of in buffer memory reached while inflating",
        24 => "invalid FCHECK in zlib header",
        25 => "invalid compression method in zlib header",
        26 => "FDICT encountered in zlib header while it's not used for PNG",
        27 => "PNG file is smaller than a PNG header",
        28 => "incorrect PNG signature, it's no PNG or corrupted",
        29 => "first chunk is not the header chunk",
        30 => "chunk length too large, chunk broken off at end of file",
        31 => "illegal PNG color type or bpp",
        32 => "illegal PNG compression method",
        33 => "illegal PNG filter method",
        34 => "illegal PNG interlace method",
        35 => "chunk length of a chunk is too large or the chunk too small",
        36 => "illegal PNG filter type encountered",
        37 => "illegal bit depth for this color type given",
        38 => "the palette is too small or too big",
        39 => "tRNS chunk before PLTE or has more entries than palette size",
        40 => "tRNS chunk has wrong size for grayscale image",
        41 => "tRNS chunk has wrong size for RGB image",
        42 => "tRNS chunk appeared while it was not allowed for this color type",
        43 => "bKGD chunk has wrong size for palette image",
        44 => "bKGD chunk has wrong size for grayscale image",
        45 => "bKGD chunk has wrong size for RGB image",
        48 => "empty input buffer given to decoder. Maybe caused by non-existing file?",
        49 => "jumped past memory while generating dynamic huffman tree",
        50 => "jumped past memory while generating dynamic huffman tree",
        51 => "jumped past memory while inflating huffman block",
        52 => "jumped past memory while inflating",
        53 => "size of zlib data too small",
        54 => "repeat symbol in tree while there was no value symbol yet",
        55 => "jumped past tree while generating huffman tree",
        56 => "given output image colortype or bitdepth not supported for color conversion",
        57 => "invalid CRC encountered (checking CRC can be disabled)",
        58 => "invalid ADLER32 encountered (checking ADLER32 can be disabled)",
        59 => "requested color conversion not supported",
        60 => "invalid window size given in the settings of the encoder (must be 0-32768)",
        61 => "invalid BTYPE given in the settings of the encoder (only 0, 1 and 2 are allowed)",
        62 => "conversion from color to grayscale not supported",
        63 => "length of a chunk too long, max allowed for PNG is 2147483647 bytes per chunk",
        64 => "the length of the END symbol 256 in the Huffman tree is 0",
        66 => {
            "the length of a text chunk keyword given to the encoder is longer than the maximum of 79 bytes"
        }
        67 => {
            "the length of a text chunk keyword given to the encoder is smaller than the minimum of 1 byte"
        }
        68 => {
            "tried to encode a PLTE chunk with a palette that has less than 1 or more than 256 colors"
        }
        69 => "unknown chunk type with 'critical' flag encountered by the decoder",
        71 => "invalid interlace mode given to encoder (must be 0 or 1)",
        72 => {
            "while decoding, invalid compression method encountering in zTXt or iTXt chunk (it must be 0)"
        }
        73 => "invalid tIME chunk size",
        74 => "invalid pHYs chunk size",
        75 => "no null termination char found while decoding text chunk",
        76 => "iTXt chunk too short to contain required bytes",
        77 => "integer overflow in buffer size",
        78 => "failed to open file for reading",
        79 => "failed to open file for writing",
        80 => "tried creating a tree of 0 symbols",
        81 => "lazy matching at pos 0 is impossible",
        82 => {
            "color conversion to palette requested while a color isn't in palette, or index out of bounds"
        }
        83 => "memory allocation failed",
        84 => "given image too small to contain all pixels to be encoded",
        86 => "impossible offset in lz77 encoding (internal bug)",
        87 => "must provide custom zlib function pointer if LODEPNG_COMPILE_ZLIB is not defined",
        88 => "invalid filter strategy given for LodePNGEncoderSettings.filter_strategy",
        89 => "text chunk keyword too short or long: must have size 1-79",
        90 => "windowsize must be a power of two",
        91 => "invalid decompressed idat size",
        92 => "integer overflow due to too many pixels",
        93 => "zero width or height is invalid",
        94 => "header chunk must have a size of 13 bytes",
        95 => "integer overflow with combined idat chunk size",
        96 => "invalid gAMA chunk size",
        97 => "invalid cHRM chunk size",
        98 => "invalid sRGB chunk size",
        99 => "invalid sRGB rendering intent",
        100 => "invalid ICC profile color type, the PNG specification only allows RGB or GRAY",
        101 => {
            "PNG specification does not allow RGB ICC profile on gray color types and vice versa"
        }
        102 => "not allowed to set grayscale ICC profile with colored pixels by PNG specification",
        103 => "invalid palette index in bKGD chunk. Maybe it came before PLTE chunk?",
        104 => "invalid bKGD color while encoding (e.g. palette index out of range)",
        105 => "integer overflow of bitsize",
        106 => "PNG file must have PLTE chunk if color type is palette",
        107 => "color convert from palette mode requested without setting the palette data in it",
        108 => "tried to add more than 256 values to a palette",
        109 => "tried to decompress zlib or deflate data larger than desired max_output_size",
        110 => "custom zlib or inflate decompression failed",
        111 => "custom zlib or deflate compression failed",
        112 => "compressed text unreasonably large",
        113 => "ICC profile unreasonably large",
        114 => "sBIT chunk has wrong size for the color type of the image",
        115 => "sBIT value out of range",
        116 => "cICP value out of range",
        117 => "invalid cICP chunk size",
        118 => "mDCV value out of range",
        119 => "invalid mDCV chunk size",
        120 => "invalid cLLI chunk size",
        121 => "invalid chunk type name: may only contain [a-zA-Z]",
        122 => "invalid chunk type name: third character must be uppercase",
        123 => "destination buffer too small",
        _ => "unknown error code",
    }
}
