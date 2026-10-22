//! Port of lib/src/info.c: printing information about KTX and KTX2 files.
//!
//! The C functions print to stdout. Each `ktx_print_*` function here does the
//! same; the matching `ktx_write_*` function appends the identical bytes to a
//! buffer instead (used by the `ktx info` tool and the tests).

use std::fs::File;
use std::io::Write as _;

use crate::defs::{
    KTX_ENDIAN_REF_REV, KTX_HEADER_SIZE, KTX_IDENTIFIER_REF, KTX2_HEADER_SIZE, KTX2_IDENTIFIER_REF,
    SupercmpScheme,
};
use crate::dfd::printdfd::{write_dfd, write_dfd_json};
use crate::error::{ErrorCode, KtxResult};
use crate::glformat_str::{gl_format_string, gl_internalformat_string, gl_type_string};
use crate::hashlist::HashList;
use crate::ktxint::{
    KtxHeader, KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry, ktx_check_header1,
};
use crate::stream::{FileStream, MemStream, Stream};
use crate::strings::{c_hash_x, ktx_bu_image_flags_bit_string, ktx_supercompression_scheme_string};
use crate::vkformat::vk_format_string;

// These constraints are not mandated by the spec and only used as a
// reasonable upper limit to stop parsing garbage data during print
const MAX_NUM_KVD_ENTRIES: usize = 100;
const MAX_NUM_LEVELS: u32 = 64;

/// Size of `ktxBasisLzGlobalHeader`.
const BASIS_LZ_GLOBAL_HEADER_SIZE: u64 = 20;
/// Size of `ktxBasisLzEtc1sImageDesc`.
const BASIS_LZ_ETC1S_IMAGE_DESC_SIZE: u64 = 20;
/// Size of `ktxUASTCHDR6x6IntermediateImageDesc`.
const UASTC_HDR_6X6_IMAGE_DESC_SIZE: u64 = 12;

macro_rules! p {
    ($out:expr, $($arg:tt)*) => {
        let _ = write!($out, $($arg)*);
    };
}

/// `PRINT_INDENT`: `%*s` with width `(base_indent + indent) * indent_width`.
fn ind(base_indent: u32, indent: u32, indent_width: u32) -> usize {
    base_indent.wrapping_add(indent).wrapping_mul(indent_width) as usize
}

fn rd_u32(b: &[u8], off: u64) -> u32 {
    let off = off as usize;
    match b.get(off..off + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

fn bool_str(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}

/// C `%f` of a float promoted to double.
fn c_float(v: f32) -> String {
    let d = f64::from(v);
    if d.is_nan() {
        if d.is_sign_negative() { "-nan" } else { "nan" }.to_string()
    } else if d.is_infinite() {
        if d < 0.0 { "-inf" } else { "inf" }.to_string()
    } else {
        format!("{d:.6}")
    }
}

/// Reads `len` bytes, growing the buffer only as data arrives so garbage
/// lengths cannot force huge allocations.
fn read_vec(stream: &mut dyn Stream, len: u64) -> KtxResult<Vec<u8>> {
    const CHUNK: u64 = 1 << 20;
    let mut v = Vec::new();
    let mut left = len;
    while left > 0 {
        let n = left.min(CHUNK) as usize;
        let start = v.len();
        v.resize(start + n, 0);
        stream.read(v.get_mut(start..).unwrap_or_default())?;
        left -= n as u64;
    }
    Ok(v)
}

fn to_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect()
}

/// Port of `printFlagBitsJSON`.
fn write_flag_bits_json(
    out: &mut Vec<u8>,
    indent: usize,
    nl: &str,
    flags: u32,
    to_string: fn(u32, bool) -> Option<&'static str>,
) {
    let mut first = true;
    for bit_index in 0..32 {
        let bit_mask = 1u32 << bit_index;
        let bit_value = bit_mask & flags != 0;
        let (comma, sep) = if first { ("", "") } else { (",", nl) };
        if let Some(s) = to_string(bit_index, bit_value) {
            p!(out, "{comma}{sep}{:indent$}\"{s}\"", "");
            first = false;
        } else if bit_value {
            p!(out, "{comma}{sep}{:indent$}{bit_mask}", "");
            first = false;
        }
    }
    if !first {
        p!(out, "{nl}");
    }
}

/// Port of `isKnownKeyValueUINT32`.
pub fn is_known_key_value_uint32(key: &[u8]) -> bool {
    matches!(key, b"KTXdxgiFormat__" | b"KTXmetalPixelFormat")
}

/// Port of `isKnownKeyValueString`.
pub fn is_known_key_value_string(key: &[u8]) -> bool {
    matches!(
        key,
        b"KTXorientation"
            | b"KTXswizzle"
            | b"KTXwriter"
            | b"KTXwriterScParams"
            | b"KTXastcDecodeMode"
    )
}

/// Port of `isKnownKeyValue`.
pub fn is_known_key_value(key: &[u8]) -> bool {
    is_known_key_value_uint32(key)
        || is_known_key_value_string(key)
        || matches!(
            key,
            b"KTXglFormat" | b"KTXanimData" | b"KTXcubemapIncomplete"
        )
}

/// `%s` of a NUL terminated value: the bytes before the first NUL.
fn c_str(v: &[u8]) -> &[u8] {
    match v.iter().position(|&c| c == 0) {
        Some(n) => v.get(..n).unwrap_or_default(),
        None => v,
    }
}

/// The `[%d, %d, ...]` dump of an unknown value (C `char` is signed).
fn write_byte_list(out: &mut Vec<u8>, value: &[u8]) {
    for (i, &b) in value.iter().enumerate() {
        let sep = if i + 1 == value.len() { "" } else { ", " };
        p!(out, "{}{sep}", b as i8);
    }
}

/// Port of `printKVData`.
pub fn write_kv_data(out: &mut Vec<u8>, kvd: &[u8]) {
    let mut list = HashList::new();
    if list.deserialize(kvd).is_err() {
        p!(
            out,
            "Failed to parse or not enough memory to build list of key/value pairs.\n"
        );
        return;
    }
    for entry in list.iter().take(MAX_NUM_KVD_ENTRIES) {
        let (_, key) = entry.get_key();
        let (_, value) = entry.get_value();
        out.extend_from_slice(key);
        p!(out, ":");
        if value.is_empty() {
            p!(out, " null\n");
        } else if key == b"KTXglFormat" {
            if value.len() == 12 {
                p!(out, "\n");
                p!(out, "    glInternalformat: 0x{:08X}\n", rd_u32(value, 0));
                p!(out, "    glFormat: 0x{:08X}\n", rd_u32(value, 4));
                p!(out, "    glType: 0x{:08X}\n", rd_u32(value, 8));
            }
        } else if key == b"KTXmapRange" {
            if value.len() == 8 {
                p!(out, "\n");
                p!(
                    out,
                    "    scale: {}\n",
                    c_float(f32::from_bits(rd_u32(value, 0)))
                );
                p!(
                    out,
                    "    offset: {}\n",
                    c_float(f32::from_bits(rd_u32(value, 4)))
                );
            }
        } else if key == b"KTXanimData" {
            if value.len() == 12 {
                let loop_count = rd_u32(value, 8);
                p!(out, "\n");
                p!(out, "    duration: {}\n", rd_u32(value, 0));
                p!(out, "    timescale: {}\n", rd_u32(value, 4));
                p!(
                    out,
                    "    loopCount: {loop_count}{}\n",
                    if loop_count == 0 { " (infinite)" } else { "" }
                );
            }
        } else if key == b"KTXcubemapIncomplete" {
            if let [faces] = value {
                p!(out, "\n");
                for (i, name) in FACE_NAMES.iter().enumerate() {
                    p!(out, "    {name}: {}\n", bool_str(faces & (1 << i) != 0));
                }
            }
        } else if is_known_key_value_uint32(key) {
            if value.len() == 4 {
                p!(out, " {}\n", rd_u32(value, 0));
            }
        } else if is_known_key_value_string(key) {
            if value.last() == Some(&0) {
                p!(out, " ");
                out.extend_from_slice(c_str(value));
                p!(out, "\n");
            }
        } else {
            p!(out, " [");
            write_byte_list(out, value);
            p!(out, "]\n");
        }
    }
}

const FACE_NAMES: [&str; 6] = [
    "positiveX",
    "negativeX",
    "positiveY",
    "negativeY",
    "positiveZ",
    "negativeZ",
];

/// Port of `printKVDataJSON`.
pub fn write_kv_data_json(
    out: &mut Vec<u8>,
    kvd: &[u8],
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) {
    let space = if minified { "" } else { " " };
    let nl = if minified { "" } else { "\n" };
    let i0 = ind(base_indent, 0, indent_width);
    let i1 = ind(base_indent, 1, indent_width);

    let mut list = HashList::new();
    if list.deserialize(kvd).is_err() {
        // Logging while printing JSON is not possible, we rely on the validation step to provide meaningful errors
        return;
    }
    if list.is_empty() {
        return;
    }
    // Marks if the first print did not occur yet (first print != first entry)
    let mut first_print = true;
    let mut begin = |out: &mut Vec<u8>| {
        if !first_print {
            p!(out, ",{nl}");
        }
        first_print = false;
    };
    for entry in list.iter().take(MAX_NUM_KVD_ENTRIES) {
        let (_, key) = entry.get_key();
        let (_, value) = entry.get_value();
        let key_open = |out: &mut Vec<u8>| {
            p!(out, "{:i0$}\"", "");
            out.extend_from_slice(key);
            p!(out, "\":{space}");
        };
        if value.is_empty() {
            if !is_known_key_value(key) {
                // Known keys are not be printed with null
                begin(out);
                key_open(out);
                p!(out, "null");
            }
        } else if key == b"KTXglFormat" {
            if value.len() == 12 {
                begin(out);
                key_open(out);
                p!(out, "{{{nl}");
                p!(
                    out,
                    "{:i1$}\"glInternalformat\":{space}{},{nl}",
                    "",
                    rd_u32(value, 0)
                );
                p!(
                    out,
                    "{:i1$}\"glFormat\":{space}{},{nl}",
                    "",
                    rd_u32(value, 4)
                );
                p!(out, "{:i1$}\"glType\":{space}{}{nl}", "", rd_u32(value, 8));
                p!(out, "{:i0$}}}", "");
            }
        } else if key == b"KTXanimData" {
            if value.len() == 12 {
                begin(out);
                key_open(out);
                p!(out, "{{{nl}");
                p!(
                    out,
                    "{:i1$}\"duration\":{space}{},{nl}",
                    "",
                    rd_u32(value, 0)
                );
                p!(
                    out,
                    "{:i1$}\"timescale\":{space}{},{nl}",
                    "",
                    rd_u32(value, 4)
                );
                p!(
                    out,
                    "{:i1$}\"loopCount\":{space}{}{nl}",
                    "",
                    rd_u32(value, 8)
                );
                p!(out, "{:i0$}}}", "");
            }
        } else if key == b"KTXcubemapIncomplete" {
            if let [faces] = value {
                begin(out);
                key_open(out);
                p!(out, "{{{nl}");
                for (i, name) in FACE_NAMES.iter().enumerate() {
                    let comma = if i == 5 { "" } else { "," };
                    let v = bool_str(faces & (1 << i) != 0);
                    p!(out, "{:i1$}\"{name}\":{space}{v}{comma}{nl}", "");
                }
                p!(out, "{:i0$}}}", "");
            }
        } else if is_known_key_value_uint32(key) {
            if value.len() == 4 {
                begin(out);
                key_open(out);
                p!(out, "{}", rd_u32(value, 0));
            }
        } else if is_known_key_value_string(key) {
            if value.last() == Some(&0) {
                begin(out);
                key_open(out);
                p!(out, "\"");
                out.extend_from_slice(c_str(value));
                p!(out, "\"");
            }
        } else {
            begin(out);
            key_open(out);
            p!(out, "[");
            write_byte_list(out, value);
            p!(out, "]");
        }
    }
    p!(out, "{nl}");
}

/// Port of `printIdentifier`. `json` escapes control characters as `\u00XX`.
pub fn write_identifier(out: &mut Vec<u8>, identifier: &[u8; 12], json: bool) {
    // Convert identifier for better display (u8identifier[30] in C).
    const CAP: usize = 30;
    let mut buf: Vec<u8> = Vec::with_capacity(CAP + 8);
    let mut idlen = 0usize;
    let set = |buf: &mut Vec<u8>, at: usize, c: u8| {
        if buf.len() <= at {
            buf.resize(at + 1, 0);
        }
        if let Some(b) = buf.get_mut(at) {
            *b = c;
        }
    };
    let mut i = 0;
    while i < 12 && idlen < CAP {
        let c = identifier[i];
        match c {
            // Convert the angle brackets to utf-8 for better printing.
            0xAB | 0xBB => {
                set(&mut buf, idlen, 0xC2);
                idlen += 1;
                set(&mut buf, idlen, c);
            }
            b'\n' => {
                set(&mut buf, idlen, b'\\');
                idlen += 1;
                set(&mut buf, idlen, b'n');
            }
            b'\r' => {
                set(&mut buf, idlen, b'\\');
                idlen += 1;
                set(&mut buf, idlen, b'r');
            }
            0..0x20 => {
                let s = if json {
                    format!("\\u{c:04X}")
                } else {
                    format!("\\x{c:02X}")
                };
                // snprintf truncates to the space left and NUL terminates.
                let room = CAP - idlen;
                for (k, &b) in s.as_bytes().iter().take(room - 1).enumerate() {
                    set(&mut buf, idlen + k, b);
                }
                set(&mut buf, idlen + s.len().min(room - 1), 0);
                idlen += s.len() - 1;
            }
            _ => set(&mut buf, idlen, c),
        }
        i += 1;
        idlen += 1;
    }
    let shown = buf.get(..idlen.min(buf.len())).unwrap_or_default();
    out.extend_from_slice(c_str(shown));
}

/// Port of `printKTXHeader`.
pub fn write_ktx_header(out: &mut Vec<u8>, header: &KtxHeader) {
    p!(out, "identifier: ");
    write_identifier(out, &header.identifier, false);
    p!(out, "\n");
    p!(out, "endianness: {}\n", c_hash_x(header.endianness));
    p!(out, "glType: {}\n", gl_type_string(header.gl_type));
    p!(out, "glTypeSize: {}\n", header.gl_type_size);
    p!(out, "glFormat: {}\n", gl_format_string(header.gl_format));
    p!(
        out,
        "glInternalformat: {}\n",
        gl_internalformat_string(header.gl_internalformat)
    );
    p!(
        out,
        "glBaseInternalformat: {}\n",
        gl_format_string(header.gl_base_internalformat)
    );
    p!(out, "pixelWidth: {}\n", header.pixel_width);
    p!(out, "pixelHeight: {}\n", header.pixel_height);
    p!(out, "pixelDepth: {}\n", header.pixel_depth);
    p!(
        out,
        "numberOfArrayElements: {}\n",
        header.number_of_array_elements
    );
    p!(out, "numberOfFaces: {}\n", header.number_of_faces);
    p!(out, "numberOfMipLevels: {}\n", header.number_of_mip_levels);
    p!(
        out,
        "bytesOfKeyValueData: {}\n",
        header.bytes_of_key_value_data
    );
}

/// Port of `printKTXInfo2`. The stream must be just past the header.
fn write_ktx_info2(out: &mut Vec<u8>, stream: &mut dyn Stream, header: &mut KtxHeader) {
    if header.endianness == KTX_ENDIAN_REF_REV {
        p!(
            out,
            "This file has opposite endianness to this machine. Following\nare the converted pHeader values\n\n"
        );
    } else {
        p!(out, "Header\n\n");
    }
    // Print first as ktxCheckHeader1_ modifies the header.
    write_ktx_header(out, header);

    if let Err(e) = ktx_check_header1(header) {
        p!(out, "The KTX 1 file pHeader is invalid:\n");
        match e {
            ErrorCode::FileDataError => {
                p!(
                    out,
                    "  it has invalid data such as bad glTypeSize, improper dimensions,\nimproper number of faces or too many levels.\n"
                );
            }
            ErrorCode::UnsupportedFeature => {
                p!(out, "  it describes an unsupported feature or format\n");
            }
            _ => {}
        }
        return;
    }

    if header.bytes_of_key_value_data != 0 {
        p!(out, "\nKey/Value Data\n\n");
        // C ignores a failed read and parses the (uninitialized) buffer; any
        // buffer that is not valid key/value data fails to parse.
        match read_vec(stream, u64::from(header.bytes_of_key_value_data)) {
            Ok(kvd) => write_kv_data(out, &kvd),
            Err(_) => write_kv_data(out, &[0]),
        }
    } else {
        p!(out, "\nNo Key/Value data.\n");
    }

    let level_count = header.number_of_mip_levels.max(1);
    let non_array_cubemap = header.number_of_array_elements == 0 && header.number_of_faces == 6;
    let mut data_size: u64 = 0;
    // A note about padding: Since KTX requires a row alignment of 4 for
    // uncompressed and all block-compressed formats have block sizes that
    // are a multiple of 4, all levels and faces will also be a multiple
    // of 4 so mipPadding and facePadding will always be 0. So they are
    // ignored here.
    p!(out, "\nData Sizes (bytes)\n------------------\n");
    for level in 0..level_count {
        let mut b = [0u8; 4];
        let _ = stream.read(&mut b);
        let mut face_lod_size = u32::from_le_bytes(b);
        if header.endianness == KTX_ENDIAN_REF_REV {
            face_lod_size = face_lod_size.swap_bytes();
        }
        let lod_size = if non_array_cubemap {
            face_lod_size.wrapping_mul(6)
        } else {
            face_lod_size
        };
        let _ = stream.skip(u64::from(lod_size));
        data_size = data_size.wrapping_add(u64::from(lod_size));
        p!(out, "Level {level}: {lod_size}\n");
    }
    p!(out, "\nTotal: {}\n", data_size as i64);
}

/// Reads the 12-byte identifier; `Ok(None)` means the read itself failed.
fn read_identifier(stream: &mut dyn Stream) -> KtxResult<[u8; 12]> {
    let mut id = [0u8; 12];
    stream.read(&mut id)?;
    Ok(id)
}

/// Reads the rest of a KTX 1 header after its identifier. On a failed read
/// the fields stay zero (uninitialized in C).
fn read_ktx1_rest(stream: &mut dyn Stream, id: [u8; 12]) -> (KtxHeader, KtxResult<()>) {
    let mut b = [0u8; KTX_HEADER_SIZE];
    b[..12].copy_from_slice(&id);
    let result = stream.read(&mut b[12..]);
    if result.is_err() {
        b[12..].fill(0);
    }
    (KtxHeader::from_bytes(&b), result)
}

fn read_ktx2_rest(stream: &mut dyn Stream, id: [u8; 12]) -> KtxResult<KtxHeader2> {
    let mut b = [0u8; KTX2_HEADER_SIZE];
    b[..12].copy_from_slice(&id);
    stream.read(&mut b[12..])?;
    Ok(KtxHeader2::from_bytes(&b))
}

/// Port of `ktxPrintKTX1InfoTextForStream`.
pub fn ktx_write_ktx1_info_text_for_stream(
    out: &mut Vec<u8>,
    stream: &mut dyn Stream,
) -> KtxResult<()> {
    let id = read_identifier(stream)?;
    // Compare identifier, is this a KTX  or KTX2 file?
    if id != KTX_IDENTIFIER_REF {
        return Err(ErrorCode::UnknownFileFormat);
    }
    let (mut header, result) = read_ktx1_rest(stream, id);
    write_ktx_info2(out, stream, &mut header);
    result
}

/// Port of `printKTX2Header`.
pub fn write_ktx2_header(out: &mut Vec<u8>, header: &KtxHeader2) {
    p!(out, "identifier: ");
    write_identifier(out, &header.identifier, false);
    p!(out, "\n");
    let vk_format_str = vk_format_string(header.vk_format);
    if vk_format_str == "VK_UNKNOWN_FORMAT" {
        p!(out, "vkFormat: 0x{:08X}\n", header.vk_format.0);
    } else {
        p!(out, "vkFormat: {vk_format_str}\n");
    }
    p!(out, "typeSize: {}\n", header.type_size);
    p!(out, "pixelWidth: {}\n", header.pixel_width);
    p!(out, "pixelHeight: {}\n", header.pixel_height);
    p!(out, "pixelDepth: {}\n", header.pixel_depth);
    p!(out, "layerCount: {}\n", header.layer_count);
    p!(out, "faceCount: {}\n", header.face_count);
    p!(out, "levelCount: {}\n", header.level_count);
    let scheme = header.supercompression_scheme.0;
    match ktx_supercompression_scheme_string(header.supercompression_scheme) {
        "Invalid scheme value" => {
            p!(
                out,
                "supercompressionScheme: Invalid scheme (0x{scheme:X})\n"
            );
        }
        "Vendor or reserved scheme" => {
            p!(
                out,
                "supercompressionScheme: Vendor or reserved scheme (0x{scheme:X})\n"
            );
        }
        s => {
            p!(out, "supercompressionScheme: {s}\n");
        }
    }
    let dfd = header.data_format_descriptor;
    let kvd = header.key_value_data;
    let sgd = header.supercompression_global_data;
    p!(
        out,
        "dataFormatDescriptor.byteOffset: {}\n",
        c_hash_x(dfd.byte_offset)
    );
    p!(
        out,
        "dataFormatDescriptor.byteLength: {}\n",
        dfd.byte_length
    );
    p!(
        out,
        "keyValueData.byteOffset: {}\n",
        c_hash_x(kvd.byte_offset)
    );
    p!(out, "keyValueData.byteLength: {}\n", kvd.byte_length);
    p!(
        out,
        "supercompressionGlobalData.byteOffset: {}\n",
        c_hash_x(sgd.byte_offset)
    );
    p!(
        out,
        "supercompressionGlobalData.byteLength: {}\n",
        sgd.byte_length as i64
    );
}

/// Port of `printLevelIndex`.
pub fn write_level_index(out: &mut Vec<u8>, level_index: &[LevelIndexEntry], num_levels: u32) {
    // Print at most 64 levels to stop parsing garbage
    for level in 0..num_levels.min(MAX_NUM_LEVELS) {
        let e = level_index.get(level as usize).copied().unwrap_or_default();
        p!(
            out,
            "Level{level}.byteOffset: {}\n",
            c_hash_x(e.byte_offset)
        );
        p!(out, "Level{level}.byteLength: {}\n", e.byte_length as i64);
        p!(
            out,
            "Level{level}.uncompressedByteLength: {}\n",
            e.uncompressed_byte_length as i64
        );
    }
}

/// Port of `printBasisSGDInfo`.
pub fn write_basis_sgd_info(out: &mut Vec<u8>, bgd: &[u8], num_images: u32) {
    let byte_length = bgd.len() as u64;
    if byte_length < BASIS_LZ_GLOBAL_HEADER_SIZE {
        return;
    }
    p!(out, "endpointCount: {}\n", rd_u32(bgd, 0) & 0xFFFF);
    p!(out, "selectorCount: {}\n", rd_u32(bgd, 2) & 0xFFFF);
    p!(out, "endpointsByteLength: {}\n", rd_u32(bgd, 4));
    p!(out, "selectorsByteLength: {}\n", rd_u32(bgd, 8));
    p!(out, "tablesByteLength: {}\n", rd_u32(bgd, 12));
    p!(out, "extendedByteLength: {}\n", rd_u32(bgd, 16));

    for i in 0..u64::from(num_images) {
        if byte_length < (i + 1) * BASIS_LZ_ETC1S_IMAGE_DESC_SIZE + BASIS_LZ_GLOBAL_HEADER_SIZE {
            break;
        }
        let o = BASIS_LZ_GLOBAL_HEADER_SIZE + i * BASIS_LZ_ETC1S_IMAGE_DESC_SIZE;
        p!(out, "\nimageFlags: {}\n", c_hash_x(rd_u32(bgd, o)));
        p!(out, "rgbSliceByteLength: {}\n", rd_u32(bgd, o + 8));
        p!(
            out,
            "rgbSliceByteOffset: {}\n",
            c_hash_x(rd_u32(bgd, o + 4))
        );
        p!(out, "alphaSliceByteLength: {}\n", rd_u32(bgd, o + 16));
        p!(
            out,
            "alphaSliceByteOffset: {}\n",
            c_hash_x(rd_u32(bgd, o + 12))
        );
    }
}

/// Port of `printUASTCHDR6X6IntermediateSGDInfo`.
pub fn write_uastc_hdr_6x6_intermediate_sgd_info(out: &mut Vec<u8>, bgd: &[u8], num_images: u32) {
    let byte_length = bgd.len() as u64;
    for i in 0..u64::from(num_images) {
        if byte_length < (i + 1) * UASTC_HDR_6X6_IMAGE_DESC_SIZE {
            break;
        }
        let o = i * UASTC_HDR_6X6_IMAGE_DESC_SIZE;
        p!(out, "\nrgbSliceByteLength: {}\n", rd_u32(bgd, o + 4));
        p!(out, "rgbSliceByteOffset: {}\n", c_hash_x(rd_u32(bgd, o)));
        p!(out, "rgbSliceType: {}\n", c_hash_x(rd_u32(bgd, o + 8)));
    }
}

/// Number of images covered by the supercompression global data.
fn sgd_num_images(header: &KtxHeader2) -> u32 {
    let layers_faces = header.layer_count.max(1).wrapping_mul(header.face_count);
    let depth = header.pixel_depth.max(1);
    let mut layer_pixel_depth = depth;
    for level in 1..header.level_count.max(1) {
        layer_pixel_depth = layer_pixel_depth.wrapping_add(depth.wrapping_shr(level).max(1));
    }
    // NOTA BENE: faceCount * layerPixelDepth is only reasonable because
    // faceCount and depth can't both be > 1. I.e there are no 3d cubemaps.
    layers_faces.wrapping_mul(layer_pixel_depth)
}

/// Reads the level index (`sizeof(ktxLevelIndexEntry) * numLevels` bytes,
/// computed in 32 bits like C).
fn read_level_index(stream: &mut dyn Stream, num_levels: u32) -> KtxResult<Vec<LevelIndexEntry>> {
    let size = (LEVEL_INDEX_ENTRY_SIZE as u32).wrapping_mul(num_levels);
    let bytes = read_vec(stream, u64::from(size))?;
    Ok(bytes
        .chunks(LEVEL_INDEX_ENTRY_SIZE)
        .filter_map(LevelIndexEntry::from_bytes)
        .collect())
}

fn read_sgd(stream: &mut dyn Stream, header: &KtxHeader2) -> KtxResult<Vec<u8>> {
    let sgd = header.supercompression_global_data;
    stream.setpos(sgd.byte_offset)?;
    read_vec(stream, sgd.byte_length)
}

/// Port of `printKTX2Info2`. The stream must be just past the header.
fn write_ktx2_info2(
    out: &mut Vec<u8>,
    stream: &mut dyn Stream,
    header: &KtxHeader2,
) -> KtxResult<()> {
    let dfd_idx = header.data_format_descriptor;
    let kvd_idx = header.key_value_data;
    let sgd_idx = header.supercompression_global_data;
    let has_dfd = dfd_idx.byte_offset != 0 && dfd_idx.byte_length != 0;
    let has_kvd = kvd_idx.byte_offset != 0 && kvd_idx.byte_length != 0;
    let has_sgd = sgd_idx.byte_offset != 0 && sgd_idx.byte_length != 0;

    p!(out, "Header\n\n");
    write_ktx2_header(out, header);

    p!(out, "\nLevel Index\n\n");
    let num_levels = header.level_count.max(1);
    let level_index = read_level_index(stream, num_levels)?;
    write_level_index(out, &level_index, num_levels);

    if has_dfd {
        p!(out, "\nData Format Descriptor\n\n");
        let dfd = to_words(&read_vec(stream, u64::from(dfd_idx.byte_length))?);
        if dfd.first().copied().unwrap_or(0) != dfd_idx.byte_length {
            return Err(ErrorCode::FileDataError);
        }
        let mut s = String::new();
        write_dfd(&mut s, &dfd, dfd_idx.byte_length);
        out.extend_from_slice(s.as_bytes());
    }

    if has_kvd {
        p!(out, "\nKey/Value Data\n\n");
        let kvd = read_vec(stream, u64::from(kvd_idx.byte_length))?;
        write_kv_data(out, &kvd);
    } else {
        p!(out, "\nNo Key/Value data.\n");
    }

    if has_sgd {
        match header.supercompression_scheme {
            SupercmpScheme::BASIS_LZ => {
                let sgd = read_sgd(stream, header)?;
                p!(out, "\nBasis Supercompression Global Data\n\n");
                write_basis_sgd_info(out, &sgd, sgd_num_images(header));
            }
            SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE => {
                let sgd = read_sgd(stream, header)?;
                p!(
                    out,
                    "\nUASTC HDR6X6 Intermediate Supercompression Global Data\n\n"
                );
                write_uastc_hdr_6x6_intermediate_sgd_info(out, &sgd, sgd_num_images(header));
            }
            _ => {
                p!(out, "\nUnrecognized supercompressionScheme.\n");
            }
        }
    }
    Ok(())
}

/// Port of `printKTX2Info2JSON`. The stream must be just past the header.
fn write_ktx2_info2_json(
    out: &mut Vec<u8>,
    stream: &mut dyn Stream,
    header: &KtxHeader2,
    mut base_indent: u32,
    mut indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    if minified {
        base_indent = 0;
        indent_width = 0;
    }
    let space = if minified { "" } else { " " };
    let nl = if minified { "" } else { "\n" };
    let i = |indent: u32| ind(base_indent, indent, indent_width);
    let (i0, i1, i2, i3) = (i(0), i(1), i(2), i(3));

    let dfd_idx = header.data_format_descriptor;
    let kvd_idx = header.key_value_data;
    let sgd_idx = header.supercompression_global_data;
    let has_dfd = dfd_idx.byte_offset != 0 && dfd_idx.byte_length != 0;
    let has_kvd = kvd_idx.byte_offset != 0 && kvd_idx.byte_length != 0;
    let has_sgd = sgd_idx.byte_offset != 0 && sgd_idx.byte_length != 0;

    p!(out, "{:i0$}\"header\":{space}{{{nl}", "");
    p!(out, "{:i1$}\"identifier\":{space}\"", "");
    write_identifier(out, &header.identifier, true);
    p!(out, "\",{nl}");
    let vk_format_str = vk_format_string(header.vk_format);
    if vk_format_str == "VK_UNKNOWN_FORMAT" {
        p!(
            out,
            "{:i1$}\"vkFormat\":{space}{},{nl}",
            "",
            header.vk_format.0
        );
    } else {
        p!(
            out,
            "{:i1$}\"vkFormat\":{space}\"{vk_format_str}\",{nl}",
            ""
        );
    }
    p!(
        out,
        "{:i1$}\"typeSize\":{space}{},{nl}",
        "",
        header.type_size
    );
    p!(
        out,
        "{:i1$}\"pixelWidth\":{space}{},{nl}",
        "",
        header.pixel_width
    );
    p!(
        out,
        "{:i1$}\"pixelHeight\":{space}{},{nl}",
        "",
        header.pixel_height
    );
    p!(
        out,
        "{:i1$}\"pixelDepth\":{space}{},{nl}",
        "",
        header.pixel_depth
    );
    p!(
        out,
        "{:i1$}\"layerCount\":{space}{},{nl}",
        "",
        header.layer_count
    );
    p!(
        out,
        "{:i1$}\"faceCount\":{space}{},{nl}",
        "",
        header.face_count
    );
    p!(
        out,
        "{:i1$}\"levelCount\":{space}{},{nl}",
        "",
        header.level_count
    );
    let scheme = header.supercompression_scheme;
    match ktx_supercompression_scheme_string(scheme) {
        "Invalid scheme value" | "Vendor or reserved scheme" => {
            p!(
                out,
                "{:i1$}\"supercompressionScheme\":{space}{}{nl}",
                "",
                scheme.0
            );
        }
        s => {
            p!(
                out,
                "{:i1$}\"supercompressionScheme\":{space}\"{s}\"{nl}",
                ""
            );
        }
    }
    p!(out, "{:i0$}}}", "");

    let num_levels = header.level_count.max(1);
    let level_index = match read_level_index(stream, num_levels) {
        Ok(l) => l,
        Err(e) => {
            p!(out, "{nl}");
            return Err(e);
        }
    };

    p!(out, ",{nl}");
    p!(out, "{:i0$}\"index\":{space}{{{nl}", "");
    let entry32 = |out: &mut Vec<u8>, name: &str, ofs: u32, len: u32| {
        p!(out, "{:i1$}\"{name}\":{space}{{{nl}", "");
        p!(out, "{:i2$}\"byteOffset\":{space}{ofs},{nl}", "");
        p!(out, "{:i2$}\"byteLength\":{space}{len}{nl}", "");
        p!(out, "{:i1$}}},{nl}", "");
    };
    entry32(
        out,
        "dataFormatDescriptor",
        dfd_idx.byte_offset,
        dfd_idx.byte_length,
    );
    entry32(
        out,
        "keyValueData",
        kvd_idx.byte_offset,
        kvd_idx.byte_length,
    );
    p!(
        out,
        "{:i1$}\"supercompressionGlobalData\":{space}{{{nl}",
        ""
    );
    p!(
        out,
        "{:i2$}\"byteOffset\":{space}{},{nl}",
        "",
        sgd_idx.byte_offset as i64
    );
    p!(
        out,
        "{:i2$}\"byteLength\":{space}{}{nl}",
        "",
        sgd_idx.byte_length as i64
    );
    p!(out, "{:i1$}}},{nl}", "");

    p!(out, "{:i1$}\"levels\":{space}[{nl}", "");
    // Print at most 64 levels to stop parsing garbage
    let shown = num_levels.min(MAX_NUM_LEVELS);
    for level in 0..shown {
        let e = level_index.get(level as usize).copied().unwrap_or_default();
        p!(out, "{:i2$}{{{nl}", "");
        p!(
            out,
            "{:i3$}\"byteOffset\":{space}{},{nl}",
            "",
            e.byte_offset as i64
        );
        p!(
            out,
            "{:i3$}\"byteLength\":{space}{},{nl}",
            "",
            e.byte_length as i64
        );
        p!(
            out,
            "{:i3$}\"uncompressedByteLength\":{space}{}{nl}",
            "",
            e.uncompressed_byte_length as i64
        );
        let comma = if level + 1 == shown { "" } else { "," };
        p!(out, "{:i2$}}}{comma}{nl}", "");
    }
    p!(out, "{:i1$}]{nl}", ""); // End of levels
    p!(out, "{:i0$}}}", ""); // End of index

    if has_dfd {
        let dfd = match read_vec(stream, u64::from(dfd_idx.byte_length)) {
            Ok(d) => to_words(&d),
            Err(e) => {
                p!(out, "{nl}");
                return Err(e);
            }
        };
        p!(out, ",{nl}");
        p!(out, "{:i0$}\"dataFormatDescriptor\":{space}{{{nl}", "");
        let mut s = String::new();
        write_dfd_json(
            &mut s,
            &dfd,
            dfd_idx.byte_length,
            base_indent.wrapping_add(1),
            indent_width,
            minified,
        );
        out.extend_from_slice(s.as_bytes());
        p!(out, "{:i0$}}}", "");
    }

    if has_kvd {
        let kvd = match read_vec(stream, u64::from(kvd_idx.byte_length)) {
            Ok(d) => d,
            Err(e) => {
                p!(out, "{nl}");
                return Err(e);
            }
        };
        p!(out, ",{nl}");
        p!(out, "{:i0$}\"keyValueData\":{space}{{{nl}", "");
        write_kv_data_json(
            out,
            &kvd,
            base_indent.wrapping_add(1),
            indent_width,
            minified,
        );
        p!(out, "{:i0$}}}", "");
    }

    if has_sgd {
        p!(out, ",{nl}");
        p!(
            out,
            "{:i0$}\"supercompressionGlobalData\":{space}{{{nl}",
            ""
        );
        match scheme {
            SupercmpScheme::NONE => {
                p!(out, "{:i1$}\"type\":{space}\"KTX_SS_NONE\"{nl}", "");
            }
            SupercmpScheme::BASIS_LZ => {
                p!(out, "{:i1$}\"type\":{space}\"KTX_SS_BASIS_LZ\"", "");
                let sgd = match read_sgd(stream, header) {
                    Ok(s) => s,
                    Err(e) => {
                        p!(out, "{nl}");
                        p!(out, "{:i0$}}}{nl}", "");
                        return Err(e);
                    }
                };
                let num_images = sgd_num_images(header);
                let sgd_len = sgd.len() as u64;
                if sgd_len < BASIS_LZ_GLOBAL_HEADER_SIZE {
                    p!(out, "{nl}");
                    p!(out, "{:i0$}}}{nl}", "");
                    return Ok(());
                }
                p!(out, ",{nl}");
                p!(
                    out,
                    "{:i1$}\"endpointCount\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 0) & 0xFFFF
                );
                p!(
                    out,
                    "{:i1$}\"selectorCount\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 2) & 0xFFFF
                );
                p!(
                    out,
                    "{:i1$}\"endpointsByteLength\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 4)
                );
                p!(
                    out,
                    "{:i1$}\"selectorsByteLength\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 8)
                );
                p!(
                    out,
                    "{:i1$}\"tablesByteLength\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 12)
                );
                p!(
                    out,
                    "{:i1$}\"extendedByteLength\":{space}{},{nl}",
                    "",
                    rd_u32(&sgd, 16)
                );
                p!(out, "{:i1$}\"images\":{space}[", "");
                for img in 0..u64::from(num_images) {
                    if sgd_len
                        < (img + 1) * BASIS_LZ_ETC1S_IMAGE_DESC_SIZE + BASIS_LZ_GLOBAL_HEADER_SIZE
                    {
                        break;
                    }
                    if img == 0 {
                        p!(out, "{nl}");
                    } else {
                        p!(out, ",{nl}");
                    }
                    let o = BASIS_LZ_GLOBAL_HEADER_SIZE + img * BASIS_LZ_ETC1S_IMAGE_DESC_SIZE;
                    p!(out, "{:i2$}{{{nl}", "");
                    let image_flags = rd_u32(&sgd, o);
                    if image_flags == 0 {
                        p!(out, "{:i3$}\"imageFlags\":{space}[],{nl}", "");
                    } else {
                        p!(out, "{:i3$}\"imageFlags\":{space}[{nl}", "");
                        write_flag_bits_json(
                            out,
                            i(4),
                            nl,
                            image_flags,
                            ktx_bu_image_flags_bit_string,
                        );
                        p!(out, "{:i3$}],{nl}", "");
                    }
                    p!(
                        out,
                        "{:i3$}\"rgbSliceByteLength\":{space}{},{nl}",
                        "",
                        rd_u32(&sgd, o + 8)
                    );
                    p!(
                        out,
                        "{:i3$}\"rgbSliceByteOffset\":{space}{},{nl}",
                        "",
                        rd_u32(&sgd, o + 4)
                    );
                    p!(
                        out,
                        "{:i3$}\"alphaSliceByteLength\":{space}{},{nl}",
                        "",
                        rd_u32(&sgd, o + 16)
                    );
                    p!(
                        out,
                        "{:i3$}\"alphaSliceByteOffset\":{space}{}{nl}",
                        "",
                        rd_u32(&sgd, o + 12)
                    );
                    p!(out, "{:i2$}}}", "");
                }
                p!(out, "{nl}");
                p!(out, "{:i1$}]{nl}", "");
            }
            SupercmpScheme::ZSTD => {
                p!(out, "{:i1$}\"type\":{space}\"KTX_SS_ZSTD\"{nl}", "");
            }
            SupercmpScheme::ZLIB => {
                p!(out, "{:i1$}\"type\":{space}\"KTX_SS_ZLIB\"{nl}", "");
            }
            s => {
                p!(out, "{:i1$}\"type\":{space}{}{nl}", "", s.0);
            }
        }
        p!(out, "{:i0$}}}", "");
    }
    p!(out, "{nl}");
    Ok(())
}

/// Port of `ktxPrintInfoForStream`: detects KTX or KTX2 and prints its info.
pub fn ktx_write_info_for_stream(out: &mut Vec<u8>, stream: &mut dyn Stream) -> KtxResult<()> {
    let id = read_identifier(stream)?;
    // Compare identifier, is this a KTX  or KTX2 file?
    if id == KTX_IDENTIFIER_REF {
        let (mut header, result) = read_ktx1_rest(stream, id);
        write_ktx_info2(out, stream, &mut header);
        result
    } else if id == KTX2_IDENTIFIER_REF {
        let header = read_ktx2_rest(stream, id)?;
        write_ktx2_info2(out, stream, &header)
    } else {
        Err(ErrorCode::UnknownFileFormat)
    }
}

/// Port of `ktxPrintKTX2InfoJSONForStream`.
pub fn ktx_write_ktx2_info_json_for_stream(
    out: &mut Vec<u8>,
    stream: &mut dyn Stream,
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    let id = read_identifier(stream)?;
    // Compare identifier, is this a KTX2 file?
    if id != KTX2_IDENTIFIER_REF {
        return Err(ErrorCode::UnknownFileFormat);
    }
    let header = read_ktx2_rest(stream, id)?;
    write_ktx2_info2_json(out, stream, &header, base_indent, indent_width, minified)
}

/// Port of `ktxPrintKTX2InfoTextForStream`.
pub fn ktx_write_ktx2_info_text_for_stream(
    out: &mut Vec<u8>,
    stream: &mut dyn Stream,
) -> KtxResult<()> {
    let id = read_identifier(stream)?;
    // Compare identifier, is this a KTX2 file?
    if id != KTX2_IDENTIFIER_REF {
        return Err(ErrorCode::UnknownFileFormat);
    }
    let header = read_ktx2_rest(stream, id)?;
    write_ktx2_info2(out, stream, &header)
}

/// Runs a writer and copies its output to stdout, like the C `printf`s.
fn to_stdout(f: impl FnOnce(&mut Vec<u8>) -> KtxResult<()>) -> KtxResult<()> {
    let mut out = Vec::new();
    let result = f(&mut out);
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&out);
    let _ = stdout.flush();
    result
}

/// `ktxFileStream_construct(stream, file, KTX_FALSE)`: the caller keeps `file`.
fn borrowed_file_stream(file: &File) -> KtxResult<FileStream> {
    let file = file.try_clone().map_err(|_| ErrorCode::FileOpenFailed)?;
    FileStream::construct(file)
}

/// Port of `ktxPrintInfoForStream`.
pub fn ktx_print_info_for_stream(stream: &mut dyn Stream) -> KtxResult<()> {
    to_stdout(|out| ktx_write_info_for_stream(out, stream))
}

/// Port of `ktxPrintKTX1InfoTextForStream`.
pub fn ktx_print_ktx1_info_text_for_stream(stream: &mut dyn Stream) -> KtxResult<()> {
    to_stdout(|out| ktx_write_ktx1_info_text_for_stream(out, stream))
}

/// Port of `ktxPrintKTX2InfoJSONForStream`.
pub fn ktx_print_ktx2_info_json_for_stream(
    stream: &mut dyn Stream,
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    to_stdout(|out| {
        ktx_write_ktx2_info_json_for_stream(out, stream, base_indent, indent_width, minified)
    })
}

/// Port of `ktxPrintKTX2InfoTextForStream`.
pub fn ktx_print_ktx2_info_text_for_stream(stream: &mut dyn Stream) -> KtxResult<()> {
    to_stdout(|out| ktx_write_ktx2_info_text_for_stream(out, stream))
}

/// Port of `ktxPrintInfoForStdioStream`. Reads from the file's current position.
pub fn ktx_print_info_for_stdio_stream(file: &File) -> KtxResult<()> {
    let mut stream = borrowed_file_stream(file)?;
    ktx_print_info_for_stream(&mut stream)
}

/// Port of `ktxPrintInfoForNamedFile`. Like C (a TODO there), prints nothing.
pub fn ktx_print_info_for_named_file(_filename: &str) -> KtxResult<()> {
    Ok(())
}

/// Port of `ktxPrintInfoForMemory`.
pub fn ktx_print_info_for_memory(bytes: &[u8]) -> KtxResult<()> {
    let mut stream = MemStream::construct_ro(bytes)?;
    ktx_print_info_for_stream(&mut stream)
}

/// Port of `ktxPrintKTX2InfoJSONForStdioStream`.
pub fn ktx_print_ktx2_info_json_for_stdio_stream(
    file: &File,
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    let mut stream = borrowed_file_stream(file)?;
    ktx_print_ktx2_info_json_for_stream(&mut stream, base_indent, indent_width, minified)
}

/// Port of `ktxPrintKTX2InfoJSONForNamedFile`.
pub fn ktx_print_ktx2_info_json_for_named_file(
    filename: &str,
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    let file = File::open(filename).map_err(|_| ErrorCode::FileOpenFailed)?;
    ktx_print_ktx2_info_json_for_stdio_stream(&file, base_indent, indent_width, minified)
}

/// Port of `ktxPrintKTX2InfoJSONForMemory`.
pub fn ktx_print_ktx2_info_json_for_memory(
    bytes: &[u8],
    base_indent: u32,
    indent_width: u32,
    minified: bool,
) -> KtxResult<()> {
    let mut stream = MemStream::construct_ro(bytes)?;
    ktx_print_ktx2_info_json_for_stream(&mut stream, base_indent, indent_width, minified)
}

/// Port of `ktxPrintKTX2InfoTextForStdioStream`.
pub fn ktx_print_ktx2_info_text_for_stdio_stream(file: &File) -> KtxResult<()> {
    let mut stream = borrowed_file_stream(file)?;
    ktx_print_ktx2_info_text_for_stream(&mut stream)
}

/// Port of `ktxPrintKTX2InfoTextForNamedFile`.
pub fn ktx_print_ktx2_info_text_for_named_file(filename: &str) -> KtxResult<()> {
    let file = File::open(filename).map_err(|_| ErrorCode::FileOpenFailed)?;
    ktx_print_ktx2_info_text_for_stdio_stream(&file)
}

/// Port of `ktxPrintKTX2InfoTextForMemory`.
pub fn ktx_print_ktx2_info_text_for_memory(bytes: &[u8]) -> KtxResult<()> {
    let mut stream = MemStream::construct_ro(bytes)?;
    ktx_print_ktx2_info_text_for_stream(&mut stream)
}
