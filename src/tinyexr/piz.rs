//! PIZ decompression: tinyexr's copy of OpenEXR `ImfPizCompressor.cpp`
//! (Haar wavelet + 16-bit Huffman, `DecompressPiz`).
//!
//! The compressed stream is addressed inside the whole file buffer like the
//! C pointers are, so reads past a chunk see the following file bytes. Reads
//! past the end of the file give 0 (undefined behavior in C).

use super::TINYEXR_PIXELTYPE_HALF;
use super::load::{ExrChannelInfo, rd_u8, rd_u16, rd_u32};

const HUF_DECBITS: i32 = 14;
const HUF_ENCSIZE: usize = (1 << 16) + 1;
const HUF_DECSIZE: usize = 1 << HUF_DECBITS;
const HUF_DECMASK: i64 = HUF_DECSIZE as i64 - 1;

const SHORT_ZEROCODE_RUN: i64 = 59;
const LONG_ZEROCODE_RUN: i64 = 63;
const SHORTEST_LONG_RUN: i64 = 2 + LONG_ZEROCODE_RUN - SHORT_ZEROCODE_RUN;

const USHORT_RANGE: usize = 1 << 16;
const BITMAP_SIZE: usize = USHORT_RANGE >> 3;

/// `wdec14`.
fn wdec14(l: u16, h: u16) -> (u16, u16) {
    let ls = i32::from(l as i16);
    let hi = i32::from(h as i16);
    let ai = ls + (hi & 1) + (hi >> 1);
    (ai as i16 as u16, (ai - hi) as i16 as u16)
}

const A_OFFSET: i32 = 1 << 15;
const MOD_MASK: i32 = (1 << 16) - 1;

/// `wdec16`.
fn wdec16(l: u16, h: u16) -> (u16, u16) {
    let m = i32::from(l);
    let d = i32::from(h);
    let bb = (m - (d >> 1)) & MOD_MASK;
    let aa = (d + bb - A_OFFSET) & MOD_MASK;
    (aa as u16, bb as u16)
}

/// Bounds checked access to the wavelet buffer (the C++ indexes raw pointers).
struct Buf<'a>(&'a mut [u16], i64);

impl Buf<'_> {
    fn get(&self, i: i64) -> u16 {
        usize::try_from(self.1 + i)
            .ok()
            .and_then(|i| self.0.get(i))
            .copied()
            .unwrap_or(0)
    }
    fn set(&mut self, i: i64, v: u16) {
        if let Some(x) = usize::try_from(self.1 + i)
            .ok()
            .and_then(|i| self.0.get_mut(i))
        {
            *x = v;
        }
    }
}

/// `wav2Decode`: 2D wavelet decoding in place, `base` is the first value.
fn wav2_decode(data: &mut [u16], base: usize, nx: i32, ox: i32, ny: i32, oy: i32, mx: u16) {
    let mut b = Buf(data, base as i64);
    let w14 = mx < (1 << 14);
    let wdec = |l, h| if w14 { wdec14(l, h) } else { wdec16(l, h) };
    let n = nx.min(ny);
    let mut p: i32 = 1;

    // Search max level
    while p <= n {
        p = p.wrapping_shl(1);
        if p <= 0 {
            break;
        }
    }
    p >>= 1;
    let mut p2 = p;
    p >>= 1;

    let (nx, ny, ox, oy) = (i64::from(nx), i64::from(ny), i64::from(ox), i64::from(oy));

    // Hierarchical loop on smaller dimension n
    while p >= 1 {
        let (pl, p2l) = (i64::from(p), i64::from(p2));
        let mut py: i64 = 0;
        let ey = oy * (ny - p2l);
        let oy1 = oy * pl;
        let oy2 = oy * p2l;
        let ox1 = ox * pl;
        let ox2 = ox * p2l;

        // Y loop
        while py <= ey {
            let mut px = py;
            let ex = py + ox * (nx - p2l);

            // X loop
            while px <= ex {
                let p01 = px + ox1;
                let p10 = px + oy1;
                let p11 = p10 + ox1;

                // 2D wavelet decoding
                let (i00, i10) = wdec(b.get(px), b.get(p10));
                let (i01, i11) = wdec(b.get(p01), b.get(p11));
                let (a, c) = wdec(i00, i01);
                b.set(px, a);
                b.set(p01, c);
                let (a, c) = wdec(i10, i11);
                b.set(p10, a);
                b.set(p11, c);
                px += ox2;
            }

            // Decode (1D) odd column (still in Y loop)
            if nx & pl != 0 {
                let p10 = px + oy1;
                let (i00, v) = wdec(b.get(px), b.get(p10));
                b.set(p10, v);
                b.set(px, i00);
            }
            py += oy2;
        }

        // Decode (1D) odd line (must loop in X)
        if ny & pl != 0 {
            let mut px = py;
            let ex = py + ox * (nx - p2l);
            while px <= ex {
                let p01 = px + ox1;
                let (i00, v) = wdec(b.get(px), b.get(p01));
                b.set(p01, v);
                b.set(px, i00);
                px += ox2;
            }
        }

        // Next level
        p2 = p;
        p >>= 1;
    }
}

/// `HufDec`: `len` (8 bits) and `lit` (24 bits) of a short code, or the
/// symbols of the long codes sharing this prefix in `p`.
#[derive(Clone, Default)]
struct HufDec {
    len: u32,
    lit: u32,
    p: Option<Vec<u32>>,
}

fn huf_length(code: i64) -> i64 {
    code & 63
}

fn huf_code(code: i64) -> i64 {
    code >> 6
}

/// Bit reader state of `getBits` / `getChar` over the file buffer.
struct Bits<'a> {
    mem: &'a [u8],
    c: i64,
    lc: i32,
}

impl Bits<'_> {
    /// `getChar`.
    fn get_char(&mut self, pos: &mut u64) {
        self.c = ((self.c as u64) << 8) as i64 | i64::from(rd_u8(self.mem, *pos));
        *pos = pos.wrapping_add(1);
        self.lc += 8;
    }

    /// `getBits`.
    fn get_bits(&mut self, n_bits: i32, pos: &mut u64) -> i64 {
        while self.lc < n_bits {
            self.get_char(pos);
        }
        self.lc -= n_bits;
        (self.c >> self.lc) & ((1i64 << n_bits) - 1)
    }
}

/// `hufCanonicalCodeTable`.
fn huf_canonical_code_table(hcode: &mut [i64]) {
    let mut n = [0i64; 59];
    for &h in hcode.iter() {
        if let Some(x) = usize::try_from(h).ok().and_then(|h| n.get_mut(h)) {
            *x += 1;
        }
    }
    let mut c: i64 = 0;
    for i in (1..=58).rev() {
        let nc = (c + n[i]) >> 1;
        n[i] = c;
        c = nc;
    }
    for h in hcode.iter_mut() {
        let l = *h as i32;
        if l > 0 {
            let li = l as usize;
            *h = i64::from(l) | (n[li] << 6);
            n[li] += 1;
        }
    }
}

/// `hufUnpackEncTable`. On success `pos` is advanced past the table; on
/// failure `hcode` keeps the lengths read so far.
fn huf_unpack_enc_table(
    mem: &[u8],
    pos: &mut u64,
    ni: i64,
    mut im: i32,
    i_max: i32,
    hcode: &mut [i64],
) -> bool {
    hcode.iter_mut().for_each(|h| *h = 0);
    let start = *pos;
    let mut p = *pos;
    let mut bits = Bits { mem, c: 0, lc: 0 };
    let used = |p: u64| p.wrapping_sub(start) as i64;

    while im <= i_max {
        if used(p) >= ni {
            return false;
        }
        let l = bits.get_bits(6, &mut p);
        hcode[im as usize] = l;

        if l == LONG_ZEROCODE_RUN {
            if used(p) > ni {
                return false;
            }
            let zerun = bits.get_bits(8, &mut p) + SHORTEST_LONG_RUN;
            if i64::from(im) + zerun > i64::from(i_max) + 1 {
                return false;
            }
            for _ in 0..zerun {
                hcode[im as usize] = 0;
                im += 1;
            }
            im -= 1;
        } else if l >= SHORT_ZEROCODE_RUN {
            let zerun = l - SHORT_ZEROCODE_RUN + 2;
            if i64::from(im) + zerun > i64::from(i_max) + 1 {
                return false;
            }
            for _ in 0..zerun {
                hcode[im as usize] = 0;
                im += 1;
            }
            im -= 1;
        }
        im += 1;
    }
    *pos = p;
    huf_canonical_code_table(hcode);
    true
}

/// `hufBuildDecTable` (the table starts cleared).
fn huf_build_dec_table(hcode: &[i64], im: i32, i_max: i32, hdecod: &mut [HufDec]) -> bool {
    for i in im..=i_max {
        let c = huf_code(hcode[i as usize]);
        let l = huf_length(hcode[i as usize]) as i32;

        if (c >> l) != 0 {
            // invalidTableEntry
            return false;
        }

        if l > HUF_DECBITS {
            // Long code: add a secondary entry
            let Some(pl) = hdecod.get_mut((c >> (l - HUF_DECBITS)) as usize) else {
                return false;
            };
            if pl.len != 0 {
                return false;
            }
            pl.lit = (pl.lit + 1) & 0xff_ffff;
            pl.p.get_or_insert_with(Vec::new).push(i as u32);
        } else if l != 0 {
            // Short code: init all primary entries
            let first = (c << (HUF_DECBITS - l)) as usize;
            for k in 0..(1usize << (HUF_DECBITS - l)) {
                let Some(pl) = hdecod.get_mut(first + k) else {
                    return false;
                };
                if pl.len != 0 || pl.p.is_some() {
                    return false;
                }
                pl.len = l as u32 & 0xff;
                pl.lit = i as u32 & 0xff_ffff;
            }
        }
    }
    true
}

/// Output cursor of `hufDecode`.
struct Out<'a> {
    buf: &'a mut [u16],
    pos: usize,
}

/// `getCode`.
fn get_code(po: i32, rlc: i32, bits: &mut Bits, pos: &mut u64, in_end: u64, out: &mut Out) -> bool {
    if po == rlc {
        if bits.lc < 8 {
            // TinyEXR issue 78 / 160
            if *pos >= in_end {
                return false;
            }
            bits.get_char(pos);
        }
        bits.lc -= 8;
        let cs = (bits.c >> bits.lc) as u8 as usize;
        if out.pos + cs > out.buf.len() {
            return false;
        }
        // Issue 100
        if out.pos == 0 {
            return false;
        }
        let s = out.buf[out.pos - 1];
        out.buf[out.pos..out.pos + cs].fill(s);
        out.pos += cs;
    } else if out.pos < out.buf.len() {
        out.buf[out.pos] = po as u16;
        out.pos += 1;
    } else {
        return false;
    }
    true
}

/// `hufDecode`: decodes `ni` bits at `pos` into `out`.
fn huf_decode(
    hcode: &[i64],
    hdecod: &[HufDec],
    mem: &[u8],
    mut pos: u64,
    ni: i32,
    rlc: i32,
    out: &mut [u16],
) -> bool {
    let no = out.len();
    let mut out = Out { buf: out, pos: 0 };
    let mut bits = Bits { mem, c: 0, lc: 0 };
    let ie = pos.wrapping_add_signed(i64::from(ni.wrapping_add(7) / 8));

    // Loop on input bytes
    while pos < ie {
        bits.get_char(&mut pos);

        // Access decoding table
        while bits.lc >= HUF_DECBITS {
            let pl = &hdecod[((bits.c >> (bits.lc - HUF_DECBITS)) & HUF_DECMASK) as usize];
            if pl.len != 0 {
                // Get short code
                bits.lc -= pl.len as i32;
                if !get_code(pl.lit as i32, rlc, &mut bits, &mut pos, ie, &mut out) {
                    return false;
                }
            } else {
                let Some(lits) = &pl.p else {
                    return false;
                };
                // Search long code
                let mut found = false;
                for &sym in lits.iter().take(pl.lit as usize) {
                    let code = hcode[sym as usize];
                    let l = huf_length(code) as i32;
                    while bits.lc < l && pos < ie {
                        bits.get_char(&mut pos);
                    }
                    if bits.lc >= l {
                        let mask = (1i64.wrapping_shl(l as u32)).wrapping_sub(1);
                        if huf_code(code) == ((bits.c >> (bits.lc - l)) & mask) {
                            // Found : get long code
                            bits.lc -= l;
                            if !get_code(sym as i32, rlc, &mut bits, &mut pos, ie, &mut out) {
                                return false;
                            }
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    return false;
                }
            }
        }
    }

    // Get remaining (short) codes
    let i = 8i32.wrapping_sub(ni) & 7;
    bits.c >>= i;
    bits.lc -= i;

    while bits.lc > 0 {
        let idx = ((bits.c as u64) << (HUF_DECBITS - bits.lc)) as i64 & HUF_DECMASK;
        let pl = &hdecod[idx as usize];
        if pl.len != 0 {
            bits.lc -= pl.len as i32;
            if !get_code(pl.lit as i32, rlc, &mut bits, &mut pos, ie, &mut out) {
                return false;
            }
        } else {
            return false;
        }
    }

    out.pos == no
}

/// `hufUncompress`. Like the C++, the results of the table unpacking, table
/// building and decoding are ignored: `raw` keeps whatever was decoded.
fn huf_uncompress(mem: &[u8], compressed: u64, n_compressed: i32, raw: &mut [u16]) -> bool {
    if n_compressed == 0 {
        return false;
    }

    let im = rd_u32(mem, compressed) as i32;
    let i_max = rd_u32(mem, compressed.wrapping_add(4)) as i32;
    let n_bits = rd_u32(mem, compressed.wrapping_add(12)) as i32;

    if im < 0 || im as usize >= HUF_ENCSIZE || i_max < 0 || i_max as usize >= HUF_ENCSIZE {
        return false;
    }

    let mut ptr = compressed.wrapping_add(20);
    let mut freq = vec![0i64; HUF_ENCSIZE];
    let mut hdec = vec![HufDec::default(); HUF_DECSIZE];

    let used = |p: u64| p.wrapping_sub(compressed) as i64;
    let ni = i64::from(n_compressed) - used(ptr);
    let _ = huf_unpack_enc_table(mem, &mut ptr, ni, im, i_max, &mut freq);

    if i64::from(n_bits) > 8 * (i64::from(n_compressed) - used(ptr)) {
        return false;
    }

    let _ = huf_build_dec_table(&freq, im, i_max, &mut hdec);
    let _ = huf_decode(&freq, &hdec, mem, ptr, n_bits, i_max, raw);
    true
}

/// `reverseLutFromBitmap`.
fn reverse_lut_from_bitmap(bitmap: &[u8], lut: &mut [u16]) -> u16 {
    let mut k = 0usize;
    for i in 0..USHORT_RANGE {
        if i == 0 || (bitmap[i >> 3] & (1 << (i & 7))) != 0 {
            lut[k] = i as u16;
            k += 1;
        }
    }
    let n = k as i32 - 1;
    lut[k..].fill(0);
    n as u16
}

/// Per channel layout in the wavelet buffer (`PIZChannelData`).
struct PizChannelData {
    start: usize,
    end: usize,
    nx: i32,
    ny: i32,
    size: i32,
}

/// `DecompressPiz`: decodes `in_len` bytes at `in_pos` of `mem` into `out`
/// (`tmpBufSizeInBytes` = `out.len()`).
#[allow(clippy::too_many_arguments)]
pub(super) fn decompress_piz(
    out: &mut [u8],
    mem: &[u8],
    in_pos: u64,
    in_len: usize,
    channels: &[ExrChannelInfo],
    data_width: i32,
    num_lines: i32,
) -> bool {
    let tmp_buf_size_in_bytes = out.len();
    if in_len == tmp_buf_size_in_bytes {
        // Data is not compressed (Issue 40).
        for (i, o) in out.iter_mut().enumerate() {
            *o = rd_u8(mem, in_pos.wrapping_add(i as u64));
        }
        return true;
    }

    let mut bitmap = vec![0u8; BITMAP_SIZE];

    if in_len < 4 {
        return false;
    }

    let mut read_len = 0usize;
    let mut ptr = in_pos;
    let min_non_zero = rd_u16(mem, ptr);
    let max_non_zero = rd_u16(mem, ptr.wrapping_add(2));
    ptr = ptr.wrapping_add(4);
    read_len += 4;

    if usize::from(max_non_zero) >= BITMAP_SIZE {
        return false;
    }

    if min_non_zero <= max_non_zero {
        let n = usize::from(max_non_zero - min_non_zero) + 1;
        if n + read_len > in_len {
            // Input too short
            return false;
        }
        for i in 0..n {
            if let Some(b) = bitmap.get_mut(usize::from(min_non_zero) + i) {
                *b = rd_u8(mem, ptr.wrapping_add(i as u64));
            }
        }
        ptr = ptr.wrapping_add(n as u64);
        read_len += n;
    } else if !(usize::from(min_non_zero) == BITMAP_SIZE - 1 && max_non_zero == 0) {
        // Issue 194: only "all pixels are zero" is valid here
        return false;
    }

    let mut lut = vec![0u16; USHORT_RANGE];
    let max_value = reverse_lut_from_bitmap(&bitmap, &mut lut);

    // Huffman decoding
    if read_len + 4 > in_len {
        return false;
    }

    let length = rd_u32(mem, ptr) as i32;
    ptr = ptr.wrapping_add(4);

    let consumed = ptr.wrapping_sub(in_pos) as i64 + i64::from(length);
    if consumed as u64 > in_len as u64 {
        return false;
    }

    let mut tmp_buffer = vec![0u16; tmp_buf_size_in_bytes / 2];
    let _ = huf_uncompress(mem, ptr, length, &mut tmp_buffer);

    // Wavelet decoding
    let mut channel_data = Vec::with_capacity(channels.len());
    let mut tmp_buffer_end = 0usize;
    for chan in channels {
        // UINT and FLOAT: 2 shorts
        let size = if chan.pixel_type == TINYEXR_PIXELTYPE_HALF {
            1
        } else {
            2
        };
        channel_data.push(PizChannelData {
            start: tmp_buffer_end,
            end: tmp_buffer_end,
            nx: data_width,
            ny: num_lines,
            size,
        });
        tmp_buffer_end = tmp_buffer_end.wrapping_add(
            (i64::from(data_width) * i64::from(num_lines) * i64::from(size)) as usize,
        );
    }

    for cd in &channel_data {
        for j in 0..cd.size {
            wav2_decode(
                &mut tmp_buffer,
                cd.start + j as usize,
                cd.nx,
                cd.size,
                cd.ny,
                cd.nx.wrapping_mul(cd.size),
                max_value,
            );
        }
    }

    // Expand the pixel data to their original range
    for v in tmp_buffer.iter_mut() {
        *v = lut[usize::from(*v)];
    }

    let mut o = 0usize;
    for _ in 0..num_lines {
        for cd in channel_data.iter_mut() {
            let n = (i64::from(cd.nx) * i64::from(cd.size)) as usize;
            for k in 0..n {
                let v = tmp_buffer
                    .get(cd.end + k)
                    .copied()
                    .unwrap_or(0)
                    .to_le_bytes();
                if let Some(dst) = out.get_mut(o..o + 2) {
                    dst.copy_from_slice(&v);
                }
                o += 2;
            }
            cd.end += n;
        }
    }
    true
}
