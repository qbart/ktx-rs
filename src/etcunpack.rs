//! Port of `lib/src/etcunpack.cxx` (`_ktxUnpackETC`): software decoding of ETC1, ETC2 and EAC
//! images for OpenGL contexts without native ETC support (GL loader).
//!
//! The upstream block decoder `external/etcdec/etcdec.cxx` is under the Ericsson SLA, which
//! only allows distributing it unmodified, so it is not translated. The block decoders below
//! are written from the ETC2 / EAC definitions of the Khronos Data Format Specification and
//! reproduce etcdec's output conventions (16-bit expansion of 11-bit EAC values, punch-through
//! transparent texels, RGBA interleaving), checked byte for byte against etcdec by
//! `tests/etcunpack.rs`.

use crate::error::{ErrorCode, KtxResult};
use crate::gl_format::*;

/// `_KTX_R16_FORMATS_NORM`: the context supports GL_R16 / GL_RG16.
pub const KTX_R16_FORMATS_NORM: u32 = 0x1;
/// `_KTX_R16_FORMATS_SNORM`: the context supports GL_R16_SNORM / GL_RG16_SNORM.
pub const KTX_R16_FORMATS_SNORM: u32 = 0x2;
/// `_KTX_ALL_R16_FORMATS`.
pub const KTX_ALL_R16_FORMATS: u32 = KTX_R16_FORMATS_NORM | KTX_R16_FORMATS_SNORM;

/// ETC1 / ETC2 intensity modifier tables (codeword -> magnitudes for indices `a`, `b`).
const ETC_MODIFIERS: [[i32; 2]; 8] = [
    [2, 8],
    [5, 17],
    [9, 29],
    [13, 42],
    [18, 60],
    [24, 80],
    [33, 106],
    [47, 183],
];

/// T / H mode distance table.
const ETC_DISTANCES: [i32; 8] = [3, 6, 11, 16, 23, 32, 41, 64];

/// EAC modifier tables, indexed by table and 3-bit texel index.
const EAC_MODIFIERS: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

fn clamp255(v: i32) -> i32 {
    v.clamp(0, 255)
}

fn bits(block: u64, size: u32, startpos: u32) -> i32 {
    ((block >> (startpos + 1 - size)) & ((1u64 << size) - 1)) as i32
}

fn ext4(v: i32) -> i32 {
    (v << 4) | v
}

fn ext5(v: i32) -> i32 {
    (v << 3) | (v >> 2)
}

fn ext6(v: i32) -> i32 {
    (v << 2) | (v >> 4)
}

fn ext7(v: i32) -> i32 {
    (v << 1) | (v >> 6)
}

/// 3-bit two's complement.
fn sext3(v: i32) -> i32 {
    if v >= 4 { v - 8 } else { v }
}

/// The 2-bit index of texel (x, y): MSB in bits 16..31, LSB in bits 0..15, column major.
fn texel_index(block: u64, x: usize, y: usize) -> usize {
    let i = x * 4 + y;
    ((((block >> (16 + i)) & 1) << 1) | ((block >> i) & 1)) as usize
}

/// A decoded texel; `None` is a punch-through transparent texel.
type Texel = Option<[u8; 3]>;

/// Decodes one ETC2 RGB / punch-through block to 16 texels (`block[y][x]`).
/// `punchthrough`: the diff bit is the opaque bit and individual mode does not exist.
fn decode_etc2_block(block: u64, punchthrough: bool) -> [[Texel; 4]; 4] {
    let mut out = [[Some([0u8; 3]); 4]; 4];
    let diff_or_opaque = bits(block, 1, 33) != 0;
    let opaque = !punchthrough || diff_or_opaque;
    let differential = punchthrough || diff_or_opaque;

    let rgb = |c: [i32; 3]| -> [u8; 3] { c.map(|v| clamp255(v) as u8) };

    if differential {
        let r = bits(block, 5, 63);
        let g = bits(block, 5, 55);
        let b = bits(block, 5, 47);
        let r2 = r + sext3(bits(block, 3, 58));
        let g2 = g + sext3(bits(block, 3, 50));
        let b2 = b + sext3(bits(block, 3, 42));
        if !(0..=31).contains(&r2) {
            return decode_t_or_h(block, true, opaque);
        }
        if !(0..=31).contains(&g2) {
            return decode_t_or_h(block, false, opaque);
        }
        if !(0..=31).contains(&b2) {
            return decode_planar(block);
        }
        let bases = [[ext5(r), ext5(g), ext5(b)], [ext5(r2), ext5(g2), ext5(b2)]];
        decode_subblocks(block, bases, opaque, &mut out, rgb);
    } else {
        let bases = [
            [
                ext4(bits(block, 4, 63)),
                ext4(bits(block, 4, 55)),
                ext4(bits(block, 4, 47)),
            ],
            [
                ext4(bits(block, 4, 59)),
                ext4(bits(block, 4, 51)),
                ext4(bits(block, 4, 43)),
            ],
        ];
        decode_subblocks(block, bases, true, &mut out, rgb);
    }
    out
}

/// Individual / differential mode: two sub-blocks with a base color and modifier table each.
fn decode_subblocks(
    block: u64,
    bases: [[i32; 3]; 2],
    opaque: bool,
    out: &mut [[Texel; 4]; 4],
    rgb: impl Fn([i32; 3]) -> [u8; 3],
) {
    let flip = bits(block, 1, 32) != 0;
    let tables = [bits(block, 3, 39) as usize, bits(block, 3, 36) as usize];
    for (y, row) in out.iter_mut().enumerate() {
        for (x, texel) in row.iter_mut().enumerate() {
            let sub = if flip {
                usize::from(y >= 2)
            } else {
                usize::from(x >= 2)
            };
            let [a, b] = ETC_MODIFIERS[tables[sub]];
            let index = texel_index(block, x, y);
            if !opaque && index == 2 {
                *texel = None;
                continue;
            }
            let modifier = match index {
                0 if opaque => a,
                0 => 0,
                1 => b,
                2 => -a,
                _ => -b,
            };
            *texel = Some(rgb(bases[sub].map(|c| c + modifier)));
        }
    }
}

/// T mode (`t_mode`) or H mode paint colors and texels.
fn decode_t_or_h(block: u64, t_mode: bool, opaque: bool) -> [[Texel; 4]; 4] {
    let (c1, c2, d) = if t_mode {
        let r1 = (bits(block, 2, 60) << 2) | bits(block, 2, 57);
        let c1 = [r1, bits(block, 4, 55), bits(block, 4, 51)];
        let c2 = [bits(block, 4, 47), bits(block, 4, 43), bits(block, 4, 39)];
        let d = (bits(block, 2, 35) << 1) | bits(block, 1, 32);
        (c1, c2, d as usize)
    } else {
        let g1 = (bits(block, 3, 58) << 1) | bits(block, 1, 52);
        let b1 = (bits(block, 1, 51) << 3) | bits(block, 3, 49);
        let c1 = [bits(block, 4, 62), g1, b1];
        let c2 = [bits(block, 4, 46), bits(block, 4, 42), bits(block, 4, 38)];
        let v1 = (c1[0] << 8) | (c1[1] << 4) | c1[2];
        let v2 = (c2[0] << 8) | (c2[1] << 4) | c2[2];
        let d = (bits(block, 1, 34) << 2) | (bits(block, 1, 32) << 1) | i32::from(v1 >= v2);
        (c1, c2, d as usize)
    };
    let (c1, c2) = (c1.map(ext4), c2.map(ext4));
    let dist = ETC_DISTANCES[d];
    let add = |c: [i32; 3], k: i32| c.map(|v| clamp255(v + k) as u8);
    let paint = if t_mode {
        [add(c1, 0), add(c2, dist), add(c2, 0), add(c2, -dist)]
    } else {
        [add(c1, dist), add(c1, -dist), add(c2, dist), add(c2, -dist)]
    };
    let mut out = [[None; 4]; 4];
    for (y, row) in out.iter_mut().enumerate() {
        for (x, texel) in row.iter_mut().enumerate() {
            let index = texel_index(block, x, y);
            *texel = if !opaque && index == 2 {
                None
            } else {
                Some(paint[index])
            };
        }
    }
    out
}

/// Planar mode: origin, horizontal and vertical colors interpolated across the block.
fn decode_planar(block: u64) -> [[Texel; 4]; 4] {
    let ro = ext6(bits(block, 6, 62));
    let go = ext7((bits(block, 1, 56) << 6) | bits(block, 6, 54));
    let bo = ext6((bits(block, 1, 48) << 5) | (bits(block, 2, 44) << 3) | bits(block, 3, 41));
    let rh = ext6((bits(block, 5, 38) << 1) | bits(block, 1, 32));
    let gh = ext7(bits(block, 7, 31));
    let bh = ext6(bits(block, 6, 24));
    let rv = ext6(bits(block, 6, 18));
    let gv = ext7(bits(block, 7, 12));
    let bv = ext6(bits(block, 6, 5));
    let mut out = [[None; 4]; 4];
    for (y, row) in out.iter_mut().enumerate() {
        for (x, texel) in row.iter_mut().enumerate() {
            let (xi, yi) = (x as i32, y as i32);
            let f = |o: i32, h: i32, v: i32| {
                clamp255((xi * (h - o) + yi * (v - o) + 4 * o + 2) >> 2) as u8
            };
            *texel = Some([f(ro, rh, rv), f(go, gh, gv), f(bo, bh, bv)]);
        }
    }
    out
}

/// Reads the base, table, multiplier and the 16 3-bit indices of an EAC block (column major).
fn eac_fields(data: &[u8; 8]) -> (u8, usize, i32, [[usize; 4]; 4]) {
    let indices = u64::from_be_bytes(*data) & 0xFFFF_FFFF_FFFF;
    let mut idx = [[0usize; 4]; 4];
    for x in 0..4 {
        for (y, row) in idx.iter_mut().enumerate() {
            let i = x * 4 + y;
            row[x] = ((indices >> (45 - 3 * i)) & 7) as usize;
        }
    }
    (
        data[0],
        usize::from(data[1] & 0xF),
        i32::from(data[1] >> 4),
        idx,
    )
}

/// EAC 8-bit alpha of GL_COMPRESSED_RGBA8_ETC2_EAC (`block[y][x]`).
fn decode_eac_alpha8(data: &[u8; 8]) -> [[u8; 4]; 4] {
    let (base, table, mul, idx) = eac_fields(data);
    idx.map(|row| row.map(|i| clamp255(i32::from(base) + EAC_MODIFIERS[table][i] * mul) as u8))
}

/// EAC R11 / signed R11, extended to 16 bits like etcdec (`block[y][x]`, native i16 / u16
/// bit patterns).
fn decode_eac_r11(data: &[u8; 8], signed: bool) -> [[u16; 4]; 4] {
    let (base, table, mul, idx) = eac_fields(data);
    idx.map(|row| {
        row.map(|i| {
            let m = EAC_MODIFIERS[table][i];
            let step = if mul != 0 { m * mul * 8 } else { m };
            if signed {
                let b = i32::from(base as i8).max(-127) * 8;
                let e = (b + step).clamp(-1023, 1023);
                let a = e.abs();
                let v = ((a << 5) + (a >> 5)) as i16;
                (if e < 0 { -v } else { v }) as u16
            } else {
                let e = (i32::from(base) * 8 + 4 + step).clamp(0, 2047);
                ((e << 5) + (e >> 6)) as u16
            }
        })
    })
}

/// Result of [`ktx_unpack_etc`]: the decoded image and its GL format, internal format and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackedEtc {
    pub image: Vec<u8>,
    pub format: u32,
    pub internal_format: u32,
    pub r#type: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlphaFormat {
    None,
    OneBit,
    EightBit,
    // AF_11BIT is used to compress R11 & RG11 though its not alpha data.
    ElevenBit,
}

/// Port of `_ktxUnpackETC`: decodes an ETC1 / ETC2 / EAC image of `active_width` x
/// `active_height` texels. `r16_formats` (`KTX_R16_FORMATS_*`) and `supports_srgb` describe
/// the context; formats it cannot hold give `KTX_UNSUPPORTED_TEXTURE_TYPE`.
pub fn ktx_unpack_etc(
    src_etc: &[u8],
    src_format: u32,
    active_width: u32,
    active_height: u32,
    r16_formats: u32,
    supports_srgb: bool,
) -> KtxResult<UnpackedEtc> {
    use AlphaFormat as AF;
    let unsupported = Err(ErrorCode::UnsupportedTextureType);
    let (dst_channel_bytes, dst_channels, internal_format, format, r#type, alpha_format, signed) =
        match src_format {
            GL_COMPRESSED_SIGNED_R11_EAC if r16_formats & KTX_R16_FORMATS_SNORM != 0 => {
                (2, 1, GL_R16_SNORM, GL_RED, GL_SHORT, AF::ElevenBit, true)
            }
            GL_COMPRESSED_R11_EAC if r16_formats & KTX_R16_FORMATS_NORM != 0 => (
                2,
                1,
                GL_R16,
                GL_RED,
                GL_UNSIGNED_SHORT,
                AF::ElevenBit,
                false,
            ),
            GL_COMPRESSED_SIGNED_RG11_EAC if r16_formats & KTX_R16_FORMATS_SNORM != 0 => {
                (2, 2, GL_RG16_SNORM, GL_RG, GL_SHORT, AF::ElevenBit, true)
            }
            GL_COMPRESSED_RG11_EAC if r16_formats & KTX_R16_FORMATS_NORM != 0 => (
                2,
                2,
                GL_RG16,
                GL_RG,
                GL_UNSIGNED_SHORT,
                AF::ElevenBit,
                false,
            ),
            GL_ETC1_RGB8_OES | GL_COMPRESSED_RGB8_ETC2 => {
                (1, 3, GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE, AF::None, false)
            }
            GL_COMPRESSED_RGBA8_ETC2_EAC => (
                1,
                4,
                GL_RGBA8,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                AF::EightBit,
                false,
            ),
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 => {
                (1, 4, GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, AF::OneBit, false)
            }
            GL_COMPRESSED_SRGB8_ETC2 if supports_srgb => {
                (1, 3, GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE, AF::None, false)
            }
            GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC if supports_srgb => (
                1,
                4,
                GL_SRGB8_ALPHA8,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                AF::EightBit,
                false,
            ),
            GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 if supports_srgb => (
                1,
                4,
                GL_SRGB8_ALPHA8,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                AF::OneBit,
                false,
            ),
            // Upper levels should pass only one of the above srcFormats.
            _ => return unsupported,
        };

    // active_{width,height} show how many pixels contain active data, (the rest are just for
    // making sure we have a 2*a x 4*b size).
    let width = active_width.div_ceil(4) as usize * 4;
    let height = active_height.div_ceil(4) as usize * 4;
    let pixel_bytes = dst_channels * dst_channel_bytes;
    let mut img = vec![0u8; pixel_bytes * width * height];
    let two_channels = matches!(
        src_format,
        GL_COMPRESSED_RG11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC
    );

    let mut src = src_etc.as_chunks::<8>().0.iter();
    let mut next = || -> KtxResult<[u8; 8]> { src.next().copied().ok_or(ErrorCode::FileDataError) };
    for by in 0..height / 4 {
        for bx in 0..width / 4 {
            let at = |x: usize, y: usize| ((by * 4 + y) * width + bx * 4 + x) * pixel_bytes;
            if alpha_format == AF::ElevenBit {
                // One or two 11-bit alpha channels for R or RG.
                for channel in 0..if two_channels { 2 } else { 1 } {
                    let values = decode_eac_r11(&next()?, signed);
                    for (y, row) in values.iter().enumerate() {
                        for (x, v) in row.iter().enumerate() {
                            let o = at(x, y) + channel * 2;
                            img[o..o + 2].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                continue;
            }
            // Decode alpha channel for RGBA
            if alpha_format == AF::EightBit {
                let alpha = decode_eac_alpha8(&next()?);
                for (y, row) in alpha.iter().enumerate() {
                    for (x, a) in row.iter().enumerate() {
                        img[at(x, y) + 3] = *a;
                    }
                }
            }
            // Decode color dstChannels
            let texels = decode_etc2_block(u64::from_be_bytes(next()?), alpha_format == AF::OneBit);
            for (y, row) in texels.iter().enumerate() {
                for (x, t) in row.iter().enumerate() {
                    let o = at(x, y);
                    match t {
                        Some(c) => img[o..o + 3].copy_from_slice(c),
                        None => img[o..o + 3].fill(0),
                    }
                    if alpha_format == AF::OneBit {
                        img[o + 3] = if t.is_some() { 255 } else { 0 };
                    }
                }
            }
        }
    }

    // Ok, now write out the active pixels to the destination image. (But only if the active
    // pixels differ from the total pixels)
    let (aw, ah) = (active_width as usize, active_height as usize);
    if !(height == ah && width == aw) {
        let mut newimg = Vec::with_capacity(pixel_bytes * aw * ah);
        for row in img.chunks_exact(pixel_bytes * width).take(ah) {
            newimg.extend_from_slice(&row[..aw * pixel_bytes]);
        }
        img = newimg;
    }
    Ok(UnpackedEtc {
        image: img,
        format,
        internal_format,
        r#type,
    })
}
