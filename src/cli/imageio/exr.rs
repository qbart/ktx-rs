//! Port of tools/imageio/exr.imageio/exrinput.cc (`ExrInput`) over the
//! `tinyexr` port.

use super::*;
use crate::dfd::colourspaces::{Primaries, find_mapping};
use crate::tinyexr::*;

pub(super) const EXR_INPUT_EXTENSIONS: &[&str] = &["exr"];

/// `ExrInput`.
pub struct ExrInput {
    base: ImageInputBase,
    exr_buffer: Vec<u8>,
    version: ExrVersion,
    header: ExrHeader,
    image: ExrImage,
    ec: i32,
    err: Option<String>,
}

impl Default for ExrInput {
    fn default() -> ExrInput {
        ExrInput::new()
    }
}

/// `fmt::format("{}", err)` of a `const char*`: a null pointer throws
/// `format_error`.
fn err_str(err: &Option<String>) -> ImageResult<&str> {
    err.as_deref()
        .ok_or_else(|| rt("string pointer is null".to_string()))
}

impl ExrInput {
    /// `exrInputCreate`.
    pub fn new() -> ExrInput {
        let mut header = ExrHeader::default();
        init_exr_header(&mut header);
        let mut image = ExrImage::default();
        init_exr_image(&mut image);
        ExrInput {
            base: ImageInputBase::new("exr"),
            exr_buffer: Vec::new(),
            version: ExrVersion::default(),
            header,
            image,
            ec: 0,
            err: None,
        }
    }

    /// `slurp`.
    fn slurp(&mut self) -> ImageResult<()> {
        let isp = self.base.stream()?;
        self.exr_buffer = isp.data().to_vec();
        Ok(())
    }

    /// The tinyexr header (valid after a successful open).
    pub fn header(&self) -> &ExrHeader {
        &self.header
    }
}

impl ImageInput for ExrInput {
    fn base(&self) -> &ImageInputBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ImageInputBase {
        &mut self.base
    }

    fn open_stream(&mut self, newspec: &mut ImageSpec) -> ImageResult<()> {
        {
            let mut version_data = [0u8; K_EXR_VERSION_SIZE];
            let isp = self.base.stream()?;
            isp.read(&mut version_data)?;
            if isp.fail() {
                return Err(self.base.throw_on_read_failure());
            }
            self.base.stream()?.seekg(0)?;

            self.ec = parse_exr_version_from_memory(&mut self.version, &version_data);
            if self.ec == TINYEXR_ERROR_INVALID_MAGIC_NUMBER {
                return Err(ImageError::DifferentFormat);
            }
            if self.ec != TINYEXR_SUCCESS {
                return Err(rt(format!("EXR version decode error: {}.", self.ec)));
            }
        }

        // It's an EXR file
        self.slurp()?;

        self.ec = parse_exr_header_from_memory(
            &mut self.header,
            &self.version,
            &self.exr_buffer,
            &mut self.err,
        );
        if self.ec != TINYEXR_SUCCESS {
            return Err(rt(format!(
                "EXR header decode error: {} - {}.",
                self.ec,
                err_str(&self.err)?
            )));
        }

        // Determine file data format
        let mut bit_depth = 0u32;
        let mut format_type = ImageInputFormatType::ExrFloat;
        let mut qualifiers = 0u32;
        for c in &self.header.channels {
            match c.pixel_type {
                TINYEXR_PIXELTYPE_FLOAT => {
                    bit_depth = bit_depth.max(32);
                    format_type = ImageInputFormatType::ExrFloat;
                    qualifiers = KHR_DF_SAMPLE_DATATYPE_SIGNED | KHR_DF_SAMPLE_DATATYPE_FLOAT;
                }
                TINYEXR_PIXELTYPE_HALF => {
                    bit_depth = bit_depth.max(16);
                    format_type = ImageInputFormatType::ExrFloat;
                    qualifiers = KHR_DF_SAMPLE_DATATYPE_SIGNED | KHR_DF_SAMPLE_DATATYPE_FLOAT;
                }
                TINYEXR_PIXELTYPE_UINT => {
                    bit_depth = bit_depth.max(32);
                    format_type = ImageInputFormatType::ExrUint;
                    qualifiers = 0;
                }
                t => {
                    return Err(rt(format!(
                        "EXR header decode error: Not supported pixel type: {t}."
                    )));
                }
            }
        }

        let dw = self.header.data_window;
        let width = dw.max_x.wrapping_sub(dw.min_x).wrapping_add(1) as u32;
        let height = dw.max_y.wrapping_sub(dw.min_y).wrapping_add(1) as u32;

        // Use "chromaticities", if present, to determine color primaries.
        // Per the EXR spec. in the absence of chromaticities, use bt.709.
        let mut color_primaries = KHR_DF_PRIMARIES_BT709;
        for a in &self.header.custom_attributes {
            if a.name == b"chromaticities" {
                let expected_size = 8 * 4;
                if a.value.len() != expected_size {
                    return Err(rt(format!(
                        "EXR chromaticities attribute decode error: Expected size {} but got {}.",
                        expected_size,
                        a.value.len()
                    )));
                }
                let f = |i: usize| {
                    f32::from_le_bytes([
                        a.value[4 * i],
                        a.value[4 * i + 1],
                        a.value[4 * i + 2],
                        a.value[4 * i + 3],
                    ])
                };
                let primaries = Primaries {
                    Rx: f(0),
                    Ry: f(1),
                    Gx: f(2),
                    Gy: f(3),
                    Bx: f(4),
                    By: f(5),
                    Wx: f(6),
                    Wy: f(7),
                };
                color_primaries = find_mapping(&primaries, 0.002);
            }
        }

        let format = FormatDescriptor::new(
            self.header.num_channels() as u32,
            bit_depth,
            qualifiers,
            KHR_DF_TRANSFER_LINEAR,
            color_primaries,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
        );
        self.base.images.push(ImageInfo {
            // We make TinyEXR decode to top-left.
            spec: ImageSpec::new(
                width,
                height,
                1,
                Origin::new(Origin::E_LEFT, Origin::E_TOP),
                format,
            ),
            format_type,
            filepos: 0,
        });

        *newspec = self.spec().clone();
        Ok(())
    }

    /// Reads the whole image converted to `request_format`: half -> [half,
    /// float, uint], float -> float, uint -> uint, R / G / B / A channels by
    /// name (missing ones get 0, 0, 0, 1).
    fn read_image(
        &mut self,
        output_buffer: &mut [u8],
        _subimage: u32,
        _miplevel: u32,
        request_format: &FormatDescriptor,
    ) -> ImageResult<()> {
        let target = if request_format.is_unknown() {
            self.spec().format().clone()
        } else {
            request_format.clone()
        };

        if !target.same_unit_all_channels() || target.samples.is_empty() {
            return Err(rt(
                "EXR load error: Requested format conversion to different channels is not supported."
                    .to_string(),
            ));
        }

        let s0 = target.samples[0];
        let target_bit_depth = s0.bit_length + 1;
        let (l, e, s, f) = (
            s0.qualifier_linear,
            s0.qualifier_exponent,
            s0.qualifier_signed,
            s0.qualifier_float,
        );

        // TinyEXR only supports half->[half,float,uint], float->float,
        // uint->uint conversions
        let requested_type = if target_bit_depth == 16 && !e && !l && s && f {
            TINYEXR_PIXELTYPE_HALF
        } else if target_bit_depth == 32 && !e && !l && s && f {
            TINYEXR_PIXELTYPE_FLOAT
        } else if target_bit_depth == 32 && !e && !l && !s && !f {
            TINYEXR_PIXELTYPE_UINT
        } else {
            return Err(rt(format!(
                "EXR load error: Requested format conversion to {}-bit{}{}{}{} is not supported.",
                target_bit_depth,
                if l { " Linear" } else { "" },
                if e { " Exponent" } else { "" },
                if s { " Signed" } else { "" },
                if f { " Float" } else { "" }
            )));
        };

        for i in 0..self.header.channels.len() {
            self.header.requested_pixel_types[i] = requested_type;
            let pt = self.header.pixel_types[i];
            if pt != TINYEXR_PIXELTYPE_HALF && pt != requested_type {
                return Err(rt(
                    "EXR load error: Requested format conversion from the input type is not supported."
                        .to_string(),
                ));
            }
        }

        // Load image version
        let mut exr_version = ExrVersion::default();
        self.ec = parse_exr_version_from_memory(&mut exr_version, &self.exr_buffer);
        if self.ec != TINYEXR_SUCCESS {
            return Err(rt(format!(
                "EXR load error: {} - {}.",
                self.ec, "Failed to parse EXR version"
            )));
        }
        if exr_version.multipart != 0 || exr_version.non_image != 0 {
            return Err(rt(format!(
                "EXR load error: {}.",
                "Unsupported EXR version (2.0)"
            )));
        }

        // Force a top-left origin regardless of the file's line order
        // (https://github.com/syoyo/tinyexr/issues/213).
        self.header.line_order = 0;
        self.ec = load_exr_image_from_memory(
            &mut self.image,
            &mut self.header,
            &self.exr_buffer,
            &mut self.err,
        );
        if self.ec != TINYEXR_SUCCESS {
            return Err(rt(format!(
                "EXR load error: {} - {}.",
                self.ec,
                err_str(&self.err)?
            )));
        }

        if self
            .header
            .custom_attributes
            .iter()
            .any(|a| a.name == b"envmap")
        {
            return Err(rt("EXR envmaps are not supported. Use exrenvmap to export images for each face and use those to create your KTX file.".to_string()));
        }
        if self.header.tiled != 0 && self.header.tile_level_mode != TINYEXR_TILE_ONE_LEVEL {
            return Err(rt(
                "Multi-level tiled EXR images are not supported.".to_string()
            ));
        }
        if (self.header.tiled != 0 && self.image.tiles.is_none())
            || (self.header.tiled == 0 && self.image.images.is_none())
        {
            return Err(rt(format!(
                "Invalid EXR file. {}",
                if self.header.tiled != 0 {
                    "Tiled with no tiles"
                } else {
                    "scanline with no images."
                }
            )));
        }

        let height = self.image.height as u32;
        let width = self.image.width as u32;
        let num_target_channels = target.channel_count();

        let expected_buffer_byte_count = height
            .wrapping_mul(width)
            .wrapping_mul(num_target_channels)
            .wrapping_mul(target_bit_depth)
            / 8;
        if output_buffer.len() != expected_buffer_byte_count as usize {
            return Err(rt(format!(
                "EXR load error: Provided target buffer size is {} does not match the expected value: {}.",
                output_buffer.len(),
                expected_buffer_byte_count
            )));
        }

        // Find the RGBA channels
        let mut channels: [Option<usize>; 4] = [None; 4];
        for (i, c) in self.header.channels.iter().enumerate() {
            match c.name.as_slice() {
                b"R" => channels[0] = Some(i),
                b"G" => channels[1] = Some(i),
                b"B" => channels[2] = Some(i),
                b"A" => channels[3] = Some(i),
                name => self.base.warning(&format!(
                    "EXR load warning: Unrecognized channel \"{}\" is ignored.",
                    String::from_utf8_lossy(name)
                )),
            }
        }

        let (data_size, default_color): (usize, [[u8; 4]; 4]) = match requested_type {
            TINYEXR_PIXELTYPE_HALF => {
                let h = |v: u16| {
                    let b = v.to_le_bytes();
                    [b[0], b[1], 0, 0]
                };
                (2, [h(0), h(0), h(0), h(0x3C00)])
            }
            TINYEXR_PIXELTYPE_FLOAT => (4, [0.0f32, 0.0, 0.0, 1.0].map(f32::to_le_bytes)),
            _ => (4, [0u32, 0, 0, 1].map(u32::to_le_bytes)),
        };

        let ntc = num_target_channels as usize;
        let mut put = |pixel_index: usize, c: usize, src: Option<&[u8]>| {
            let dst = (pixel_index * ntc + c) * data_size;
            let Some(d) = output_buffer.get_mut(dst..dst + data_size) else {
                return;
            };
            match src {
                Some(s) => d.copy_from_slice(s),
                None => d.copy_from_slice(&default_color[c.min(3)][..data_size]),
            }
        };

        if self.header.tiled != 0 {
            let tsx = self.header.tile_size_x as u32;
            let tsy = self.header.tile_size_y as u32;
            for tile in self.image.tiles.iter().flatten() {
                for ty in 0..tile.height.max(0) as u32 {
                    for tx in 0..tile.width.max(0) as u32 {
                        let x = (tile.offset_x as u32).wrapping_mul(tsx).wrapping_add(tx);
                        let y = (tile.offset_y as u32).wrapping_mul(tsy).wrapping_add(ty);
                        let pixel = y.wrapping_mul(width).wrapping_add(x) as usize;
                        for c in 0..ntc {
                            let src = channels.get(c).copied().flatten().and_then(|ch| {
                                let off = ((ty * tsx + tx) as usize) * data_size;
                                tile.images
                                    .as_ref()
                                    .and_then(|im| im.get(ch))
                                    .and_then(|im| im.get(off..off + data_size))
                            });
                            put(pixel, c, src);
                        }
                    }
                }
            }
        } else if let Some(images) = &self.image.images {
            for y in 0..height {
                for x in 0..width {
                    let pixel = (y * width + x) as usize;
                    for c in 0..ntc {
                        let src = channels.get(c).copied().flatten().and_then(|ch| {
                            let off = pixel * data_size;
                            images.get(ch).and_then(|im| im.get(off..off + data_size))
                        });
                        put(pixel, c, src);
                    }
                }
            }
        }
        Ok(())
    }

    fn read_native_scanline(
        &mut self,
        _buffer: &mut [u8],
        _buffer_byte_count: usize,
        _y: u32,
        _z: u32,
        _subimage: u32,
        _miplevel: u32,
    ) -> ImageResult<()> {
        Ok(())
    }
}
