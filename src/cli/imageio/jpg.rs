//! Port of tools/imageio/jpg.imageio/jpginput.cc (`JpegInput`) over the
//! `jpgd` port. JPEG files are treated as sRGB, 8 bits per channel.

use super::*;
use crate::jpgd::{
    JPGD_NOT_JPEG, JPGD_NOTENOUGHMEM, JPGD_SUCCESS, JpegDecoder, JpegDecoderStream,
    jpgd_status_name,
};

pub(super) const JPEG_INPUT_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

/// `myjpgdstream`: jpgd stream over the input `std::istream`.
struct MyJpgdStream {
    isp: Option<IStream>,
}

impl JpegDecoderStream for MyJpgdStream {
    fn read(&mut self, buf: &mut [u8], eof_flag: &mut bool) -> i32 {
        let Some(isp) = &mut self.isp else {
            return -1;
        };
        if isp.eof() {
            *eof_flag = true;
            return 0;
        }
        if isp.fail() {
            return -1;
        }
        let n = isp.read(buf).unwrap_or(0);
        // eof check first: fail is also set on a short read.
        if isp.eof() {
            *eof_flag = true;
        } else if isp.fail() {
            return -1;
        }
        n as i32
    }
}

/// `JpegInput`.
pub struct JpegInput {
    base: ImageInputBase,
    p_jd: Option<JpegDecoder>,
    next_scanline: u32,
    decoding_begun: bool,
}

impl Default for JpegInput {
    fn default() -> JpegInput {
        JpegInput::new()
    }
}

impl JpegInput {
    /// `jpegInputCreate`.
    pub fn new() -> JpegInput {
        JpegInput {
            base: ImageInputBase::new("jpeg"),
            p_jd: None,
            next_scanline: 0,
            decoding_begun: false,
        }
    }

    /// `readHeader`.
    fn read_header(&mut self) -> ImageResult<()> {
        let stream = MyJpgdStream {
            isp: self.base.isp.clone(),
        };
        let jd = JpegDecoder::new(
            Box::new(stream),
            JpegDecoder::C_FLAG_LINEAR_CHROMA_FILTERING,
        );
        let error_code = jd.get_error_code();
        let (w, h, nc) = (jd.get_width(), jd.get_height(), jd.get_num_components());
        self.p_jd = Some(jd);
        if error_code != JPGD_SUCCESS {
            return Err(match error_code {
                JPGD_NOT_JPEG => ImageError::DifferentFormat,
                JPGD_NOTENOUGHMEM => rt("JPEG decoder out of memory".to_string()),
                e => rt(format!("JPEG decode failed: {}", jpgd_status_name(e))),
            });
        }
        let format = FormatDescriptor::new(
            nc as u32,
            8, // component bit length
            0,
            KHR_DF_TRANSFER_SRGB,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
        );
        self.base.images.push(ImageInfo {
            spec: ImageSpec::new(
                w as u32,
                h as u32,
                1,
                Origin::new(Origin::E_LEFT, Origin::E_TOP),
                format,
            ),
            format_type: ImageInputFormatType::Jpg,
            filepos: 0,
        });
        Ok(())
    }
}

impl ImageInput for JpegInput {
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
        self.decoding_begun = false;
        self.p_jd = None;
        self.base.isp = None;
    }

    /// Reads the whole image: [GREY, RGB] -> [GREY, RGB, RGBA] 8-bit.
    fn read_image(
        &mut self,
        buffer: &mut [u8],
        subimage: u32,
        miplevel: u32,
        format: &FormatDescriptor,
    ) -> ImageResult<()> {
        if let Some(jd) = &mut self.p_jd {
            jd.begin_decoding();
        }
        self.decoding_begun = true;
        read_image_default(self, buffer, subimage, miplevel, format)
    }

    /// Reads the next scan line converted to `format`. Reducing to 1
    /// channel computes luma, growing from 1 replicates it, alpha is 255.
    fn read_scanline(
        &mut self,
        buffer: &mut [u8],
        y: u32,
        _z: u32,
        _subimage: u32,
        _miplevel: u32,
        format: &FormatDescriptor,
    ) -> ImageResult<()> {
        let target = if format.is_unknown() {
            self.spec().format().clone()
        } else {
            format.clone()
        };
        let request_bits = target.largest_channel_bit_length();
        if request_bits != 8 {
            return Err(rt(format!(
                "Requested decode into {request_bits}-bit format is not supported."
            )));
        }
        let s0 = first_sample(&target);
        // Only UNORM requests are allowed for JPEG inputs
        if s0.qualifier_exponent || s0.qualifier_linear || s0.qualifier_signed || s0.qualifier_float
        {
            return Err(rt(format!(
                "Requested format conversion to {}-bit{}{}{}{} is not supported.",
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

        let height = self.spec().height();
        let width = self.spec().width();
        let y = if y >= height {
            height.wrapping_sub(1)
        } else {
            y
        };
        if y != self.next_scanline {
            return Err(rt(
                "Random scanline seeking not yet implemented.".to_string()
            ));
        }

        let Some(jd) = &mut self.p_jd else {
            return Err(rt("No file opened.".to_string()));
        };
        if !self.decoding_begun {
            jd.begin_decoding();
            self.decoding_begun = true;
        }
        let (decode_status, scan_line) = jd.decode();
        if decode_status != JPGD_SUCCESS {
            // Only other decodeStatus is JPGD_FAILED
            let e = jd.get_error_code();
            return Err(rt(format!("JPEG decode failed: {}", jpgd_status_name(e))));
        }
        let scan_line = scan_line.to_vec();
        let scanline_byte_count = scan_line.len();

        let target_channel_count = target.channel_count();
        if target_channel_count == 2 {
            return Err(rt(
                "Requested decode into 2 channels is not supported.".to_string()
            ));
        }

        let input_channel_count = self.spec().format().channel_count();
        let w = width as usize;
        let px = |i: usize| scan_line.get(i).copied().unwrap_or(0);
        if (target_channel_count == 1 && input_channel_count == 1)
            || (target_channel_count == 4 && input_channel_count == 3)
        {
            if buffer.len() < scanline_byte_count {
                return Err(ImageError::buffer_too_small());
            }
            buffer[..scanline_byte_count].copy_from_slice(&scan_line);
        } else if input_channel_count == 1 {
            if target_channel_count == 3 {
                if buffer.len() < scanline_byte_count * 3 {
                    return Err(ImageError::buffer_too_small());
                }
                for x in 0..w {
                    buffer[x * 3..x * 3 + 3].fill(px(x));
                }
            } else {
                if buffer.len() < scanline_byte_count * 4 {
                    return Err(ImageError::buffer_too_small());
                }
                for x in 0..w {
                    buffer[x * 4..x * 4 + 3].fill(px(x));
                    buffer[x * 4 + 3] = 255;
                }
            }
        } else if input_channel_count == 3 {
            if target_channel_count == 1 {
                if buffer.len() < w {
                    return Err(ImageError::buffer_too_small());
                }
                const YR: i32 = 19595;
                const YG: i32 = 38470;
                const YB: i32 = 7471;
                for (x, d) in buffer.iter_mut().take(w).enumerate() {
                    let r = i32::from(px(x * 4));
                    let g = i32::from(px(x * 4 + 1));
                    let b = i32::from(px(x * 4 + 2));
                    *d = ((r * YR + g * YG + b * YB + 32768) >> 16) as u8;
                }
            } else {
                if buffer.len() < w * 3 {
                    return Err(ImageError::buffer_too_small());
                }
                for x in 0..w {
                    for c in 0..3 {
                        buffer[x * 3 + c] = px(x * 4 + c);
                    }
                }
            }
        }

        self.next_scanline += 1;
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
