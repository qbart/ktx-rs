// SPDX-FileCopyrightText: 2014-2021 Syoyo Fujita and many contributors
// SPDX-FileCopyrightText: 2002 Industrial Light & Magic, a division of Lucas Digital Ltd. LLC
// SPDX-License-Identifier: BSD-3-Clause
// Translated to Rust from tinyexr and modified; see NOTICE.md.
//! tinyexr reader: `ParseEXRVersionFromMemory`, `ParseEXRHeaderFromMemory`,
//! `LoadEXRImageFromMemory` and the init / free helpers.
//!
//! Chunk data is addressed inside the whole file buffer like the C pointers,
//! so a read past a chunk sees the following file bytes; reads past the end
//! of the file give 0 and writes outside an image are dropped (both undefined
//! behavior in C). Allocations that fail (or exceed `MAX_ALLOC`) are reported
//! like the C allocation failure checks instead of aborting.

use super::piz::decompress_piz;
use super::*;

/// Largest single buffer the decoder allocates (C would `malloc` / throw).
const MAX_ALLOC: u64 = 1 << 32;

/// `TINYEXR_DIMENSION_THRESHOLD`.
const TINYEXR_DIMENSION_THRESHOLD: i64 = 1024 * 8192;

pub(super) fn rd_u8(mem: &[u8], pos: u64) -> u8 {
    usize::try_from(pos)
        .ok()
        .and_then(|p| mem.get(p))
        .copied()
        .unwrap_or(0)
}

pub(super) fn rd_u16(mem: &[u8], pos: u64) -> u16 {
    u16::from_le_bytes([rd_u8(mem, pos), rd_u8(mem, pos.wrapping_add(1))])
}

pub(super) fn rd_u32(mem: &[u8], pos: u64) -> u32 {
    let mut b = [0u8; 4];
    for (i, x) in b.iter_mut().enumerate() {
        *x = rd_u8(mem, pos.wrapping_add(i as u64));
    }
    u32::from_le_bytes(b)
}

fn rd_u64(mem: &[u8], pos: u64) -> u64 {
    u64::from(rd_u32(mem, pos)) | (u64::from(rd_u32(mem, pos.wrapping_add(4))) << 32)
}

/// Zero filled buffer, `None` when it cannot be allocated.
fn try_zeroed<T: Clone + Default>(n: usize) -> Option<Vec<T>> {
    if n.checked_mul(size_of::<T>())? as u64 > MAX_ALLOC {
        return None;
    }
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, T::default());
    Some(v)
}

/// `EXRVersion`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExrVersion {
    /// This must be 2.
    pub version: i32,
    /// Tile format image (single part).
    pub tiled: i32,
    /// Long name attribute.
    pub long_name: i32,
    /// Deep image (EXR 2.0).
    pub non_image: i32,
    /// Multi-part (EXR 2.0).
    pub multipart: i32,
}

/// `EXRBox2i`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExrBox2i {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// `EXRChannelInfo`. `name` holds the bytes of the C string (at most 255).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExrChannelInfo {
    pub name: Vec<u8>,
    pub pixel_type: i32,
    pub x_sampling: i32,
    pub y_sampling: i32,
    pub p_linear: u8,
}

/// `EXRHeader`. `num_channels` / `num_custom_attributes` are the lengths of
/// `channels` / `custom_attributes`; `pixel_types` and
/// `requested_pixel_types` have one entry per channel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExrHeader {
    pub pixel_aspect_ratio: f32,
    pub line_order: i32,
    pub data_window: ExrBox2i,
    pub display_window: ExrBox2i,
    pub screen_window_center: [f32; 2],
    pub screen_window_width: f32,
    pub chunk_count: i32,
    pub tiled: i32,
    pub tile_size_x: i32,
    pub tile_size_y: i32,
    pub tile_level_mode: i32,
    pub tile_rounding_mode: i32,
    pub long_name: i32,
    pub non_image: i32,
    pub multipart: i32,
    pub header_len: u32,
    /// Custom attributes (all but the required ones).
    pub custom_attributes: Vec<ExrAttribute>,
    pub channels: Vec<ExrChannelInfo>,
    /// Loaded pixel type of each channel of the image data, overwritten with
    /// `requested_pixel_types` when loading.
    pub pixel_types: Vec<i32>,
    pub compression_type: i32,
    /// Filled by `parse_exr_header_from_memory`; may be changed to FLOAT for
    /// HALF channels before loading.
    pub requested_pixel_types: Vec<i32>,
    /// `name` attribute (at most 255 bytes).
    pub name: Vec<u8>,
}

impl ExrHeader {
    pub fn num_channels(&self) -> i32 {
        self.channels.len() as i32
    }

    pub fn num_custom_attributes(&self) -> i32 {
        self.custom_attributes.len() as i32
    }
}

/// `EXRTile`. `images[channel]` holds the tile pixels (`tile_size_x` *
/// `tile_size_y` values, 2 bytes for HALF else 4, little endian).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExrTile {
    pub offset_x: i32,
    pub offset_y: i32,
    pub level_x: i32,
    pub level_y: i32,
    /// Actual width in a tile.
    pub width: i32,
    /// Actual height in a tile.
    pub height: i32,
    pub images: Option<Vec<Vec<u8>>>,
}

/// `EXRImage`. `images[channel]` holds width * height values (2 bytes for
/// HALF else 4, little endian) of a scanline image; `None` if tiled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExrImage {
    /// Tiled pixel data, `None` if scanline format.
    pub tiles: Option<Vec<ExrTile>>,
    /// `None` if scanline format or the image is the last level.
    pub next_level: Option<Box<ExrImage>>,
    pub level_x: i32,
    pub level_y: i32,
    pub images: Option<Vec<Vec<u8>>>,
    pub width: i32,
    pub height: i32,
    pub num_channels: i32,
    pub num_tiles: i32,
}

/// `SetErrorMessage`.
fn set_error_message(msg: &str, err: &mut Option<String>) {
    *err = Some(msg.to_string());
}

/// `InitEXRImage`.
pub fn init_exr_image(exr_image: &mut ExrImage) {
    exr_image.width = 0;
    exr_image.height = 0;
    exr_image.num_channels = 0;
    exr_image.images = None;
    exr_image.tiles = None;
    exr_image.next_level = None;
    exr_image.level_x = 0;
    exr_image.level_y = 0;
    exr_image.num_tiles = 0;
}

/// `FreeEXRErrorMessage`.
pub fn free_exr_error_message(msg: &mut Option<String>) {
    *msg = None;
}

/// `InitEXRHeader`.
pub fn init_exr_header(exr_header: &mut ExrHeader) {
    *exr_header = ExrHeader::default();
}

/// `FreeEXRHeader`.
pub fn free_exr_header(exr_header: &mut ExrHeader) -> i32 {
    exr_header.channels = Vec::new();
    exr_header.pixel_types = Vec::new();
    exr_header.requested_pixel_types = Vec::new();
    exr_header.custom_attributes = Vec::new();
    exr_set_name_attr(exr_header, None);
    TINYEXR_SUCCESS
}

/// `EXRSetNameAttr`.
pub fn exr_set_name_attr(exr_header: &mut ExrHeader, name: Option<&[u8]>) {
    let name = name.unwrap_or_default();
    let len = name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name.len())
        .min(255);
    exr_header.name = name[..len].to_vec();
}

/// `EXRNumLevels`.
pub fn exr_num_levels(exr_image: &ExrImage) -> i32 {
    if exr_image.images.is_some() {
        return 1; // scanlines
    }
    let mut levels = 1;
    let mut level_image = exr_image;
    while let Some(next) = &level_image.next_level {
        level_image = next;
        levels += 1;
    }
    levels
}

/// `FreeEXRImage`: releases the pixel data, tiles and levels.
pub fn free_exr_image(exr_image: &mut ExrImage) -> i32 {
    exr_image.next_level = None;
    exr_image.images = None;
    exr_image.tiles = None;
    TINYEXR_SUCCESS
}

/// `ParseEXRVersionFromMemory`.
pub fn parse_exr_version_from_memory(version: &mut ExrVersion, memory: &[u8]) -> i32 {
    if memory.len() < K_EXR_VERSION_SIZE {
        return TINYEXR_ERROR_INVALID_DATA;
    }

    // Header check.
    if memory[..4] != [0x76, 0x2f, 0x31, 0x01] {
        return TINYEXR_ERROR_INVALID_MAGIC_NUMBER;
    }

    version.tiled = 0;
    version.long_name = 0;
    version.non_image = 0;
    version.multipart = 0;

    // must be 2
    if memory[4] != 2 {
        return TINYEXR_ERROR_INVALID_EXR_VERSION;
    }

    version.version = 2;

    let flags = memory[5];
    if flags & 0x2 != 0 {
        version.tiled = 1; // 9th bit
    }
    if flags & 0x4 != 0 {
        version.long_name = 1; // 10th bit
    }
    if flags & 0x8 != 0 {
        version.non_image = 1; // 11th bit (deep image)
    }
    if flags & 0x10 != 0 {
        version.multipart = 1; // 12th bit
    }

    TINYEXR_SUCCESS
}

/// Length of the C string at the start of `s` (`strnlen`).
fn strnlen(s: &[u8]) -> usize {
    s.iter().position(|&b| b == 0).unwrap_or(s.len())
}

fn le_i32(data: &[u8], at: usize) -> i32 {
    rd_u32(data, at as u64) as i32
}

/// Name, type, data and byte size of a header attribute.
type Attribute = (Vec<u8>, Vec<u8>, Vec<u8>, usize);

/// `ReadAttribute`.
fn read_attribute(marker: &[u8]) -> Option<Attribute> {
    let mut size = marker.len();
    let name_len = strnlen(marker);
    if name_len == size {
        // String does not have a terminating character.
        return None;
    }
    let name = marker[..name_len].to_vec();
    let mut pos = name_len + 1;
    size -= name_len + 1;

    let type_len = strnlen(&marker[pos..]);
    if type_len == size {
        return None;
    }
    let attr_type = marker[pos..pos + type_len].to_vec();
    pos += type_len + 1;
    size -= type_len + 1;

    if size < 4 {
        return None;
    }

    let data_len = rd_u32(marker, pos as u64) as usize;

    if data_len == 0 {
        if attr_type == b"string" {
            // Accept empty string attribute.
            let marker_size = name_len + 1 + type_len + 1 + 4;
            return Some((name, attr_type, vec![0], marker_size));
        }
        return None;
    }

    pos += 4;
    size -= 4;

    if size < data_len {
        return None;
    }

    let data = marker[pos..pos + data_len].to_vec();
    let marker_size = name_len + 1 + type_len + 1 + 4 + data_len;
    Some((name, attr_type, data, marker_size))
}

/// `ReadChannelInfo`: appends the channels of a `chlist` attribute.
fn read_channel_info(channels: &mut Vec<ExrChannelInfo>, data: &[u8]) -> bool {
    let mut p = 0usize;
    loop {
        if data.get(p).copied().unwrap_or(0) == 0 {
            break;
        }

        // ReadString
        let rest = &data[p..];
        let len = strnlen(rest);
        if len >= rest.len() {
            // Buffer overrun. Issue #51.
            return false;
        }
        let name = rest[..len].to_vec();
        p += len + 1;

        if p + 16 >= data.len() {
            return false;
        }

        channels.push(ExrChannelInfo {
            name,
            pixel_type: le_i32(data, p),
            p_linear: data[p + 4],
            x_sampling: le_i32(data, p + 8),
            y_sampling: le_i32(data, p + 12),
        });
        p += 16;
    }
    true
}

/// `HeaderInfo`.
#[derive(Default)]
struct HeaderInfo {
    channels: Vec<ExrChannelInfo>,
    attributes: Vec<ExrAttribute>,
    data_window: ExrBox2i,
    line_order: i32,
    display_window: ExrBox2i,
    screen_window_center: [f32; 2],
    screen_window_width: f32,
    pixel_aspect_ratio: f32,
    chunk_count: i32,
    tiled: i32,
    tile_size_x: i32,
    tile_size_y: i32,
    tile_level_mode: i32,
    tile_rounding_mode: i32,
    header_len: u32,
    compression_type: i32,
    name: Vec<u8>,
    type_: Vec<u8>,
}

fn le_f32(data: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_u32(data, at as u64))
}

fn box2i(data: &[u8]) -> ExrBox2i {
    ExrBox2i {
        min_x: le_i32(data, 0),
        min_y: le_i32(data, 4),
        max_x: le_i32(data, 8),
        max_y: le_i32(data, 12),
    }
}

/// `ParseEXRHeader` (single part header at the start of `buf`).
fn parse_exr_header(
    info: &mut HeaderInfo,
    empty_header: Option<&mut bool>,
    version: &ExrVersion,
    err: &mut String,
    buf: &[u8],
) -> i32 {
    let mut empty = false;
    if version.multipart != 0 && buf.first() == Some(&0) {
        empty = true;
    }
    if let Some(e) = empty_header {
        *e = empty;
    }
    if empty {
        return TINYEXR_SUCCESS;
    }

    let mut has_channels = false;
    let mut has_compression = false;
    let mut has_data_window = false;
    let mut has_display_window = false;
    let mut has_line_order = false;
    let mut has_pixel_aspect_ratio = false;
    let mut has_screen_window_center = false;
    let mut has_screen_window_width = false;
    let mut has_name = false;
    let mut has_type = false;

    info.name.clear();
    info.type_.clear();

    info.data_window = ExrBox2i::default();
    info.line_order = 0; // @fixme
    info.display_window = ExrBox2i::default();
    info.screen_window_center = [0.0, 0.0];
    info.screen_window_width = -1.0;
    info.pixel_aspect_ratio = -1.0;

    info.tiled = 0;
    info.tile_size_x = -1;
    info.tile_size_y = -1;
    info.tile_level_mode = -1;
    info.tile_rounding_mode = -1;

    info.attributes.clear();

    let orig_size = buf.len();
    let mut pos = 0usize;
    for _ in 0..TINYEXR_MAX_HEADER_ATTRIBUTES {
        let size = buf.len() - pos;
        if size == 0 {
            err.push_str("Insufficient data size for attributes.\n");
            return TINYEXR_ERROR_INVALID_DATA;
        } else if buf[pos] == 0 {
            pos += 1;
            break;
        }

        let Some((attr_name, attr_type, data, marker_size)) = read_attribute(&buf[pos..]) else {
            err.push_str("Failed to read attribute.\n");
            return TINYEXR_ERROR_INVALID_DATA;
        };
        pos += marker_size;

        let name = attr_name.as_slice();
        if (version.tiled != 0 || version.multipart != 0 || version.non_image != 0)
            && name == b"tiles"
        {
            if data.len() != 9 {
                err.push_str(
                    "(ParseEXRHeader) Invalid attribute data size. Attribute data size must be 9.\n",
                );
                return TINYEXR_ERROR_INVALID_DATA;
            }
            let x_size = rd_u32(&data, 0);
            let y_size = rd_u32(&data, 4);
            let tile_mode = data[8];

            if x_size > i32::MAX as u32 || y_size > i32::MAX as u32 {
                *err = "Tile sizes were invalid.".to_string();
                return TINYEXR_ERROR_UNSUPPORTED_FORMAT;
            }

            info.tile_size_x = x_size as i32;
            info.tile_size_y = y_size as i32;
            info.tile_level_mode = i32::from(tile_mode & 0x3);
            info.tile_rounding_mode = i32::from((tile_mode >> 4) & 0x1);
            info.tiled = 1;
        } else if name == b"compression" {
            let c = i32::from(data[0]);
            if c == TINYEXR_COMPRESSIONTYPE_ZFP {
                *err = "ZFP compression is not supported.".to_string();
                return TINYEXR_ERROR_UNSUPPORTED_FORMAT;
            }
            if c > TINYEXR_COMPRESSIONTYPE_PIZ {
                *err = "Unknown compression type.".to_string();
                return TINYEXR_ERROR_UNSUPPORTED_FORMAT;
            }
            info.compression_type = c;
            has_compression = true;
        } else if name == b"channels" {
            if !read_channel_info(&mut info.channels, &data) {
                err.push_str("Failed to parse channel info.\n");
                return TINYEXR_ERROR_INVALID_DATA;
            }
            if info.channels.is_empty() {
                err.push_str("# of channels is zero.\n");
                return TINYEXR_ERROR_INVALID_DATA;
            }
            has_channels = true;
        } else if name == b"dataWindow" {
            if data.len() >= 16 {
                info.data_window = box2i(&data);
                has_data_window = true;
            }
        } else if name == b"displayWindow" {
            if data.len() >= 16 {
                info.display_window = box2i(&data);
                has_display_window = true;
            }
        } else if name == b"lineOrder" {
            info.line_order = i32::from(data[0]);
            has_line_order = true;
        } else if name == b"pixelAspectRatio" {
            if data.len() >= 4 {
                info.pixel_aspect_ratio = le_f32(&data, 0);
                has_pixel_aspect_ratio = true;
            }
        } else if name == b"screenWindowCenter" {
            if data.len() >= 8 {
                info.screen_window_center = [le_f32(&data, 0), le_f32(&data, 4)];
                has_screen_window_center = true;
            }
        } else if name == b"screenWindowWidth" {
            if data.len() >= 4 {
                info.screen_window_width = le_f32(&data, 0);
                has_screen_window_width = true;
            }
        } else if name == b"chunkCount" {
            if data.len() >= 4 {
                info.chunk_count = le_i32(&data, 0);
            }
        } else if name == b"name" {
            if data[0] != 0 {
                info.name = data[..strnlen(&data)].to_vec();
                has_name = true;
            }
        } else if name == b"type" {
            if data[0] != 0 {
                info.type_ = data[..strnlen(&data)].to_vec();
                has_type = true;
            }
        } else if info.attributes.len() < TINYEXR_MAX_CUSTOM_ATTRIBUTES {
            info.attributes.push(ExrAttribute {
                name: attr_name[..attr_name.len().min(255)].to_vec(),
                attr_type: attr_type[..attr_type.len().min(255)].to_vec(),
                value: data,
            });
        }
    }

    let mut ss_err = String::new();
    if !has_compression {
        ss_err.push_str("\"compression\" attribute not found in the header.\n");
    }
    if !has_channels {
        ss_err.push_str("\"channels\" attribute not found in the header.\n");
    }
    if !has_line_order {
        ss_err.push_str("\"lineOrder\" attribute not found in the header.\n");
    }
    if !has_display_window {
        ss_err.push_str("\"displayWindow\" attribute not found in the header.\n");
    }
    if !has_data_window {
        ss_err.push_str("\"dataWindow\" attribute not found in the header or invalid.\n");
    }
    if !has_pixel_aspect_ratio {
        ss_err.push_str("\"pixelAspectRatio\" attribute not found in the header.\n");
    }
    if !has_screen_window_width {
        ss_err.push_str("\"screenWindowWidth\" attribute not found in the header.\n");
    }
    if !has_screen_window_center {
        ss_err.push_str("\"screenWindowCenter\" attribute not found in the header.\n");
    }
    if version.multipart != 0 || version.non_image != 0 {
        if !has_name {
            ss_err.push_str("\"name\" attribute not found in the header.\n");
        }
        if !has_type {
            ss_err.push_str("\"type\" attribute not found in the header.\n");
        }
    }
    if !ss_err.is_empty() {
        err.push_str(&ss_err);
        return TINYEXR_ERROR_INVALID_HEADER;
    }

    info.header_len = (orig_size - (buf.len() - pos)) as u32;
    TINYEXR_SUCCESS
}

/// `ConvertHeader`: moves the parsed header into `exr_header`.
fn convert_header(exr_header: &mut ExrHeader, info: &mut HeaderInfo, err: &mut String) -> bool {
    exr_header.pixel_aspect_ratio = info.pixel_aspect_ratio;
    exr_header.screen_window_center = info.screen_window_center;
    exr_header.screen_window_width = info.screen_window_width;
    exr_header.chunk_count = info.chunk_count;
    exr_header.display_window = info.display_window;
    exr_header.data_window = info.data_window;
    exr_header.line_order = info.line_order;
    exr_header.compression_type = info.compression_type;
    exr_header.tiled = info.tiled;
    exr_header.tile_size_x = info.tile_size_x;
    exr_header.tile_size_y = info.tile_size_y;
    exr_header.tile_level_mode = info.tile_level_mode;
    exr_header.tile_rounding_mode = info.tile_rounding_mode;

    exr_set_name_attr(exr_header, Some(&info.name));

    if !info.type_.is_empty() {
        let mut valid = true;
        match info.type_.as_slice() {
            b"scanlineimage" => {
                if exr_header.tiled != 0 {
                    err.push_str(
                        "(ConvertHeader) tiled bit must be off for `scanlineimage` type.\n",
                    );
                    valid = false;
                }
            }
            b"tiledimage" => {
                if exr_header.tiled == 0 {
                    err.push_str("(ConvertHeader) tiled bit must be on for `tiledimage` type.\n");
                    valid = false;
                }
            }
            b"deeptile" => {
                exr_header.non_image = 1;
                if exr_header.tiled == 0 {
                    err.push_str("(ConvertHeader) tiled bit must be on for `deeptile` type.\n");
                    valid = false;
                }
            }
            b"deepscanline" => {
                exr_header.non_image = 1;
                if exr_header.tiled != 0 {
                    err.push_str(
                        "(ConvertHeader) tiled bit must be off for `deepscanline` type.\n",
                    );
                }
            }
            _ => {
                // Warning only: "(ConvertHeader) Unsupported or unknown info.type"
            }
        }
        if !valid {
            return false;
        }
    }

    let channels = std::mem::take(&mut info.channels);
    exr_header.pixel_types = channels.iter().map(|c| c.pixel_type).collect();
    exr_header.requested_pixel_types = exr_header.pixel_types.clone();
    exr_header.channels = channels
        .into_iter()
        .map(|mut c| {
            c.name.truncate(255);
            c
        })
        .collect();

    exr_header.custom_attributes = std::mem::take(&mut info.attributes);
    exr_header
        .custom_attributes
        .truncate(TINYEXR_MAX_CUSTOM_ATTRIBUTES);

    exr_header.header_len = info.header_len;
    true
}

/// `ParseEXRHeaderFromMemory`.
pub fn parse_exr_header_from_memory(
    exr_header: &mut ExrHeader,
    version: &ExrVersion,
    memory: &[u8],
    err: &mut Option<String>,
) -> i32 {
    if memory.len() < K_EXR_VERSION_SIZE {
        set_error_message("Insufficient header/data size.\n", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }

    let mut info = HeaderInfo::default();

    let mut ret;
    {
        let mut err_str = String::new();
        ret = parse_exr_header(
            &mut info,
            None,
            version,
            &mut err_str,
            &memory[K_EXR_VERSION_SIZE..],
        );
        if ret != TINYEXR_SUCCESS && !err_str.is_empty() {
            set_error_message(&err_str, err);
        }
    }

    {
        let mut err_str = String::new();
        if !convert_header(exr_header, &mut info, &mut err_str) {
            if !err_str.is_empty() {
                set_error_message(&err_str, err);
            }
            ret = TINYEXR_ERROR_INVALID_HEADER;
        }
    }

    exr_header.multipart = i32::from(version.multipart != 0);
    exr_header.non_image = i32::from(version.non_image != 0);

    ret
}

/// Applies the ZIP / RLE predictor and interleaves the two halves into `dst`
/// (OpenEXR `ImfZipCompressor.cpp` / `ImfRleCompressor.cpp`).
fn predict_and_reorder(tmp: &mut [u8], dst: &mut [u8]) {
    // Predictor.
    for t in 1..tmp.len() {
        let d = i32::from(tmp[t - 1]) + i32::from(tmp[t]) - 128;
        tmp[t] = d as u8;
    }

    // Reorder the pixel data.
    let n = tmp.len();
    let mut t1 = 0usize;
    let mut t2 = n.div_ceil(2);
    let mut s = 0usize;
    loop {
        if s < n {
            dst[s] = tmp[t1];
            s += 1;
            t1 += 1;
        } else {
            break;
        }
        if s < n {
            dst[s] = tmp[t2];
            s += 1;
            t2 += 1;
        } else {
            break;
        }
    }
}

/// Copies `n` file bytes at `pos` (an uncompressed chunk, Issue 40).
fn copy_from(dst: &mut [u8], mem: &[u8], pos: u64) {
    for (i, d) in dst.iter_mut().enumerate() {
        *d = rd_u8(mem, pos.wrapping_add(i as u64));
    }
}

/// `DecompressZip` with basisu miniz `mz_uncompress`.
fn decompress_zip(
    dst: &mut [u8],
    uncompressed_size: &mut usize,
    mem: &[u8],
    src_pos: u64,
    src_size: usize,
) -> bool {
    if *uncompressed_size == src_size {
        // Data is not compressed (Issue 40).
        copy_from(&mut dst[..src_size], mem, src_pos);
        return true;
    }
    let Some(mut tmp_buf) = try_zeroed::<u8>(*uncompressed_size) else {
        return false;
    };

    // mz_uncompress
    if (src_size as u64 | *uncompressed_size as u64) > 0xFFFF_FFFF {
        return false;
    }
    let src = usize::try_from(src_pos)
        .ok()
        .and_then(|p| mem.get(p..p.checked_add(src_size)?));
    let Some(src) = src else {
        return false;
    };
    use miniz_oxide::inflate::TINFLStatus;
    use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
    let mut state = DecompressorOxide::new();
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    let (status, _, out_written) = decompress(&mut state, src, &mut tmp_buf, 0, flags);
    if status != TINFLStatus::Done {
        return false;
    }
    *uncompressed_size = out_written;

    predict_and_reorder(&mut tmp_buf[..out_written], dst);
    true
}

/// `rleUncompress`: returns the number of bytes written to `out` or 0.
fn rle_uncompress(
    in_length: i32,
    max_length: i32,
    mem: &[u8],
    src_pos: u64,
    out: &mut [u8],
) -> i32 {
    let mut in_length = in_length;
    let mut max_length = max_length;
    let mut inp = src_pos;
    let mut o = 0usize;

    while in_length > 0 {
        let b = rd_u8(mem, inp) as i8;
        inp = inp.wrapping_add(1);
        if b < 0 {
            let count = -i32::from(b);
            in_length = in_length.wrapping_sub(count + 1);

            // Fixes #116: Add bounds check to in buffer.
            max_length = max_length.wrapping_sub(count);
            if 0 > max_length || in_length < 0 {
                return 0;
            }

            let count = count as usize;
            let Some(dst) = out.get_mut(o..o + count) else {
                return 0;
            };
            copy_from(dst, mem, inp);
            o += count;
            inp = inp.wrapping_add(count as u64);
        } else {
            let count = i32::from(b);
            in_length = in_length.wrapping_sub(2);

            max_length = max_length.wrapping_sub(count + 1);
            if 0 > max_length || in_length < 0 {
                return 0;
            }

            let count = count as usize + 1;
            let Some(dst) = out.get_mut(o..o + count) else {
                return 0;
            };
            dst.fill(rd_u8(mem, inp));
            o += count;
            inp = inp.wrapping_add(1);
        }
    }
    o as i32
}

/// `DecompressRle`.
fn decompress_rle(
    dst: &mut [u8],
    uncompressed_size: usize,
    mem: &[u8],
    src_pos: u64,
    src_size: usize,
) -> bool {
    if uncompressed_size == src_size {
        // Data is not compressed (Issue 40).
        copy_from(&mut dst[..src_size], mem, src_pos);
        return true;
    }

    // Workaround for issue #112.
    if src_size <= 2 {
        return false;
    }

    let Some(mut tmp_buf) = try_zeroed::<u8>(uncompressed_size) else {
        return false;
    };

    let ret = rle_uncompress(
        src_size as i32,
        uncompressed_size as i32,
        mem,
        src_pos,
        &mut tmp_buf,
    );
    if ret != uncompressed_size as i32 {
        return false;
    }

    predict_and_reorder(&mut tmp_buf, dst);
    true
}

/// Element write into a channel image (dropped outside the image).
fn put(image: &mut [u8], index: i64, bytes: &[u8]) {
    let n = bytes.len() as i64;
    if index < 0 {
        return;
    }
    let Some(start) = index.checked_mul(n).and_then(|s| usize::try_from(s).ok()) else {
        return;
    };
    if let Some(dst) = image.get_mut(start..start.saturating_add(bytes.len())) {
        dst.copy_from_slice(bytes);
    }
}

/// Pixel geometry of one `DecodePixelData` call.
struct PixelRegion {
    line_order: i32,
    width: i32,
    height: i32,
    x_stride: i32,
    y: i32,
    line_no: i32,
    num_lines: i32,
    pixel_data_size: usize,
}

impl PixelRegion {
    /// Index of the first value of output line `v` (C `size_t` math wraps
    /// like this `i64`).
    fn line_start(&self, first: i32, v: i64) -> i64 {
        let row = i64::from(first) + v;
        let row = if self.line_order == 0 {
            row
        } else {
            i64::from(self.height) - 1 - row
        };
        row.wrapping_mul(i64::from(self.x_stride))
    }
}

/// Scatters the decompressed `out_buf` of a PIZ / ZIP / RLE block into the
/// channel images.
fn scatter_decompressed(
    out_images: &mut [Vec<u8>],
    requested_pixel_types: &[i32],
    out_buf: &[u8],
    r: &PixelRegion,
    channels: &[ExrChannelInfo],
    channel_offset_list: &[usize],
) -> bool {
    let width = i64::from(r.width);
    for (c, chan) in channels.iter().enumerate() {
        let requested = requested_pixel_types.get(c).copied().unwrap_or(-1);
        let off = channel_offset_list[c] as i64;
        let elem = match chan.pixel_type {
            TINYEXR_PIXELTYPE_HALF => 2,
            TINYEXR_PIXELTYPE_UINT if requested == TINYEXR_PIXELTYPE_UINT => 4,
            TINYEXR_PIXELTYPE_FLOAT if requested == TINYEXR_PIXELTYPE_FLOAT => 4,
            _ => return false,
        };
        let Some(image) = out_images.get_mut(c) else {
            return false;
        };
        for v in 0..i64::from(r.num_lines) {
            let line = v * r.pixel_data_size as i64 * width + off * width;
            let start = r.line_start(r.line_no, v);
            for u in 0..width {
                let src = (line + u * elem) as u64;
                let index = start.wrapping_add(u);
                if elem == 2 {
                    let hf = rd_u16(out_buf, src);
                    if requested == TINYEXR_PIXELTYPE_HALF {
                        put(image, index, &hf.to_le_bytes());
                    } else {
                        // HALF -> FLOAT
                        put(image, index, &half_to_float(hf).to_le_bytes());
                    }
                } else {
                    put(image, index, &rd_u32(out_buf, src).to_le_bytes());
                }
            }
        }
    }
    true
}

/// `DecodePixelData`: decodes one block of `data_len` bytes at `data_pos`.
#[allow(clippy::too_many_arguments)]
fn decode_pixel_data(
    out_images: &mut [Vec<u8>],
    requested_pixel_types: &[i32],
    mem: &[u8],
    data_pos: u64,
    data_len: usize,
    compression_type: i32,
    r: &PixelRegion,
    channels: &[ExrChannelInfo],
    channel_offset_list: &[usize],
) -> bool {
    let out_len = |w: i64| -> Option<usize> {
        let n = u64::try_from(w).ok()?;
        let n = n.checked_mul(u64::try_from(r.num_lines).ok()?)?;
        usize::try_from(n.checked_mul(r.pixel_data_size as u64)?).ok()
    };
    if compression_type == TINYEXR_COMPRESSIONTYPE_PIZ {
        if r.width == 0 || r.num_lines == 0 || r.pixel_data_size == 0 {
            // Invalid input #90
            return false;
        }
        // static_cast<size_t>(width * num_lines) * pixel_data_size
        let Some(mut out_buf) = usize::try_from(r.width.wrapping_mul(r.num_lines))
            .ok()
            .and_then(|n| n.checked_mul(r.pixel_data_size))
            .and_then(try_zeroed::<u8>)
        else {
            return false;
        };
        if !decompress_piz(
            &mut out_buf,
            mem,
            data_pos,
            data_len,
            channels,
            r.width,
            r.num_lines,
        ) {
            return false;
        }
        return scatter_decompressed(
            out_images,
            requested_pixel_types,
            &out_buf,
            r,
            channels,
            channel_offset_list,
        );
    }

    if compression_type == TINYEXR_COMPRESSIONTYPE_ZIPS
        || compression_type == TINYEXR_COMPRESSIONTYPE_ZIP
        || compression_type == TINYEXR_COMPRESSIONTYPE_RLE
    {
        let Some(mut out_buf) = out_len(i64::from(r.width)).and_then(try_zeroed::<u8>) else {
            return false;
        };
        let mut dst_len = out_buf.len();
        if dst_len == 0 {
            return false;
        }
        let ok = if compression_type == TINYEXR_COMPRESSIONTYPE_RLE {
            decompress_rle(&mut out_buf, dst_len, mem, data_pos, data_len)
        } else {
            decompress_zip(&mut out_buf, &mut dst_len, mem, data_pos, data_len)
        };
        if !ok {
            return false;
        }
        return scatter_decompressed(
            out_images,
            requested_pixel_types,
            &out_buf,
            r,
            channels,
            channel_offset_list,
        );
    }

    if compression_type == TINYEXR_COMPRESSIONTYPE_NONE {
        let width = i64::from(r.width);
        for (c, chan) in channels.iter().enumerate() {
            let requested = requested_pixel_types.get(c).copied().unwrap_or(-1);
            let off = channel_offset_list[c] as i64;
            let Some(image) = out_images.get_mut(c) else {
                return false;
            };
            for v in 0..i64::from(r.num_lines) {
                let line = v * r.pixel_data_size as i64 * width + off * width;
                let start = r.line_start(r.y, v);
                let fits = |elem: i64| line + elem * width <= data_len as i64;
                let src = |u: i64, elem: i64| data_pos.wrapping_add((line + u * elem) as u64);
                match chan.pixel_type {
                    TINYEXR_PIXELTYPE_HALF => {
                        if requested == TINYEXR_PIXELTYPE_HALF {
                            for u in 0..width {
                                let hf = rd_u16(mem, src(u, 2));
                                put(image, start.wrapping_add(u), &hf.to_le_bytes());
                            }
                        } else if requested == TINYEXR_PIXELTYPE_FLOAT {
                            if !fits(2) {
                                return false;
                            }
                            for u in 0..width {
                                let f = half_to_float(rd_u16(mem, src(u, 2)));
                                put(image, start.wrapping_add(u), &f.to_le_bytes());
                            }
                        } else {
                            return false;
                        }
                    }
                    TINYEXR_PIXELTYPE_FLOAT | TINYEXR_PIXELTYPE_UINT => {
                        if !fits(4) {
                            return false;
                        }
                        for u in 0..width {
                            let val = rd_u32(mem, src(u, 4));
                            put(image, start.wrapping_add(u), &val.to_le_bytes());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    true
}

/// `ComputeChannelLayout`: per channel byte offsets and the pixel size.
fn compute_channel_layout(channels: &[ExrChannelInfo]) -> Option<(Vec<usize>, usize)> {
    let mut offsets = Vec::with_capacity(channels.len());
    let mut pixel_data_size = 0usize;
    for c in channels {
        offsets.push(pixel_data_size);
        pixel_data_size += match c.pixel_type {
            TINYEXR_PIXELTYPE_HALF => 2,
            TINYEXR_PIXELTYPE_FLOAT | TINYEXR_PIXELTYPE_UINT => 4,
            _ => return None,
        };
    }
    Some((offsets, pixel_data_size))
}

/// `AllocateImage`: zeroed channel images (the C `malloc` leaves them
/// uninitialized). The flag is false for an unsupported type conversion or
/// a failed allocation.
fn allocate_image(
    channels: &[ExrChannelInfo],
    requested_pixel_types: &[i32],
    data_width: i32,
    data_height: i32,
) -> (Vec<Vec<u8>>, bool) {
    let data_len = (i64::from(data_width) as u64).wrapping_mul(i64::from(data_height) as u64);
    let mut images = Vec::with_capacity(channels.len());
    for (c, chan) in channels.iter().enumerate() {
        let requested = requested_pixel_types.get(c).copied().unwrap_or(-1);
        let elem: u64 = match (chan.pixel_type, requested) {
            (TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF) => 2,
            (TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_FLOAT) => 4,
            (TINYEXR_PIXELTYPE_FLOAT, _) | (TINYEXR_PIXELTYPE_UINT, _) => 4,
            _ => return (Vec::new(), false),
        };
        let Some(image) = data_len
            .checked_mul(elem)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(try_zeroed::<u8>)
        else {
            return (Vec::new(), false);
        };
        images.push(image);
    }
    (images, true)
}

/// `OffsetData`: chunk offsets per level, tile row and tile column.
#[derive(Default)]
struct OffsetData {
    offsets: Vec<Vec<Vec<u64>>>,
    num_x_levels: i32,
    num_y_levels: i32,
}

/// `LevelIndex`.
fn level_index(lx: i32, ly: i32, tile_level_mode: i32, num_x_levels: i32) -> i32 {
    match tile_level_mode {
        TINYEXR_TILE_ONE_LEVEL => 0,
        TINYEXR_TILE_MIPMAP_LEVELS => lx,
        TINYEXR_TILE_RIPMAP_LEVELS => lx.wrapping_add(ly.wrapping_mul(num_x_levels)),
        _ => -1,
    }
}

/// `LevelSize`.
fn level_size(toplevel_size: i32, level: i32, tile_rounding_mode: i32) -> i32 {
    if level < 0 {
        return -1;
    }
    let b = 1u32.wrapping_shl(level as u32) as i32;
    let mut level_size = toplevel_size.checked_div(b).unwrap_or(0);
    if tile_rounding_mode == TINYEXR_TILE_ROUND_UP && level_size.wrapping_mul(b) < toplevel_size {
        level_size += 1;
    }
    level_size.max(1)
}

/// Tile decoding failure flags of `DecodeTiledLevel`.
const EF_INVALID_DATA: u32 = 1;
const EF_INSUFFICIENT_DATA: u32 = 2;
const EF_FAILED_TO_DECODE: u32 = 4;

/// `DecodeTiledPixelData`: decodes one tile into `tile`.
#[allow(clippy::too_many_arguments)]
fn decode_tiled_pixel_data(
    tile: &mut ExrTile,
    exr_header: &ExrHeader,
    mem: &[u8],
    data_pos: u64,
    data_len: usize,
    data_width: i32,
    data_height: i32,
    tile_offset_x: i32,
    tile_offset_y: i32,
    pixel_data_size: usize,
    channel_offset_list: &[usize],
) -> bool {
    let tile_size_x = exr_header.tile_size_x;
    let tile_size_y = exr_header.tile_size_y;
    if tile_size_x.wrapping_mul(tile_offset_x) > data_width
        || tile_size_y.wrapping_mul(tile_offset_y) > data_height
    {
        return false;
    }

    tile.width = if tile_offset_x.wrapping_add(1).wrapping_mul(tile_size_x) >= data_width {
        data_width.wrapping_sub(tile_offset_x.wrapping_mul(tile_size_x))
    } else {
        tile_size_x
    };
    tile.height = if tile_offset_y.wrapping_add(1).wrapping_mul(tile_size_y) >= data_height {
        data_height.wrapping_sub(tile_offset_y.wrapping_mul(tile_size_y))
    } else {
        tile_size_y
    };

    let region = PixelRegion {
        line_order: exr_header.line_order,
        width: tile.width,
        height: tile_size_y,
        x_stride: tile_size_x,
        y: 0,
        line_no: 0,
        num_lines: tile.height,
        pixel_data_size,
    };
    let images = tile.images.get_or_insert_with(Vec::new);
    decode_pixel_data(
        images,
        &exr_header.requested_pixel_types,
        mem,
        data_pos,
        data_len,
        exr_header.compression_type,
        &region,
        &exr_header.channels,
        channel_offset_list,
    )
}

/// `DecodeTiledLevel`: decodes all tiles of `exr_image`'s level.
fn decode_tiled_level(
    exr_image: &mut ExrImage,
    exr_header: &ExrHeader,
    offset_data: &OffsetData,
    channel_offset_list: &[usize],
    pixel_data_size: usize,
    mem: &[u8],
    err: &mut String,
) -> i32 {
    let num_channels = exr_header.num_channels();
    let level_idx = level_index(
        exr_image.level_x,
        exr_image.level_y,
        exr_header.tile_level_mode,
        offset_data.num_x_levels,
    );
    let Some(level) = usize::try_from(level_idx)
        .ok()
        .and_then(|l| offset_data.offsets.get(l))
    else {
        return TINYEXR_ERROR_INVALID_DATA;
    };
    let num_y_tiles = level.len();
    if num_y_tiles < 1 {
        return TINYEXR_ERROR_INVALID_DATA;
    }
    let num_x_tiles = level[0].len();
    if num_x_tiles < 1 {
        return TINYEXR_ERROR_INVALID_DATA;
    }
    let num_tiles = num_x_tiles * num_y_tiles;

    let mut error_flag = 0u32;
    let size = mem.len() as u64;

    let mut tiles = vec![ExrTile::default(); num_tiles];
    for (tile_idx, tile) in tiles.iter_mut().enumerate() {
        let (images, alloc_success) = allocate_image(
            &exr_header.channels,
            &exr_header.requested_pixel_types,
            exr_header.tile_size_x,
            exr_header.tile_size_y,
        );
        tile.images = Some(images);
        if !alloc_success {
            error_flag |= EF_INVALID_DATA;
            continue;
        }

        let x_tile = tile_idx % num_x_tiles;
        let y_tile = tile_idx / num_x_tiles;
        let offset = level[y_tile][x_tile];
        if offset.wrapping_add(20) > size {
            error_flag |= EF_INSUFFICIENT_DATA;
            continue;
        }

        let data_size = size - (offset + 20);
        let tile_coordinates = [0u64, 4, 8, 12].map(|o| rd_u32(mem, offset + o) as i32);

        if tile_coordinates[2] != exr_image.level_x {
            error_flag |= EF_INVALID_DATA;
            continue;
        }
        if tile_coordinates[3] != exr_image.level_y {
            error_flag |= EF_INVALID_DATA;
            continue;
        }

        let data_len = rd_u32(mem, offset + 16) as i32;
        if data_len < 2 || data_len as u64 > data_size {
            error_flag |= EF_INSUFFICIENT_DATA;
            continue;
        }

        let ret = decode_tiled_pixel_data(
            tile,
            exr_header,
            mem,
            offset + 20,
            data_len as usize,
            exr_image.width,
            exr_image.height,
            tile_coordinates[0],
            tile_coordinates[1],
            pixel_data_size,
            channel_offset_list,
        );
        if !ret {
            error_flag |= EF_FAILED_TO_DECODE;
        }

        tile.offset_x = tile_coordinates[0];
        tile.offset_y = tile_coordinates[1];
        tile.level_x = tile_coordinates[2];
        tile.level_y = tile_coordinates[3];
    }
    exr_image.tiles = Some(tiles);

    exr_image.num_channels = num_channels;
    exr_image.num_tiles = num_tiles as i32;

    let mut err_code = TINYEXR_SUCCESS;
    if error_flag != 0 {
        err_code = TINYEXR_ERROR_INVALID_DATA;
    }
    if error_flag & EF_INSUFFICIENT_DATA != 0 {
        err.push_str("Insufficient data length.\n");
    }
    if error_flag & EF_FAILED_TO_DECODE != 0 {
        err.push_str("Failed to decode tile data.\n");
    }
    err_code
}

/// Scanline block count of a compression (`num_scanline_blocks`).
fn num_scanline_blocks(compression_type: i32) -> i32 {
    match compression_type {
        TINYEXR_COMPRESSIONTYPE_ZIP => 16,
        TINYEXR_COMPRESSIONTYPE_PIZ => 32,
        TINYEXR_COMPRESSIONTYPE_ZFP => 16,
        _ => 1,
    }
}

/// Decodes the levels of a tiled image (part of `DecodeChunk`); level 0 is
/// `exr_image`, the following ones are chained through `next_level`.
fn decode_tiled_levels(
    exr_image: &mut ExrImage,
    exr_header: &ExrHeader,
    offset_data: &OffsetData,
    channel_offset_list: &[usize],
    pixel_data_size: usize,
    mem: &[u8],
    err: &mut String,
) -> i32 {
    let dw = exr_header.data_window;
    let levels: Vec<(i32, i32)> = if exr_header.tile_level_mode != TINYEXR_TILE_RIPMAP_LEVELS {
        (0..offset_data.num_x_levels).map(|l| (l, l)).collect()
    } else {
        (0..offset_data.num_y_levels)
            .flat_map(|ly| (0..offset_data.num_x_levels).map(move |lx| (lx, ly)))
            .collect()
    };

    let mut decoded: Vec<ExrImage> = Vec::with_capacity(levels.len());
    let mut ret = TINYEXR_SUCCESS;
    for (i, &(level_x, level_y)) in levels.iter().enumerate() {
        let mut level_image = if i == 0 {
            std::mem::take(exr_image)
        } else {
            ExrImage::default()
        };
        let width = level_size(
            dw.max_x.wrapping_sub(dw.min_x).wrapping_add(1),
            level_x,
            exr_header.tile_rounding_mode,
        );
        let height = level_size(
            dw.max_y.wrapping_sub(dw.min_y).wrapping_add(1),
            level_y,
            exr_header.tile_rounding_mode,
        );
        level_image.width = width;
        if width >= 1 {
            level_image.height = height;
        }
        if width < 1 || height < 1 {
            ret = TINYEXR_ERROR_INVALID_DATA;
        } else {
            level_image.level_x = level_x;
            level_image.level_y = level_y;
            ret = decode_tiled_level(
                &mut level_image,
                exr_header,
                offset_data,
                channel_offset_list,
                pixel_data_size,
                mem,
                err,
            );
        }
        decoded.push(level_image);
        if ret != TINYEXR_SUCCESS {
            break;
        }
    }

    // Link the levels.
    let mut next: Option<Box<ExrImage>> = None;
    while let Some(mut level) = decoded.pop() {
        level.next_level = next;
        if decoded.is_empty() {
            *exr_image = level;
            break;
        }
        next = Some(Box::new(level));
    }
    ret
}

/// `DecodeChunk`.
fn decode_chunk(
    exr_image: &mut ExrImage,
    exr_header: &mut ExrHeader,
    offset_data: &OffsetData,
    mem: &[u8],
    err: &mut String,
) -> i32 {
    let num_channels = exr_header.num_channels();
    let nsb = num_scanline_blocks(exr_header.compression_type);
    let dw = exr_header.data_window;

    if dw.max_x < dw.min_x || dw.max_y < dw.min_y {
        err.push_str("Invalid data window.\n");
        return TINYEXR_ERROR_INVALID_DATA;
    }

    let data_width = i64::from(dw.max_x) - i64::from(dw.min_x) + 1;
    let data_height = i64::from(dw.max_y) - i64::from(dw.min_y) + 1;

    if data_width <= 0 {
        err.push_str("Invalid data window width.\n");
        return TINYEXR_ERROR_INVALID_DATA;
    }
    if data_height <= 0 {
        err.push_str("Invalid data window height.\n");
        return TINYEXR_ERROR_INVALID_DATA;
    }

    if data_width > TINYEXR_DIMENSION_THRESHOLD || data_height > TINYEXR_DIMENSION_THRESHOLD {
        err.push_str(&format!(
            "data_with or data_height too large. data_width: {data_width}, data_height = {data_height}\n"
        ));
        return TINYEXR_ERROR_INVALID_DATA;
    }
    if exr_header.tiled != 0
        && (i64::from(exr_header.tile_size_x) > TINYEXR_DIMENSION_THRESHOLD
            || i64::from(exr_header.tile_size_y) > TINYEXR_DIMENSION_THRESHOLD)
    {
        err.push_str(&format!(
            "tile with or tile height too large. tile width: {}, tile height = {}\n",
            exr_header.tile_size_x, exr_header.tile_size_y
        ));
        return TINYEXR_ERROR_INVALID_DATA;
    }

    let Some((channel_offset_list, pixel_data_size)) = compute_channel_layout(&exr_header.channels)
    else {
        err.push_str("Failed to compute channel layout.\n");
        return TINYEXR_ERROR_INVALID_DATA;
    };

    let mut invalid_data = false;

    if exr_header.tiled != 0 {
        if exr_header.tile_size_x < 0 {
            err.push_str(&format!(
                "Invalid tile size x : {}\n",
                exr_header.tile_size_x
            ));
            return TINYEXR_ERROR_INVALID_HEADER;
        }
        if exr_header.tile_size_y < 0 {
            err.push_str(&format!(
                "Invalid tile size y : {}\n",
                exr_header.tile_size_y
            ));
            return TINYEXR_ERROR_INVALID_HEADER;
        }
        let ret = decode_tiled_levels(
            exr_image,
            exr_header,
            offset_data,
            &channel_offset_list,
            pixel_data_size,
            mem,
            err,
        );
        if ret != TINYEXR_SUCCESS {
            return ret;
        }
    } else {
        // scanline format
        let total_data_len = (data_width as u64)
            .wrapping_mul(data_height as u64)
            .wrapping_mul(num_channels as u64);
        if total_data_len == 0 || total_data_len >= 0x40_0000_0000 {
            err.push_str(&format!(
                "Image data size is zero or too large: width = {data_width}, height = {data_height}, channels = {num_channels}\n"
            ));
            return TINYEXR_ERROR_INVALID_DATA;
        }

        let (mut images, alloc_success) = allocate_image(
            &exr_header.channels,
            &exr_header.requested_pixel_types,
            data_width as i32,
            data_height as i32,
        );
        if !alloc_success {
            exr_image.images = Some(images);
            err.push_str(&format!(
                "Failed to allocate memory for Images. Maybe EXR header is corrupted or Image data size is too large: width = {data_width}, height = {data_height}, channels = {num_channels}\n"
            ));
            return TINYEXR_ERROR_INVALID_DATA;
        }

        let size = mem.len() as u64;
        let empty = Vec::new();
        let offsets = offset_data
            .offsets
            .first()
            .and_then(|l| l.first())
            .unwrap_or(&empty);
        for (y, &offset) in offsets.iter().enumerate() {
            if offset.wrapping_add(8) > size {
                invalid_data = true;
                continue;
            }
            let data_size = size - (offset + 8);
            let mut line_no = rd_u32(mem, offset) as i32;
            let data_len = rd_u32(mem, offset + 4) as i32;

            if i64::from(data_len) as u64 > data_size
                || !(-(2 << 20)..=(2 << 20)).contains(&line_no)
                || data_len == 0
            {
                invalid_data = true;
                continue;
            }

            let end_line_no = line_no.wrapping_add(nsb).min(dw.max_y.wrapping_add(1));
            let num_lines = end_line_no.wrapping_sub(line_no);
            if num_lines <= 0 {
                invalid_data = true;
                continue;
            }

            let lno = i64::from(line_no) - i64::from(dw.min_y);
            if lno > i64::from(i32::MAX) || lno < -i64::from(i32::MAX) {
                line_no = -1; // invalid
            } else {
                line_no = line_no.wrapping_sub(dw.min_y);
            }

            if line_no < 0 {
                invalid_data = true;
                continue;
            }

            let region = PixelRegion {
                line_order: exr_header.line_order,
                width: data_width as i32,
                height: data_height as i32,
                x_stride: data_width as i32,
                y: y as i32,
                line_no,
                num_lines,
                pixel_data_size,
            };
            if !decode_pixel_data(
                &mut images,
                &exr_header.requested_pixel_types,
                mem,
                offset + 8,
                data_len as usize,
                exr_header.compression_type,
                &region,
                &exr_header.channels,
                &channel_offset_list,
            ) {
                invalid_data = true;
            }
        }

        if invalid_data {
            err.push_str("Invalid/Corrupted data found when decoding pixels.\n");
            images.iter_mut().for_each(|i| *i = Vec::new());
            exr_image.images = Some(images);
            return TINYEXR_ERROR_INVALID_DATA;
        }
        exr_image.images = Some(images);
    }

    let requested = exr_header.requested_pixel_types.clone();
    for (p, r) in exr_header.pixel_types.iter_mut().zip(requested) {
        *p = r;
    }

    exr_image.num_channels = num_channels;
    exr_image.width = data_width as i32;
    exr_image.height = data_height as i32;

    TINYEXR_SUCCESS
}

/// `ReconstructLineOffsets`: rebuilds the scanline offsets by walking the
/// chunks from `marker`.
fn reconstruct_line_offsets(offsets: &mut [u64], mem: &[u8], marker: u64) -> bool {
    let size = mem.len() as u64;
    if marker == 0 {
        return false;
    }
    let mut marker = marker;
    for o in offsets.iter_mut() {
        let offset = marker;
        if offset.wrapping_add(8) >= size {
            return false;
        }
        let data_len = rd_u32(mem, marker + 4);
        if u64::from(data_len) >= size {
            return false;
        }
        *o = offset;
        marker = marker.wrapping_add(u64::from(data_len.wrapping_add(8)));
    }
    true
}

/// `FloorLog2`.
fn floor_log2(mut x: u32) -> i32 {
    let mut y = 0;
    while x > 1 {
        y += 1;
        x >>= 1;
    }
    y
}

/// `CeilLog2`.
fn ceil_log2(mut x: u32) -> i32 {
    let mut y = 0;
    let mut r = 0;
    while x > 1 {
        if x & 1 != 0 {
            r = 1;
        }
        y += 1;
        x >>= 1;
    }
    y + r
}

/// `RoundLog2`.
fn round_log2(x: i32, tile_rounding_mode: i32) -> i32 {
    if tile_rounding_mode == TINYEXR_TILE_ROUND_DOWN {
        floor_log2(x as u32)
    } else {
        ceil_log2(x as u32)
    }
}

/// `CalculateNumXLevels` / `CalculateNumYLevels`.
fn calculate_num_levels(exr_header: &ExrHeader, y: bool) -> i32 {
    let dw = exr_header.data_window;
    let w = dw.max_x.wrapping_sub(dw.min_x).wrapping_add(1);
    let h = dw.max_y.wrapping_sub(dw.min_y).wrapping_add(1);
    match exr_header.tile_level_mode {
        TINYEXR_TILE_ONE_LEVEL => 1,
        TINYEXR_TILE_MIPMAP_LEVELS => round_log2(w.max(h), exr_header.tile_rounding_mode) + 1,
        TINYEXR_TILE_RIPMAP_LEVELS => {
            round_log2(if y { h } else { w }, exr_header.tile_rounding_mode) + 1
        }
        _ => -1,
    }
}

/// `CalculateNumTiles`. A zero tile size (a division by zero in C) fails.
fn calculate_num_tiles(
    num_tiles: &mut [i32],
    toplevel_size: i32,
    size: i32,
    tile_rounding_mode: i32,
) -> bool {
    for (i, n) in num_tiles.iter_mut().enumerate() {
        let l = level_size(toplevel_size, i as i32, tile_rounding_mode);
        if l < 0 {
            return false;
        }
        if i64::from(l) > i64::from(i32::MAX) - i64::from(size) + 1 {
            return false;
        }
        let Some(t) = (l + size - 1).checked_div(size) else {
            return false;
        };
        *n = t;
    }
    true
}

/// `PrecalculateTileInfo`: tiles per level in x and y.
fn precalculate_tile_info(exr_header: &ExrHeader) -> Option<(Vec<i32>, Vec<i32>)> {
    let dw = exr_header.data_window;
    let num_x_levels = calculate_num_levels(exr_header, false);
    if num_x_levels < 0 {
        return None;
    }
    let num_y_levels = calculate_num_levels(exr_header, true);
    if num_y_levels < 0 {
        return None;
    }
    let mut num_x_tiles = vec![0; num_x_levels as usize];
    let mut num_y_tiles = vec![0; num_y_levels as usize];
    if !calculate_num_tiles(
        &mut num_x_tiles,
        dw.max_x.wrapping_sub(dw.min_x).wrapping_add(1),
        exr_header.tile_size_x,
        exr_header.tile_rounding_mode,
    ) {
        return None;
    }
    if !calculate_num_tiles(
        &mut num_y_tiles,
        dw.max_y.wrapping_sub(dw.min_y).wrapping_add(1),
        exr_header.tile_size_y,
        exr_header.tile_rounding_mode,
    ) {
        return None;
    }
    Some((num_x_tiles, num_y_tiles))
}

/// Offset table shape of `InitTileOffsets`: (tile rows, tile columns) per
/// level index.
fn tile_offsets_shape(
    exr_header: &ExrHeader,
    num_x_tiles: &[i32],
    num_y_tiles: &[i32],
) -> Vec<(usize, usize)> {
    let to_usize = |v: i32| usize::try_from(v).unwrap_or(0);
    match exr_header.tile_level_mode {
        TINYEXR_TILE_ONE_LEVEL | TINYEXR_TILE_MIPMAP_LEVELS => {
            if num_x_tiles.len() != num_y_tiles.len() {
                return Vec::new();
            }
            num_x_tiles
                .iter()
                .zip(num_y_tiles)
                .map(|(&x, &y)| (to_usize(y), to_usize(x)))
                .collect()
        }
        TINYEXR_TILE_RIPMAP_LEVELS => num_y_tiles
            .iter()
            .flat_map(|&y| num_x_tiles.iter().map(move |&x| (to_usize(y), to_usize(x))))
            .collect(),
        _ => Vec::new(),
    }
}

/// `isValidTile`.
fn is_valid_tile(
    exr_header: &ExrHeader,
    offset_data: &OffsetData,
    dx: i32,
    dy: i32,
    lx: i32,
    ly: i32,
) -> bool {
    if lx < 0 || ly < 0 || dx < 0 || dy < 0 {
        return false;
    }
    let has = |l: usize| {
        offset_data
            .offsets
            .get(l)
            .and_then(|rows| rows.get(dy as usize))
            .is_some_and(|row| row.len() > dx as usize)
    };
    match exr_header.tile_level_mode {
        TINYEXR_TILE_ONE_LEVEL => lx == 0 && ly == 0 && has(0),
        TINYEXR_TILE_MIPMAP_LEVELS => {
            lx < offset_data.num_x_levels && ly < offset_data.num_y_levels && has(lx as usize)
        }
        TINYEXR_TILE_RIPMAP_LEVELS => {
            let idx = (lx as usize)
                .wrapping_add((ly as usize).wrapping_mul(offset_data.num_x_levels.max(0) as usize));
            lx < offset_data.num_x_levels && ly < offset_data.num_y_levels && has(idx)
        }
        _ => false,
    }
}

/// `ReconstructTileOffsets`: rebuilds the tile offsets by walking the chunks
/// from `marker`.
fn reconstruct_tile_offsets(
    offset_data: &mut OffsetData,
    exr_header: &ExrHeader,
    mem: &[u8],
    marker: u64,
    is_multi_part_file: bool,
    is_deep: bool,
) -> bool {
    let size = mem.len() as i64;
    let num_x_levels = offset_data.num_x_levels;
    let mut marker = marker as i64;
    let count: usize = offset_data
        .offsets
        .iter()
        .map(|rows| rows.iter().map(Vec::len).sum::<usize>())
        .sum();
    for _ in 0..count {
        let tile_offset = marker as u64;

        if is_multi_part_file {
            if marker.wrapping_add(4) >= size {
                return false;
            }
            marker = marker.wrapping_add(4);
        }

        if marker.wrapping_add(16) >= size {
            return false;
        }

        let rd = |m: i64| rd_u32(mem, m as u64) as i32;
        let tile_x = rd(marker);
        let tile_y = rd(marker.wrapping_add(4));
        let level_x = rd(marker.wrapping_add(8));
        let level_y = rd(marker.wrapping_add(12));
        marker = marker.wrapping_add(16);

        if is_deep {
            if marker.wrapping_add(16) >= size {
                return false;
            }
            let packed_offset_table_size = rd_u64(mem, marker as u64) as i64;
            let packed_sample_size = rd_u64(mem, marker.wrapping_add(8) as u64) as i64;
            marker = marker.wrapping_add(16);
            marker = marker
                .wrapping_add(packed_offset_table_size)
                .wrapping_add(packed_sample_size)
                .wrapping_add(8);
            if marker >= size {
                return false;
            }
        } else {
            if marker.wrapping_add(4) >= size {
                return false;
            }
            let data_size = rd_u32(mem, marker as u64);
            marker = marker.wrapping_add(4);
            marker = marker.wrapping_add(i64::from(data_size));
            if marker >= size {
                return false;
            }
        }

        if !is_valid_tile(exr_header, offset_data, tile_x, tile_y, level_x, level_y) {
            return false;
        }

        let level_idx = level_index(level_x, level_y, exr_header.tile_level_mode, num_x_levels);
        let Some(slot) = usize::try_from(level_idx)
            .ok()
            .and_then(|l| offset_data.offsets.get_mut(l))
            .and_then(|rows| rows.get_mut(tile_y as usize))
            .and_then(|row| row.get_mut(tile_x as usize))
        else {
            return false;
        };
        *slot = tile_offset;
    }
    true
}

/// Reads `count` offsets of the offset table at `marker` (`ReadOffsets` /
/// the scanline loop of `DecodeEXRImage`). The table is only kept when every
/// read succeeds, so a corrupted count never allocates more than the file.
fn read_offsets(
    count: u64,
    mem: &[u8],
    marker: &mut u64,
    err: &mut Option<String>,
) -> Result<Vec<u64>, i32> {
    let size = mem.len() as u64;
    let mut offsets = Vec::new();
    for _ in 0..count {
        if marker.wrapping_add(8) >= size {
            set_error_message("Insufficient data size in offset table.", err);
            return Err(TINYEXR_ERROR_INVALID_DATA);
        }
        let offset = rd_u64(mem, *marker);
        if offset >= size {
            set_error_message("Invalid offset value in DecodeEXRImage.", err);
            return Err(TINYEXR_ERROR_INVALID_DATA);
        }
        *marker += 8;
        offsets.push(offset);
    }
    Ok(offsets)
}

/// `DecodeEXRImage`.
fn decode_exr_image(
    exr_image: &mut ExrImage,
    exr_header: &mut ExrHeader,
    mem: &[u8],
    marker: u64,
    err: &mut Option<String>,
) -> i32 {
    let size = mem.len();
    if size <= K_EXR_VERSION_SIZE {
        set_error_message("Invalid argument for DecodeEXRImage().", err);
        return TINYEXR_ERROR_INVALID_ARGUMENT;
    }
    let mut marker = marker;

    let nsb = num_scanline_blocks(exr_header.compression_type);
    let dw = exr_header.data_window;

    if dw.max_x < dw.min_x || dw.max_x.wrapping_sub(dw.min_x) == i32::MAX {
        set_error_message("Invalid data width value", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }
    let data_width = i64::from(dw.max_x) - i64::from(dw.min_x) + 1;
    if data_width <= 0 {
        set_error_message("Invalid data window width value", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }

    if dw.max_y < dw.min_y || dw.max_y.wrapping_sub(dw.min_y) == i32::MAX {
        set_error_message("Invalid data height value", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }
    let data_height = i64::from(dw.max_y) - i64::from(dw.min_y) + 1;
    if data_height <= 0 {
        set_error_message("Invalid data window height value", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }

    if data_width > TINYEXR_DIMENSION_THRESHOLD {
        set_error_message("data width too large.", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }
    if data_height > TINYEXR_DIMENSION_THRESHOLD {
        set_error_message("data height too large.", err);
        return TINYEXR_ERROR_INVALID_DATA;
    }

    if exr_header.tiled != 0 {
        if i64::from(exr_header.tile_size_x) > TINYEXR_DIMENSION_THRESHOLD {
            set_error_message("tile width too large.", err);
            return TINYEXR_ERROR_INVALID_DATA;
        }
        if i64::from(exr_header.tile_size_y) > TINYEXR_DIMENSION_THRESHOLD {
            set_error_message("tile height too large.", err);
            return TINYEXR_ERROR_INVALID_DATA;
        }
    }

    let mut offset_data = OffsetData::default();
    if exr_header.tiled != 0 {
        let Some((num_x_tiles, num_y_tiles)) = precalculate_tile_info(exr_header) else {
            set_error_message("Failed to precalculate tile info.", err);
            return TINYEXR_ERROR_INVALID_DATA;
        };
        let shape = tile_offsets_shape(exr_header, &num_x_tiles, &num_y_tiles);
        // InitTileOffsets returns the block count as an int.
        let num_blocks = shape.iter().fold(0i32, |n, &(rows, cols)| {
            n.wrapping_add((rows as i32).wrapping_mul(cols as i32))
        });
        if exr_header.chunk_count > 0 && exr_header.chunk_count != num_blocks {
            set_error_message("Invalid offset table size.", err);
            return TINYEXR_ERROR_INVALID_DATA;
        }

        let total: u64 = shape.iter().map(|&(r, c)| r as u64 * c as u64).sum();
        let flat = match read_offsets(total, mem, &mut marker, err) {
            Ok(flat) => flat,
            Err(ret) => return ret,
        };
        let mut it = flat.into_iter();
        offset_data.num_x_levels = num_x_tiles.len() as i32;
        offset_data.num_y_levels = num_y_tiles.len() as i32;
        offset_data.offsets = shape
            .iter()
            .map(|&(rows, cols)| {
                (0..rows)
                    .map(|_| it.by_ref().take(cols).collect())
                    .collect()
            })
            .collect();

        let any_invalid = offset_data
            .offsets
            .iter()
            .flatten()
            .flatten()
            .any(|&o| o as i64 <= 0);
        if any_invalid
            && !reconstruct_tile_offsets(
                &mut offset_data,
                exr_header,
                mem,
                marker,
                exr_header.multipart != 0,
                exr_header.non_image != 0,
            )
        {
            set_error_message("Invalid Tile Offsets data.", err);
            return TINYEXR_ERROR_INVALID_DATA;
        }
    } else {
        let num_blocks = if exr_header.chunk_count > 0 {
            exr_header.chunk_count as u64
        } else {
            (data_height as u64).div_ceil(nsb as u64)
        };

        let mut offsets = match read_offsets(num_blocks, mem, &mut marker, err) {
            Ok(offsets) => offsets,
            Err(ret) => return ret,
        };

        if offsets.contains(&0) && !reconstruct_line_offsets(&mut offsets, mem, marker) {
            set_error_message(
                "Cannot reconstruct lineOffset table in DecodeEXRImage.",
                err,
            );
            return TINYEXR_ERROR_INVALID_DATA;
        }
        offset_data.offsets = vec![vec![offsets]];
        offset_data.num_x_levels = 1;
        offset_data.num_y_levels = 1;
    }

    let mut e = String::new();
    let ret = decode_chunk(exr_image, exr_header, &offset_data, mem, &mut e);
    if ret != TINYEXR_SUCCESS {
        if !e.is_empty() {
            set_error_message(&e, err);
        }
        free_exr_image(exr_image);
    }
    ret
}

/// `LoadEXRImageFromMemory`: decodes the image described by `exr_header`
/// (from `parse_exr_header_from_memory` of the same `memory`). Like the C
/// version it updates `exr_header.pixel_types` to the requested types.
pub fn load_exr_image_from_memory(
    exr_image: &mut ExrImage,
    exr_header: &mut ExrHeader,
    memory: &[u8],
    err: &mut Option<String>,
) -> i32 {
    if memory.len() < K_EXR_VERSION_SIZE {
        set_error_message("Invalid argument for LoadEXRImageFromMemory", err);
        return TINYEXR_ERROR_INVALID_ARGUMENT;
    }

    if exr_header.header_len == 0 {
        set_error_message("EXRHeader variable is not initialized.", err);
        return TINYEXR_ERROR_INVALID_ARGUMENT;
    }

    // +8 for magic number + version header.
    let marker = u64::from(exr_header.header_len) + 8;
    decode_exr_image(exr_image, exr_header, memory, marker, err)
}
