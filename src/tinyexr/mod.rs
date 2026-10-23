//! Port of the tinyexr scanline writer (basis_universal
//! encoder/3rdparty/tinyexr.h, `SaveEXRImageToMemory`) for single part,
//! scanline, uncompressed images: the configuration `ktx extract` uses.
//! Byte exact with the C++ writer (`tests/tinyexr.rs`).

pub const TINYEXR_PIXELTYPE_UINT: i32 = 0;
pub const TINYEXR_PIXELTYPE_HALF: i32 = 1;
pub const TINYEXR_PIXELTYPE_FLOAT: i32 = 2;

pub const TINYEXR_COMPRESSIONTYPE_NONE: i32 = 0;

pub const TINYEXR_ERROR_INVALID_DATA: i32 = -4;
pub const TINYEXR_ERROR_UNSUPPORTED_FEATURE: i32 = -10;

/// One channel of an `EXRHeader` / `EXRImage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExrChannel {
    /// `EXRChannelInfo::name` (at most 255 bytes).
    pub name: String,
    /// `pixel_types[c]`: type of `data` (UINT values, HALF in the low 16 bits
    /// or FLOAT bit patterns).
    pub pixel_type: i32,
    /// `requested_pixel_types[c]`: type stored in the file.
    pub requested_pixel_type: i32,
    /// `images[c]`: width * height values.
    pub data: Vec<u32>,
}

/// `EXRAttribute` (custom header attribute).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrAttribute {
    pub name: String,
    pub attr_type: String,
    pub value: Vec<u8>,
}

/// tinyexr `half_to_float`.
pub fn half_to_float(h: u16) -> f32 {
    let magic = f32::from_bits(113 << 23);
    let shifted_exp: u32 = 0x7c00 << 13; // exponent mask after shift
    let mut o = (u32::from(h) & 0x7fff) << 13; // exponent/mantissa bits
    let exp = shifted_exp & o; // just the exponent
    o += (127 - 15) << 23; // exponent adjust

    // handle exponent special cases
    if exp == shifted_exp {
        o += (128 - 16) << 23; // Inf/NaN: extra exp adjust
    } else if exp == 0 {
        o += 1 << 23; // Zero/Denormal: extra exp adjust
        o = (f32::from_bits(o) - magic).to_bits(); // renormalize
    }
    o |= (u32::from(h) & 0x8000) << 16; // sign bit
    f32::from_bits(o)
}

/// tinyexr `float_to_half_full` (ISPC reference code, round half up).
pub fn float_to_half_full(f: f32) -> u16 {
    let bits = f.to_bits();
    let f_exp = (bits >> 23) & 0xff;
    let f_mant = bits & 0x7f_ffff;
    let sign = (bits >> 31) as u16;
    let mut o: u16 = 0;
    if f_exp == 0 {
        // Signed zero/denormal (which will underflow)
    } else if f_exp == 255 {
        // Inf or NaN (all exponent bits set): NaN->qNaN and Inf->Inf
        o = (31 << 10) | if f_mant != 0 { 0x200 } else { 0 };
    } else {
        // Normalized number: exponent unbias the single, then bias the halfp
        let newexp = f_exp as i32 - 127 + 15;
        if newexp >= 31 {
            o = 31 << 10; // Overflow, return signed infinity
        } else if newexp <= 0 {
            // Underflow
            if (14 - newexp) <= 24 {
                // Mantissa might be non-zero
                let mant = f_mant | 0x80_0000; // Hidden 1 bit
                o = ((mant >> (14 - newexp)) & 0x3ff) as u16;
                if (mant >> (13 - newexp)) & 1 != 0 {
                    o = o.wrapping_add(1); // Round, might overflow into exp bit, but this is OK
                }
            }
        } else {
            o = ((newexp as u16) << 10) | (f_mant >> 13) as u16;
            if f_mant & 0x1000 != 0 {
                o = o.wrapping_add(1); // Round, might overflow to inf, this is OK
            }
        }
    }
    // o.s.Sign = f.s.Sign
    (o & 0x7fff) | (sign << 15)
}

/// `WriteAttributeToMemory`.
fn write_attribute(out: &mut Vec<u8>, name: &str, attr_type: &str, data: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(attr_type.as_bytes());
    out.push(0);
    out.extend_from_slice(&(data.len() as i32).to_le_bytes());
    out.extend_from_slice(data);
}

/// `WriteChannelInfo`.
fn write_channel_info(channels: &[ExrChannel]) -> Vec<u8> {
    let mut data = Vec::new();
    for c in channels {
        data.extend_from_slice(c.name.as_bytes());
        data.push(0);
        data.extend_from_slice(&c.requested_pixel_type.to_le_bytes());
        data.extend_from_slice(&[0, 0, 0, 0]); // p_linear + reserved
        data.extend_from_slice(&1i32.to_le_bytes()); // x_sampling
        data.extend_from_slice(&1i32.to_le_bytes()); // y_sampling
    }
    data.push(0);
    data
}

/// `EncodePixelData` for one uncompressed scanline.
fn encode_scanline(
    out: &mut Vec<u8>,
    channels: &[ExrChannel],
    width: usize,
    y: usize,
) -> Result<(), String> {
    for c in channels {
        let line = c.data.get(y * width..(y + 1) * width).unwrap_or_default();
        let value = |x: usize| line.get(x).copied().unwrap_or(0);
        match (c.pixel_type, c.requested_pixel_type) {
            (TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_FLOAT) => {
                for x in 0..width {
                    let f = half_to_float(value(x) as u16);
                    out.extend_from_slice(&f.to_le_bytes());
                }
            }
            (TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF) => {
                for x in 0..width {
                    out.extend_from_slice(&(value(x) as u16).to_le_bytes());
                }
            }
            (TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF) => {
                for x in 0..width {
                    let h = float_to_half_full(f32::from_bits(value(x)));
                    out.extend_from_slice(&h.to_le_bytes());
                }
            }
            (TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_FLOAT)
            | (TINYEXR_PIXELTYPE_UINT, TINYEXR_PIXELTYPE_UINT) => {
                for x in 0..width {
                    out.extend_from_slice(&value(x).to_le_bytes());
                }
            }
            _ => return Err("Invalid requested_pixel_type.\n".to_string()),
        }
    }
    Ok(())
}

/// Port of `SaveEXRImageToMemory` for a single part, scanline, uncompressed
/// image (`compression_type` NONE, `long_name` 0, no tiles). Returns the file
/// or tinyexr's error message.
pub fn save_exr_image_to_memory(
    width: i32,
    height: i32,
    channels: &[ExrChannel],
    custom_attributes: &[ExrAttribute],
) -> Result<Vec<u8>, String> {
    // Header + version
    let mut memory: Vec<u8> = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];

    // NumScanlines(NONE) = 1
    let chunk_count = height.max(0) as usize;

    // Attributes
    write_attribute(
        &mut memory,
        "channels",
        "chlist",
        &write_channel_info(channels),
    );
    write_attribute(
        &mut memory,
        "compression",
        "compression",
        &[TINYEXR_COMPRESSIONTYPE_NONE as u8],
    );
    let mut window = Vec::with_capacity(16);
    for v in [0, 0, width - 1, height - 1] {
        window.extend_from_slice(&v.to_le_bytes());
    }
    write_attribute(&mut memory, "dataWindow", "box2i", &window);
    write_attribute(&mut memory, "displayWindow", "box2i", &window);
    write_attribute(&mut memory, "lineOrder", "lineOrder", &[0]);
    write_attribute(
        &mut memory,
        "pixelAspectRatio",
        "float",
        &1.0f32.to_le_bytes(),
    );
    let mut center = Vec::with_capacity(8);
    center.extend_from_slice(&0.0f32.to_le_bytes());
    center.extend_from_slice(&0.0f32.to_le_bytes());
    write_attribute(&mut memory, "screenWindowCenter", "v2f", &center);
    write_attribute(
        &mut memory,
        "screenWindowWidth",
        "float",
        &1.0f32.to_le_bytes(),
    );
    for a in custom_attributes {
        write_attribute(&mut memory, &a.name, &a.attr_type, &a.value);
    }
    memory.push(0); // end of header

    // EncodeChunk: one chunk per scanline
    let w = width.max(0) as usize;
    let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(chunk_count);
    for y in 0..chunk_count {
        let mut chunk = vec![0u8; 8];
        encode_scanline(&mut chunk, channels, w, y)?;
        if chunk.len() <= 8 {
            return Err("Failed to encode scanline data.\n".to_string());
        }
        let data_len = (chunk.len() - 8) as i32;
        chunk[..4].copy_from_slice(&(y as i32).to_le_bytes());
        chunk[4..8].copy_from_slice(&data_len.to_le_bytes());
        chunks.push(chunk);
    }

    let mut offset = (memory.len() + chunk_count * 8) as u64;
    let mut out = memory;
    for chunk in &chunks {
        out.extend_from_slice(&offset.to_le_bytes());
        offset += chunk.len() as u64;
    }
    for chunk in &chunks {
        out.extend_from_slice(chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_conversions() {
        for h in [
            0u16, 1, 0x3c00, 0x7bff, 0x7c00, 0x7e00, 0x8001, 0xfc00, 0x03ff,
        ] {
            let f = half_to_float(h);
            if !f.is_nan() {
                assert_eq!(float_to_half_full(f), h, "{h:04x}");
            }
        }
        assert_eq!(float_to_half_full(1.0e9), 0x7c00);
        assert_eq!(float_to_half_full(-1.0e-10), 0x8000);
        assert_eq!(float_to_half_full(f32::NAN), 0x7e00);
    }
}
