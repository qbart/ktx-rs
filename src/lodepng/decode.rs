// SPDX-FileCopyrightText: 2005-2025 Lode Vandevenne
// SPDX-License-Identifier: Zlib
// Translated to Rust from LodePNG and modified; see NOTICE.md.
//! Port of the LodePNG decoder with KTX-Software's split API: `decodeGeneric`
//! is `lodepng_decode_chunks` (parse all chunks, collect IDAT) followed by
//! `inflateIdat`, and `lodepng_finish_decode` inflates / converts into a
//! caller buffer. In-place C buffer reuse (unfilter, padding removal) is done
//! with temporary copies, which gives the same bytes.

use super::zlib::{DecompressSettings, zlib_decompressv};
use super::*;

/// Port of `LodePNGDecoderSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderSettings {
    pub zlibsettings: DecompressSettings,
    /// Ignore CRC checksums.
    pub ignore_crc: bool,
    /// Ignore unknown critical chunks.
    pub ignore_critical: bool,
    /// Ignore issues at end of file if possible (missing IEND chunk, too large chunk, ...).
    pub ignore_end: bool,
    /// Convert to `info_raw` (or keep the PNG's color type).
    pub color_convert: bool,
    /// If false, ignore all text chunks.
    pub read_text_chunks: bool,
    /// Store all unrecognized chunks and their data in `unknown_chunks_data`.
    pub remember_unknown_chunks: bool,
    /// Maximum size for decompressed text chunks.
    pub max_text_size: usize,
    /// Maximum size for decompressed ICC chunks.
    pub max_icc_size: usize,
}

/// `lodepng_decoder_settings_init`.
impl Default for DecoderSettings {
    fn default() -> DecoderSettings {
        DecoderSettings {
            zlibsettings: DecompressSettings::default(),
            ignore_crc: false,
            ignore_critical: false,
            ignore_end: false,
            color_convert: true,
            read_text_chunks: true,
            remember_unknown_chunks: false,
            max_text_size: 16_777_216,
            max_icc_size: 16_777_216,
        }
    }
}

fn read32(b: &[u8], pos: usize) -> u32 {
    let byte = |i: usize| b.get(pos + i).copied().unwrap_or(0);
    u32::from_be_bytes([byte(0), byte(1), byte(2), byte(3)])
}

fn read16(b: &[u8], pos: usize) -> u32 {
    256 * u32::from(b.get(pos).copied().unwrap_or(0))
        + u32::from(b.get(pos + 1).copied().unwrap_or(0))
}

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// `lodepng_chunk_type_equals` for the chunk at `pos`.
fn chunk_type_equals(input: &[u8], pos: usize, t: &[u8; 4]) -> bool {
    input.get(pos + 4..pos + 8) == Some(&t[..])
}

/// `lodepng_chunk_check_crc`: true if the CRC is wrong.
fn chunk_crc_wrong(input: &[u8], pos: usize) -> bool {
    let length = read32(input, pos) as usize;
    let crc = read32(input, pos + 8 + length);
    let data = input.get(pos + 4..pos + 8 + length).unwrap_or_default();
    lodepng_crc32(data) != crc
}

/// Port of `lodepng_chunk_next_const` on chunk offsets (`end` = input length).
pub fn lodepng_chunk_next_const(input: &[u8], pos: usize, end: usize) -> usize {
    if pos >= end || end - pos < 12 {
        return end;
    }
    if input.get(pos..pos + 8) == Some(&PNG_SIGNATURE[..]) {
        // PNG magic header at start of PNG file: jump to first actual chunk
        return pos + 8;
    }
    let total_chunk_length = read32(input, pos) as usize + 12;
    if total_chunk_length > end - pos {
        return end;
    }
    pos + total_chunk_length
}

/// Port of `lodepng_chunk_find_const` on chunk offsets.
pub fn lodepng_chunk_find_const(
    input: &[u8],
    mut pos: usize,
    end: usize,
    t: &[u8; 4],
) -> Option<usize> {
    loop {
        if pos >= end || end - pos < 12 {
            return None;
        }
        if chunk_type_equals(input, pos, t) {
            return Some(pos);
        }
        pos = lodepng_chunk_next_const(input, pos, end);
    }
}

/// `lodepng_color_mode_equal`.
pub fn lodepng_color_mode_equal(a: &ColorMode, b: &ColorMode) -> bool {
    if a.colortype != b.colortype || a.bitdepth != b.bitdepth || a.key_defined != b.key_defined {
        return false;
    }
    if a.key_defined && (a.key_r != b.key_r || a.key_g != b.key_g || a.key_b != b.key_b) {
        return false;
    }
    if a.palettesize != b.palettesize {
        return false;
    }
    let n = a.palettesize * 4;
    a.palette.get(..n) == b.palette.get(..n)
}

/// `lodepng_pixel_overflow`.
fn pixel_overflow(w: u32, h: u32, pngcolor: &ColorMode, rawcolor: &ColorMode) -> bool {
    let bpp = pngcolor.bpp().max(rawcolor.bpp()) as usize;
    let (w, h) = (w as usize, h as usize);
    let check = || -> Option<()> {
        let numpixels = w.checked_mul(h)?;
        numpixels.checked_mul(8)?;
        let line = (w / 8).checked_mul(bpp)?;
        let line = line.checked_add(((w & 7) * bpp).div_ceil(8))?;
        let line = line.checked_add(5)?;
        line.checked_mul(h)?;
        Some(())
    };
    check().is_none()
}

/// Port of `lodepng_inspect`: reads the header and resets `state.info_png`.
pub fn lodepng_inspect(state: &mut State, input: &[u8]) -> Result<(u32, u32), u32> {
    let r = inspect(state, input);
    if let Err(e) = r {
        state.error = e;
    }
    r
}

fn inspect(state: &mut State, input: &[u8]) -> Result<(u32, u32), u32> {
    if input.is_empty() {
        return Err(48);
    }
    if input.len() < 33 {
        return Err(27);
    }
    // make sure all parameters created after previous decoding are reset
    state.info_png = Info::default();
    let info = &mut state.info_png;
    if input[..8] != PNG_SIGNATURE {
        return Err(28);
    }
    if read32(input, 8) != 13 {
        return Err(94);
    }
    if !chunk_type_equals(input, 8, b"IHDR") {
        return Err(29);
    }
    let width = read32(input, 16);
    let height = read32(input, 20);
    info.color.bitdepth = u32::from(input[24]);
    info.color.colortype = u32::from(input[25]);
    info.compression_method = u32::from(input[26]);
    info.filter_method = u32::from(input[27]);
    info.interlace_method = u32::from(input[28]);

    if width == 0 || height == 0 {
        return Err(93);
    }
    check_color_validity(info.color.colortype, info.color.bitdepth)?;
    if info.compression_method != 0 {
        return Err(32);
    }
    if info.filter_method != 0 {
        return Err(33);
    }
    if info.interlace_method > 1 {
        return Err(34);
    }
    if !state.decoder.ignore_crc && read32(input, 29) != lodepng_crc32(&input[12..29]) {
        return Err(57);
    }
    Ok((width, height))
}

/// `unfilterScanline`: `precon` is the previous unfiltered scanline.
fn unfilter_scanline(
    recon: &mut [u8],
    scanline: &[u8],
    precon: Option<&[u8]>,
    bytewidth: usize,
    filter_type: u8,
) -> Result<(), u32> {
    let length = recon.len();
    let bw = bytewidth.min(length);
    match filter_type {
        0 => recon.copy_from_slice(scanline),
        1 => {
            recon[..bw].copy_from_slice(&scanline[..bw]);
            for i in bw..length {
                recon[i] = scanline[i].wrapping_add(recon[i - bytewidth]);
            }
        }
        2 => match precon {
            Some(p) => {
                for i in 0..length {
                    recon[i] = scanline[i].wrapping_add(p[i]);
                }
            }
            None => recon.copy_from_slice(scanline),
        },
        3 => match precon {
            Some(p) => {
                for i in 0..bw {
                    recon[i] = scanline[i].wrapping_add(p[i] >> 1);
                }
                for i in bw..length {
                    let avg = (u32::from(recon[i - bytewidth]) + u32::from(p[i])) >> 1;
                    recon[i] = scanline[i].wrapping_add(avg as u8);
                }
            }
            None => {
                recon[..bw].copy_from_slice(&scanline[..bw]);
                for i in bw..length {
                    recon[i] = scanline[i].wrapping_add(recon[i - bytewidth] >> 1);
                }
            }
        },
        4 => match precon {
            Some(p) => {
                for i in 0..bw {
                    // paethPredictor(0, precon[i], 0) is always precon[i]
                    recon[i] = scanline[i].wrapping_add(p[i]);
                }
                for i in bw..length {
                    let pred = paeth_predictor(recon[i - bytewidth], p[i], p[i - bytewidth]);
                    recon[i] = scanline[i].wrapping_add(pred);
                }
            }
            None => {
                recon[..bw].copy_from_slice(&scanline[..bw]);
                for i in bw..length {
                    recon[i] = scanline[i].wrapping_add(recon[i - bytewidth]);
                }
            }
        },
        _ => return Err(36),
    }
    Ok(())
}

/// `unfilter`: `input` has one filter type byte per scanline.
fn unfilter(out: &mut [u8], input: &[u8], w: u32, h: u32, bpp: u32) -> Result<(), u32> {
    let bytewidth = bpp.div_ceil(8) as usize;
    let linebytes = get_raw_size_idat(w, 1, bpp) - 1;
    for y in 0..h as usize {
        let outindex = linebytes * y;
        let inindex = (1 + linebytes) * y;
        let filter_type = input[inindex];
        let (prev, rest) = out.split_at_mut(outindex);
        let precon = if y == 0 {
            None
        } else {
            Some(&prev[outindex - linebytes..])
        };
        unfilter_scanline(
            &mut rest[..linebytes],
            &input[inindex + 1..inindex + 1 + linebytes],
            precon,
            bytewidth,
            filter_type,
        )?;
    }
    Ok(())
}

/// `removePaddingBits`.
fn remove_padding_bits(out: &mut [u8], input: &[u8], olinebits: usize, ilinebits: usize, h: u32) {
    let diff = ilinebits - olinebits;
    let (mut ibp, mut obp) = (0usize, 0usize);
    for _ in 0..h {
        for _ in 0..olinebits {
            let bit = read_bit_from_reversed_stream(&mut ibp, input);
            set_bit_of_reversed_stream(&mut obp, out, bit);
        }
        ibp += diff;
    }
}

/// `Adam7_deinterlace`.
fn adam7_deinterlace(out: &mut [u8], input: &[u8], w: u32, h: u32, bpp: u32) {
    let p = adam7_getpassvalues(w, h, bpp);
    let w = w as usize;
    for i in 0..7 {
        let (pw, ph) = (p.passw[i] as usize, p.passh[i] as usize);
        let (ix, iy, dx, dy) = (
            ADAM7_IX[i] as usize,
            ADAM7_IY[i] as usize,
            ADAM7_DX[i] as usize,
            ADAM7_DY[i] as usize,
        );
        if bpp >= 8 {
            let bytewidth = bpp as usize / 8;
            for y in 0..ph {
                for x in 0..pw {
                    let pixelinstart = p.passstart[i] + (y * pw + x) * bytewidth;
                    let pixeloutstart = ((iy + y * dy) * w + ix + x * dx) * bytewidth;
                    out[pixeloutstart..pixeloutstart + bytewidth]
                        .copy_from_slice(&input[pixelinstart..pixelinstart + bytewidth]);
                }
            }
        } else {
            let bpp = bpp as usize;
            let ilinebits = bpp * pw;
            let olinebits = bpp * w;
            for y in 0..ph {
                for x in 0..pw {
                    let mut ibp = 8 * p.passstart[i] + (y * ilinebits + x * bpp);
                    let mut obp = (iy + y * dy) * olinebits + (ix + x * dx) * bpp;
                    for _ in 0..bpp {
                        let bit = read_bit_from_reversed_stream(&mut ibp, input);
                        set_bit_of_reversed_stream(&mut obp, out, bit);
                    }
                }
            }
        }
    }
}

/// `postProcessScanlines`: `input` is the full decompressed IDAT data.
fn post_process_scanlines(
    out: &mut [u8],
    input: &mut [u8],
    w: u32,
    h: u32,
    info_png: &Info,
) -> Result<(), u32> {
    let bpp = info_png.color.bpp();
    if bpp == 0 {
        return Err(31);
    }
    if info_png.interlace_method == 0 {
        let linebits = w.wrapping_mul(bpp);
        let padded = linebits.wrapping_add(7) / 8 * 8;
        if bpp < 8 && linebits != padded {
            let filtered = input.to_vec();
            unfilter(input, &filtered, w, h, bpp)?;
            remove_padding_bits(out, input, linebits as usize, padded as usize, h);
        } else {
            unfilter(out, input, w, h, bpp)?;
        }
    } else {
        let p = adam7_getpassvalues(w, h, bpp);
        for i in 0..7 {
            let (pw, ph) = (p.passw[i], p.passh[i]);
            let filtered = input[p.filter_passstart[i]..p.filter_passstart[i + 1]].to_vec();
            unfilter(&mut input[p.padded_passstart[i]..], &filtered, pw, ph, bpp)?;
            if bpp < 8 {
                // remove padding bits in scanlines; each reduced image still starts at a byte
                let padded = input[p.padded_passstart[i]..p.padded_passstart[i + 1]].to_vec();
                let olinebits = (pw * bpp) as usize;
                let ilinebits = (pw * bpp).div_ceil(8) as usize * 8;
                remove_padding_bits(
                    &mut input[p.passstart[i]..],
                    &padded,
                    olinebits,
                    ilinebits,
                    ph,
                );
            }
        }
        adam7_deinterlace(out, input, w, h, bpp);
    }
    Ok(())
}

/// `readChunk_PLTE`.
fn read_chunk_plte(color: &mut ColorMode, data: &[u8]) -> Result<(), u32> {
    color.palettesize = data.len() / 3;
    if color.palettesize == 0 || color.palettesize > 256 {
        return Err(38);
    }
    // lodepng_color_mode_alloc_palette: unused colors are opaque black
    color.palette = [0, 0, 0, 255].repeat(256);
    for i in 0..color.palettesize {
        color.palette[4 * i..4 * i + 3].copy_from_slice(&data[3 * i..3 * i + 3]);
        color.palette[4 * i + 3] = 255;
    }
    Ok(())
}

/// `readChunk_tRNS`.
fn read_chunk_trns(color: &mut ColorMode, data: &[u8]) -> Result<(), u32> {
    match color.colortype {
        LCT_PALETTE => {
            if data.len() > color.palettesize {
                return Err(39);
            }
            for (i, &a) in data.iter().enumerate() {
                color.palette[4 * i + 3] = a;
            }
        }
        LCT_GREY => {
            if data.len() != 2 {
                return Err(30);
            }
            color.key_defined = true;
            let k = read16(data, 0);
            (color.key_r, color.key_g, color.key_b) = (k, k, k);
        }
        LCT_RGB => {
            if data.len() != 6 {
                return Err(41);
            }
            color.key_defined = true;
            color.key_r = read16(data, 0);
            color.key_g = read16(data, 2);
            color.key_b = read16(data, 4);
        }
        _ => return Err(42),
    }
    Ok(())
}

/// `readChunk_bKGD`.
fn read_chunk_bkgd(info: &mut Info, data: &[u8]) -> Result<(), u32> {
    match info.color.colortype {
        LCT_PALETTE => {
            if data.len() != 1 {
                return Err(43);
            }
            if data[0] as usize >= info.color.palettesize {
                return Err(103);
            }
            info.background_defined = true;
            let v = u32::from(data[0]);
            (info.background_r, info.background_g, info.background_b) = (v, v, v);
        }
        LCT_GREY | LCT_GREY_ALPHA => {
            if data.len() != 2 {
                return Err(44);
            }
            info.background_defined = true;
            let v = read16(data, 0);
            (info.background_r, info.background_g, info.background_b) = (v, v, v);
        }
        LCT_RGB | LCT_RGBA => {
            if data.len() != 6 {
                return Err(45);
            }
            info.background_defined = true;
            info.background_r = read16(data, 0);
            info.background_g = read16(data, 2);
            info.background_b = read16(data, 4);
        }
        _ => {}
    }
    Ok(())
}

/// Length of the NUL terminated string at the start of `data` (bounded).
fn strlen_bounded(data: &[u8]) -> usize {
    data.iter().position(|&c| c == 0).unwrap_or(data.len())
}

/// A C string copied with `strlen` semantics (`lodepng_add_text` uses strlen).
fn c_string(data: &[u8]) -> Vec<u8> {
    data[..strlen_bounded(data)].to_vec()
}

/// `readChunk_tEXt`.
fn read_chunk_text(info: &mut Info, data: &[u8]) -> Result<(), u32> {
    let length = strlen_bounded(data);
    if !(1..=79).contains(&length) {
        return Err(89);
    }
    let key = data[..length].to_vec();
    let string2_begin = length + 1;
    let str = data.get(string2_begin..).unwrap_or_default();
    info.texts.push((key, c_string(str)));
    Ok(())
}

/// `zlib_decompress` of a text / ICC payload with `max_output_size`; maps a
/// too large output to `too_large`.
fn decompress_limited(
    data: &[u8],
    settings: &DecoderSettings,
    max: usize,
    too_large: u32,
) -> Result<Vec<u8>, u32> {
    let mut zs = settings.zlibsettings;
    zs.max_output_size = max;
    let mut out = Vec::new();
    match zlib_decompressv(&mut out, data, &zs) {
        Ok(()) => Ok(out),
        Err(e) => Err(if out.len() > max { too_large } else { e }),
    }
}

/// `readChunk_zTXt`.
fn read_chunk_ztxt(info: &mut Info, decoder: &DecoderSettings, data: &[u8]) -> Result<(), u32> {
    let length = strlen_bounded(data);
    if length + 2 >= data.len() {
        return Err(75);
    }
    if !(1..=79).contains(&length) {
        return Err(89);
    }
    let key = data[..length].to_vec();
    if data[length + 1] != 0 {
        return Err(72);
    }
    let str = decompress_limited(&data[length + 2..], decoder, decoder.max_text_size, 112)?;
    info.texts.push((key, str));
    Ok(())
}

/// `readChunk_iTXt`.
fn read_chunk_itxt(info: &mut Info, decoder: &DecoderSettings, data: &[u8]) -> Result<(), u32> {
    if data.len() < 5 {
        return Err(30);
    }
    let length = strlen_bounded(data);
    if length + 3 >= data.len() {
        return Err(75);
    }
    if !(1..=79).contains(&length) {
        return Err(89);
    }
    let key = data[..length].to_vec();
    let compressed = data[length + 1];
    if data[length + 2] != 0 {
        return Err(72);
    }
    let mut begin = length + 3;
    let langtag = c_string(data.get(begin..).unwrap_or_default());
    begin += langtag.len() + 1;
    let transkey = c_string(data.get(begin..).unwrap_or_default());
    begin += transkey.len() + 1;
    let text = data.get(begin..).unwrap_or_default();
    let str = if compressed != 0 {
        decompress_limited(text, decoder, decoder.max_text_size, 112)?
    } else {
        text.to_vec()
    };
    info.itexts.push([key, langtag, transkey, str]);
    Ok(())
}

/// `readChunk_iCCP`.
fn read_chunk_iccp(info: &mut Info, decoder: &DecoderSettings, data: &[u8]) -> Result<(), u32> {
    info.iccp_name.clear();
    info.iccp_profile.clear();
    info.iccp_defined = true;
    let length = strlen_bounded(data);
    if length + 2 >= data.len() {
        return Err(75);
    }
    if !(1..=79).contains(&length) {
        return Err(89);
    }
    info.iccp_name = data[..length].to_vec();
    if data[length + 1] != 0 {
        return Err(72);
    }
    let mut zs = decoder.zlibsettings;
    zs.max_output_size = decoder.max_icc_size;
    let mut profile = Vec::new();
    let result = zlib_decompressv(&mut profile, &data[length + 2..], &zs);
    let too_large = profile.len() > decoder.max_icc_size;
    info.iccp_profile = profile;
    match result {
        Err(_) if too_large => Err(113),
        Err(e) => Err(e),
        Ok(()) if info.iccp_profile.is_empty() => Err(100),
        Ok(()) => Ok(()),
    }
}

/// `readChunk_sBIT`.
fn read_chunk_sbit(info: &mut Info, data: &[u8]) -> Result<(), u32> {
    let bitdepth = if info.color.colortype == LCT_PALETTE {
        8
    } else {
        info.color.bitdepth
    };
    let n = match info.color.colortype {
        LCT_GREY => 1,
        LCT_RGB | LCT_PALETTE => 3,
        LCT_GREY_ALPHA => 2,
        LCT_RGBA => 4,
        _ => return Ok(()),
    };
    if data.len() != n {
        return Err(114);
    }
    if data.contains(&0) {
        return Err(115);
    }
    if data.iter().any(|&b| u32::from(b) > bitdepth) {
        return Err(115);
    }
    let d = data.iter().map(|&b| u32::from(b)).collect::<Vec<_>>();
    info.sbit_defined = true;
    match info.color.colortype {
        LCT_GREY => (info.sbit_r, info.sbit_g, info.sbit_b) = (d[0], d[0], d[0]),
        LCT_GREY_ALPHA => {
            (info.sbit_r, info.sbit_g, info.sbit_b) = (d[0], d[0], d[0]);
            info.sbit_a = d[1];
        }
        _ => {
            (info.sbit_r, info.sbit_g, info.sbit_b) = (d[0], d[1], d[2]);
            if n == 4 {
                info.sbit_a = d[3];
            }
        }
    }
    Ok(())
}

/// Reads one known chunk; `None` for an unhandled chunk type.
fn read_known_chunk(
    state: &mut State,
    chunk_type: &[u8],
    data: &[u8],
    in_decode: bool,
) -> Option<Result<(), u32>> {
    let info = &mut state.info_png;
    let decoder = state.decoder;
    let be = |i: usize| read32(data, i);
    let r = match chunk_type {
        b"PLTE" => read_chunk_plte(&mut info.color, data),
        b"tRNS" => read_chunk_trns(&mut info.color, data),
        b"bKGD" => read_chunk_bkgd(info, data),
        b"tEXt" if !in_decode || decoder.read_text_chunks => read_chunk_text(info, data),
        b"zTXt" if !in_decode || decoder.read_text_chunks => read_chunk_ztxt(info, &decoder, data),
        b"iTXt" if !in_decode || decoder.read_text_chunks => read_chunk_itxt(info, &decoder, data),
        b"tEXt" | b"zTXt" | b"iTXt" => Ok(()),
        b"tIME" => {
            if data.len() != 7 {
                return Some(Err(73));
            }
            info.time_defined = true;
            info.time = [read16(data, 0), 0, 0, 0, 0, 0];
            for k in 1..6 {
                info.time[k] = u32::from(data[k + 1]);
            }
            Ok(())
        }
        b"pHYs" => {
            if data.len() != 9 {
                return Some(Err(74));
            }
            info.phys_defined = true;
            (info.phys_x, info.phys_y, info.phys_unit) = (be(0), be(4), u32::from(data[8]));
            Ok(())
        }
        b"gAMA" => {
            if data.len() != 4 {
                return Some(Err(96));
            }
            info.gama_defined = true;
            info.gama_gamma = be(0);
            Ok(())
        }
        b"cHRM" => {
            if data.len() != 32 {
                return Some(Err(97));
            }
            info.chrm_defined = true;
            info.chrm_white_x = be(0);
            info.chrm_white_y = be(4);
            info.chrm_red_x = be(8);
            info.chrm_red_y = be(12);
            info.chrm_green_x = be(16);
            info.chrm_green_y = be(20);
            info.chrm_blue_x = be(24);
            info.chrm_blue_y = be(28);
            Ok(())
        }
        b"sRGB" => {
            if data.len() != 1 {
                return Some(Err(98));
            }
            info.srgb_defined = true;
            info.srgb_intent = u32::from(data[0]);
            Ok(())
        }
        b"iCCP" => read_chunk_iccp(info, &decoder, data),
        b"cICP" => {
            if data.len() != 4 {
                return Some(Err(117));
            }
            info.cicp_defined = true;
            info.cicp = [0, 1, 2, 3].map(|k| u32::from(data[k]));
            Ok(())
        }
        b"mDCV" => {
            if data.len() != 24 {
                return Some(Err(119));
            }
            info.mdcv_defined = true;
            info.mdcv = std::array::from_fn(|k| {
                if k < 8 {
                    read16(data, 2 * k)
                } else {
                    be(16 + 4 * (k - 8))
                }
            });
            Ok(())
        }
        b"cLLI" => {
            if data.len() != 8 {
                return Some(Err(120));
            }
            info.clli_defined = true;
            info.clli = [be(0), be(4)];
            Ok(())
        }
        b"eXIf" => {
            info.exif_defined = true;
            info.exif = data.to_vec();
            Ok(())
        }
        b"sBIT" => read_chunk_sbit(info, data),
        _ => return None,
    };
    Some(r)
}

/// Port of `lodepng_inspect_chunk`.
pub fn lodepng_inspect_chunk(state: &mut State, pos: usize, input: &[u8]) -> Result<(), u32> {
    if pos + 4 > input.len() {
        return Err(30);
    }
    let chunk_length = read32(input, pos) as usize;
    if chunk_length > 2_147_483_647 {
        return Err(63);
    }
    if chunk_length + 12 > input.len() - pos {
        return Err(30);
    }
    let chunk_type = input[pos + 4..pos + 8].to_vec();
    let data = &input[pos + 8..pos + 8 + chunk_length];
    match read_known_chunk(state, &chunk_type, data, false) {
        Some(Err(e)) => Err(e),
        Some(Ok(())) if !state.decoder.ignore_crc && chunk_crc_wrong(input, pos) => Err(57),
        _ => Ok(()),
    }
}

/// Port of `lodepng_decode_chunks`: reads the PNG's chunks, updating the
/// state, and returns (IDAT data, width, height).
pub fn lodepng_decode_chunks(state: &mut State, input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    let r = decode_chunks(state, input);
    if let Err(e) = r {
        state.error = e;
    }
    r
}

fn decode_chunks(state: &mut State, input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    let (w, h) = inspect(state, input)?;
    if pixel_overflow(w, h, &state.info_png.color, &state.info_raw) {
        return Err(92);
    }
    let insize = input.len();
    let mut idat = Vec::new();
    // 1 = after IHDR, 2 = after PLTE, 3 = after IDAT
    let mut critical_pos = 1usize;
    let mut pos = 33usize;
    loop {
        if pos + 12 > insize {
            if state.decoder.ignore_end {
                break;
            }
            return Err(30);
        }
        let chunk_length = read32(input, pos) as usize;
        if chunk_length > 2_147_483_647 {
            if state.decoder.ignore_end {
                break;
            }
            return Err(63);
        }
        if pos + chunk_length + 12 > insize {
            return Err(64);
        }
        let chunk_type: [u8; 4] = [
            input[pos + 4],
            input[pos + 5],
            input[pos + 6],
            input[pos + 7],
        ];
        let data = &input[pos + 8..pos + 8 + chunk_length];
        let mut unknown = false;
        let mut iend = false;

        match &chunk_type {
            b"IDAT" => {
                if idat.len() + chunk_length > insize {
                    return Err(95);
                }
                idat.extend_from_slice(data);
                critical_pos = 3;
            }
            b"IEND" => iend = true,
            _ => match read_known_chunk(state, &chunk_type, data, true) {
                Some(r) => {
                    r?;
                    if &chunk_type == b"PLTE" {
                        critical_pos = 2;
                    }
                }
                None => {
                    if !chunk_type.iter().all(u8::is_ascii_alphabetic) {
                        return Err(121); // invalid chunk type name
                    }
                    if chunk_type[2] & 32 != 0 {
                        return Err(122); // invalid third lowercase character
                    }
                    // unknown critical chunk (5th bit of first byte of chunk type is 0)
                    if !state.decoder.ignore_critical && chunk_type[0] & 32 == 0 {
                        return Err(69);
                    }
                    unknown = true;
                    if state.decoder.remember_unknown_chunks {
                        state.info_png.unknown_chunks_data[critical_pos - 1]
                            .extend_from_slice(&input[pos..pos + chunk_length + 12]);
                    }
                }
            },
        }

        // check CRC if wanted, only on known chunk types
        if !state.decoder.ignore_crc && !unknown && chunk_crc_wrong(input, pos) {
            return Err(57);
        }
        if iend {
            break;
        }
        pos = lodepng_chunk_next_const(input, pos, insize);
    }
    Ok((idat, w, h))
}

/// `inflateIdat` producing a new buffer of `lodepng_get_raw_size` bytes.
fn inflate_idat(w: u32, h: u32, state: &State, idat: &[u8]) -> Result<Vec<u8>, u32> {
    let info = &state.info_png;
    if info.color.colortype == LCT_PALETTE && info.color.palette.is_empty() {
        return Err(106); // PNG file must have PLTE chunk if color type is palette
    }
    let bpp = info.color.bpp();
    let expected_size = if info.interlace_method == 0 {
        get_raw_size_idat(w, h, bpp)
    } else {
        // Adam-7 interlaced: expected size is the sum of the 7 sub-images sizes
        let mut s = get_raw_size_idat(w.wrapping_add(7) >> 3, h.wrapping_add(7) >> 3, bpp);
        if w > 4 {
            s += get_raw_size_idat(w.wrapping_add(3) >> 3, h.wrapping_add(7) >> 3, bpp);
        }
        s += get_raw_size_idat(w.wrapping_add(3) >> 2, h.wrapping_add(3) >> 3, bpp);
        if w > 2 {
            s += get_raw_size_idat(w.wrapping_add(1) >> 2, h.wrapping_add(3) >> 2, bpp);
        }
        s += get_raw_size_idat(w.wrapping_add(1) >> 1, h.wrapping_add(1) >> 2, bpp);
        if w > 1 {
            s += get_raw_size_idat(w >> 1, h.wrapping_add(1) >> 1, bpp);
        }
        s + get_raw_size_idat(w, h >> 1, bpp)
    };
    let mut scanlines = Vec::new();
    zlib_decompressv(&mut scanlines, idat, &state.decoder.zlibsettings)?;
    if scanlines.len() != expected_size {
        return Err(91); // decompressed size doesn't match prediction
    }
    let mut out = vec![0u8; get_raw_size(w, h, &info.color)];
    post_process_scanlines(&mut out, &mut scanlines, w, h, info)?;
    Ok(out)
}

/// Port of `lodepng_finish_decode`: inflates the IDAT data collected by
/// [`lodepng_decode_chunks`] into `cbuffer`, converting to `state.info_raw`.
pub fn lodepng_finish_decode(
    cbuffer: &mut [u8],
    w: u32,
    h: u32,
    state: &mut State,
    idat: &[u8],
) -> Result<(), u32> {
    let r = finish_decode(cbuffer, w, h, state, idat);
    if let Err(e) = r {
        state.error = e;
    }
    r
}

fn finish_decode(
    cbuffer: &mut [u8],
    w: u32,
    h: u32,
    state: &mut State,
    idat: &[u8],
) -> Result<(), u32> {
    if cbuffer.is_empty() {
        return Err(105);
    }
    if !state.decoder.color_convert
        || lodepng_color_mode_equal(&state.info_raw, &state.info_png.color)
    {
        if !state.decoder.color_convert {
            state.info_raw = state.info_png.color.clone();
        }
        let out = inflate_idat(w, h, state, idat)?;
        let dest = cbuffer.get_mut(..out.len()).ok_or(123u32)?;
        dest.copy_from_slice(&out);
        Ok(())
    } else {
        // color conversion needed
        let tbuf = inflate_idat(w, h, state, idat);
        let raw = &state.info_raw;
        if !(raw.colortype == LCT_RGB || raw.colortype == LCT_RGBA) && raw.bitdepth != 8 {
            return Err(56); // unsupported color mode conversion
        }
        let outsize = get_raw_size(w, h, raw);
        if cbuffer.len() < outsize {
            return Err(123);
        }
        // The C converts an unset buffer when inflating failed; report the error instead
        let tbuf = tbuf?;
        lodepng_convert(cbuffer, &tbuf, &state.info_raw, &state.info_png.color, w, h)
    }
}

/// Port of `lodepng_decode`: returns (image, width, height).
pub fn lodepng_decode(state: &mut State, input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    let r = decode(state, input);
    if let Err(e) = r {
        state.error = e;
    }
    r
}

fn decode(state: &mut State, input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    let (idat, w, h) = decode_chunks(state, input)?;
    let image = inflate_idat(w, h, state, &idat)?;
    if !state.decoder.color_convert
        || lodepng_color_mode_equal(&state.info_raw, &state.info_png.color)
    {
        if !state.decoder.color_convert {
            state.info_raw = state.info_png.color.clone();
        }
        Ok((image, w, h))
    } else {
        let raw = &state.info_raw;
        if !(raw.colortype == LCT_RGB || raw.colortype == LCT_RGBA) && raw.bitdepth != 8 {
            return Err(56);
        }
        let mut out = vec![0u8; get_raw_size(w, h, raw)];
        lodepng_convert(
            &mut out,
            &image,
            &state.info_raw,
            &state.info_png.color,
            w,
            h,
        )?;
        Ok((out, w, h))
    }
}

/// Port of `lodepng_decode_memory`.
pub fn lodepng_decode_memory(
    input: &[u8],
    colortype: ColorType,
    bitdepth: u32,
) -> Result<(Vec<u8>, u32, u32), u32> {
    let mut state = State::new();
    state.info_raw.colortype = colortype;
    state.info_raw.bitdepth = bitdepth;
    // disable reading things that this function doesn't output
    state.decoder.read_text_chunks = false;
    state.decoder.remember_unknown_chunks = false;
    lodepng_decode(&mut state, input)
}

/// Port of `lodepng_decode32`.
pub fn lodepng_decode32(input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    lodepng_decode_memory(input, LCT_RGBA, 8)
}

/// Port of `lodepng_decode24`.
pub fn lodepng_decode24(input: &[u8]) -> Result<(Vec<u8>, u32, u32), u32> {
    lodepng_decode_memory(input, LCT_RGB, 8)
}
