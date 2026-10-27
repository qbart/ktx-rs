// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/encode_utils_common.h, encode_utils_astc.h and
//! encode_utils_basis.h: the encoder option groups shared by `ktx create`
//! and `ktx encode` (parsing, validation and `KTXwriterScParams` capture).
//!
//! `ClampedOption` assignments are `clamp_*` helpers on the plain
//! `BasisParams` / `AstcParams` fields.

use super::command::*;
use super::fmt::float_shortest;
use super::options::{OptionSpec, Options, ParseResult};
use crate::encode_params::*;

fn hardware_concurrency() -> u32 {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() as u32)
        .max(1)
}

fn clamp_u32(v: u32, lo: u32, hi: u32) -> u32 {
    v.clamp(lo, hi)
}

/// `std::clamp<float>` (NaN passes through like the C++).
fn clamp_f32(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

// -------------------------------------------------------------------------------------------------

/// Port of `OptionsEncodeCommon`.
#[derive(Debug, Clone)]
pub struct OptionsEncodeCommon {
    pub common_options: String,
    pub normal_map: bool,
    pub thread_count: u32,
    pub no_sse: bool,
}

impl Default for OptionsEncodeCommon {
    fn default() -> Self {
        OptionsEncodeCommon {
            common_options: String::new(),
            normal_map: false,
            thread_count: hardware_concurrency(),
            no_sse: false,
        }
    }
}

impl OptionsEncodeCommon {
    pub const K_NORMAL_MODE: &'static str = "normal-mode";
    pub const K_THREADS: &'static str = "threads";
    pub const K_NO_SSE: &'static str = "no-sse";

    fn capture_common_option(&mut self, name: &str) {
        self.common_options += &format!(" --{name}");
    }
}

impl OptionGroup for OptionsEncodeCommon {
    fn init(&mut self, opts: &mut Options) {
        let g = "Encode common";
        opts.add(OptionSpec::flag(Self::K_NORMAL_MODE, "Optimizes for encoding textures with normal data. If the input texture has three or four linear components it is assumed to be a three component linear normal map storing unit length normals as (R=X, G=Y, B=Z). A fourth component will be ignored. The map will be converted to a two component X+Y normal map stored as (RGB=X, A=Y) prior to encoding. If unsure that your normals are unit length, use --normalize. If the input has 2 linear components it is assumed to be an X+Y map of unit normals.\nThe Z component can be recovered programmatically in shader code by using the equations:\n    nml.xy = texture(...).ga;              // Load in [0,1]\n    nml.xy = nml.xy * 2.0 - 1.0;           // Unpack to [-1,1]\n    nml.z = sqrt(1 - dot(nml.xy, nml.xy)); // Compute Z\nETC1S / BasisLZ encoding, RDO is disabled (no selector RDO, no endpoint RDO) to provide better quality.").group(g));
        opts.add(OptionSpec::value(Self::K_THREADS, "Sets the number of threads to use during encoding. By default, encoding will use the number of threads reported by thread::hardware_concurrency or 1 if value returned is 0.", "<count>").u32().group(g));
        opts.add(OptionSpec::flag(Self::K_NO_SSE, "Forbid use of the SSE instruction set. Ignored if CPU does not support SSE. SSE can only be disabled on the basis-lz and uastc compressors.").group(g));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, _report: &Reporter) -> CmdResult {
        if args.count(Self::K_NORMAL_MODE) > 0 {
            self.capture_common_option(Self::K_NORMAL_MODE);
            self.normal_map = true;
        }
        if args.count(Self::K_THREADS) > 0 {
            let v = args.as_u32(Self::K_THREADS);
            self.common_options += &format!(" --{} {v}", Self::K_THREADS);
            self.thread_count = v;
        }
        if args.count(Self::K_NO_SSE) > 0 {
            self.capture_common_option(Self::K_NO_SSE);
            self.no_sse = true;
        }
        Ok(())
    }
}

/// `fillOptionsCodecBasis`.
pub fn fill_options_codec_basis(common: &OptionsEncodeCommon, params: &mut BasisParams) {
    params.thread_count = common.thread_count;
    params.normal_map = common.normal_map;
    params.no_sse = common.no_sse;
}

/// `fillOptionsCodecAstc`.
pub fn fill_options_codec_astc(common: &OptionsEncodeCommon, params: &mut AstcParams) {
    params.thread_count = common.thread_count;
    params.normal_map = common.normal_map;
}

// -------------------------------------------------------------------------------------------------

/// Port of `OptionsEncodeASTC`.
#[derive(Debug, Clone)]
pub struct OptionsEncodeASTC {
    pub params: AstcParams,
    pub astc_options: String,
    pub encode_astc: bool,
}

impl Default for OptionsEncodeASTC {
    fn default() -> Self {
        OptionsEncodeASTC {
            params: AstcParams {
                struct_size: KTX_ASTC_PARAMS_SIZE,
                thread_count: hardware_concurrency(),
                ..AstcParams::default()
            },
            astc_options: String::new(),
            encode_astc: false,
        }
    }
}

impl OptionsEncodeASTC {
    pub const K_ASTC_QUALITY: &'static str = "astc-quality";
    pub const K_ASTC_PERCEPTUAL: &'static str = "astc-perceptual";
    pub const K_ASTC_OPTIONS: [&'static str; 2] = [Self::K_ASTC_QUALITY, Self::K_ASTC_PERCEPTUAL];
}

impl OptionGroup for OptionsEncodeASTC {
    fn init(&mut self, opts: &mut Options) {
        let g = "Encode ASTC";
        opts.add(OptionSpec::value(Self::K_ASTC_QUALITY, "The quality level configures the quality-performance tradeoff for the compressor; more complete searches of the search space improve image quality at the expense of compression time. Default is 'medium'. The quality level can be set between fastest (0) and exhaustive (100) via the following fixed quality presets:\n\n    Level      |  Quality\n    ---------- | -----------------------------\n    fastest    | (equivalent to quality =   0)\n    fast       | (equivalent to quality =  10)\n    medium     | (equivalent to quality =  60)\n    thorough   | (equivalent to quality =  98)\n    exhaustive | (equivalent to quality = 100)", "<level>").group(g));
        opts.add(OptionSpec::flag(Self::K_ASTC_PERCEPTUAL, "The codec should optimize for perceptual error, instead of direct RMS error. This aims to improve perceived image quality, but typically lowers the measured PSNR score. Perceptual methods are currently only available for normal maps and RGB color data.").group(g));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count(Self::K_ASTC_QUALITY) > 0 {
            let value = args
                .get(Self::K_ASTC_QUALITY)
                .unwrap_or_default()
                .to_string();
            self.astc_options += &format!(" --{} {value}", Self::K_ASTC_QUALITY);
            let level = value.to_lowercase();
            let q = match level.as_str() {
                "fastest" => KTX_PACK_ASTC_QUALITY_LEVEL_FASTEST,
                "fast" => KTX_PACK_ASTC_QUALITY_LEVEL_FAST,
                "medium" => KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM,
                "thorough" => KTX_PACK_ASTC_QUALITY_LEVEL_THOROUGH,
                "exhaustive" => KTX_PACK_ASTC_QUALITY_LEVEL_EXHAUSTIVE,
                _ => {
                    return Err(report.fatal_usage(&format!("Invalid astc-quality: \"{level}\"")));
                }
            };
            self.params.quality_level = clamp_u32(q, 0, KTX_PACK_ASTC_QUALITY_LEVEL_MAX);
        } else {
            self.params.quality_level = KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM;
        }
        if args.count(Self::K_ASTC_PERCEPTUAL) > 0 {
            self.astc_options += &format!(" --{}", Self::K_ASTC_PERCEPTUAL);
            self.params.perceptual = true;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Port of `BasisCodec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BasisCodec {
    #[default]
    None,
    BasisLZ,
    UastcLdr4x4,
    UastcHdr4x4,
    UastcHdr6x6i,
    Invalid,
}

/// `toString(BasisCodec)`.
pub fn to_string_basis_codec(codec: BasisCodec) -> &'static str {
    match codec {
        BasisCodec::None => "NONE",
        BasisCodec::BasisLZ => "BasisLZ",
        BasisCodec::UastcLdr4x4 => "UASTC_LDR_4X4",
        BasisCodec::UastcHdr4x4 => "UASTC_HDR_4x4",
        BasisCodec::UastcHdr6x6i => "UASTC_HDR_6x6i",
        BasisCodec::Invalid => "INVALID",
    }
}

/// Port of `OptionsEncodeBasis<ENCODE_CMD>`.
#[derive(Debug, Clone)]
pub struct OptionsEncodeBasis<const ENCODE_CMD: bool> {
    pub params: BasisParams,
    pub codec_options: String,
    pub codec_name: String,
    pub selected_codec: BasisCodec,
}

impl<const ENCODE_CMD: bool> Default for OptionsEncodeBasis<ENCODE_CMD> {
    fn default() -> Self {
        let params = BasisParams {
            struct_size: KTX_BASIS_PARAMS_SIZE,
            thread_count: hardware_concurrency(),
            // - 1 is to match what basisu_tool does (since 1.13).
            etc1s_compression_level: KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL - 1,
            codec: KTX_BASIS_CODEC_ETC1S, // Default to ETC1S.
            uastc_flags: KTX_PACK_UASTC_LEVEL_DEFAULT,
            uastc_hdr_lambda: 0.0,
            uastc_hdr_level: 2,
            uastc_hdr_quality: 1,
            ..BasisParams::default()
        };
        OptionsEncodeBasis {
            params,
            codec_options: String::new(),
            codec_name: String::new(),
            selected_codec: BasisCodec::None,
        }
    }
}

impl<const ENCODE_CMD: bool> OptionsEncodeBasis<ENCODE_CMD> {
    pub const K_CLEVEL: &'static str = "clevel";
    pub const K_QLEVEL: &'static str = "qlevel";
    pub const K_MAX_ENDPOINTS: &'static str = "max-endpoints";
    pub const K_ENDPOINT_RDO_THRESHOLD: &'static str = "endpoint-rdo-threshold";
    pub const K_MAX_SELECTORS: &'static str = "max-selectors";
    pub const K_SELECTOR_RDO_THRESHOLD: &'static str = "selector-rdo-threshold";
    pub const K_NO_ENDPOINT_RDO: &'static str = "no-endpoint-rdo";
    pub const K_NO_SELECTOR_RDO: &'static str = "no-selector-rdo";
    pub const K_UASTC_QUALITY: &'static str = "uastc-quality";
    pub const K_UASTC_RDO: &'static str = "uastc-rdo";
    pub const K_UASTC_RDO_L: &'static str = "uastc-rdo-l";
    pub const K_UASTC_RDO_D: &'static str = "uastc-rdo-d";
    pub const K_UASTC_RDO_B: &'static str = "uastc-rdo-b";
    pub const K_UASTC_RDO_S: &'static str = "uastc-rdo-s";
    pub const K_UASTC_RDO_F: &'static str = "uastc-rdo-f";
    pub const K_UASTC_RDO_M: &'static str = "uastc-rdo-m";
    pub const K_UASTC_HDR_UBER_MODE: &'static str = "uastc-hdr-uber-mode";
    pub const K_UASTC_HDR_ULTRA_QUANT: &'static str = "uastc-hdr-ultra-quant";
    pub const K_UASTC_HDR_FAVOR_ASTC: &'static str = "uastc-hdr-favor-astc";
    pub const K_UASTC_HDR_LAMBDA: &'static str = "uastc-hdr-lambda";
    pub const K_UASTC_HDR_6X6I_LEVEL: &'static str = "uastc-hdr-6x6i-level";

    /// `validateBasisCodec`.
    fn validate_basis_codec(args: &ParseResult, name: &str) -> BasisCodec {
        if args.count(name) == 0 {
            return BasisCodec::None;
        }
        match args.get(name).unwrap_or_default().to_lowercase().as_str() {
            "basis-lz" => BasisCodec::BasisLZ,
            "uastc" | "uastc-ldr-4x4" => BasisCodec::UastcLdr4x4,
            "uastc-hdr-4x4" => BasisCodec::UastcHdr4x4,
            "uastc-hdr-6x6i" => BasisCodec::UastcHdr6x6i,
            _ => BasisCodec::Invalid,
        }
    }

    fn capture_flag(&mut self, name: &str) {
        self.codec_options += &format!(" --{name}");
    }

    fn capture_u32(&mut self, args: &ParseResult, name: &str) -> u32 {
        let v = args.as_u32(name);
        self.codec_options += &format!(" --{name} {v}");
        v
    }

    fn capture_f32(&mut self, args: &ParseResult, name: &str) -> f32 {
        let v = args.as_f32(name);
        self.codec_options += &format!(" --{name} {}", float_shortest(v));
        v
    }

    fn capture_bool(&mut self, args: &ParseResult, name: &str) -> bool {
        let v = args.as_bool(name);
        self.codec_options += &format!(" --{name} {v}");
        v
    }

    fn invalid(report: &Reporter, msg: String) -> FatalError {
        report.fatal(ReturnCode::InvalidArguments, &msg)
    }

    /// `validateCommonEncodeArg`.
    pub fn validate_common_encode_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.selected_codec == BasisCodec::None {
            return Err(Self::invalid(
                report,
                format!("Invalid use of argument --{name} that only applies to encoding."),
            ));
        }
        Ok(())
    }

    fn validate_basis_lz_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.selected_codec != BasisCodec::BasisLZ {
            return Err(Self::invalid(
                report,
                format!(
                    "Invalid use of argument --{name} that only applies when the used codec is BasisLZ."
                ),
            ));
        }
        Ok(())
    }

    fn validate_basis_lz_endpoint_rdo_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        self.validate_basis_lz_arg(report, name)?;
        if self.params.no_endpoint_rdo {
            return Err(Self::invalid(
                report,
                format!("Invalid use of argument --{name} when endpoint RDO is disabled."),
            ));
        }
        Ok(())
    }

    fn validate_basis_lz_selector_rdo_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        self.validate_basis_lz_arg(report, name)?;
        if self.params.no_selector_rdo {
            return Err(Self::invalid(
                report,
                format!("Invalid use of argument --{name} when selector RDO is disabled."),
            ));
        }
        Ok(())
    }

    fn validate_uastc_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.selected_codec != BasisCodec::UastcLdr4x4 {
            return Err(Self::invalid(
                report,
                format!(
                    "Invalid use of argument --{name} that only applies when the used codec is UASTC."
                ),
            ));
        }
        Ok(())
    }

    fn validate_uastc_rdo_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        self.validate_uastc_arg(report, name)?;
        if !self.params.uastc_rdo {
            return Err(Self::invalid(
                report,
                format!(
                    "Invalid use of argument --{name} when UASTC RDO post-processing was not enabled."
                ),
            ));
        }
        Ok(())
    }

    fn validate_uastc_or_uastc_hdr4x4_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.selected_codec != BasisCodec::UastcLdr4x4
            && self.selected_codec != BasisCodec::UastcHdr4x4
        {
            return Err(Self::invalid(
                report,
                format!(
                    "Invalid use of argument, --{name}, that only applies when the used codec is UASTC or UASTC HDR 4x4."
                ),
            ));
        }
        Ok(())
    }

    fn validate_uastc4x4_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.params.codec != KTX_BASIS_CODEC_UASTC_HDR_4X4 {
            return Err(Self::invalid(
                report,
                format!("Invalid use of argument --{name} when UASTC HDR 4x4 was not enabled."),
            ));
        }
        Ok(())
    }

    fn validate_uastc6x6i_arg(&self, report: &Reporter, name: &str) -> CmdResult {
        if self.params.codec != KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE {
            return Err(Self::invalid(
                report,
                format!("Invalid use of argument --{name} when UASTC HDR 6x6i was not enabled."),
            ));
        }
        Ok(())
    }
}

impl<const ENCODE_CMD: bool> OptionGroup for OptionsEncodeBasis<ENCODE_CMD> {
    fn init(&mut self, opts: &mut Options) {
        let g = "Encode BasisLZ";
        opts.add(OptionSpec::value(Self::K_CLEVEL, "BasisLZ compression level, an encoding speed vs. quality level tradeoff. Range is [0,6], default is 1. Higher values are slower but give higher quality.", "<level>").u32().group(g));
        opts.add(OptionSpec::value(Self::K_QLEVEL, "BasisLZ quality level. Range is [1,255]. Lower gives better compression/lower quality/faster. Higher gives less compression/higher quality/slower. --qlevel automatically determines values for --max-endpoints, --max-selectors, --endpoint-rdo-threshold and --selector-rdo-threshold for the target quality level. Setting these options overrides the values determined by --qlevel which defaults to 128 if neither it nor --max-endpoints and --max-selectors have been set.", "<level>").u32().group(g));
        opts.add(OptionSpec::value(Self::K_MAX_ENDPOINTS, "Manually set the maximum number of color endpoint clusters. Range is [1,16128]. Default is 0, unset.", "<arg>").u32().group(g));
        opts.add(OptionSpec::value(Self::K_ENDPOINT_RDO_THRESHOLD, "Set endpoint RDO quality threshold. The default is 1.25. Lower is higher quality but less quality per output bit (try [1.0,3.0]). This will override the value chosen by --qlevel.", "<arg>").f32().group(g));
        opts.add(OptionSpec::value(Self::K_MAX_SELECTORS, "Manually set the maximum number of color selector clusters from [1,16128]. Default is 0, unset.", "<arg>").u32().group(g));
        opts.add(OptionSpec::value(Self::K_SELECTOR_RDO_THRESHOLD, "Set selector RDO quality threshold. The default is 1.25. Lower is higher quality but less quality per output bit (try [1.0,3.0]). This will override the value chosen by --qlevel.", "<arg>").f32().group(g));
        opts.add(OptionSpec::flag(Self::K_NO_ENDPOINT_RDO, "Disable endpoint rate distortion optimizations. Slightly faster, less noisy output, but lower quality per output bit. Default is to do endpoint RDO.").group(g));
        opts.add(OptionSpec::flag(Self::K_NO_SELECTOR_RDO, "Disable selector rate distortion optimizations. Slightly faster, less noisy output, but lower quality per output bit. Default is to do selector RDO.").group(g));
        let g = "Encode UASTC";
        opts.add(OptionSpec::value(Self::K_UASTC_QUALITY, "UASTC compression level, an encoding speed vs. quality level tradeoff. Range is [0,4], default is 1. Higher values are slower but give higher quality.", "<level>").u32().group(g));
        opts.add(OptionSpec::flag(Self::K_UASTC_RDO, "Enable UASTC RDO post-processing.").group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_RDO_L, "Set UASTC RDO quality scalar to the specified value. Lower values yield higher quality/larger supercompressed files, higher values yield lower quality/smaller supercompressed files. A good range to try is [.25,10]. For normal maps a good range is [.25,.75]. The full range is [.001,10.0]. Default is 1.0.", "<lambda>").f32().group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_RDO_D, "Set UASTC RDO dictionary size in bytes. Default is 4096. Lower values=faster, but give less compression. Range is [64,65536].", "<dictsize>").u32().group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_RDO_B, "Set UASTC RDO max smooth block error scale. Range is [1.0,300.0]. Default is 10.0, 1.0 is disabled. Larger values suppress more artifacts (and allocate more bits) on smooth blocks.", "<scale>").f32().group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_RDO_S, "Set UASTC RDO max smooth block standard deviation. Range is [.01,65536.0]. Default is 18.0. Larger values expand the range of blocks considered smooth.", "<deviation>").f32().group(g));
        opts.add(
            OptionSpec::flag(
                Self::K_UASTC_RDO_F,
                "Do not favor simpler UASTC modes in RDO mode.",
            )
            .group(g),
        );
        opts.add(
            OptionSpec::flag(
                Self::K_UASTC_RDO_M,
                "Disable RDO multithreading (slightly higher compression, deterministic).",
            )
            .group(g),
        );
        opts.add(OptionSpec::flag(Self::K_UASTC_HDR_UBER_MODE, "Allow the UASTC HDR 4x4 encoder to try varying the CEM 11 selectors more for slightly higher quality (slower). This may negatively impact BC6H quality, however.").group(g));
        opts.add(OptionSpec::flag(Self::K_UASTC_HDR_ULTRA_QUANT, "Allow the UASTC HDR 4x4 encoder to try and find better quantized CEM 7/11 endpoint values (slower).").group(g));
        opts.add(OptionSpec::flag(Self::K_UASTC_HDR_FAVOR_ASTC, "By default the UASTC HDR 4x4 encoder tries to strike a balance or even slightly favor BC6H quality. If this option is specified, ASTC HDR 4x4 quality is favored instead.").group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_HDR_LAMBDA, "Enables rate distortion optimization (RDO). The higher this value, the lower the quality, but the smaller the file size. Try 100-20000, or higher values on some images.", "<level>").f32().group(g));
        opts.add(OptionSpec::value(Self::K_UASTC_HDR_6X6I_LEVEL, "Controls the 6x6 HDR intermediate mode encoder performance vs. max quality tradeoff. Range is [0,12]. Default level is 2.", "<level>").u32().group(g));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        let codec_option = if ENCODE_CMD { "codec" } else { "encode" };
        self.selected_codec = Self::validate_basis_codec(args, codec_option);
        match self.selected_codec {
            BasisCodec::None => {}
            BasisCodec::Invalid => {
                return Err(report.fatal_usage(&format!(
                    "Invalid encode codec: \"{}\".",
                    args.get(codec_option).unwrap_or_default()
                )));
            }
            _ => self.codec_name = args.get(codec_option).unwrap_or_default().to_lowercase(),
        }
        match self.selected_codec {
            BasisCodec::BasisLZ => self.params.codec = KTX_BASIS_CODEC_ETC1S,
            BasisCodec::UastcLdr4x4 => self.params.codec = KTX_BASIS_CODEC_UASTC_LDR_4X4,
            BasisCodec::UastcHdr4x4 => self.params.codec = KTX_BASIS_CODEC_UASTC_HDR_4X4,
            BasisCodec::UastcHdr6x6i => {
                self.params.codec = KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE
            }
            _ => {}
        }

        // NOTE: The order of the validation below matters
        if args.count(Self::K_CLEVEL) > 0 {
            self.validate_basis_lz_arg(report, Self::K_CLEVEL)?;
            self.params.etc1s_compression_level = self.capture_u32(args, Self::K_CLEVEL);
        }
        if args.count(Self::K_QLEVEL) > 0 {
            self.validate_basis_lz_arg(report, Self::K_QLEVEL)?;
            self.params.quality_level = clamp_u32(self.capture_u32(args, Self::K_QLEVEL), 1, 255);
        }
        if args.count(Self::K_NO_ENDPOINT_RDO) > 0 {
            self.validate_basis_lz_arg(report, Self::K_NO_ENDPOINT_RDO)?;
            self.capture_flag(Self::K_NO_ENDPOINT_RDO);
            self.params.no_endpoint_rdo = true;
        }
        if args.count(Self::K_NO_SELECTOR_RDO) > 0 {
            self.validate_basis_lz_arg(report, Self::K_NO_SELECTOR_RDO)?;
            self.capture_flag(Self::K_NO_SELECTOR_RDO);
            self.params.no_selector_rdo = true;
        }
        if args.count(Self::K_MAX_ENDPOINTS) > 0 {
            self.validate_basis_lz_endpoint_rdo_arg(report, Self::K_MAX_ENDPOINTS)?;
            self.params.max_endpoints =
                clamp_u32(self.capture_u32(args, Self::K_MAX_ENDPOINTS), 1, 16128);
        }
        if args.count(Self::K_ENDPOINT_RDO_THRESHOLD) > 0 {
            self.validate_basis_lz_endpoint_rdo_arg(report, Self::K_ENDPOINT_RDO_THRESHOLD)?;
            self.params.endpoint_rdo_threshold =
                self.capture_f32(args, Self::K_ENDPOINT_RDO_THRESHOLD);
        }
        if args.count(Self::K_MAX_SELECTORS) > 0 {
            self.validate_basis_lz_selector_rdo_arg(report, Self::K_MAX_SELECTORS)?;
            self.params.max_selectors =
                clamp_u32(self.capture_u32(args, Self::K_MAX_SELECTORS), 1, 16128);
        }
        if args.count(Self::K_SELECTOR_RDO_THRESHOLD) > 0 {
            self.validate_basis_lz_selector_rdo_arg(report, Self::K_SELECTOR_RDO_THRESHOLD)?;
            self.params.selector_rdo_threshold =
                self.capture_f32(args, Self::K_SELECTOR_RDO_THRESHOLD);
        }
        if args.count(Self::K_UASTC_QUALITY) > 0 {
            self.validate_uastc_or_uastc_hdr4x4_arg(report, Self::K_UASTC_QUALITY)?;
            let level = self
                .capture_u32(args, Self::K_UASTC_QUALITY)
                .min(KTX_PACK_UASTC_MAX_LEVEL);
            // keep non-level flags as they are (i.e., clear level bits)
            self.params.uastc_flags &= !KTX_PACK_UASTC_LEVEL_MASK;
            self.params.uastc_flags |= level;
            self.params.uastc_hdr_quality = clamp_u32(level, 0, 4);
        }
        if args.count(Self::K_UASTC_RDO) > 0 {
            self.validate_uastc_arg(report, Self::K_UASTC_RDO)?;
            self.capture_flag(Self::K_UASTC_RDO);
            self.params.uastc_rdo = true;
        }
        if args.count(Self::K_UASTC_RDO_L) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_L)?;
            self.params.uastc_rdo_quality_scalar =
                clamp_f32(self.capture_f32(args, Self::K_UASTC_RDO_L), 0.001, 50.0);
        }
        if args.count(Self::K_UASTC_RDO_D) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_D)?;
            self.params.uastc_rdo_dict_size =
                clamp_u32(self.capture_u32(args, Self::K_UASTC_RDO_D), 256, 65536);
        }
        if args.count(Self::K_UASTC_RDO_B) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_B)?;
            self.params.uastc_rdo_max_smooth_block_error_scale =
                clamp_f32(self.capture_f32(args, Self::K_UASTC_RDO_B), 1.0, 300.0);
        }
        if args.count(Self::K_UASTC_RDO_S) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_S)?;
            self.params.uastc_rdo_max_smooth_block_std_dev =
                clamp_f32(self.capture_f32(args, Self::K_UASTC_RDO_S), 0.01, 65536.0);
        }
        if args.count(Self::K_UASTC_RDO_F) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_F)?;
            self.capture_flag(Self::K_UASTC_RDO_F);
            self.params.uastc_rdo_dont_favor_simpler_modes = true;
        }
        if args.count(Self::K_UASTC_RDO_M) > 0 {
            self.validate_uastc_rdo_arg(report, Self::K_UASTC_RDO_M)?;
            self.capture_flag(Self::K_UASTC_RDO_M);
            self.params.uastc_rdo_no_multithreading = true;
        }
        if args.count(Self::K_UASTC_HDR_UBER_MODE) > 0 {
            self.validate_uastc4x4_arg(report, Self::K_UASTC_HDR_UBER_MODE)?;
            self.params.uastc_hdr_uber_mode = self.capture_bool(args, Self::K_UASTC_HDR_UBER_MODE);
        }
        if args.count(Self::K_UASTC_HDR_ULTRA_QUANT) > 0 {
            self.validate_uastc4x4_arg(report, Self::K_UASTC_HDR_ULTRA_QUANT)?;
            self.params.uastc_hdr_ultra_quant =
                self.capture_bool(args, Self::K_UASTC_HDR_ULTRA_QUANT);
        }
        if args.count(Self::K_UASTC_HDR_FAVOR_ASTC) > 0 {
            self.validate_uastc4x4_arg(report, Self::K_UASTC_HDR_FAVOR_ASTC)?;
            self.params.uastc_hdr_favor_astc =
                self.capture_bool(args, Self::K_UASTC_HDR_FAVOR_ASTC);
        }
        if args.count(Self::K_UASTC_HDR_LAMBDA) > 0 {
            self.validate_uastc6x6i_arg(report, Self::K_UASTC_HDR_LAMBDA)?;
            self.params.uastc_hdr_lambda = clamp_f32(
                self.capture_f32(args, Self::K_UASTC_HDR_LAMBDA),
                0.0,
                f32::MAX,
            );
        }
        if args.count(Self::K_UASTC_HDR_6X6I_LEVEL) > 0 {
            self.validate_uastc6x6i_arg(report, Self::K_UASTC_HDR_6X6I_LEVEL)?;
            self.params.uastc_hdr_level =
                clamp_u32(self.capture_u32(args, Self::K_UASTC_HDR_6X6I_LEVEL), 0, 12);
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Port of `OptionsMetrics` (metrics_utils.h).
#[derive(Debug, Clone, Default)]
pub struct OptionsMetrics {
    pub compare_ssim: bool,
    pub compare_psnr: bool,
}

impl OptionGroup for OptionsMetrics {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag("compare-ssim", "Calculate encoding structural similarity index measure (SSIM) and print it to stdout or stderr if stdout is used to write a KTX file. Requires Basis-LZ, UASTC or ASTC encoding."));
        opts.add(OptionSpec::flag("compare-psnr", "Calculate encoding peak signal-to-noise ratio (PSNR) and print it to stdout or stderr if stdout is used to write a KTX file. Requires Basis-LZ, UASTC or ASTC encoding."));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, _report: &Reporter) -> CmdResult {
        self.compare_ssim = args.as_bool("compare-ssim");
        self.compare_psnr = args.as_bool("compare-psnr");
        Ok(())
    }
}
