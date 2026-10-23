//! Port of tools/imageio/png.imageio/pnginput.cc (`PngInput`): lodepng's
//! split decode (`lodepng_decode_chunks` at open, `lodepng_finish_decode` in
//! `readImage`).

use super::*;
use crate::cli::format_descriptor::convert_unorm;
use crate::dfd::colourspaces::{Primaries, find_mapping};
use crate::lodepng::{
    LCT_GREY, LCT_GREY_ALPHA, LCT_PALETTE, LCT_RGB, LCT_RGBA, State, lodepng_chunk_find_const,
    lodepng_decode_chunks, lodepng_error_text, lodepng_finish_decode,
};

pub(super) const PNG_INPUT_EXTENSIONS: &[&str] = &["png"];

/// `PngInput`.
pub struct PngInput {
    base: ImageInputBase,
    png_buffer: Vec<u8>,
    state: State,
    /// `pIdat`: the concatenated IDAT data until `readImage` consumes it.
    p_idat: Option<Vec<u8>>,
    next_scanline: u32,
}

impl Default for PngInput {
    fn default() -> PngInput {
        PngInput::new()
    }
}

impl PngInput {
    /// `pngInputCreate`.
    pub fn new() -> PngInput {
        PngInput {
            base: ImageInputBase::new("png"),
            png_buffer: Vec::new(),
            state: State::new(),
            p_idat: None,
            next_scanline: 0,
        }
    }

    /// `slurp`: the whole stream.
    fn slurp(&mut self) -> ImageResult<()> {
        let isp = self.base.stream()?;
        self.png_buffer = isp.data().to_vec();
        Ok(())
    }

    /// `readHeader`.
    fn read_header(&mut self) -> ImageResult<()> {
        const PNGSIG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut filesig = [0u8; 8];
        let isp = self.base.stream()?;
        isp.read(&mut filesig)?;
        if isp.fail() {
            return Err(self.base.throw_on_read_failure());
        }
        if filesig != PNGSIG {
            return Err(ImageError::DifferentFormat);
        }
        // It's a PNG file.
        self.base.stream()?.seekg(0)?;
        self.slurp()?;

        let (idat, w, h) = match lodepng_decode_chunks(&mut self.state, &self.png_buffer) {
            Ok(r) => r,
            Err(e) => {
                return Err(rt(format!(
                    "PNG decode chunks error: {}.",
                    lodepng_error_text(e)
                )));
            }
        };
        self.p_idat = Some(idat);

        // Tell the decoder to produce the same color type as the file.
        self.state.info_raw = self.state.info_png.color.clone();

        let color = self.state.info_png.color.clone();
        let mut component_count = 0u32;
        let mut bit_depth = color.bitdepth;
        let mut color_model = KHR_DF_MODEL_RGBSDA;
        match color.colortype {
            LCT_GREY => {
                color_model = KHR_DF_MODEL_YUVSDA;
                if color.key_defined {
                    self.state.info_raw.colortype = LCT_GREY_ALPHA;
                    component_count = 2;
                } else {
                    component_count = 1;
                }
            }
            LCT_RGB => {
                if color.key_defined {
                    self.state.info_raw.colortype = LCT_RGBA;
                    component_count = 4;
                } else {
                    component_count = 3;
                }
            }
            LCT_PALETTE => {
                // tRNS info is written directly into the palette; expand to
                // RGBA when there is a tRNS chunk.
                let end = self.png_buffer.len().saturating_sub(1);
                let trns = lodepng_chunk_find_const(&self.png_buffer, 33, end, b"tRNS");
                if trns.is_some() {
                    self.state.info_raw.colortype = LCT_RGBA;
                    component_count = 4;
                } else {
                    self.state.info_raw.colortype = LCT_RGB;
                    component_count = 3;
                }
                if color.bitdepth < 8 {
                    bit_depth = 8;
                }
                self.base.fwarning(&format!(
                    "Expanding {}-bit paletted image to {}",
                    color.bitdepth,
                    if self.state.info_raw.colortype == LCT_RGBA {
                        "R8G8B8A8"
                    } else {
                        "R8G8B8"
                    }
                ));
            }
            LCT_GREY_ALPHA => {
                component_count = 2;
                color_model = KHR_DF_MODEL_YUVSDA;
            }
            LCT_RGBA => {
                component_count = 4;
            }
            _ => {}
        }

        let format_type = match color.colortype {
            LCT_GREY => ImageInputFormatType::PngL,
            LCT_GREY_ALPHA => ImageInputFormatType::PngLa,
            LCT_RGB => ImageInputFormatType::PngRgb,
            _ => ImageInputFormatType::PngRgba,
        };

        let mut format = FormatDescriptor::new(
            component_count,
            bit_depth,
            0,
            KHR_DF_TRANSFER_UNSPECIFIED,
            // PNG spec. says BT.709 primaries are a reasonable default.
            KHR_DF_PRIMARIES_BT709,
            color_model,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
        );

        let info = &self.state.info_png;
        if info.iccp_defined {
            format.set_primaries(KHR_DF_PRIMARIES_UNSPECIFIED);
            format.set_transfer(KHR_DF_TRANSFER_UNSPECIFIED);
            let name_len = info
                .iccp_name
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(info.iccp_name.len());
            format.icc_profile_name = info.iccp_name[..name_len].to_vec();
            // resize() then insert() at the front: the profile followed by as
            // many zeros.
            let mut profile = info.iccp_profile.clone();
            profile.resize(2 * info.iccp_profile.len(), 0);
            format.icc_profile = profile;
            if format.icc_profile_name == b"ITUR_2100_PQ_FULL" {
                format.set_primaries(KHR_DF_PRIMARIES_BT2020);
                format.set_transfer(KHR_DF_TRANSFER_PQ_EOTF);
            }
        } else if info.srgb_defined {
            format.set_transfer(KHR_DF_TRANSFER_SRGB);
            format.set_primaries(KHR_DF_PRIMARIES_SRGB);
        } else if info.gama_defined {
            format.set_transfer(KHR_DF_TRANSFER_UNSPECIFIED);
            // gAMA is the exponent of the encoding power curve * 100000.
            format.oe_gamma = info.gama_gamma as f32 / 100000.0;
        } else {
            format.set_transfer(KHR_DF_TRANSFER_UNSPECIFIED);
        }
        if info.chrm_defined && !info.srgb_defined && !info.iccp_defined {
            let c = |v: u32| v as f32 / 100000.0;
            let primaries = Primaries {
                Rx: c(info.chrm_red_x),
                Ry: c(info.chrm_red_y),
                Gx: c(info.chrm_green_x),
                Gy: c(info.chrm_green_y),
                Bx: c(info.chrm_blue_x),
                By: c(info.chrm_blue_y),
                Wx: c(info.chrm_white_x),
                Wy: c(info.chrm_white_y),
            };
            format.set_primaries(find_mapping(&primaries, 0.002));
        }

        self.base.images.push(ImageInfo {
            spec: ImageSpec::new(w, h, 1, Origin::new(Origin::E_LEFT, Origin::E_TOP), format),
            format_type,
            filepos: 0,
        });
        Ok(())
    }
}

impl ImageInput for PngInput {
    fn base(&self) -> &ImageInputBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ImageInputBase {
        &mut self.base
    }

    fn open_stream(&mut self, newspec: &mut ImageSpec) -> ImageResult<()> {
        self.read_header()?;
        *newspec = self.spec().clone();
        self.next_scanline = 0;
        Ok(())
    }

    fn close(&mut self) {
        self.p_idat = None;
        self.base.isp = None;
    }

    /// Reads the whole image converted to `format`: unorm <= 8 -> 8 / 16,
    /// 8 <-> 16 bits, GREY / GREY_ALPHA / RGB / RGBA conversions, sBIT
    /// rescaling.
    fn read_image(
        &mut self,
        buffer: &mut [u8],
        _subimage: u32,
        _miplevel: u32,
        format: &FormatDescriptor,
    ) -> ImageResult<()> {
        let target = if format.is_unknown() {
            self.spec().format().clone()
        } else {
            format.clone()
        };
        let channel_count = target.channel_count();
        let height = self.spec().height();
        let width = self.spec().width();
        let target_bit_length = target.largest_channel_bit_length();
        let request_bits = bit_ceil(target_bit_length).max(8);
        if request_bits != 8 && request_bits != 16 {
            return Err(rt(format!(
                "PNG decode error: Requested decode into {request_bits}-bit format is not supported."
            )));
        }
        let s0 = first_sample(&target);
        // Only UNORM requests are allowed for PNG inputs
        if s0.qualifier_exponent || s0.qualifier_linear || s0.qualifier_signed || s0.qualifier_float
        {
            return Err(rt(format!(
                "PNG decode error: Requested format conversion to {}-bit{}{}{}{} is not supported.",
                request_bits,
                if s0.qualifier_linear { " Linear" } else { "" },
                if s0.qualifier_exponent {
                    " Exponent"
                } else {
                    ""
                },
                if s0.qualifier_signed { " Signed" } else { "" },
                if s0.qualifier_float { " Float" } else { "" }
            )));
        }

        self.state.info_raw.bitdepth = request_bits;
        self.state.info_raw.colortype = match channel_count {
            1 => LCT_GREY,
            2 => LCT_GREY_ALPHA,
            3 => LCT_RGB,
            4 => LCT_RGBA,
            n => {
                return Err(rt(format!(
                    "PNG decode error: Requested decode into {n} channels is not supported."
                )));
            }
        };

        // lodepng_finish_decode frees pIdat regardless of error
        let idat = self.p_idat.take().unwrap_or_default();
        if let Err(e) = lodepng_finish_decode(buffer, width, height, &mut self.state, &idat) {
            return Err(rt(format!("PNG decode error: {}.", lodepng_error_text(e))));
        }

        if request_bits == 16 {
            // LodePNG loads 16 bit channels in big endian order
            for pair in buffer.as_chunks_mut::<2>().0 {
                pair.swap(0, 1);
            }
        }

        let info = &self.state.info_png;
        if info.sbit_defined {
            // Recalculate the UNORM values from the significant bits.
            let mut s_bits = [info.sbit_r, info.sbit_g, info.sbit_b, info.sbit_a];
            for b in s_bits.iter_mut().take(channel_count as usize) {
                if *b == 0 {
                    // channel added
                    *b = request_bits;
                }
            }
            let cc = channel_count;
            for y in 0..height {
                for x in 0..width {
                    for c in 0..cc.min(4) {
                        let index = y
                            .wrapping_mul(width)
                            .wrapping_mul(cc)
                            .wrapping_add(x.wrapping_mul(cc))
                            .wrapping_add(c) as usize;
                        let sb = s_bits[c as usize];
                        // x86 masks the shift count of the int shift
                        let shift = request_bits.wrapping_sub(sb) & 31;
                        if request_bits == 8 {
                            if let Some(v) = buffer.get_mut(index) {
                                *v = convert_unorm(u32::from(*v) >> shift, sb, 8) as u8;
                            }
                        } else if let Some(b) = buffer.get_mut(2 * index..2 * index + 2) {
                            let v = u32::from(u16::from_le_bytes([b[0], b[1]]));
                            let r = convert_unorm(v >> shift, sb, 16) as u16;
                            b.copy_from_slice(&r.to_le_bytes());
                        }
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
