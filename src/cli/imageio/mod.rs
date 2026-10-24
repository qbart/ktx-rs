//! Port of the parts of tools/imageio that `ktx create` needs for its common
//! input formats: `ImageSpec`, the `ImageInput` base class (`imageio.h`,
//! `imageinput.cc`, `imageio.cc`) and the PNG, EXR and JPEG readers. The
//! netpbm reader and the output plugins are not ported. C++
//! exceptions are `ImageError` values; streams are the whole input in memory
//! (`IStream` emulates the `std::istream` calls the readers make).

mod exr;
pub mod image;
mod jpg;
mod png;

use super::format_descriptor::{FormatDescriptor, Sample};
use super::utility::io_error_message;
use crate::dfd::khr_df::*;
use std::collections::BTreeMap;
use std::io::Read;
use std::rc::Rc;

pub use exr::ExrInput;
pub use jpg::JpegInput;
pub use png::PngInput;

/// Exception thrown by an image reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// `ImageInput::different_format` (`what()` is empty).
    DifferentFormat,
    /// `std::runtime_error` and subclasses (`invalid_file`,
    /// `buffer_too_small`, `fmt::format_error`).
    Runtime(String),
    /// `std::ios_base::failure` thrown by a stream with exceptions enabled.
    Stream,
}

impl ImageError {
    /// `what()`.
    pub fn what(&self) -> &str {
        match self {
            ImageError::DifferentFormat => "",
            ImageError::Runtime(s) => s,
            ImageError::Stream => "basic_ios::clear: iostream error",
        }
    }

    /// `invalid_file(error)`.
    pub fn invalid_file(error: &str) -> ImageError {
        ImageError::Runtime(format!("Invalid file: {error}"))
    }

    /// `buffer_too_small()`.
    pub fn buffer_too_small() -> ImageError {
        ImageError::Runtime("Image buffer too small.".to_string())
    }
}

pub type ImageResult<T> = Result<T, ImageError>;

fn rt(msg: String) -> ImageError {
    ImageError::Runtime(msg)
}

/// `imageio::bit_ceil`.
pub fn bit_ceil(x: u32) -> u32 {
    let x = x.wrapping_sub(1);
    for i in 0..32 {
        if (1u32 << i) > x {
            return 1u32 << i;
        }
    }
    0
}

/// `ImageSpec::Origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub x: u8,
    pub y: u8,
    pub z: u8,
}

impl Origin {
    pub const E_LEFT: u8 = 0;
    pub const E_RIGHT: u8 = 1;
    pub const E_TOP: u8 = 0;
    pub const E_BOTTOM: u8 = 1;
    pub const E_FRONT: u8 = 0;
    pub const E_BACK: u8 = 1;
    pub const E_UNSPECIFIED: u8 = 0xff;

    pub fn new(x: u8, y: u8) -> Origin {
        Origin {
            x,
            y,
            z: Origin::E_FRONT,
        }
    }

    pub fn unspecified(&self) -> bool {
        self.x == Origin::E_UNSPECIFIED
            || self.y == Origin::E_UNSPECIFIED
            || self.z == Origin::E_UNSPECIFIED
    }
}

impl Default for Origin {
    fn default() -> Origin {
        Origin {
            x: Origin::E_LEFT,
            y: Origin::E_TOP,
            z: Origin::E_FRONT,
        }
    }
}

/// `toString(const ImageSpec::Origin&)`.
pub fn origin_to_string(o: &Origin) -> String {
    let part = |v: u8, a: &str, b: &str| match v {
        0 => a.to_string(),
        1 => b.to_string(),
        Origin::E_UNSPECIFIED => "unspecified".to_string(),
        _ => String::new(),
    };
    format!(
        "{},{},{}",
        part(o.x, "left", "right"),
        part(o.y, "top", "bottom"),
        part(o.z, "front", "back")
    )
}

/// `ImageSpec`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageSpec {
    format_desc: FormatDescriptor,
    image_width: u32,
    image_height: u32,
    image_depth: u32,
    image_origin: Origin,
}

impl ImageSpec {
    /// `ImageSpec(w, h, d, Origin, formatDesc)`.
    pub fn new(w: u32, h: u32, d: u32, o: Origin, format_desc: FormatDescriptor) -> ImageSpec {
        ImageSpec {
            format_desc,
            image_width: w,
            image_height: h,
            image_depth: d,
            image_origin: o,
        }
    }

    pub fn format(&self) -> &FormatDescriptor {
        &self.format_desc
    }
    pub fn format_mut(&mut self) -> &mut FormatDescriptor {
        &mut self.format_desc
    }
    pub fn width(&self) -> u32 {
        self.image_width
    }
    pub fn height(&self) -> u32 {
        self.image_height
    }
    pub fn depth(&self) -> u32 {
        self.image_depth
    }
    pub fn origin(&self) -> &Origin {
        &self.image_origin
    }
    pub fn set_width(&mut self, w: u32) {
        self.image_width = w;
    }
    pub fn set_height(&mut self, h: u32) {
        self.image_height = h;
    }
    pub fn set_depth(&mut self, d: u32) {
        self.image_depth = d;
    }
    pub fn set_origin(&mut self, o: Origin) {
        self.image_origin = o;
    }
    /// `imagePixelCount` (the product is a `uint32_t`).
    pub fn image_pixel_count(&self) -> usize {
        self.depth()
            .wrapping_mul(self.width())
            .wrapping_mul(self.height()) as usize
    }
    pub fn image_channel_count(&self) -> usize {
        self.image_pixel_count()
            .wrapping_mul(self.format().channel_count() as usize)
    }
    pub fn image_byte_count(&self) -> usize {
        self.image_pixel_count()
            .wrapping_mul(self.format().pixel_byte_count() as usize)
    }
    pub fn scanline_byte_count(&self) -> usize {
        self.width().wrapping_mul(self.format().pixel_byte_count()) as usize
    }
    pub fn scanline_channel_count(&self) -> usize {
        self.width().wrapping_mul(self.format().channel_count()) as usize
    }
}

/// `ImageInputFormatType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageInputFormatType {
    PngL,
    PngLa,
    PngRgb,
    PngRgba,
    ExrUint,
    ExrFloat,
    Npbm,
    Jpg,
}

/// `toString(ImageInputFormatType)`.
pub fn format_type_to_string(t: ImageInputFormatType) -> &'static str {
    match t {
        ImageInputFormatType::PngL => "png_l",
        ImageInputFormatType::PngLa => "png_la",
        ImageInputFormatType::PngRgb => "png_rgb",
        ImageInputFormatType::PngRgba => "png_rgba",
        ImageInputFormatType::ExrUint => "exr_uint",
        ImageInputFormatType::ExrFloat => "exr_float",
        ImageInputFormatType::Npbm => "npbm",
        ImageInputFormatType::Jpg => "jpg",
    }
}

/// `std::istream` over the whole input with the state bits and exception
/// mask the readers rely on (positions past the end are allowed like a
/// `std::filebuf`).
#[derive(Debug, Clone)]
pub struct IStream {
    data: Rc<Vec<u8>>,
    pos: usize,
    eof: bool,
    fail: bool,
    /// `exceptions(failbit | badbit | eofbit)`.
    exceptions: bool,
}

impl IStream {
    pub fn new(data: Rc<Vec<u8>>) -> IStream {
        IStream {
            data,
            pos: 0,
            eof: false,
            fail: false,
            exceptions: false,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn eof(&self) -> bool {
        self.eof
    }
    pub fn fail(&self) -> bool {
        self.fail
    }
    pub fn good(&self) -> bool {
        !self.eof && !self.fail
    }
    pub fn clear(&mut self) {
        self.eof = false;
        self.fail = false;
    }

    fn check(&self) -> ImageResult<()> {
        if self.exceptions && !self.good() {
            Err(ImageError::Stream)
        } else {
            Ok(())
        }
    }

    /// `sentry`: a stream that is not good fails the operation.
    fn sentry(&mut self) -> bool {
        if !self.good() {
            self.fail = true;
            return false;
        }
        true
    }

    /// `read(buf, n)`: copies up to `buf.len()` bytes, eof + fail when short.
    pub fn read(&mut self, buf: &mut [u8]) -> ImageResult<usize> {
        if !self.sentry() {
            self.check()?;
            return Ok(0);
        }
        let avail = self.data.len().saturating_sub(self.pos);
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if n < buf.len() {
            self.eof = true;
            self.fail = true;
        }
        self.check()?;
        Ok(n)
    }

    /// `seekg(pos)` / `seekg(off, dir)`: clears eof first like C++11.
    pub fn seekg(&mut self, pos: i64) -> ImageResult<()> {
        self.eof = false;
        if self.fail {
            return self.check();
        }
        match usize::try_from(pos) {
            Ok(p) => self.pos = p,
            Err(_) => self.fail = true,
        }
        self.check()
    }

    /// `tellg`.
    pub fn tellg(&self) -> i64 {
        if self.fail { -1 } else { self.pos as i64 }
    }
}

/// `WarningCallbackFunction`.
pub type WarningCallback = Rc<dyn Fn(&str)>;

/// `ImageInput::imageInfo`.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub spec: ImageSpec,
    pub format_type: ImageInputFormatType,
    pub filepos: usize,
}

/// Data members of the `ImageInput` base class.
pub struct ImageInputBase {
    pub isp: Option<IStream>,
    pub name: String,
    pub filename: String,
    /// `nativeBuffer16` as bytes (2 per element).
    pub native_buffer16: Vec<u8>,
    pub native_buffer8: Vec<u8>,
    pub images: Vec<ImageInfo>,
    pub cur_subimage: u32,
    pub cur_miplevel: u32,
    pub send_warning: Option<WarningCallback>,
    empty_spec: ImageSpec,
}

impl ImageInputBase {
    pub fn new(name: &str) -> ImageInputBase {
        ImageInputBase {
            isp: None,
            name: name.to_string(),
            filename: String::new(),
            native_buffer16: Vec::new(),
            native_buffer8: Vec::new(),
            images: Vec::new(),
            cur_subimage: u32::MAX,
            cur_miplevel: u32::MAX,
            send_warning: None,
            empty_spec: ImageSpec::default(),
        }
    }

    /// `images[i].spec` (a default spec when there is no such image).
    pub fn image_spec(&self, i: usize) -> &ImageSpec {
        self.images
            .get(i)
            .map_or(&self.empty_spec, |info| &info.spec)
    }

    /// `throwOnReadFailure`.
    pub fn throw_on_read_failure(&self) -> ImageError {
        if self.isp.as_ref().is_some_and(IStream::eof) {
            rt("Unexpected end-of-file.".to_string())
        } else {
            rt("I/O error reading file: Success".to_string())
        }
    }

    /// `warning`.
    pub fn warning(&self, wmsg: &str) {
        if let Some(cb) = &self.send_warning {
            cb(wmsg);
        }
    }

    /// `fwarning`: the message prefixed with the file name.
    pub fn fwarning(&self, wmsg: &str) {
        self.warning(&format!("{}: {wmsg}", self.filename));
    }

    pub fn stream(&mut self) -> ImageResult<&mut IStream> {
        self.isp
            .as_mut()
            .ok_or_else(|| rt("istream not initialized".to_string()))
    }
}

/// `ImageInput`: the reader interface. `open_stream` is the protected
/// `open(ImageSpec&)` called once `base().isp` is set.
pub trait ImageInput {
    fn base(&self) -> &ImageInputBase;
    fn base_mut(&mut self) -> &mut ImageInputBase;

    /// `open(ImageSpec& newspec)` on the connected stream.
    fn open_stream(&mut self, newspec: &mut ImageSpec) -> ImageResult<()>;

    /// `close`.
    fn close(&mut self) {
        self.base_mut().isp = None;
    }

    fn format_name(&self) -> &str {
        &self.base().name
    }
    fn filename(&self) -> &str {
        &self.base().filename
    }
    /// `spec()`.
    fn spec(&self) -> &ImageSpec {
        self.base().image_spec(0)
    }
    /// `formatType()`.
    fn format_type(&self) -> ImageInputFormatType {
        self.base()
            .images
            .first()
            .map_or(ImageInputFormatType::PngL, |i| i.format_type)
    }
    /// `spec(subimage, miplevel)`.
    fn spec_at(&self, _subimage: u32, _miplevel: u32) -> ImageSpec {
        let b = self.base();
        match b.images.get(b.cur_subimage as usize) {
            Some(i) => i.spec.clone(),
            None => ImageSpec::default(),
        }
    }
    fn current_subimage(&self) -> u32 {
        self.base().cur_subimage
    }
    fn current_miplevel(&self) -> u32 {
        self.base().cur_miplevel
    }
    fn subimage_count(&self) -> u32 {
        1
    }
    fn miplevel_count(&self) -> u32 {
        1
    }
    /// `seekSubimage`.
    fn seek_subimage(&mut self, subimage: u32, miplevel: u32) -> ImageResult<bool> {
        Ok(subimage == self.current_subimage() && miplevel == self.current_miplevel())
    }

    /// `readImage`: the whole image converted to `format` (unknown = the
    /// native format). `buffer.len()` is `bufferByteCount`.
    fn read_image(
        &mut self,
        buffer: &mut [u8],
        subimage: u32,
        miplevel: u32,
        format: &FormatDescriptor,
    ) -> ImageResult<()> {
        read_image_default(self, buffer, subimage, miplevel, format)
    }

    /// `readScanline`.
    #[allow(clippy::too_many_arguments)]
    fn read_scanline(
        &mut self,
        buffer: &mut [u8],
        y: u32,
        z: u32,
        subimage: u32,
        miplevel: u32,
        format: &FormatDescriptor,
    ) -> ImageResult<()> {
        read_scanline_default(self, buffer, y, z, subimage, miplevel, format)
    }

    /// `readNativeScanline`: `buffer_byte_count` is the C++ argument,
    /// `buffer` the whole destination the reader may touch.
    fn read_native_scanline(
        &mut self,
        buffer: &mut [u8],
        buffer_byte_count: usize,
        y: u32,
        z: u32,
        subimage: u32,
        miplevel: u32,
    ) -> ImageResult<()>;
}

/// `ImageInput::readImage`: reads scanline by scanline.
pub fn read_image_default<T: ImageInput + ?Sized>(
    input: &mut T,
    buffer: &mut [u8],
    subimage: u32,
    miplevel: u32,
    format: &FormatDescriptor,
) -> ImageResult<()> {
    let target = if format.is_unknown() {
        input.spec().format().clone()
    } else {
        format.clone()
    };
    let width = input.spec().width();
    let height = input.spec().height();
    let out_scanline_byte_count = target.pixel_byte_count().wrapping_mul(width) as usize;
    if buffer.len() < out_scanline_byte_count.wrapping_mul(height as usize) {
        return Err(ImageError::buffer_too_small());
    }
    let mut off = 0usize;
    for y in 0..height {
        let dst = buffer.get_mut(off..).unwrap_or_default();
        input.read_scanline(dst, y, 0, subimage, miplevel, &target)?;
        off += out_scanline_byte_count;
    }
    Ok(())
}

/// float -> int conversion of x86 `cvttss2si` (out of range: `INT_MIN`).
fn f32_to_i32_x86(f: f32) -> i32 {
    if f.is_nan() || f >= 2147483648.0 || f < -2147483648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// Element access for the `convert` / `rescale` / `rechannel` templates on
/// byte buffers holding `uint8_t` (size 1) or `uint16_t` (size 2) values.
fn get_val(buf: &[u8], size: usize, i: usize) -> u32 {
    if size == 1 {
        buf.get(i).map_or(0, |&v| u32::from(v))
    } else {
        match buf.get(2 * i..2 * i + 2) {
            Some(b) => u32::from(u16::from_le_bytes([b[0], b[1]])),
            None => 0,
        }
    }
}

fn set_val(buf: &mut [u8], size: usize, i: usize, v: u32) {
    if size == 1 {
        if let Some(x) = buf.get_mut(i) {
            *x = v as u8;
        }
    } else if let Some(b) = buf.get_mut(2 * i..2 * i + 2) {
        b.copy_from_slice(&(v as u16).to_le_bytes());
    }
}

/// `rescale(write, maxw, read, maxr, nvals)` (values already read).
fn rescale_vals(read: &[u32], maxw: u32, maxr: u32) -> Option<Vec<u32>> {
    if maxr == 0 {
        return None;
    }
    let multiplier = maxw as f32 / maxr as f32;
    Some(
        read.iter()
            .map(|&r| f32_to_i32_x86((r as f32 * multiplier).round()) as u32)
            .collect(),
    )
}

/// `luminance`.
fn luminance(rgb: &[u32]) -> u32 {
    let l = 0.2126f32 * rgb[0] as f32 + 0.7152f32 * rgb[1] as f32 + 0.0722f32 * rgb[2] as f32;
    f32_to_i32_x86(l.round()) as u32
}

/// `rechannel`: one pixel from `nc_in` to `nc_out` channels (`None` = no
/// write for that channel).
fn rechannel(read: &[u32], nc_in: u32, nc_out: u32) -> [Option<u32>; 4] {
    let r = |i: usize| read.get(i).copied().unwrap_or(0);
    const OPAQUE: u32 = 0xffff;
    match (nc_out, nc_in) {
        (4, 3) => [Some(r(0)), Some(r(1)), Some(r(2)), Some(OPAQUE)],
        (4, 2) => [Some(r(0)), Some(r(0)), Some(r(0)), Some(r(1))],
        (4, 1) => [Some(r(0)), Some(r(0)), Some(r(0)), Some(OPAQUE)],
        (3, 4) => [Some(r(0)), Some(r(1)), Some(r(2)), None],
        (3, 2) | (3, 1) => [Some(r(0)), Some(r(0)), Some(r(0)), None],
        (2, 4) => [Some(luminance(&[r(0), r(1), r(2)])), Some(r(3)), None, None],
        (2, 3) => [
            Some(luminance(&[r(0), r(1), r(2)])),
            Some(OPAQUE),
            None,
            None,
        ],
        (2, 1) => [Some(r(0)), Some(OPAQUE), None, None],
        (1, 4) | (1, 3) => [Some(luminance(&[r(0), r(1), r(2)])), None, None, None],
        (1, 2) => [Some(r(0)), None, None, None],
        _ => [None; 4],
    }
}

/// `convert<Tr, Tw>(write, maxw, read, maxr, npixels, ncIn, ncOut)` with
/// element sizes `w_size` / `r_size` (1 = `uint8_t`, 2 = `uint16_t`).
#[allow(clippy::too_many_arguments)]
fn convert(
    write: &mut [u8],
    w_size: usize,
    maxw: u32,
    read: &[u8],
    r_size: usize,
    maxr: u32,
    npixels: usize,
    nc_in: u32,
    nc_out: u32,
) {
    let mask = |size: usize, v: u32| if size == 1 { v & 0xff } else { v & 0xffff };
    if w_size != r_size && nc_in == nc_out {
        let n = npixels * nc_out as usize;
        let vals: Vec<u32> = (0..n).map(|i| get_val(read, r_size, i)).collect();
        if let Some(out) = rescale_vals(&vals, maxw, maxr) {
            for (i, v) in out.into_iter().enumerate() {
                set_val(write, w_size, i, v);
            }
        }
        return;
    }
    let (nci, nco) = (nc_in as usize, nc_out as usize);
    for p in 0..npixels {
        let src: Vec<u32> = (0..nci)
            .map(|c| get_val(read, r_size, p * nci + c))
            .collect();
        // `channels` is only used (and only filled) when the types differ.
        let mut channels = [0u32; 4];
        if mask(2, maxw) != mask(2, maxr)
            && let Some(out) = rescale_vals(&src, maxw, maxr)
        {
            for (c, v) in out.into_iter().enumerate().take(4) {
                channels[c] = mask(w_size, v);
            }
        }
        let pixel: Vec<u32> = if w_size == r_size {
            src
        } else {
            channels.to_vec()
        };
        for (c, v) in rechannel(&pixel, nc_in, nc_out).into_iter().enumerate() {
            if let Some(v) = v {
                set_val(write, w_size, p * nco + c, mask(w_size, v));
            }
        }
    }
}

/// `ImageInput::readScanline`: reads one native scanline and converts it to
/// `format` (unorm 8 <-> 16 bits, rescaling, 1-4 channels).
#[allow(clippy::too_many_arguments)]
pub fn read_scanline_default<T: ImageInput + ?Sized>(
    input: &mut T,
    buffer: &mut [u8],
    y: u32,
    z: u32,
    subimage: u32,
    miplevel: u32,
    format: &FormatDescriptor,
) -> ImageResult<()> {
    let target = if format.is_unknown() {
        input.spec().format().clone()
    } else {
        format.clone()
    };
    let target_bit_length = target.largest_channel_bit_length();
    let request_bits = bit_ceil(target_bit_length).max(8);
    if request_bits != 8 && request_bits != 16 {
        return Err(rt(format!(
            "Requested decode into {request_bits}-bit format is not supported."
        )));
    }
    let s0 = target.samples.first().copied().unwrap_or_default();
    if s0.qualifier_exponent || s0.qualifier_linear || s0.qualifier_signed || s0.qualifier_float {
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

    input.seek_subimage(subimage, miplevel)?;
    let spec = input.spec().clone();
    let sf = spec.format();
    if target != *sf
        && target.channel_bit_length().map_err(rt)? == sf.channel_bit_length().map_err(rt)?
        && target.channel_count() == sf.channel_count()
        && target.transfer == sf.transfer
    {
        return Err(rt("Requested unsupported format conversion.".to_string()));
    }

    let out_scanline_byte_count = target.pixel_byte_count().wrapping_mul(spec.width()) as usize;
    if buffer.len() < out_scanline_byte_count {
        return Err(ImageError::buffer_too_small());
    }

    let target_cbl = target.channel_bit_length().map_err(rt)?;
    let spec_cbl = sf.channel_bit_length().map_err(rt)?;
    let rescale_channels = target_cbl != spec_cbl;
    let add_remove_channels = target.channel_count() != sf.channel_count();
    let scanline_byte_count = spec.scanline_byte_count();

    enum Native {
        B16,
        B8,
        Out,
    }
    let native = if rescale_channels || add_remove_channels {
        if spec_cbl == 16 {
            Native::B16
        } else {
            Native::B8
        }
    } else {
        Native::Out
    };

    match native {
        Native::B16 => {
            let mut nb = std::mem::take(&mut input.base_mut().native_buffer16);
            if nb.len() < 2 * scanline_byte_count {
                nb.resize(2 * scanline_byte_count, 0);
            }
            let r =
                input.read_native_scanline(&mut nb, scanline_byte_count, y, z, subimage, miplevel);
            input.base_mut().native_buffer16 = nb;
            r?;
        }
        Native::B8 => {
            let mut nb = std::mem::take(&mut input.base_mut().native_buffer8);
            if nb.len() < scanline_byte_count {
                nb.resize(scanline_byte_count, 0);
            }
            let r =
                input.read_native_scanline(&mut nb, scanline_byte_count, y, z, subimage, miplevel);
            input.base_mut().native_buffer8 = nb;
            r?;
        }
        Native::Out => {
            let n = buffer.len();
            input.read_native_scanline(buffer, n, y, z, subimage, miplevel)?;
        }
    }

    let width = spec.width() as usize;
    let (cc_in, cc_out) = (sf.channel_count(), target.channel_count());
    match native {
        Native::B16 => {
            let maxw = target.channel_upper().map_err(rt)?;
            let maxr = sf.channel_upper().map_err(rt)? & 0xffff;
            let nb = &input.base().native_buffer16;
            if target_cbl == 8 {
                convert(buffer, 1, maxw & 0xff, nb, 2, maxr, width, cc_in, cc_out);
            } else {
                convert(buffer, 2, maxw & 0xffff, nb, 2, maxr, width, cc_in, cc_out);
            }
        }
        Native::B8 => {
            let maxw = target.channel_upper().map_err(rt)?;
            let maxr = sf.channel_upper().map_err(rt)? & 0xff;
            let nb = &input.base().native_buffer8;
            if target_cbl == 16 {
                convert(buffer, 2, maxw & 0xffff, nb, 1, maxr, width, cc_in, cc_out);
            } else {
                convert(buffer, 1, maxw & 0xff, nb, 1, maxr, width, cc_in, cc_out);
            }
        }
        Native::Out => {
            let target_upper = target.channel_upper().map_err(rt)?;
            let spec_upper = sf.channel_upper().map_err(rt)?;
            if target_upper != spec_upper {
                let size = if spec_cbl == 16 { 2 } else { 1 };
                let mask = if size == 2 { 0xffff } else { 0xff };
                let n = spec.scanline_channel_count();
                let vals: Vec<u32> = (0..n).map(|i| get_val(buffer, size, i)).collect();
                if let Some(out) = rescale_vals(&vals, target_upper & mask, spec_upper & mask) {
                    for (i, v) in out.into_iter().enumerate() {
                        set_val(buffer, size, i, v);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Reader plugins (`Imageio::inputFormats` creators).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plugin {
    Exr,
    Jpeg,
    Png,
}

impl Plugin {
    fn create(self) -> Box<dyn ImageInput> {
        match self {
            Plugin::Exr => Box::new(ExrInput::new()),
            Plugin::Jpeg => Box::new(JpegInput::new()),
            Plugin::Png => Box::new(PngInput::new()),
        }
    }
}

/// `Imageio::catalogBuiltinPlugins`: extension / format name -> plugin,
/// iterated in `std::map` key order.
fn input_formats() -> BTreeMap<String, Plugin> {
    let mut m = BTreeMap::new();
    let mut declare = |name: &str, plugin: Plugin, extensions: &[&str]| {
        for e in extensions {
            m.entry(e.to_ascii_lowercase()).or_insert(plugin);
        }
        m.entry(name.to_string()).or_insert(plugin);
    };
    declare("exr", Plugin::Exr, exr::EXR_INPUT_EXTENSIONS);
    declare("jpeg", Plugin::Jpeg, jpg::JPEG_INPUT_EXTENSIONS);
    declare("png", Plugin::Png, png::PNG_INPUT_EXTENSIONS);
    m
}

/// Connects `input` to `data` and opens it (private `ImageInput::open`).
fn open_with(
    input: &mut dyn ImageInput,
    filename: &str,
    data: &Rc<Vec<u8>>,
    newspec: &mut ImageSpec,
) -> ImageResult<()> {
    input.base_mut().filename = filename.to_string();
    input.base_mut().isp = Some(IStream::new(Rc::clone(data)));
    input.open_stream(newspec)
}

/// `ImageInput::open(filename, config, wcb)`: opens `filename` (`-` is
/// stdin) with the reader for its extension, then every other reader.
pub fn open(filename: &str, wcb: Option<WarningCallback>) -> ImageResult<Box<dyn ImageInput>> {
    let formats = input_formats();
    let mut create_function = None;
    let data;
    let fname;
    if filename != "-" {
        data = match std::fs::read(filename) {
            Ok(d) => d,
            Err(e) => {
                return Err(rt(format!(
                    "Open of \"{filename}\" failed. {}",
                    io_error_message(&e)
                )));
            }
        };
        let format = match filename.rfind('.') {
            Some(p) => &filename[p + 1..],
            None => filename,
        }
        .to_ascii_lowercase();
        if !format.is_empty() {
            create_function = formats.get(&format).copied();
        }
        fname = filename;
    } else {
        let mut v = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut v);
        data = v;
        fname = "stdin";
    }
    let data = Rc::new(data);

    let mut formats_tried = Vec::new();
    let mut specific_error = String::new();
    if let Some(plugin) = create_function {
        formats_tried.push(plugin);
        let mut input = plugin.create();
        input.base_mut().send_warning = wcb.clone();
        let mut tmpspec = ImageSpec::default();
        match open_with(input.as_mut(), fname, &data, &mut tmpspec) {
            Ok(()) => return Ok(input),
            Err(e) => specific_error = e.what().to_string(),
        }
    }
    for &plugin in formats.values() {
        if formats_tried.contains(&plugin) {
            continue;
        }
        formats_tried.push(plugin);
        let mut input = plugin.create();
        input.base_mut().send_warning = wcb.clone();
        let mut tmpspec = ImageSpec::default();
        if open_with(input.as_mut(), fname, &data, &mut tmpspec).is_ok() {
            return Ok(input);
        }
    }
    if !specific_error.is_empty() {
        Err(rt(specific_error))
    } else {
        Err(rt(format!(
            "No image plugin recognized the format of \"{}\". Is it a file format that we don't know about?\n",
            if filename != "-" {
                filename
            } else {
                "the data on stdin"
            }
        )))
    }
}

/// Default sample used where the C++ reads `samples[0]` of a format without
/// samples.
pub(crate) fn first_sample(f: &FormatDescriptor) -> Sample {
    f.samples.first().copied().unwrap_or_default()
}
