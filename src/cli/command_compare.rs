// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/command_compare.cpp (`ktx compare`).
//!
//! Output goes to one in-memory stdout buffer that is printed before
//! returning, also on errors (the C++ streams to `std::cout` as it goes).
//! Structure reads beyond the loaded DFD / SGD bytes give zeros where the C++
//! reads past its buffers.

use super::command::*;
use super::command_validate::write_issue_text;
use super::fmt::float_shortest;
use super::format_descriptor::FormatDescriptor;
use super::image_codec::{ImageCodec, ImageSpan};
use super::options::{OptionSpec, Options, ParseResult};
use super::utility::{PrintIndent, escape_json_copy, fmt_in_file};
use super::validate::{MAX_NUM_BDFD_SAMPLES, MAX_NUM_DFD_BLOCKS, MAX_NUM_KV_ENTRIES};
use super::validate::{ValidationReport, validate_memory};
use crate::dfd::khr_df::*;
use crate::dfd::printdfd::*;
use crate::hashlist::HashList;
use crate::ktxint::{KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry};
use crate::strings::{ktx_bu_image_flags_bit_string, ktx_supercompression_scheme_string};
use crate::texture::KtxTexture;
use crate::vkformat::vk_format_string;
use crate::{
    KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, SupercmpScheme, Texture2, TranscodeFmt, VkFormat,
    ktx_error_string,
};
use std::collections::{BTreeMap, HashSet};

const KTX2_HEADER_SIZE: u64 = 80;

/// (text header, JSON path, accessor) of a plain header field.
type HeaderField = (&'static str, &'static str, fn(&KtxHeader2) -> u32);
const BASIS_LZ_GLOBAL_HEADER_SIZE: u64 = 20;
const BASIS_LZ_IMAGE_DESC_SIZE: u64 = 20;

/// `CommandCompare::ContentMode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ContentMode {
    #[default]
    Raw,
    Image,
    Ignore,
}

/// `CommandCompare::IgnoreIndex`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum IgnoreIndex {
    All,
    Level,
    #[default]
    None,
}

/// `CommandCompare::IgnoreDFD`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum IgnoreDfd {
    All,
    AllExceptColorSpace,
    Unknown,
    Extended,
    #[default]
    None,
}

/// `CommandCompare::IgnoreSGD`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum IgnoreSgd {
    All,
    Unknown,
    Payload,
    #[default]
    None,
}

/// `CommandCompare::OptionsCompare`.
#[derive(Debug, Clone, Default)]
struct OptionsCompare {
    input_filepaths: [String; 2],
    content_mode: ContentMode,
    per_pixel_output_limit: usize,
    allow_invalid_input: bool,
    ignore_format_header: bool,
    ignore_supercomp: bool,
    ignore_index: IgnoreIndex,
    ignore_dfd: IgnoreDfd,
    ignore_bdfd_bytes_plane: bool,
    ignore_all_metadata: bool,
    ignore_metadata_keys: HashSet<Vec<u8>>,
    ignore_sgd: IgnoreSgd,
}

impl OptionsCompare {
    const K_CONTENT: &'static str = "content";
    const K_PER_PIXEL_OUTPUT: &'static str = "per-pixel-output";
    const K_ALLOW_INVALID_INPUT: &'static str = "allow-invalid-input";
    const K_IGNORE_FORMAT_HEADER: &'static str = "ignore-format-header";
    const K_IGNORE_SUPERCOMP: &'static str = "ignore-supercomp";
    const K_IGNORE_INDEX: &'static str = "ignore-index";
    const K_IGNORE_DFD: &'static str = "ignore-dfd";
    const K_IGNORE_BDFD_BYTES_PLANE: &'static str = "ignore-bdfd-bytesplane";
    const K_IGNORE_METADATA: &'static str = "ignore-metadata";
    const K_IGNORE_SGD: &'static str = "ignore-sgd";
}

/// `std::stoull(str, &pos)` (base 10): the parsed value and the number of
/// characters used, or `None` where it throws.
fn stoull(s: &str) -> Option<(u64, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while b
        .get(i)
        .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
    {
        i += 1;
    }
    let negative = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut value = 0u64;
    let mut overflow = false;
    while let Some(c) = b.get(i).filter(|c| c.is_ascii_digit()) {
        match value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(c - b'0')))
        {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start || overflow {
        return None;
    }
    Some((
        if negative {
            value.wrapping_neg()
        } else {
            value
        },
        i,
    ))
}

impl OptionGroup for OptionsCompare {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            "input-file1",
            "The first input file to compare.",
            "filepath",
        ));
        opts.add(OptionSpec::value(
            "input-file2",
            "The second input file to compare.",
            "filepath",
        ));
        opts.add(
            OptionSpec::value(
                Self::K_CONTENT,
                "Controls how image content is compared. Possible values are:\n\
                 \x20 raw: Encoded image data is compared verbatim, as it appears in the file\n\
                 \x20 image: Effective image data is compared per texel block\n\
                 \x20 ignore: Ignore image contents\n\
                 Note: When the mode is set to @b image, the effective image data of individual mip \
                 levels, layers, and faces is compared texel block by texel block after applying \
                 one or more of the following transformations, if needed:\n\
                 - If the texture is supercompressed (e.g. using Zstandard or ZLIB), the images \
                 are deflated before comparison.\n\
                 - If the texture is encoded using BasisLZ, the images are transcoded to \
                 R8G8B8A_UNORM before comparison.\n\
                 For block compressed textures and textures encoded using UASTC the texel blocks are \
                 the individual compressed blocks, while for other formats the texel blocks are the \
                 individual pixels of the image.\n\
                 When comparing files that use different supercompression schemes, or otherwise \
                 different encoding, additional options may need to be used to avoid unexpected \
                 differences to be reported related to the meta information of the files. \
                 For example:\n\
                 - The supercompression scheme can be ignored with --ignore-supercomp\n\
                 - Compressed byte length and other index section differences can be ignored \
                 with --ignore-index all or --ignore-index level\n\
                 - DFD section differences can be ignored with --ignore-dfd all or --ignore-dfd \
                 all-except-color-space\n\
                 - SGD section differences can be ignored with --ignore-sgd all or --ignore-sgd \
                 payload\n",
                "raw|image|ignore",
            )
            .default_value("raw"),
        );
        opts.add(
            OptionSpec::value(
                Self::K_PER_PIXEL_OUTPUT,
                "Controls whether per pixel / texel block difference output is generated when \
                 --content is set to image:\n\
                 \x20 all - Every single difference is output (may result in a very large output)\n\
                 \x20 <number> - At most the specified number of differences are output\n\
                 \x20 none - No per pixel / texel block differences are output\n",
                "all|<number>|none",
            )
            .default_value("none"),
        );
        opts.add(OptionSpec::flag(
            Self::K_ALLOW_INVALID_INPUT,
            "Perform best effort comparison even if any of the input files are invalid.",
        ));
        opts.add(OptionSpec::flag(
            Self::K_IGNORE_FORMAT_HEADER,
            "Ignore the vkFormat and typeSize fields in the file header.\n\
             Note: useful when comparing textures with and without BasisLZ encoding, respectively.",
        ));
        opts.add(OptionSpec::flag(
            Self::K_IGNORE_SUPERCOMP,
            "Ignore supercompression scheme in the file header.\n\
             Note: use the --ignore-sgd option to also ignore the SGD section, if needed.",
        ));
        opts.add(
            OptionSpec::value(
                Self::K_IGNORE_INDEX,
                "Controls the comparison of index entries in the file headers. Possible options are:\n\
                 \x20 all: Ignore all index entries\n\
                 \x20 level: Ignore level index entries only\n\
                 \x20 none: Do not ignore any index entries\n",
                "all|level|none",
            )
            .default_value("none"),
        );
        opts.add(
            OptionSpec::value(
                Self::K_IGNORE_DFD,
                "Controls the comparison of DFD blocks. Possible options are:\n\
                 \x20 all: Ignore all DFD blocks\n\
                 \x20 all-except-color-space: Ignore all DFD blocks except the color space information \
                 in the basic DFD block\n\
                 \x20 unknown: Ignore any unrecognized DFD blocks\n\
                 \x20 extended: Ignore all DFD blocks except the basic DFD block\n\
                 \x20 none: Do not ignore any DFD blocks\n",
                "all|all-except-color-space|unknown|extended|none",
            )
            .default_value("none"),
        );
        opts.add(OptionSpec::flag(
            Self::K_IGNORE_BDFD_BYTES_PLANE,
            "Ignore BDFD bytesPlane values. Useful when comparing pre-spec-2.0.4 \
             supercompressed files with 2.0.4+ supercompressed or non-supercompressed ones, as bytesPlane \
             is set to zero in older supercompressed files.",
        ));
        opts.add(
            OptionSpec::value(
                Self::K_IGNORE_METADATA,
                "Controls the comparison of metadata (KVD) entries. Possible options are:\n\
                 \x20 all: Ignore all metadata entries\n\
                 \x20 <key>[,...]: Ignore the specified comma separated list of metadata keys\n\
                 \x20 none: Do not ignore any metadata entries\n",
                "all|<key>[,...]|none",
            )
            .default_value("none"),
        );
        opts.add(
            OptionSpec::value(
                Self::K_IGNORE_SGD,
                "Controls the comparison of the SGD section. Possible options are:\n\
                 \x20 all: Ignore the SGD section\n\
                 \x20 unknown: Ignore any unrecognized SGD section\n\
                 \x20 payload: Ignore any unrecognized SGD section and the payload of any known SGD section\n\
                 \x20 none: Do not ignore the SGD section\n\
                 Note: --ignore-sgd payload can be used to compare BasisLZ SGD headers without \
                 expecting an exact match for the individual SGD payload sections.",
                "all|unknown|payload|none",
            )
            .default_value("none"),
        );
        opts.parse_positional(&["input-file1", "input-file2"]);
        opts.positional_help("<input-file1> <input-file2>");
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count("input-file1") == 0 {
            return Err(report.fatal_usage("Missing input files."));
        }
        if args.count("input-file2") == 0 {
            return Err(report.fatal_usage("Missing second input file."));
        }
        self.input_filepaths = [
            args.get("input-file1").unwrap_or_default().to_string(),
            args.get("input-file2").unwrap_or_default().to_string(),
        ];
        let lower = |name: &str| args.get(name).unwrap_or_default().to_ascii_lowercase();

        if args.count(Self::K_CONTENT) > 0 {
            let s = lower(Self::K_CONTENT);
            self.content_mode = match s.as_str() {
                "raw" => ContentMode::Raw,
                "image" => ContentMode::Image,
                "ignore" => ContentMode::Ignore,
                _ => {
                    return Err(
                        report.fatal_usage(&format!("Invalid --content argument: \"{s}\"."))
                    );
                }
            };
        }

        if args.count(Self::K_PER_PIXEL_OUTPUT) > 0 {
            if self.content_mode != ContentMode::Image {
                return Err(report.fatal_usage(
                    "--per-pixel-output is specified but --content was not set to \"image\".",
                ));
            }
            let s = lower(Self::K_PER_PIXEL_OUTPUT);
            if s == "all" {
                self.per_pixel_output_limit = usize::MAX;
            } else if s == "none" {
                self.per_pixel_output_limit = 0;
            } else {
                let mut parsed_characters = 0;
                if let Some((v, n)) = stoull(&s) {
                    self.per_pixel_output_limit = v as usize;
                    parsed_characters = n;
                }
                if parsed_characters != s.len() {
                    return Err(report
                        .fatal_usage(&format!("Invalid --per-pixel-output arugment: \"{s}\".")));
                }
            }
        }

        self.allow_invalid_input = args.as_bool(Self::K_ALLOW_INVALID_INPUT);
        self.ignore_format_header = args.as_bool(Self::K_IGNORE_FORMAT_HEADER);
        self.ignore_supercomp = args.as_bool(Self::K_IGNORE_SUPERCOMP);

        if args.count(Self::K_IGNORE_INDEX) > 0 {
            let s = lower(Self::K_IGNORE_INDEX);
            self.ignore_index = match s.as_str() {
                "all" => IgnoreIndex::All,
                "level" => IgnoreIndex::Level,
                "none" => IgnoreIndex::None,
                _ => {
                    return Err(
                        report.fatal_usage(&format!("Invalid --ignore-index argument: \"{s}\"."))
                    );
                }
            };
        }

        if args.count(Self::K_IGNORE_DFD) > 0 {
            let s = lower(Self::K_IGNORE_DFD);
            self.ignore_dfd = match s.as_str() {
                "all" => IgnoreDfd::All,
                "all-except-color-space" => IgnoreDfd::AllExceptColorSpace,
                "unknown" => IgnoreDfd::Unknown,
                "extended" => IgnoreDfd::Extended,
                "none" => IgnoreDfd::None,
                _ => {
                    return Err(
                        report.fatal_usage(&format!("Invalid --ignore-dfd argument: \"{s}\"."))
                    );
                }
            };
        }

        self.ignore_bdfd_bytes_plane = args.as_bool(Self::K_IGNORE_BDFD_BYTES_PLANE);

        if args.count(Self::K_IGNORE_METADATA) > 0 {
            match lower(Self::K_IGNORE_METADATA).as_str() {
                "all" => self.ignore_all_metadata = true,
                "none" => self.ignore_all_metadata = false,
                _ => {
                    // Comma separated list of keys (std::getline drops a trailing empty item)
                    let list = args.get(Self::K_IGNORE_METADATA).unwrap_or_default();
                    let list = list.strip_suffix(',').unwrap_or(list);
                    if !list.is_empty() {
                        for key in list.split(',') {
                            self.ignore_metadata_keys.insert(key.as_bytes().to_vec());
                        }
                    }
                }
            }
        }

        if args.count(Self::K_IGNORE_SGD) > 0 {
            let s = lower(Self::K_IGNORE_SGD);
            self.ignore_sgd = match s.as_str() {
                "all" => IgnoreSgd::All,
                "unknown" => IgnoreSgd::Unknown,
                "payload" => IgnoreSgd::Payload,
                "none" => IgnoreSgd::None,
                _ => {
                    return Err(
                        report.fatal_usage(&format!("Invalid --ignore-sgd argument: \"{s}\"."))
                    );
                }
            };
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// A `DiffBase<T>` with its two values already formatted for the output format.
#[derive(Debug, Clone, Default)]
struct Diff {
    text_header: Vec<u8>,
    json_path: Vec<u8>,
    values: [Option<Vec<u8>>; 2],
    different: bool,
}

/// One `KVEntry` subclass instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KvKind {
    Uint32,
    String,
    GlFormat,
    AnimData,
    CubemapIncomplete,
    Unknown,
}

#[derive(Debug, Clone)]
struct KvEntry<'a> {
    kind: KvKind,
    value: &'a [u8],
}

/// `KVEntry::extractRawBytes`.
fn extract_raw_bytes(value: &[u8], text: bool, space: &str) -> String {
    let items: Vec<String> = value
        .iter()
        .map(|b| {
            if text {
                format!("0x{b:x}")
            } else {
                b.to_string()
            }
        })
        .collect();
    let s = items.join(&format!(",{space}"));
    if text {
        format!("[{s}]")
    } else if s.is_empty() {
        "[]".to_string()
    } else {
        format!("[{space}{s}{space}]")
    }
}

/// A NUL terminated `char*` value printed with `{}`.
fn c_str(value: &[u8]) -> &[u8] {
    let n = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    value.get(..n).unwrap_or_default()
}

impl KvEntry<'_> {
    fn u32_at(&self, offset: usize) -> u32 {
        match self.value.get(offset..offset + 4) {
            Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        }
    }

    fn is_valid(&self) -> bool {
        match self.kind {
            KvKind::Uint32 => self.value.len() == 4,
            KvKind::String => self.value.last() == Some(&0),
            KvKind::GlFormat | KvKind::AnimData => self.value.len() == 12,
            KvKind::CubemapIncomplete => self.value.len() == 1,
            KvKind::Unknown => false,
        }
    }

    fn bit_value(&self, bit_index: u32) -> &'static str {
        let faces = self.value.first().copied().unwrap_or(0);
        // "false;" is the C++ spelling
        if u32::from(faces) & (1 << bit_index) != 0 {
            "true"
        } else {
            "false;"
        }
    }

    fn fields(&self) -> Vec<(&'static str, String)> {
        match self.kind {
            KvKind::GlFormat => vec![
                ("glInternalformat", self.u32_at(0).to_string()),
                ("glFormat", self.u32_at(4).to_string()),
                ("glType", self.u32_at(8).to_string()),
            ],
            KvKind::AnimData => vec![
                ("duration", self.u32_at(0).to_string()),
                ("timescale", self.u32_at(4).to_string()),
                ("loopCount", self.u32_at(8).to_string()),
            ],
            _ => [
                "positiveX",
                "negativeX",
                "positiveY",
                "negativeY",
                "positiveZ",
                "negativeZ",
            ]
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, self.bit_value(i as u32).to_string()))
            .collect(),
        }
    }

    /// `printText(out, prefix)`.
    fn print_text(&self, prefix: &str) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.is_valid() {
            out.extend_from_slice(
                format!(" {}\n", extract_raw_bytes(self.value, true, " ")).as_bytes(),
            );
            return out;
        }
        match self.kind {
            KvKind::Uint32 => out.extend_from_slice(format!(" {}\n", self.u32_at(0)).as_bytes()),
            KvKind::String => {
                out.push(b' ');
                out.extend_from_slice(c_str(self.value));
                out.push(b'\n');
            }
            KvKind::GlFormat => {
                out.push(b'\n');
                for (i, name) in ["glInternalformat", "glFormat", "glType"]
                    .iter()
                    .enumerate()
                {
                    out.extend_from_slice(
                        format!("{prefix}    {name}: 0x{:08X}\n", self.u32_at(i * 4)).as_bytes(),
                    );
                }
            }
            _ => {
                out.push(b'\n');
                for (name, value) in self.fields() {
                    out.extend_from_slice(format!("{prefix}    {name}: {value}\n").as_bytes());
                }
            }
        }
        out
    }

    /// `printJson(out, indent, space, nl)`.
    fn print_json(&self, pi: PrintIndent, indent: usize, space: &str, nl: &str) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.is_valid() {
            pi.print(
                &mut out,
                indent,
                extract_raw_bytes(self.value, false, space).as_bytes(),
            );
            return out;
        }
        match self.kind {
            KvKind::Uint32 => pi.print(&mut out, indent, self.u32_at(0).to_string().as_bytes()),
            KvKind::String => {
                pi.print(&mut out, indent, b"\"");
                out.extend_from_slice(c_str(self.value));
                out.push(b'"');
            }
            _ => {
                pi.print(&mut out, indent, format!("{{{nl}").as_bytes());
                let fields = self.fields();
                let n = fields.len();
                for (i, (name, value)) in fields.into_iter().enumerate() {
                    let comma = if i + 1 == n { "" } else { "," };
                    pi.print(
                        &mut out,
                        indent + 1,
                        format!("\"{name}\":{space}{value}{comma}{nl}").as_bytes(),
                    );
                }
                pi.print(&mut out, indent, b"}");
            }
        }
        out
    }
}

/// `DiffComplex<KVEntry...>`.
struct DiffComplex<'a> {
    text_header: Vec<u8>,
    json_path: Vec<u8>,
    values: [Option<KvEntry<'a>>; 2],
    different: bool,
}

/// One texel block of a `DiffImage` pair (`ImageSpan::TexelBlockPtr`).
#[derive(Debug, Clone)]
struct TexelBlock {
    /// Pixel coordinates (`getPixelLocation`).
    pixel: [u32; 3],
    /// `getTexelBlockByteOffset`.
    byte_offset: usize,
    /// Formatted packed elements (text, JSON).
    packed: [String; 2],
    /// Formatted channels (text, JSON), `None` for block compressed formats.
    channels: Option<[String; 2]>,
}

/// `DiffImage`.
struct DiffImage {
    text_header: String,
    fragment_uri: String,
    file_offsets: [Option<u64>; 2],
    pairs: Vec<(TexelBlock, TexelBlock)>,
}

/// Port of `PrintDiff`.
struct PrintDiff {
    pi: PrintIndent,
    out: Vec<u8>,
    format: OutputFormat,
    json_section: Option<String>,
    json_section_empty: bool,
    first_context: bool,
    context: Vec<String>,
    different: bool,
}

impl PrintDiff {
    fn new(pi: PrintIndent, format: OutputFormat) -> PrintDiff {
        PrintDiff {
            pi,
            out: Vec::new(),
            format,
            json_section: None,
            json_section_empty: false,
            first_context: true,
            context: Vec::new(),
            different: false,
        }
    }

    fn space(&self) -> &'static str {
        if self.format != OutputFormat::JsonMini {
            " "
        } else {
            ""
        }
    }

    fn nl(&self) -> &'static str {
        if self.format != OutputFormat::JsonMini {
            "\n"
        } else {
            ""
        }
    }

    fn print(&mut self, depth: usize, text: &[u8]) {
        self.pi.print(&mut self.out, depth, text);
    }

    fn print_context(&mut self) {
        if !self.context.is_empty() {
            if !std::mem::replace(&mut self.first_context, false) {
                self.print(0, b"\n");
            }
            for ctx in std::mem::take(&mut self.context) {
                self.print(0, ctx.as_bytes());
            }
        }
    }

    fn begin_json_output(&mut self) {
        let (space, nl) = (self.space(), self.nl());
        self.print(0, format!(",{nl}").as_bytes());
        if std::mem::replace(&mut self.json_section_empty, false) {
            let section = self.json_section.clone().unwrap_or_default();
            self.print(1, format!("\"{section}\":{space}{{{nl}").as_bytes());
        }
    }

    fn begin_json_section(&mut self, section: &str) {
        self.json_section = Some(section.to_string());
        self.json_section_empty = true;
    }

    fn end_json_section(&mut self) {
        if self.json_section.is_some() {
            if !self.json_section_empty {
                let nl = self.nl();
                self.print(0, nl.as_bytes());
                self.print(1, b"}");
            }
            self.json_section = None;
        }
    }

    fn set_context(&mut self, ctx: &str) {
        self.context.clear();
        self.context.push(ctx.to_string());
    }

    fn add_context(&mut self, ctx: &str) {
        self.context.push(ctx.to_string());
    }

    fn update_context(&mut self, ctx: String) {
        self.context.pop();
        self.context.push(ctx);
    }

    /// `operator<<(const DiffBase<T>&)`.
    fn diff(&mut self, diff: Diff) {
        if !diff.different {
            return;
        }
        self.different = true;
        let (space, nl) = (self.space(), self.nl());
        if self.format == OutputFormat::Text {
            self.print_context();
            for (sign, value) in [(b'-', &diff.values[0]), (b'+', &diff.values[1])] {
                if let Some(v) = value {
                    let mut line = vec![sign];
                    line.extend_from_slice(&diff.text_header);
                    line.extend_from_slice(b": ");
                    line.extend_from_slice(v);
                    line.push(b'\n');
                    self.print(0, &line);
                }
            }
        } else {
            self.begin_json_output();
            self.print(
                2,
                &[
                    b"\"",
                    &diff.json_path[..],
                    format!("\":{space}[{nl}").as_bytes(),
                ]
                .concat(),
            );
            match &diff.values[0] {
                Some(v) => self.print(3, &[&v[..], format!(",{nl}").as_bytes()].concat()),
                None => self.print(3, format!("null,{nl}").as_bytes()),
            }
            match &diff.values[1] {
                Some(v) => self.print(3, &[&v[..], nl.as_bytes()].concat()),
                None => self.print(3, format!("null{nl}").as_bytes()),
            }
            self.print(2, b"]");
        }
    }

    /// `operator<<(const DiffComplex<T>&)`.
    fn complex(&mut self, diff: DiffComplex) {
        if !diff.different {
            return;
        }
        self.different = true;
        let (space, nl) = (self.space(), self.nl());
        if self.format == OutputFormat::Text {
            self.print_context();
            for (sign, value) in [("-", &diff.values[0]), ("+", &diff.values[1])] {
                if let Some(v) = value {
                    self.print(0, &[sign.as_bytes(), &diff.text_header[..], b":"].concat());
                    let text = v.print_text(sign);
                    self.out.extend_from_slice(&text);
                }
            }
        } else {
            self.begin_json_output();
            self.print(
                2,
                &[
                    b"\"",
                    &diff.json_path[..],
                    format!("\":{space}[{nl}").as_bytes(),
                ]
                .concat(),
            );
            match &diff.values[0] {
                Some(v) => {
                    let json = v.print_json(self.pi, 3, space, nl);
                    self.out.extend_from_slice(&json);
                    self.print(0, format!(",{nl}").as_bytes());
                }
                None => self.print(3, format!("null,{nl}").as_bytes()),
            }
            match &diff.values[1] {
                Some(v) => {
                    let json = v.print_json(self.pi, 3, space, nl);
                    self.out.extend_from_slice(&json);
                    self.print(0, nl.as_bytes());
                }
                None => self.print(3, format!("null{nl}").as_bytes()),
            }
            self.print(2, b"]");
        }
    }

    /// `operator<<(const DiffTextCustom&)` (text output only, no context).
    fn text_custom(&mut self, texts: [Option<String>; 2]) {
        if texts[0] == texts[1] {
            return;
        }
        self.different = true;
        for (sign, text) in [("-", &texts[0]), ("+", &texts[1])] {
            if let Some(t) = text {
                self.print(0, format!("{sign}{t}\n").as_bytes());
            }
        }
    }

    /// `operator<<(const DiffMismatch&)`.
    fn mismatch(&mut self, text_msg: &str, json_path: &str) {
        self.different = true;
        if self.format == OutputFormat::Text {
            self.print(0, format!("+{text_msg}\n").as_bytes());
        } else {
            let space = self.space();
            self.begin_json_output();
            self.print(2, format!("\"{json_path}\":{space}[]").as_bytes());
        }
    }

    /// `operator<<(const DiffImage&)`.
    fn image(&mut self, diff: DiffImage) {
        self.different = true;
        let (space, nl) = (self.space(), self.nl());
        let file_offset = |o: Option<u64>, image_offset: usize, json: bool| match o {
            Some(o) => {
                let v = o.wrapping_add(image_offset as u64);
                if json {
                    v.to_string()
                } else {
                    format!("0x{v:x}")
                }
            }
            None => if json { "null" } else { "N/A" }.to_string(),
        };

        if self.format == OutputFormat::Text {
            self.print_context();
            self.print(0, format!("+{}\n", diff.text_header).as_bytes());
            for (a, b) in &diff.pairs {
                let [x, y, z] = a.pixel;
                self.print(0, format!("  Coordinates: {x}, {y}, {z}\n").as_bytes());
                self.print(
                    0,
                    format!("    Image byte offset: 0x{:x}\n", a.byte_offset).as_bytes(),
                );
                let mut print_diff = |header: &str, first: String, second: String| {
                    if first == second {
                        self.print(0, format!("    {header}: {first}\n").as_bytes());
                    } else {
                        self.print(0, format!("-    {header}: {first}\n").as_bytes());
                        self.print(0, format!("+    {header}: {second}\n").as_bytes());
                    }
                };
                print_diff(
                    "File byte offset",
                    file_offset(diff.file_offsets[0], a.byte_offset, false),
                    file_offset(diff.file_offsets[1], a.byte_offset, false),
                );
                print_diff("Packed", a.packed[0].clone(), b.packed[0].clone());
                if let (Some(ca), Some(cb)) = (&a.channels, &b.channels) {
                    print_diff("Channels", ca[0].clone(), cb[0].clone());
                }
            }
        } else {
            self.begin_json_output();
            self.print(2, format!("\"{}\":{space}[", diff.fragment_uri).as_bytes());
            let mut first = true;
            for (a, b) in &diff.pairs {
                let comma = if std::mem::replace(&mut first, false) {
                    String::new()
                } else {
                    format!(",{space}")
                };
                self.print(0, format!("{comma}{nl}").as_bytes());
                self.print(3, format!("{{{nl}").as_bytes());
                let [x, y, z] = a.pixel;
                self.print(
                    4,
                    format!(
                        "\"coordinates\":{space}[{space}{x},{space}{y},{space}{z}{space}],{nl}"
                    )
                    .as_bytes(),
                );
                let o = a.byte_offset;
                self.print(
                    4,
                    format!("\"imageByteOffset\":{space}[{space}{o},{space}{o}{space}],{nl}")
                        .as_bytes(),
                );
                self.print(
                    4,
                    format!(
                        "\"fileByteOffset\":{space}[{space}{},{space}{}{space}],{nl}",
                        file_offset(diff.file_offsets[0], o, true),
                        file_offset(diff.file_offsets[1], o, true)
                    )
                    .as_bytes(),
                );
                self.print(4, format!("\"packed\":{space}[{nl}").as_bytes());
                self.print(
                    5,
                    format!("[{space}{}{space}],{nl}", a.packed[1]).as_bytes(),
                );
                self.print(5, format!("[{space}{}{space}]{nl}", b.packed[1]).as_bytes());
                self.print(4, b"]");
                if let (Some(ca), Some(cb)) = (&a.channels, &b.channels) {
                    self.print(0, format!(",{nl}").as_bytes());
                    self.print(4, format!("\"channels\":{space}[{nl}").as_bytes());
                    self.print(5, format!("[{space}{}{space}],{nl}", ca[1]).as_bytes());
                    self.print(5, format!("[{space}{}{space}]{nl}", cb[1]).as_bytes());
                    self.print(4, b"]");
                }
                self.print(0, nl.as_bytes());
                self.print(3, b"}");
            }
            if diff.pairs.is_empty() {
                self.print(0, b"]");
            } else {
                self.print(0, nl.as_bytes());
                self.print(2, b"]");
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Diff value formatters (`Diff`, `DiffHex`, `DiffEnum`, ...)

fn bytes(s: impl AsRef<[u8]>) -> Vec<u8> {
    s.as_ref().to_vec()
}

/// `Diff<T>`: values printed with `{}`.
fn diff_plain<T: PartialEq + std::fmt::Display>(
    header: &str,
    path: &str,
    v: [Option<T>; 2],
) -> Diff {
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: v.map(|x| x.map(|x| x.to_string().into_bytes())),
    }
}

/// `DiffHex<T>`.
fn diff_hex(format: OutputFormat, header: &str, path: &str, v: [Option<u64>; 2]) -> Diff {
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: v.map(|x| {
            x.map(|x| {
                if format == OutputFormat::Text {
                    format!("0x{x:x}").into_bytes()
                } else {
                    x.to_string().into_bytes()
                }
            })
        }),
    }
}

/// `DiffHexFixedWidth<uint32_t>`.
fn diff_hex_fixed_width(
    format: OutputFormat,
    header: &str,
    path: &str,
    v: [Option<u32>; 2],
) -> Diff {
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: v.map(|x| {
            x.map(|x| {
                if format == OutputFormat::Text {
                    format!("0x{x:08x}").into_bytes()
                } else {
                    x.to_string().into_bytes()
                }
            })
        }),
    }
}

/// `DiffEnum<T>`: `names` are the `strFunc` results.
fn diff_enum(
    format: OutputFormat,
    header: &str,
    path: &str,
    v: [Option<u32>; 2],
    names: [Option<&str>; 2],
    hex_in_text: bool,
) -> Diff {
    let value = |raw: u32, name: Option<&str>| -> Vec<u8> {
        if format == OutputFormat::Text {
            match name {
                Some(n) if hex_in_text => format!("0x{raw:x} ({n})"),
                Some(n) => n.to_string(),
                None => format!("0x{raw:x}"),
            }
        } else {
            match name {
                Some(n) => format!("\"{n}\""),
                None => (raw as i32).to_string(),
            }
        }
        .into_bytes()
    };
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: [
            v[0].map(|x| value(x, names[0])),
            v[1].map(|x| value(x, names[1])),
        ],
    }
}

/// `DiffEnum<ktxSupercmpScheme>`.
fn diff_supercmp(format: OutputFormat, header: &str, path: &str, v: [u32; 2]) -> Diff {
    let value = |raw: u32| -> Vec<u8> {
        let name = ktx_supercompression_scheme_string(SupercmpScheme(raw));
        let invalid = name == "Invalid scheme value";
        let vendor = name == "Vendor or reserved scheme";
        if format == OutputFormat::Text {
            if invalid {
                format!("Invalid scheme (0x{raw:x})")
            } else if vendor {
                format!("Vendor or reserved scheme (0x{raw:x})")
            } else {
                name.to_string()
            }
        } else if !invalid && !vendor {
            format!("\"{name}\"")
        } else {
            (raw as i32).to_string()
        }
        .into_bytes()
    };
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: [Some(value(v[0])), Some(value(v[1]))],
    }
}

/// `DiffFlags`.
fn diff_flags(
    format: OutputFormat,
    header: &str,
    path: &str,
    v: [Option<u32>; 2],
    to_string: fn(u32, bool) -> Option<&'static str>,
) -> Diff {
    let space = if format != OutputFormat::JsonMini {
        " "
    } else {
        ""
    };
    let quote = if format == OutputFormat::Text {
        ""
    } else {
        "\""
    };
    let value = |raw: u32| -> Vec<u8> {
        let mut s = String::new();
        let mut first = true;
        let mut comma = String::new();
        for bit_index in 0..32 {
            let bit_mask = 1u32 << bit_index;
            let bit_value = raw & bit_mask != 0;
            match to_string(bit_index, bit_value) {
                Some(b) => s.push_str(&format!("{comma}{quote}{b}{quote}")),
                None if bit_value => s.push_str(&format!("{comma}{bit_mask}")),
                None => {}
            }
            if bit_value && first {
                first = false;
                comma = format!(",{space}");
            }
        }
        if format == OutputFormat::Text {
            format!("0x{raw:x} ({s})")
        } else if s.is_empty() {
            "[]".to_string()
        } else {
            format!("[{space}{s}{space}]")
        }
        .into_bytes()
    };
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: v.map(|x| x.map(value)),
    }
}

/// `DiffArray<T, N>` and `DiffRawBytes` (`hex` for the latter's text output).
fn diff_array(
    format: OutputFormat,
    header: &str,
    path: &str,
    v: [Option<Vec<u32>>; 2],
    raw_bytes: bool,
) -> Diff {
    let space = if format != OutputFormat::JsonMini {
        " "
    } else {
        ""
    };
    let value = |items: &Vec<u32>| -> Vec<u8> {
        let s = items
            .iter()
            .map(|x| {
                if raw_bytes && format == OutputFormat::Text {
                    format!("0x{x:x}")
                } else {
                    x.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(&format!(",{space}"));
        if format == OutputFormat::Text {
            if raw_bytes { format!("[{s}]") } else { s }
        } else if s.is_empty() {
            "[]".to_string()
        } else {
            format!("[{space}{s}{space}]")
        }
        .into_bytes()
    };
    Diff {
        text_header: bytes(header),
        json_path: bytes(path),
        different: v[0] != v[1],
        values: [v[0].as_ref().map(value), v[1].as_ref().map(value)],
    }
}

/// `DiffIdentifier::value`: emulates the 30 byte `u8identifier` buffer.
fn identifier_value(identifier: &[u8; 12], format: OutputFormat) -> Vec<u8> {
    const SIZE: usize = 30;
    let mut buf = [0u8; SIZE];
    let mut idlen = 0usize;
    let put = |buf: &mut [u8; SIZE], at: usize, c: u8| {
        if let Some(b) = buf.get_mut(at) {
            *b = c;
        }
    };
    let mut i = 0;
    while i < 12 && idlen < SIZE {
        let c = identifier[i];
        if c == 0xAB || c == 0xBB {
            put(&mut buf, idlen, 0xC2);
            idlen += 1;
            put(&mut buf, idlen, c);
        } else if c < 0x20 {
            match c {
                b'\n' => {
                    put(&mut buf, idlen, b'\\');
                    idlen += 1;
                    put(&mut buf, idlen, b'n');
                }
                b'\r' => {
                    put(&mut buf, idlen, b'\\');
                    idlen += 1;
                    put(&mut buf, idlen, b'r');
                }
                _ => {
                    let s = if format == OutputFormat::Text {
                        format!("\\x{c:02X}")
                    } else {
                        format!("\\u{c:04X}")
                    };
                    // snprintf writes at most SIZE - idlen - 1 characters plus NUL
                    let room = SIZE.saturating_sub(idlen);
                    for (k, &ch) in s.as_bytes().iter().enumerate() {
                        if k + 1 < room {
                            put(&mut buf, idlen + k, ch);
                        } else if k + 1 == room {
                            put(&mut buf, idlen + k, 0);
                        }
                    }
                    idlen += s.len() - 1;
                }
            }
        } else {
            put(&mut buf, idlen, c);
        }
        i += 1;
        idlen += 1;
    }
    let s = buf.get(..idlen.min(SIZE)).unwrap_or_default();
    if format == OutputFormat::Text {
        s.to_vec()
    } else {
        [b"\"", s, b"\""].concat()
    }
}

// -------------------------------------------------------------------------------------------------
// Raw structure decoding (bitfields of validate.h, ktxBasisLz* of basis_sgd.h)

fn u32_at(buf: &[u8], pos: usize) -> u32 {
    let b = |i: usize| buf.get(pos.wrapping_add(i)).copied().unwrap_or(0);
    u32::from_le_bytes([b(0), b(1), b(2), b(3)])
}

fn u16_at(buf: &[u8], pos: usize) -> u16 {
    let b = |i: usize| buf.get(pos.wrapping_add(i)).copied().unwrap_or(0);
    u16::from_le_bytes([b(0), b(1)])
}

/// `DFDHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DfdHeader {
    vendor_id: u32,
    descriptor_type: u32,
    version_number: u32,
    descriptor_block_size: u32,
}

impl DfdHeader {
    fn read(buf: &[u8], pos: usize) -> DfdHeader {
        let (w0, w1) = (u32_at(buf, pos), u32_at(buf, pos.wrapping_add(4)));
        DfdHeader {
            vendor_id: w0 & 0x1FFFF,
            descriptor_type: w0 >> 17,
            version_number: w1 & 0xFFFF,
            descriptor_block_size: w1 >> 16,
        }
    }

    fn is_basic(&self) -> bool {
        self.vendor_id == KHR_DF_VENDORID_KHRONOS
            && self.descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT
    }
}

/// The `BDFD` fields after the block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bdfd {
    model: u32,
    primaries: u32,
    transfer: u32,
    flags: u32,
    texel_block_dimension: [u32; 4],
    bytes_planes: [u32; 8],
}

impl Bdfd {
    fn read(buf: &[u8], pos: usize) -> Bdfd {
        let w2 = u32_at(buf, pos.wrapping_add(8));
        let w3 = u32_at(buf, pos.wrapping_add(12));
        let byte = |i: usize| u32::from(buf.get(pos.wrapping_add(16 + i)).copied().unwrap_or(0));
        Bdfd {
            model: w2 & 0xFF,
            primaries: (w2 >> 8) & 0xFF,
            transfer: (w2 >> 16) & 0xFF,
            flags: w2 >> 24,
            texel_block_dimension: [w3 & 0xFF, (w3 >> 8) & 0xFF, (w3 >> 16) & 0xFF, w3 >> 24],
            bytes_planes: std::array::from_fn(byte),
        }
    }
}

/// `SampleType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SampleType {
    bit_offset: u32,
    bit_length: u32,
    channel_id: u32,
    qualifier_linear: bool,
    qualifier_exponent: bool,
    qualifier_signed: bool,
    qualifier_float: bool,
    sample_position: [u32; 4],
    lower: u32,
    upper: u32,
}

impl SampleType {
    fn read(buf: &[u8], pos: usize) -> SampleType {
        let w0 = u32_at(buf, pos);
        let w1 = u32_at(buf, pos.wrapping_add(4));
        SampleType {
            bit_offset: w0 & 0xFFFF,
            bit_length: (w0 >> 16) & 0xFF,
            channel_id: (w0 >> 24) & 0xF,
            qualifier_linear: w0 & (1 << 28) != 0,
            qualifier_exponent: w0 & (1 << 29) != 0,
            qualifier_signed: w0 & (1 << 30) != 0,
            qualifier_float: w0 & (1 << 31) != 0,
            sample_position: [w1 & 0xFF, (w1 >> 8) & 0xFF, (w1 >> 16) & 0xFF, w1 >> 24],
            lower: u32_at(buf, pos.wrapping_add(8)),
            upper: u32_at(buf, pos.wrapping_add(12)),
        }
    }
}

/// `compareSGD`'s `SGDBasisLZ`.
#[derive(Debug, Clone, Default)]
struct SgdBasisLz {
    endpoint_count: Option<u16>,
    selector_count: Option<u16>,
    endpoints_byte_length: Option<u64>,
    selectors_byte_length: Option<u64>,
    tables_byte_length: Option<u64>,
    extended_byte_length: Option<u64>,
    image_flags: Vec<Option<u32>>,
    rgb_slice_byte_offset: Vec<Option<u32>>,
    rgb_slice_byte_length: Vec<Option<u32>>,
    alpha_slice_byte_offset: Vec<Option<u32>>,
    alpha_slice_byte_length: Vec<Option<u32>>,
    endpoints_byte_offset: Option<u64>,
    selectors_byte_offset: Option<u64>,
    tables_byte_offset: Option<u64>,
    extended_byte_offset: Option<u64>,
}

/// `formatChannels` of one texel block: (text, JSON).
fn format_channels(codec: &ImageCodec, data: &[u8], space: &str) -> [String; 2] {
    let n = codec.get_channel_count().min(4) as usize;
    let comma = |i: usize| {
        if i == 0 {
            String::new()
        } else {
            format!(",{space}")
        }
    };
    std::array::from_fn(|k| {
        let json = k == 1;
        let quote = if json { "\"" } else { "" };
        let mut s = String::new();
        if codec.can_decode_uint() {
            for (i, c) in codec.decode_uint(data).iter().take(n).enumerate() {
                s.push_str(&format!("{}{c}", comma(i)));
            }
        } else if codec.can_decode_sint() {
            for (i, c) in codec.decode_sint(data).iter().take(n).enumerate() {
                s.push_str(&format!("{}{c}", comma(i)));
            }
        } else if codec.can_decode_float() {
            for (i, c) in codec.decode_float(data).iter().take(n).enumerate() {
                let v = if c.is_infinite() {
                    let sign = if c.is_sign_negative() { "-" } else { "+" };
                    format!("{quote}{sign}inf{quote}")
                } else if c.is_nan() {
                    format!("{quote}nan{quote}")
                } else {
                    // The sign is printed separately to keep -0.0
                    let sign = if c.is_sign_negative() { "-" } else { "" };
                    format!("{sign}{}", float_shortest(c.abs()))
                };
                s.push_str(&format!("{}{v}", comma(i)));
            }
        }
        s
    })
}

/// `formatPacked` of one texel block: (text, JSON).
fn format_packed(codec: &ImageCodec, data: &[u8], space: &str) -> [String; 2] {
    let digits = (codec.get_packed_element_byte_size() << 1) as usize;
    std::array::from_fn(|k| {
        let quote = if k == 1 { "\"" } else { "" };
        (0..codec.get_packed_element_count())
            .map(|i| {
                format!(
                    "{quote}0x{:0digits$x}{quote}",
                    codec.get_packed_element(data, i)
                )
            })
            .collect::<Vec<_>>()
            .join(&format!(",{space}"))
    })
}

// -------------------------------------------------------------------------------------------------

/// Port of `CommandCompare`.
#[derive(Debug, Default)]
pub struct CommandCompare {
    report: Reporter,
    format: OptionsFormat,
    options: OptionsCompare,
    generic: OptionsGeneric,
    headers: [KtxHeader2; 2],
}

impl CommandCompare {
    /// Port of `CommandCompare::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandCompare::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx compare",
            "Compares the two KTX files specified as the input-file1 and input-file2 arguments.\n    The command implicitly calls validate and prints any found errors\n    and warnings to stdout.",
            args,
            &mut [&mut self.format, &mut self.options, &mut self.generic],
        )?;
        let streams = [
            InputStream::new(&self.options.input_filepaths[0], &self.report)?,
            InputStream::new(&self.options.input_filepaths[1], &self.report)?,
        ];
        let mut out = Vec::new();
        let result = self.execute_compare(&streams, &mut out);
        print_out(&out);
        result
    }

    fn output_format(&self) -> OutputFormat {
        self.format.format
    }

    /// `read(stream, offset, readDst, readSize, what)`.
    fn read<'a>(
        &self,
        stream: &'a InputStream,
        offset: u64,
        size: u64,
        what: &str,
    ) -> CmdResult<&'a [u8]> {
        let len = stream.data.len() as u64;
        let available = len.saturating_sub(offset).min(size);
        if available < size {
            return Err(self.report.fatal(
                ReturnCode::IoFailure,
                &format!(
                    "Unexpected end of file reading {what} from file \"{}\".",
                    stream.filepath
                ),
            ));
        }
        let start = offset as usize;
        Ok(stream
            .data
            .get(start..start + size as usize)
            .unwrap_or_default())
    }

    fn read_level_index(&self, stream: &InputStream, level: u32) -> CmdResult<LevelIndexEntry> {
        let offset = KTX2_HEADER_SIZE + u64::from(level) * LEVEL_INDEX_ENTRY_SIZE as u64;
        let b = self.read(
            stream,
            offset,
            LEVEL_INDEX_ENTRY_SIZE as u64,
            "the level index",
        )?;
        Ok(LevelIndexEntry::from_bytes(b).unwrap_or_default())
    }

    fn execute_compare(&mut self, streams: &[InputStream; 2], out: &mut Vec<u8>) -> CmdResult {
        let format = self.output_format();
        let paths = self.options.input_filepaths.clone();
        let mut validation_messages: Vec<Vec<u8>> = Vec::new();
        let mut validation_results: Vec<i32> = Vec::new();

        if format == OutputFormat::Text {
            for stream in streams {
                let mut messages = Vec::new();
                let result = validate_memory(&stream.data, false, false, &mut |issue| {
                    write_issue_text(&mut messages, issue);
                });
                validation_results.push(self.runtime(result, out)?);
                validation_messages.push(messages);
            }

            let mut has_validation_messages = false;
            for i in 0..2 {
                if !validation_messages[i].is_empty() {
                    if std::mem::replace(&mut has_validation_messages, true) {
                        out.push(b'\n');
                    }
                    let status = if validation_results[i] == 0 {
                        "successful"
                    } else {
                        "failed"
                    };
                    out.extend_from_slice(
                        format!("Validation {status} for '{}'\n\n", paths[i]).as_bytes(),
                    );
                    out.extend_from_slice(&validation_messages[i]);
                }
            }

            for &result in &validation_results {
                if result != 0 {
                    if result != ReturnCode::InvalidFile as i32 || !self.options.allow_invalid_input
                    {
                        return Err(FatalError(result));
                    }
                    // Image comparison is only supported for valid input files
                    self.options.content_mode = ContentMode::Ignore;
                }
            }

            if has_validation_messages {
                out.push(b'\n');
            }

            let mut diff = PrintDiff::new(
                PrintIndent {
                    indent_base: 0,
                    indent_width: 4,
                },
                format,
            );
            let result = self.compare_all(&mut diff, streams);
            out.append(&mut diff.out);
            result?;
            if diff.different {
                return Err(ReturnCode::DifferenceFound.into());
            }
        } else {
            let indent_width = if format == OutputFormat::Json { 4 } else { 0 };
            let space = if format == OutputFormat::Json {
                " "
            } else {
                ""
            };
            let nl = if format == OutputFormat::Json {
                "\n"
            } else {
                ""
            };
            let pi = PrintIndent {
                indent_base: 0,
                indent_width,
            };

            pi.print(out, 0, format!("{{{nl}").as_bytes());
            pi.print(
                out,
                1,
                format!(
                    "\"$schema\":{space}\"https://schema.khronos.org/ktx/compare_v1.json\",{nl}"
                )
                .as_bytes(),
            );

            for (i, stream) in streams.iter().enumerate() {
                let mut messages = Vec::new();
                let mut first = true;
                let result = validate_memory(&stream.data, false, false, &mut |issue| {
                    write_issue_json(&mut messages, pi, &mut first, space, nl, issue);
                });
                let _ = fmt_in_file(&paths[i]);
                validation_results.push(self.runtime(result, out)?);
                validation_messages.push(messages);
            }

            pi.print(out, 1, format!("\"valid\":{space}[{nl}").as_bytes());
            for (i, result) in validation_results.iter().enumerate() {
                let comma = if i == 1 { "" } else { "," };
                pi.print(out, 2, format!("{}{comma}{nl}", *result == 0).as_bytes());
            }
            pi.print(out, 1, format!("],{nl}").as_bytes());

            pi.print(out, 1, format!("\"messages\":{space}[{nl}").as_bytes());
            for (i, messages) in validation_messages.iter().enumerate() {
                let comma = if i == 1 { "" } else { "," };
                if !messages.is_empty() {
                    pi.print(out, 2, format!("[{nl}").as_bytes());
                    out.extend_from_slice(messages);
                    pi.print(out, 3, format!("}}{nl}").as_bytes());
                    pi.print(out, 2, format!("]{comma}{nl}").as_bytes());
                } else {
                    pi.print(out, 2, format!("[]{comma}{nl}").as_bytes());
                }
            }
            pi.print(out, 1, b"]");

            let mut fatal_validation_error = 0;
            for &result in &validation_results {
                if result != 0 {
                    if result != ReturnCode::InvalidFile as i32 || !self.options.allow_invalid_input
                    {
                        fatal_validation_error = result;
                        break;
                    }
                    // Image comparison is only supported for valid input files
                    self.options.content_mode = ContentMode::Ignore;
                }
            }

            let mut diff = PrintDiff::new(pi, format);
            let result = if fatal_validation_error != 0 {
                Err(FatalError(fatal_validation_error))
            } else {
                self.compare_all(&mut diff, streams)
            };
            diff.end_json_section();
            out.append(&mut diff.out);
            out.extend_from_slice(format!("{nl}}}{nl}").as_bytes());
            result?;
            if diff.different {
                return Err(ReturnCode::DifferenceFound.into());
            }
        }
        Ok(())
    }

    /// The comparison steps of `executeCompare` (JSON sections included).
    fn compare_all(&mut self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        let json = self.output_format() != OutputFormat::Text;
        if json {
            diff.begin_json_section("info");
        }
        self.compare_header(diff, streams)?;
        self.compare_level_index(diff, streams)?;
        self.compare_dfd(diff, streams)?;
        self.compare_kvd(diff, streams)?;
        self.compare_sgd(diff, streams)?;
        if json {
            diff.end_json_section();
            diff.begin_json_section("image");
        }
        self.compare_images(diff, streams)?;
        if json {
            diff.end_json_section();
        }
        Ok(())
    }

    /// A `std::exception` escaping validation: `{name} fatal: {what}`, exit 4.
    fn runtime(&self, result: Result<i32, String>, _out: &mut Vec<u8>) -> CmdResult<i32> {
        result.map_err(|what| runtime_error(&self.report, &what))
    }

    fn compare_header(&mut self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        diff.set_context("Header\n\n");
        let format = self.output_format();

        for (i, stream) in streams.iter().enumerate() {
            let b = self.read(stream, 0, KTX2_HEADER_SIZE, "header")?;
            let mut raw = [0u8; 80];
            raw.copy_from_slice(b.get(..80).unwrap_or(&[0; 80]));
            self.headers[i] = KtxHeader2::from_bytes(&raw);
        }
        let h = self.headers;

        let identifiers = [
            identifier_value(&h[0].identifier, format),
            identifier_value(&h[1].identifier, format),
        ];
        diff.diff(Diff {
            text_header: bytes("identifier"),
            json_path: bytes("/header/identifier"),
            different: h[0].identifier != h[1].identifier,
            values: identifiers.map(Some),
        });

        if !self.options.ignore_format_header {
            let vk = [h[0].vk_format.0, h[1].vk_format.0];
            diff.diff(diff_enum(
                format,
                "vkFormat",
                "/header/vkFormat",
                vk.map(Some),
                vk.map(|f| Some(vk_format_string(VkFormat(f)))),
                false,
            ));
            diff.diff(diff_plain(
                "typeSize",
                "/header/typeSize",
                [Some(h[0].type_size), Some(h[1].type_size)],
            ));
        }

        let fields: [HeaderField; 6] = [
            ("pixelWidth", "/header/pixelWidth", |h| h.pixel_width),
            ("pixelHeight", "/header/pixelHeight", |h| h.pixel_height),
            ("pixelDepth", "/header/pixelDepth", |h| h.pixel_depth),
            ("layerCount", "/header/layerCount", |h| h.layer_count),
            ("faceCount", "/header/faceCount", |h| h.face_count),
            ("levelCount", "/header/levelCount", |h| h.level_count),
        ];
        for (header, path, get) in fields {
            diff.diff(diff_plain(
                header,
                path,
                [Some(get(&h[0])), Some(get(&h[1]))],
            ));
        }

        if !self.options.ignore_supercomp {
            diff.diff(diff_supercmp(
                format,
                "supercompressionScheme",
                "/header/supercompressionScheme",
                [
                    h[0].supercompression_scheme.0,
                    h[1].supercompression_scheme.0,
                ],
            ));
        }

        if self.options.ignore_index != IgnoreIndex::All {
            let dfd = [h[0].data_format_descriptor, h[1].data_format_descriptor];
            let kvd = [h[0].key_value_data, h[1].key_value_data];
            let sgd = [
                h[0].supercompression_global_data,
                h[1].supercompression_global_data,
            ];
            diff.diff(diff_hex(
                format,
                "dataFormatDescriptor.byteOffset",
                "/index/dataFormatDescriptor/byteOffset",
                dfd.map(|e| Some(u64::from(e.byte_offset))),
            ));
            diff.diff(diff_plain(
                "dataFormatDescriptor.byteLength",
                "/index/dataFormatDescriptor/byteLength",
                dfd.map(|e| Some(e.byte_length)),
            ));
            diff.diff(diff_hex(
                format,
                "keyValueData.byteOffset",
                "/index/keyValueData/byteOffset",
                kvd.map(|e| Some(u64::from(e.byte_offset))),
            ));
            diff.diff(diff_plain(
                "keyValueData.byteLength",
                "/index/keyValueData/byteLength",
                kvd.map(|e| Some(e.byte_length)),
            ));
            diff.diff(diff_hex(
                format,
                "supercompressionGlobalData.byteOffset",
                "/index/supercompressionGlobalData/byteOffset",
                sgd.map(|e| Some(e.byte_offset)),
            ));
            diff.diff(diff_plain(
                "supercompressionGlobalData.byteLength",
                "/index/supercompressionGlobalData/byteLength",
                sgd.map(|e| Some(e.byte_length)),
            ));
        }
        Ok(())
    }

    fn compare_level_index(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        if self.options.ignore_index != IgnoreIndex::None {
            return Ok(());
        }
        diff.set_context("Level Index\n\n");
        let format = self.output_format();
        let num_levels = self.headers.map(|h| h.level_count.max(1));
        let max_num_levels = num_levels[0].max(num_levels[1]);

        for level in 0..max_num_levels {
            let mut entries: [Option<LevelIndexEntry>; 2] = [None, None];
            for i in 0..2 {
                if level < num_levels[i] {
                    entries[i] = Some(self.read_level_index(&streams[i], level)?);
                }
            }
            diff.diff(diff_hex(
                format,
                &format!("Level{level}.byteOffset"),
                &format!("/index/levels/{level}/byteOffset"),
                entries.map(|e| e.map(|e| e.byte_offset)),
            ));
            diff.diff(diff_plain(
                &format!("Level{level}.byteLength"),
                &format!("/index/levels/{level}/byteLength"),
                entries.map(|e| e.map(|e| e.byte_length)),
            ));
            diff.diff(diff_plain(
                &format!("Level{level}.uncompressedByteLength"),
                &format!("/index/levels/{level}/uncompressedByteLength"),
                entries.map(|e| e.map(|e| e.uncompressed_byte_length)),
            ));
        }
        Ok(())
    }

    fn compare_dfd(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        let ignore = self.options.ignore_dfd;
        if ignore == IgnoreDfd::All {
            return Ok(());
        }
        diff.set_context("Data Format Descriptor\n\n");
        let format = self.output_format();

        let mut buffers: [&[u8]; 2] = [&[], &[]];
        for i in 0..2 {
            let index = self.headers[i].data_format_descriptor;
            buffers[i] = self.read(
                &streams[i],
                u64::from(index.byte_offset),
                u64::from(index.byte_length),
                "the DFD blocks",
            )?;
        }
        let ends = buffers.map(|b| b.len());
        let mut its = [0usize; 2];

        let dfd_total_size = [u32_at(buffers[0], 0), u32_at(buffers[1], 0)];
        its = its.map(|p| p + 4);

        if ignore != IgnoreDfd::AllExceptColorSpace && dfd_total_size[0] != dfd_total_size[1] {
            diff.diff(diff_plain(
                "DFD total bytes",
                "/dataFormatDescriptor/totalSize",
                dfd_total_size.map(Some),
            ));
        }

        let mut block_index = 0u32;
        while its[0] < ends[0] || its[1] < ends[1] {
            // Pointer differences past the end wrap like the C++ size_t casts
            let remaining = [ends[0].wrapping_sub(its[0]), ends[1].wrapping_sub(its[1])];
            if remaining[0] < 8 && remaining[1] < 8 {
                break;
            }
            let block_headers: [Option<DfdHeader>; 2] = std::array::from_fn(|i| {
                (remaining[i] >= 8).then(|| DfdHeader::read(buffers[i], its[i]))
            });
            let dfd_basic = block_headers.map(|h| h.is_some_and(|h| h.is_basic()));
            let dfd_known = dfd_basic;

            // Consider the ignore-dfd option before comparing the headers
            let compare_dfds = match ignore {
                IgnoreDfd::Unknown => dfd_known[0] || dfd_known[1],
                IgnoreDfd::AllExceptColorSpace | IgnoreDfd::Extended => {
                    dfd_basic[0] || dfd_basic[1]
                }
                _ => true,
            };

            if compare_dfds {
                if ignore != IgnoreDfd::AllExceptColorSpace {
                    let vendor = block_headers.map(|h| h.map(|h| h.vendor_id));
                    diff.diff(diff_enum(
                        format,
                        "Vendor ID",
                        &format!("/dataFormatDescriptor/blocks/{block_index}/vendorId"),
                        vendor,
                        vendor.map(|v| v.and_then(dfd_to_string_vendor_id)),
                        false,
                    ));
                    let descriptor_type = block_headers.map(|h| h.map(|h| h.descriptor_type));
                    diff.diff(diff_enum(
                        format,
                        "Descriptor type",
                        &format!("/dataFormatDescriptor/blocks/{block_index}/descriptorType"),
                        descriptor_type,
                        block_headers.map(|h| {
                            h.and_then(|h| {
                                if h.vendor_id == KHR_DF_VENDORID_KHRONOS {
                                    dfd_to_string_descriptor_type(h.descriptor_type)
                                } else {
                                    None
                                }
                            })
                        }),
                        false,
                    ));
                    let version = block_headers.map(|h| h.map(|h| h.version_number));
                    diff.diff(diff_enum(
                        format,
                        "Version",
                        &format!("/dataFormatDescriptor/blocks/{block_index}/versionNumber"),
                        version,
                        version.map(|v| v.and_then(dfd_to_string_version_number)),
                        false,
                    ));
                    diff.diff(diff_plain(
                        "Descriptor block size",
                        &format!("/dataFormatDescriptor/blocks/{block_index}/descriptorBlockSize"),
                        block_headers.map(|h| h.map(|h| h.descriptor_block_size)),
                    ));
                }

                // Compare basic DFD data if possible
                if dfd_basic[0] || dfd_basic[1] {
                    let mut bdfds: [Option<Bdfd>; 2] = [None, None];
                    let mut samples: [Option<Vec<SampleType>>; 2] = [None, None];
                    for i in 0..2 {
                        if let Some(h) = block_headers[i].filter(|h| h.is_basic()) {
                            bdfds[i] = Some(Bdfd::read(buffers[i], its[i]));
                            let count = MAX_NUM_BDFD_SAMPLES
                                .min(h.descriptor_block_size.wrapping_sub(24) / 16);
                            samples[i] = Some(
                                (0..count as usize)
                                    .map(|s| {
                                        SampleType::read(
                                            buffers[i],
                                            its[i].wrapping_add(24 + s * 16),
                                        )
                                    })
                                    .collect(),
                            );
                        }
                    }
                    self.compare_dfd_basic(diff, block_index, &bdfds, &samples);
                }

                // Compare any unrecognized DFD data as raw payload
                if ignore != IgnoreDfd::AllExceptColorSpace && (!dfd_known[0] || !dfd_known[1]) {
                    let mut raw_payloads: [Option<Vec<u32>>; 2] = [None, None];
                    for i in 0..2 {
                        if let Some(h) = block_headers[i] {
                            let Some(size) = h.descriptor_block_size.checked_sub(8) else {
                                return Err(runtime_error(
                                    &self.report,
                                    "cannot create std::vector larger than max_size()",
                                ));
                            };
                            // The C++ copies from the start of the block, header included
                            raw_payloads[i] = Some(
                                (0..size as usize)
                                    .map(|k| {
                                        u32::from(
                                            buffers[i]
                                                .get(its[i].wrapping_add(k))
                                                .copied()
                                                .unwrap_or(0),
                                        )
                                    })
                                    .collect(),
                            );
                            // Printed once per present block, like the C++
                            diff.diff(diff_array(
                                format,
                                "Raw payload",
                                &format!("/dataFormatDescriptor/blocks/{block_index}/rawPayload"),
                                raw_payloads.clone(),
                                true,
                            ));
                        }
                    }
                }
            }

            block_index += 1;
            if block_index >= MAX_NUM_DFD_BLOCKS {
                return Ok(());
            }

            for i in 0..2 {
                if let Some(h) = block_headers[i] {
                    its[i] = its[i].wrapping_add(h.descriptor_block_size.max(8) as usize);
                }
            }
        }
        Ok(())
    }

    fn compare_dfd_basic(
        &self,
        diff: &mut PrintDiff,
        block_index: u32,
        bdfds: &[Option<Bdfd>; 2],
        bdfd_samples: &[Option<Vec<SampleType>>; 2],
    ) {
        let format = self.output_format();
        let ignore = self.options.ignore_dfd;
        let path = |field: &str| format!("/dataFormatDescriptor/blocks/{block_index}/{field}");

        if ignore != IgnoreDfd::AllExceptColorSpace {
            diff.diff(diff_flags(
                format,
                "Flags",
                &path("flags"),
                bdfds.map(|b| b.map(|b| b.flags)),
                dfd_to_string_flags_bit,
            ));
        }

        let transfer = bdfds.map(|b| b.map(|b| b.transfer));
        diff.diff(diff_enum(
            format,
            "Transfer",
            &path("transferFunction"),
            transfer,
            transfer.map(|v| v.and_then(dfd_to_string_transfer_function)),
            false,
        ));
        let primaries = bdfds.map(|b| b.map(|b| b.primaries));
        diff.diff(diff_enum(
            format,
            "Primaries",
            &path("colorPrimaries"),
            primaries,
            primaries.map(|v| v.and_then(dfd_to_string_color_primaries)),
            false,
        ));

        // Do not compare the remainder of the BDFD if everything but color space information is ignored
        if ignore == IgnoreDfd::AllExceptColorSpace {
            return;
        }

        let model = bdfds.map(|b| b.map(|b| b.model));
        diff.diff(diff_enum(
            format,
            "Model",
            &path("colorModel"),
            model,
            model.map(|v| v.and_then(dfd_to_string_color_model)),
            false,
        ));
        diff.diff(diff_array(
            format,
            "Dimensions",
            &path("texelBlockDimension"),
            bdfds.map(|b| b.map(|b| b.texel_block_dimension.to_vec())),
            false,
        ));
        if !self.options.ignore_bdfd_bytes_plane {
            diff.diff(diff_array(
                format,
                "Plane bytes",
                &path("bytesPlane"),
                bdfds.map(|b| b.map(|b| b.bytes_planes.to_vec())),
                false,
            ));
        }

        diff.add_context("Sample <i>:\n");

        let max_num_samples = bdfd_samples[0]
            .as_ref()
            .map_or(0, Vec::len)
            .max(bdfd_samples[1].as_ref().map_or(0, Vec::len));
        for sample_index in 0..max_num_samples {
            diff.update_context(format!("Sample {sample_index}:\n"));
            let spath = |field: &str| path(&format!("samples/{sample_index}/{field}"));

            let samples: [Option<SampleType>; 2] = std::array::from_fn(|i| {
                bdfd_samples[i]
                    .as_ref()
                    .and_then(|s| s.get(sample_index).copied())
            });

            let qualifier_flags = samples.map(|s| {
                s.map(|s| {
                    let mut flags = 0;
                    if s.qualifier_linear {
                        flags |= KHR_DF_SAMPLE_DATATYPE_LINEAR;
                    }
                    if s.qualifier_exponent {
                        flags |= KHR_DF_SAMPLE_DATATYPE_EXPONENT;
                    }
                    if s.qualifier_signed {
                        flags |= KHR_DF_SAMPLE_DATATYPE_SIGNED;
                    }
                    if s.qualifier_float {
                        flags |= KHR_DF_SAMPLE_DATATYPE_FLOAT;
                    }
                    flags
                })
            });
            diff.diff(diff_flags(
                format,
                "    Qualifiers",
                &spath("qualifiers"),
                qualifier_flags,
                dfd_to_string_sample_datatype_qualifiers_bit,
            ));

            let channel_id = samples.map(|s| s.map(|s| s.channel_id));
            let names: [Option<&str>; 2] = std::array::from_fn(|i| match (bdfds[i], samples[i]) {
                (Some(b), Some(s)) => dfd_to_string_channel_id(b.model, s.channel_id),
                _ => None,
            });
            diff.diff(diff_enum(
                format,
                "    Channel Id",
                &spath("channelId"),
                channel_id,
                names,
                true,
            ));

            // Text output combines length and offset so we have to special-case here
            if format == OutputFormat::Text {
                diff.text_custom(samples.map(|s| {
                    s.map(|s| {
                        format!(
                            "    Length: {} bits Offset: {}",
                            s.bit_length + 1,
                            s.bit_offset
                        )
                    })
                }));
            } else {
                diff.diff(diff_plain(
                    "",
                    &spath("bitLength"),
                    samples.map(|s| s.map(|s| s.bit_length)),
                ));
                diff.diff(diff_plain(
                    "",
                    &spath("bitOffset"),
                    samples.map(|s| s.map(|s| s.bit_offset)),
                ));
            }

            diff.diff(diff_array(
                format,
                "    Position",
                &spath("samplePosition"),
                samples.map(|s| s.map(|s| s.sample_position.to_vec())),
                false,
            ));
            diff.diff(diff_hex_fixed_width(
                format,
                "    Lower",
                &spath("sampleLower"),
                samples.map(|s| s.map(|s| s.lower)),
            ));
            diff.diff(diff_hex_fixed_width(
                format,
                "    Upper",
                &spath("sampleUpper"),
                samples.map(|s| s.map(|s| s.upper)),
            ));
        }
    }

    fn compare_kvd(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        if self.options.ignore_all_metadata {
            return Ok(());
        }
        diff.set_context("Key/Value Data\n\n");

        let mut lists = [HashList::new(), HashList::new()];
        for i in 0..2 {
            let index = self.headers[i].key_value_data;
            if index.byte_length == 0 {
                continue;
            }
            let data = self.read(
                &streams[i],
                u64::from(index.byte_offset),
                u64::from(index.byte_length),
                "the KVD",
            )?;
            if lists[i].deserialize(data).is_err() {
                return Err(self.report.fatal(
                    ReturnCode::RuntimeError,
                    &format!("Failed to parse KVD in file \"{}\".", streams[i].filepath),
                ));
            }
        }

        // std::map<std::string_view, ...>::emplace keeps the first entry of a key
        let mut keys: [BTreeMap<&[u8], &[u8]>; 2] = [BTreeMap::new(), BTreeMap::new()];
        for i in 0..2 {
            for entry in lists[i].iter().take(MAX_NUM_KV_ENTRIES as usize) {
                let (_, key) = entry.get_key();
                let (_, value) = entry.get_value();
                keys[i].entry(key).or_insert(value);
            }
        }

        let mut its = [keys[0].iter().peekable(), keys[1].iter().peekable()];
        loop {
            let heads = [
                its[0].peek().map(|(k, v)| (**k, **v)),
                its[1].peek().map(|(k, v)| (**k, **v)),
            ];
            if heads[0].is_none() && heads[1].is_none() {
                break;
            }
            if let Some(i) = (0..2).find(|&i| {
                heads[i].is_some_and(|(k, _)| self.options.ignore_metadata_keys.contains(k))
            }) {
                // Key is on the ignore list, skip it
                its[i].next();
                continue;
            }
            match (heads[0], heads[1]) {
                (Some((k0, v0)), h1) if h1.is_none_or(|(k1, _)| k0 < k1) => {
                    // First stream has a key that is not present in the second stream
                    self.compare_kv_entry(diff, k0, Some(v0), None);
                    its[0].next();
                }
                (h0, Some((k1, v1))) if h0.is_none_or(|(k0, _)| k0 > k1) => {
                    // Second stream has a key that is not present in the first stream
                    self.compare_kv_entry(diff, k1, None, Some(v1));
                    its[1].next();
                }
                (Some((k0, v0)), Some((_, v1))) => {
                    // Both streams have the key
                    self.compare_kv_entry(diff, k0, Some(v0), Some(v1));
                    its[0].next();
                    its[1].next();
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn compare_kv_entry(
        &self,
        diff: &mut PrintDiff,
        key: &[u8],
        value1: Option<&[u8]>,
        value2: Option<&[u8]>,
    ) {
        let kind = match key {
            b"KTXdxgiFormat__" | b"KTXmetalPixelFormat" => KvKind::Uint32,
            b"KTXorientation" | b"KTXswizzle" | b"KTXwriter" | b"KTXwriterScParams"
            | b"KTXastcDecodeMode" => KvKind::String,
            b"KTXglFormat" => KvKind::GlFormat,
            b"KTXanimData" => KvKind::AnimData,
            b"KTXcubemapIncomplete" => KvKind::CubemapIncomplete,
            _ => KvKind::Unknown,
        };
        // KVEntry::load: a NULL (empty) value counts as a missing entry
        let values = [value1, value2].map(|v| v.filter(|v| !v.is_empty()));
        let different = match (values[0], values[1]) {
            (Some(a), Some(b)) => a != b,
            _ => true,
        };
        diff.complex(DiffComplex {
            text_header: key.to_vec(),
            json_path: [b"/keyValueData/", key].concat(),
            values: values.map(|v| v.map(|value| KvEntry { kind, value })),
            different,
        });
    }

    fn compare_sgd(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        let ignore = self.options.ignore_sgd;
        if ignore == IgnoreSgd::All {
            return Ok(());
        }
        let format = self.output_format();
        let h = self.headers;
        let is_basis_lz = |i: usize| h[i].supercompression_scheme == SupercmpScheme::BASIS_LZ;

        let mut buffers: [&[u8]; 2] = [&[], &[]];
        for i in 0..2 {
            let index = h[i].supercompression_global_data;
            buffers[i] = self.read(&streams[i], index.byte_offset, index.byte_length, "the SGD")?;
        }

        let mut basis_lz = [SgdBasisLz::default(), SgdBasisLz::default()];
        for i in 0..2 {
            let sgd_byte_length = h[i].supercompression_global_data.byte_length;
            if sgd_byte_length == 0 || !is_basis_lz(i) {
                continue;
            }
            let levels = h[i].level_count.max(1);
            let layers = h[i].layer_count.max(1);
            let depth = |level: u32| h[i].pixel_depth.checked_shr(level).unwrap_or(0).max(1);
            let mut image_count = 0u32;
            for level in 0..levels {
                image_count = image_count.wrapping_add(
                    layers
                        .wrapping_mul(h[i].face_count)
                        .wrapping_mul(depth(level)),
                );
            }

            if sgd_byte_length < BASIS_LZ_GLOBAL_HEADER_SIZE {
                continue;
            }
            let b = buffers[i];
            let bl = &mut basis_lz[i];
            bl.endpoint_count = Some(u16_at(b, 0));
            bl.selector_count = Some(u16_at(b, 2));
            bl.endpoints_byte_length = Some(u64::from(u32_at(b, 4)));
            bl.selectors_byte_length = Some(u64::from(u32_at(b, 8)));
            bl.tables_byte_length = Some(u64::from(u32_at(b, 12)));
            bl.extended_byte_length = Some(u64::from(u32_at(b, 16)));

            let descs_end =
                BASIS_LZ_GLOBAL_HEADER_SIZE + BASIS_LZ_IMAGE_DESC_SIZE * u64::from(image_count);
            if sgd_byte_length < descs_end {
                continue;
            }

            let mut pos = BASIS_LZ_GLOBAL_HEADER_SIZE as usize;
            for level in 0..levels {
                for _layer in 0..layers {
                    for _face in 0..h[i].face_count {
                        for _z_slice in 0..depth(level) {
                            bl.image_flags.push(Some(u32_at(b, pos)));
                            bl.rgb_slice_byte_offset.push(Some(u32_at(b, pos + 4)));
                            bl.rgb_slice_byte_length.push(Some(u32_at(b, pos + 8)));
                            bl.alpha_slice_byte_offset.push(Some(u32_at(b, pos + 12)));
                            bl.alpha_slice_byte_length.push(Some(u32_at(b, pos + 16)));
                            pos += BASIS_LZ_IMAGE_DESC_SIZE as usize;
                        }
                    }
                }
            }

            // Calculate payload offsets
            let endpoints = descs_end;
            let selectors = endpoints.wrapping_add(bl.endpoints_byte_length.unwrap_or(0));
            let tables = selectors.wrapping_add(bl.selectors_byte_length.unwrap_or(0));
            let extended = tables.wrapping_add(bl.tables_byte_length.unwrap_or(0));
            bl.endpoints_byte_offset = Some(endpoints);
            bl.selectors_byte_offset = Some(selectors);
            bl.tables_byte_offset = Some(tables);
            bl.extended_byte_offset = Some(extended);
        }

        // Helper for comparing SGD payloads
        let compare_sgd_payload = |diff: &mut PrintDiff,
                                   text_name: &str,
                                   json_path: &str,
                                   o1: Option<u64>,
                                   l1: Option<u64>,
                                   o2: Option<u64>,
                                   l2: Option<u64>| {
            let mismatch = match (o1, l1, o2, l2) {
                (Some(o1), Some(l1), Some(o2), Some(l2)) => {
                    // An out of bounds situation is considered a mismatch
                    if o1.wrapping_add(l1) > h[0].supercompression_global_data.byte_length
                        || o2.wrapping_add(l2) > h[1].supercompression_global_data.byte_length
                        || l1 != l2
                    {
                        true
                    } else {
                        let a = buffers[0].get(o1 as usize..(o1 + l1) as usize);
                        let b = buffers[1].get(o2 as usize..(o2 + l2) as usize);
                        a != b
                    }
                }
                // If SGD is not present in both files then consider that a mismatch
                _ => true,
            };
            if mismatch {
                diff.mismatch(&format!("{text_name} mismatch"), json_path);
            }
        };

        if is_basis_lz(0) || is_basis_lz(1) {
            diff.set_context("Basis Supercompression Global Data\n\n");

            // Supercompression global data type is only needed in JSON format
            if format != OutputFormat::Text {
                diff.diff(diff_supercmp(
                    format,
                    "supercompressionScheme",
                    "/supercompressionGlobalData/type",
                    [
                        h[0].supercompression_scheme.0,
                        h[1].supercompression_scheme.0,
                    ],
                ));
            }

            let p = |f: &str| format!("/supercompressionGlobalData/{f}");
            diff.diff(diff_plain(
                "endpointCount",
                &p("endpointCount"),
                basis_lz.each_ref().map(|b| b.endpoint_count),
            ));
            diff.diff(diff_plain(
                "selectorCount",
                &p("selectorCount"),
                basis_lz.each_ref().map(|b| b.selector_count),
            ));
            diff.diff(diff_plain(
                "endpointsByteLength",
                &p("endpointsByteLength"),
                basis_lz.each_ref().map(|b| b.endpoints_byte_length),
            ));
            diff.diff(diff_plain(
                "selectorsByteLength",
                &p("selectorsByteLength"),
                basis_lz.each_ref().map(|b| b.selectors_byte_length),
            ));
            diff.diff(diff_plain(
                "tablesByteLength",
                &p("tablesByteLength"),
                basis_lz.each_ref().map(|b| b.tables_byte_length),
            ));
            diff.diff(diff_plain(
                "extendedByteLength",
                &p("extendedByteLength"),
                basis_lz.each_ref().map(|b| b.extended_byte_length),
            ));

            // Make the per image arrays the same size for easier diffing
            let max_image_count = basis_lz[0]
                .image_flags
                .len()
                .max(basis_lz[1].image_flags.len());
            let at = |v: &Vec<Option<u32>>, k: usize| v.get(k).copied().flatten();
            for k in 0..max_image_count {
                let ip = |f: &str| format!("/supercompressionGlobalData/images/{k}/{f}");
                diff.diff(diff_flags(
                    format,
                    &format!("Image{k}.imageFlags"),
                    &ip("imageFlags"),
                    basis_lz.each_ref().map(|b| at(&b.image_flags, k)),
                    ktx_bu_image_flags_bit_string,
                ));
                diff.diff(diff_plain(
                    &format!("Image{k}.rgbSliceByteLength"),
                    &ip("rgbSliceByteLength"),
                    basis_lz.each_ref().map(|b| at(&b.rgb_slice_byte_length, k)),
                ));
                diff.diff(diff_plain(
                    &format!("Image{k}.rgbSliceByteOffset"),
                    &ip("rgbSliceByteOffset"),
                    basis_lz.each_ref().map(|b| at(&b.rgb_slice_byte_offset, k)),
                ));
                diff.diff(diff_plain(
                    &format!("Image{k}.alphaSliceByteLength"),
                    &ip("alphaSliceByteLength"),
                    basis_lz
                        .each_ref()
                        .map(|b| at(&b.alpha_slice_byte_length, k)),
                ));
                diff.diff(diff_plain(
                    &format!("Image{k}.alphaSliceByteOffset"),
                    &ip("alphaSliceByteOffset"),
                    basis_lz
                        .each_ref()
                        .map(|b| at(&b.alpha_slice_byte_offset, k)),
                ));
            }

            if ignore != IgnoreSgd::Payload {
                let [a, b] = &basis_lz;
                compare_sgd_payload(
                    diff,
                    "endpointsData",
                    "/supercompressionGlobalData/endpointsData",
                    a.endpoints_byte_offset,
                    a.endpoints_byte_length,
                    b.endpoints_byte_offset,
                    b.endpoints_byte_length,
                );
                compare_sgd_payload(
                    diff,
                    "selectorsData",
                    "/supercompressionGlobalData/selectorsData",
                    a.selectors_byte_offset,
                    a.selectors_byte_length,
                    b.selectors_byte_offset,
                    b.selectors_byte_length,
                );
                compare_sgd_payload(
                    diff,
                    "tablesData",
                    "/supercompressionGlobalData/tablesData",
                    a.tables_byte_offset,
                    a.tables_byte_length,
                    b.tables_byte_offset,
                    b.tables_byte_length,
                );
                compare_sgd_payload(
                    diff,
                    "extendedData",
                    "/supercompressionGlobalData/extendedData",
                    a.extended_byte_offset,
                    a.extended_byte_length,
                    b.extended_byte_offset,
                    b.extended_byte_length,
                );
            }
        } else if ignore == IgnoreSgd::None {
            diff.set_context("Unrecognized Supercompression Global Data\n\n");
            // Just compare raw payloads of the SGDs
            compare_sgd_payload(
                diff,
                "SGD",
                "/supercompressionGlobalData/rawPayload",
                Some(0),
                Some(h[0].supercompression_global_data.byte_length),
                Some(0),
                Some(h[1].supercompression_global_data.byte_length),
            );
        }
        Ok(())
    }

    fn compare_images(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        match self.options.content_mode {
            ContentMode::Ignore => Ok(()),
            ContentMode::Raw => self.compare_images_raw(diff, streams),
            ContentMode::Image => self.compare_images_per_pixel(diff, streams),
        }
    }

    fn compare_images_raw(&self, diff: &mut PrintDiff, streams: &[InputStream; 2]) -> CmdResult {
        diff.set_context("Image Data\n\n");
        let num_levels = self.headers.map(|h| h.level_count.max(1));
        let max_num_levels = num_levels[0].max(num_levels[1]);

        for level in 0..max_num_levels {
            let mut entries: [Option<LevelIndexEntry>; 2] = [None, None];
            for i in 0..2 {
                if level < num_levels[i] {
                    entries[i] = Some(self.read_level_index(&streams[i], level)?);
                }
            }
            // Missing levels and mismatching level data sizes are always considered a mismatch
            let mismatch = match entries {
                [Some(a), Some(b)] if a.byte_length == b.byte_length => {
                    let what = format!("level {level} data");
                    let da = self.read(&streams[0], a.byte_offset, a.byte_length, &what)?;
                    let db = self.read(&streams[1], b.byte_offset, b.byte_length, &what)?;
                    da != db
                }
                _ => true,
            };
            if mismatch {
                diff.mismatch(
                    &format!("Mismatch in level {level} data"),
                    &format!("m={level}"),
                );
            }
        }
        Ok(())
    }

    fn compare_images_per_pixel(
        &self,
        diff: &mut PrintDiff,
        streams: &[InputStream; 2],
    ) -> CmdResult {
        diff.set_context("Image Data\n\n");
        let space = diff.space();

        // Load texture data, setup format descriptors and image codecs
        let mut textures: Vec<Texture2> = Vec::with_capacity(2);
        let mut codecs: Vec<ImageCodec> = Vec::with_capacity(2);
        let mut file_offsets_valid = [true, true];
        for i in 0..2 {
            let invalid = |what: &str, e| {
                self.report.fatal(
                    ReturnCode::InvalidFile,
                    &format!(
                        "Failed to {what} KTX2 texture from file \"{}\": {}",
                        streams[i].filepath,
                        ktx_error_string(e)
                    ),
                )
            };
            let mut texture = Texture2::create_from_memory(
                &streams[i].data,
                KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
            )
            .map_err(|e| invalid("create", e))?;
            let mut codec = ImageCodec::new(
                VkFormat(self.headers[i].vk_format.0),
                self.headers[i].type_size,
                &texture.p_dfd,
            );
            let model = FormatDescriptor::from_dfd(&texture.p_dfd).model;
            if model == KHR_DF_MODEL_ETC1S {
                // Transcode BasisLZ textures to RGBA8 before comparison
                texture
                    .transcode_basis(TranscodeFmt::RGBA32, 0)
                    .map_err(|e| invalid("transcode", e))?;
                codec = ImageCodec::new(VkFormat::R8G8B8A8_UNORM, 1, &texture.p_dfd);
            } else if model == KHR_DF_MODEL_UASTC_HDR_6x6 {
                // Transcode HDR/BasisLZ textures to RGBA16F before comparison
                texture
                    .transcode_basis(TranscodeFmt::RGBA_HALF, 0)
                    .map_err(|e| invalid("transcode", e))?;
                codec = ImageCodec::new(VkFormat::R16G16B16A16_SFLOAT, 2, &texture.p_dfd);
            }
            // If the image data was supercompressed then file offsets of texel blocks cannot be calculated
            if self.headers[i].supercompression_scheme != SupercmpScheme::NONE {
                file_offsets_valid[i] = false;
            }
            textures.push(texture);
            codecs.push(codec);
        }
        let (t, c) = (&textures, &codecs);

        // Currently, we only support comparing images with matching dimensions
        if t[0].num_dimensions != t[1].num_dimensions
            || t[0].base_width != t[1].base_width
            || t[0].base_height != t[1].base_height
            || t[0].base_depth != t[1].base_depth
            || c[0].get_texel_block_dimensions() != c[1].get_texel_block_dimensions()
        {
            return Err(self.report.fatal(
                ReturnCode::InvalidArguments,
                "Comparison requires matching texture and texel block dimensions.",
            ));
        }
        // Currently, we only support comparing images with matching formats (barring BasisLZ decoding)
        if t[0].vk_format != t[1].vk_format {
            return Err(self.report.fatal(
                ReturnCode::InvalidArguments,
                "Comparison requires matching texture formats (BasisLZ is treated as R8G8B8A8_UNORM).",
            ));
        }

        let max_num_levels = t[0].num_levels.max(t[1].num_levels);
        let max_num_faces = t[0].num_faces.max(t[1].num_faces);
        let max_num_layers = t[0].num_layers.max(t[1].num_layers);
        let mut texel_block_differences = 0usize;

        for level in 0..max_num_levels {
            // Calculate base file offset of level from level data
            let mut level_file_offsets: [Option<u64>; 2] = [None, None];
            for i in 0..2 {
                if file_offsets_valid[i] && level < t[i].num_levels {
                    level_file_offsets[i] =
                        Some(self.read_level_index(&streams[i], level)?.byte_offset);
                }
            }

            let dim = |v: u32| v.checked_shr(level).unwrap_or(0).max(1);
            let (width, height, depth) = (
                dim(t[0].base_width),
                dim(t[0].base_height),
                dim(t[0].base_depth),
            );
            let blocks = c[0].pixel_to_texel_block_size([width, height, depth, 1]);

            // Size returned by libktx is only for a single layer/face/slice
            let image_sizes = [0, 1]
                .map(|i| (t[i].get_image_size(level) as u64).wrapping_mul(u64::from(blocks[2])));

            for layer in 0..max_num_layers {
                for face in 0..max_num_faces {
                    // Handle when image is missing from one of the files
                    if (0..2).any(|i| {
                        level >= t[i].num_levels
                            || face >= t[i].num_faces
                            || layer >= t[i].num_layers
                    }) {
                        diff.image(DiffImage {
                            text_header: format!(
                                "Mismatch in level {level}, layer {layer}, face {face}"
                            ),
                            fragment_uri: format!("m={level},a={layer},f={face}"),
                            file_offsets: [Some(0), Some(0)],
                            pairs: Vec::new(),
                        });
                        continue;
                    }

                    // Calculate image file offset
                    let image_file_offsets: [Option<u64>; 2] = std::array::from_fn(|i| {
                        level_file_offsets[i].map(|o| {
                            let index =
                                u64::from(face.wrapping_add(layer.wrapping_mul(t[i].num_faces)));
                            o.wrapping_add(index.wrapping_mul(image_sizes[i]))
                        })
                    });

                    // Get image data and create image spans from them
                    let image_data: [&[u8]; 2] = std::array::from_fn(|i| {
                        let offset = t[i].get_image_offset(level, layer, face).unwrap_or(0);
                        t[i].get_data()
                            .and_then(|d| d.get(offset..))
                            .unwrap_or_default()
                    });
                    let spans = [0, 1].map(|i| ImageSpan::new(width, height, depth, &c[i]));

                    // Loop through texel blocks
                    let mut image_mismatch = false;
                    let mut pairs = Vec::new();
                    for bz in 0..blocks[2] {
                        for by in 0..blocks[1] {
                            for bx in 0..blocks[0] {
                                let offsets = spans.map(|s| s.texel_block_byte_offset(bx, by, bz));
                                let block =
                                    |i: usize| image_data[i].get(offsets[i]..).unwrap_or_default();
                                // compareTexelBlocksPacked
                                let equal = (0..c[0].get_packed_element_count()).all(|e| {
                                    c[0].get_packed_element(block(0), e)
                                        == c[1].get_packed_element(block(1), e)
                                });
                                if equal {
                                    continue;
                                }
                                image_mismatch = true;
                                texel_block_differences += 1;
                                if texel_block_differences <= self.options.per_pixel_output_limit {
                                    let dims = c[0].get_texel_block_dimensions();
                                    let texel_block = |i: usize| TexelBlock {
                                        pixel: [
                                            bx.wrapping_mul(c[i].get_texel_block_dimensions()[0]),
                                            by.wrapping_mul(c[i].get_texel_block_dimensions()[1]),
                                            bz.wrapping_mul(dims[2]),
                                        ],
                                        byte_offset: offsets[i],
                                        packed: format_packed(&c[i], block(i), space),
                                        channels: (!c[i].is_block_compressed())
                                            .then(|| format_channels(&c[i], block(i), space)),
                                    };
                                    pairs.push((texel_block(0), texel_block(1)));
                                }
                            }
                        }
                    }

                    if image_mismatch {
                        diff.image(DiffImage {
                            text_header: format!(
                                "Mismatch in level {level} layer {layer} face {face}"
                            ),
                            fragment_uri: format!("m={level},a={layer},f={face}"),
                            file_offsets: image_file_offsets,
                            pairs,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

/// Prints one issue as a JSON message object of `compare` (one level deeper
/// than `validate`, without its closing brace).
fn write_issue_json(
    out: &mut Vec<u8>,
    pi: PrintIndent,
    first: &mut bool,
    space: &str,
    nl: &str,
    issue: &ValidationReport,
) {
    if !std::mem::replace(first, false) {
        pi.print(out, 3, format!("}},{nl}").as_bytes());
    }
    pi.print(out, 3, format!("{{{nl}").as_bytes());
    pi.print(
        out,
        4,
        format!("\"id\":{space}{},{nl}", issue.id).as_bytes(),
    );
    pi.print(
        out,
        4,
        format!("\"type\":{space}\"{}\",{nl}", issue.type_.as_str()).as_bytes(),
    );
    pi.print(out, 4, format!("\"message\":{space}\"").as_bytes());
    out.extend_from_slice(&escape_json_copy(issue.message.as_bytes()));
    out.extend_from_slice(format!("\",{nl}").as_bytes());
    pi.print(out, 4, format!("\"details\":{space}\"").as_bytes());
    out.extend_from_slice(&escape_json_copy(&issue.details));
    out.extend_from_slice(format!("\"{nl}").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stoull_like_libstdcxx() {
        assert_eq!(stoull("20"), Some((20, 2)));
        assert_eq!(stoull("3.14"), Some((3, 1)));
        assert_eq!(stoull("0xFF"), Some((0, 1)));
        assert_eq!(stoull(" +7"), Some((7, 3)));
        assert_eq!(stoull("-1"), Some((u64::MAX, 2)));
        assert_eq!(stoull("some"), None);
        assert_eq!(stoull("99999999999999999999"), None);
    }

    #[test]
    fn identifier() {
        let id = *b"\xABKTX 20\xBB\r\n\x1A\n";
        assert_eq!(
            identifier_value(&id, OutputFormat::Text),
            "«KTX 20»\\r\\n\\x1A\\n".as_bytes()
        );
        assert_eq!(
            identifier_value(&id, OutputFormat::Json),
            "\"«KTX 20»\\r\\n\\u001A\\n\"".as_bytes()
        );
    }
}
