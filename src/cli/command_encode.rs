//! Port of tools/ktx/command_encode.cpp (`ktx encode`).

use super::command::*;
use super::deflate_utils::OptionsDeflate;
use super::encode_utils::*;
use super::formats::{parse_vk_format, to_string_transfer, to_string_vk_format};
use super::formats_gen::is_format_astc;
use super::metrics_utils::MetricsCalculator;
use super::options::{OptionSpec, Options, ParseResult};
use crate::defs::{KTX_WRITER_KEY, KTX_WRITER_SCPARAMS_KEY, SupercmpScheme};
use crate::dfd::khr_df::{KHR_DF_MODEL_ASTC, KHR_DF_TRANSFER_LINEAR};
use crate::dfd::{MODEL, bdb, dfdval};
use crate::encode_params::KTX_PACK_ASTC_ENCODER_MODE_LDR;
use crate::strings::ktx_supercompression_scheme_string;
use crate::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, VkFormat, ktx_error_string};

/// `CommandEncode::OptionsEncode`.
#[derive(Debug, Clone)]
struct OptionsEncode {
    vk_format: VkFormat,
}

impl Default for OptionsEncode {
    fn default() -> Self {
        OptionsEncode {
            vk_format: VkFormat::UNDEFINED,
        }
    }
}

impl OptionsEncode {
    const K_FORMAT: &'static str = "format";
    const K_CODEC: &'static str = "codec";
}

impl OptionGroup for OptionsEncode {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            Self::K_FORMAT,
            "KTX format enum that specifies the KTX file output format. The enum names are matching the VkFormats without the VK_FORMAT_ prefix. The VK_FORMAT_ prefix is ignored if present.\nIt can't be used with --codec.\nThe value must be an ASTC format. When specified the ASTC encoder specific options becomes valid. Case insensitive.",
            "<enum>",
        ));
        opts.add(OptionSpec::value(
            Self::K_CODEC,
            "Target codec. With each encoding option the encoder specific options become valid, otherwise they are ignored. Case-insensitive.\nPossible options are: basis-lz | uastc | uastc-ldr-4x4 | uastc-hdr-4x4 | uastc-hdr-6x6i",
            "<target>",
        ));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count(Self::K_CODEC) > 0 && args.count(Self::K_FORMAT) > 0 {
            return Err(report.fatal_usage("Format and codec can't be both specified together."));
        }

        if args.count(Self::K_FORMAT) > 0 {
            let format_str = args.get(Self::K_FORMAT).unwrap_or_default();
            let Some(parsed) = parse_vk_format(format_str) else {
                return Err(report.fatal_usage(&format!(
                    "The requested format is invalid or unsupported: \"{format_str}\"."
                )));
            };
            self.vk_format = parsed;
            if !is_format_astc(self.vk_format) {
                return Err(report.fatal_usage("Optional option 'format' is not an ASTC format."));
            }
        }
        Ok(())
    }
}

/// Port of `CommandEncode`.
#[derive(Debug, Default)]
pub struct CommandEncode {
    report: Reporter,
    options: OptionsEncode,
    astc: OptionsEncodeASTC,
    basis: OptionsEncodeBasis<true>,
    common: OptionsEncodeCommon,
    metrics: OptionsMetrics,
    deflate: OptionsDeflate,
    in_out: OptionsSingleInSingleOut,
    generic: OptionsGeneric,
}

impl CommandEncode {
    /// Port of `CommandEncode::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandEncode::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx encode",
            "Encode the KTX file specified as the input-file argument,\n    optionally supercompress the result, and save it as the output-file.",
            args,
            &mut [
                &mut self.options,
                &mut self.astc,
                &mut self.basis,
                &mut self.common,
                &mut self.metrics,
                &mut self.deflate,
                &mut self.in_out,
                &mut self.generic,
            ],
        )?;
        self.process_options()?;
        self.execute_encode()
    }

    /// `CommandEncode::processOptions` (after the option groups).
    fn process_options(&mut self) -> CmdResult {
        let report = &self.report;
        fill_options_codec_basis(&self.common, &mut self.basis.params);

        let codec = self.basis.selected_codec;
        if matches!(codec, BasisCodec::None | BasisCodec::Invalid)
            && self.options.vk_format == VkFormat::UNDEFINED
        {
            return Err(report.fatal_usage("Either codec or format must be specified"));
        }

        if codec == BasisCodec::BasisLZ {
            if self.deflate.zstd.is_some() {
                return Err(
                    report.fatal_usage("Cannot encode to BasisLZ and supercompress with Zstd.")
                );
            }
            if self.deflate.zlib.is_some() {
                return Err(
                    report.fatal_usage("Cannot encode to BasisLZ and supercompress with ZLIB.")
                );
            }
        }

        if codec == BasisCodec::UastcHdr6x6i {
            if self.deflate.zstd.is_some() {
                return Err(
                    report.fatal_usage("Cannot encode to UASTC-HDR and supercompress with Zstd.")
                );
            }
            if self.deflate.zlib.is_some() {
                return Err(
                    report.fatal_usage("Cannot encode to UASTC-HDR and supercompress with ZLIB.")
                );
            }
        }

        let basis_codec = matches!(
            codec,
            BasisCodec::BasisLZ
                | BasisCodec::UastcLdr4x4
                | BasisCodec::UastcHdr4x4
                | BasisCodec::UastcHdr6x6i
        );
        let astc_codec = is_format_astc(self.options.vk_format);
        let can_compare = basis_codec || astc_codec;

        if self.metrics.compare_ssim && !can_compare {
            return Err(report.fatal_usage(
                "--compare-ssim can only be used with BasisLZ, UASTC or ASTC encoding.",
            ));
        }
        if self.metrics.compare_psnr && !can_compare {
            return Err(report.fatal_usage(
                "--compare-psnr can only be used with BasisLZ, UASTC or ASTC encoding.",
            ));
        }

        if astc_codec {
            self.astc.encode_astc = true;
            fill_options_codec_astc(&self.common, &mut self.astc.params);
            if self.common.no_sse {
                return Err(report.fatal_usage(&format!(
                    "--{} is not allowed with ASTC encode",
                    OptionsEncodeCommon::K_NO_SSE
                )));
            }
        }
        Ok(())
    }

    fn execute_encode(&mut self) -> CmdResult {
        let report = &self.report;
        let input = InputStream::new(&self.in_out.input_filepath, report)?;
        validate_tool_input(&input.data, report)?;

        let mut texture =
            match Texture2::create_from_memory(&input.data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
            {
                Ok(t) => t,
                Err(e) => {
                    return Err(report.fatal(
                        ReturnCode::InvalidFile,
                        &format!("Failed to create KTX2 texture: {}", ktx_error_string(e)),
                    ));
                }
            };

        if texture.supercompression_scheme != SupercmpScheme::NONE {
            return Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "Cannot encode KTX2 file with {} supercompression.",
                    ktx_supercompression_scheme_string(texture.supercompression_scheme)
                ),
            ));
        }

        if dfdval(bdb(&texture.p_dfd), MODEL) == KHR_DF_MODEL_ASTC && self.astc.encode_astc {
            return Err(report.fatal_usage(&format!(
                "Encoding from ASTC format {} to another ASTC format {} is not supported.",
                to_string_vk_format(texture.vk_format),
                to_string_vk_format(self.options.vk_format)
            )));
        }

        let codec = self.basis.selected_codec;
        if matches!(
            codec,
            BasisCodec::None | BasisCodec::BasisLZ | BasisCodec::UastcLdr4x4
        ) {
            match texture.vk_format {
                VkFormat::R8_UNORM
                | VkFormat::R8_SRGB
                | VkFormat::R8G8_UNORM
                | VkFormat::R8G8_SRGB
                | VkFormat::R8G8B8_UNORM
                | VkFormat::R8G8B8_SRGB
                | VkFormat::R8G8B8A8_UNORM
                | VkFormat::R8G8B8A8_SRGB => {
                    // Allowed formats
                }
                _ => {
                    return Err(report.fatal_usage(&format!(
                        "Only R8, RG8, RGB8, or RGBA8 UNORM and SRGB formats can be encoded to {}, but format is {}.",
                        to_string_basis_codec(codec),
                        to_string_vk_format(texture.vk_format)
                    )));
                }
            }
        } else if matches!(codec, BasisCodec::UastcHdr4x4 | BasisCodec::UastcHdr6x6i) {
            match texture.vk_format {
                VkFormat::R16G16B16_SFLOAT | VkFormat::R16G16B16A16_SFLOAT => {
                    // Allowed formats
                }
                _ => {
                    return Err(report.fatal_usage(&format!(
                        "Only RGB16 or RGBA16 SFLOAT can be encoded to {}, but format is {}.",
                        to_string_basis_codec(codec),
                        to_string_vk_format(texture.vk_format)
                    )));
                }
            }
        }

        // Convert 1D textures to 2D (we could consider 1D as an invalid input)
        texture.num_dimensions = texture.num_dimensions.max(2);

        // Modify KTXwriter metadata
        let writer = format!("{} {}", report.command_name, version(self.generic.testrun));
        let _ = texture.kv_data_head.delete_kv_pair(KTX_WRITER_KEY);
        let mut value = writer.into_bytes();
        value.push(0); // +1 to include the \0
        let _ = texture.kv_data_head.add_kv_pair(KTX_WRITER_KEY, &value);

        let tf = texture.get_transfer_function_e();
        if self.basis.params.normal_map && tf != KHR_DF_TRANSFER_LINEAR {
            return Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "--normal-mode specified but the input file uses non-linear transfer function {}.",
                    to_string_transfer(tf)
                ),
            ));
        }

        let mut metrics = MetricsCalculator::default();
        metrics.save_reference_images(&texture, &self.metrics);

        if self.options.vk_format != VkFormat::UNDEFINED {
            // The C++ never sets the block dimension (TODO: fix me for HDR textures)
            self.astc.params.mode = KTX_PACK_ASTC_ENCODER_MODE_LDR;
            if let Err(e) = texture.compress_astc_ex(&self.astc.params) {
                return Err(report.fatal(
                    ReturnCode::IoFailure,
                    &format!(
                        "Failed to encode KTX2 file to ASTC. KTX Error: {}",
                        ktx_error_string(e)
                    ),
                ));
            }
        } else if matches!(self.basis.selected_codec, BasisCodec::UastcHdr6x6i) {
            // TODO: the UASTC HDR 6x6i encoder is not ported yet.
            return Err(report.fatal(
                ReturnCode::NotImplemented,
                &format!(
                    "Failed to encode KTX2 file with codec \"{}\". The UASTC HDR 6x6i encoder is not implemented.",
                    self.basis.codec_name
                ),
            ));
        } else if let Err(e) = texture.compress_basis_ex(&self.basis.params) {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!(
                    "Failed to encode KTX2 file with codec \"{}\". KTX Error: {}",
                    self.basis.codec_name,
                    ktx_error_string(e)
                ),
            ));
        }

        let output_path = std::path::Path::new(&self.in_out.output_filepath);
        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut output = OutputStream::new(&self.in_out.output_filepath, report)?;

        // If output is stdout, we write metrics to stderr (this is done to avoid polluting the
        // KTX file binary in stdout)
        metrics.decode_and_calculate_metrics(
            &mut texture,
            &self.metrics,
            report,
            output.is_stdout(),
        )?;

        if let Some(level) = self.deflate.zstd
            && let Err(e) = texture.deflate_zstd(level)
        {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!("Zstd deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }
        if let Some(level) = self.deflate.zlib
            && let Err(e) = texture.deflate_zlib(level)
        {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!("ZLIB deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }

        // Add KTXwriterScParams metadata
        let sc_params = format!(
            "{}{}{}{}",
            self.astc.astc_options,
            self.basis.codec_options,
            self.common.common_options,
            self.deflate.compress_options
        );
        let _ = texture.kv_data_head.delete_kv_pair(KTX_WRITER_SCPARAMS_KEY);
        if let Some(rest) = sc_params.strip_prefix(' ') {
            // Options always contain a leading space; size() bytes from c_str() + 1
            // include the terminating NUL.
            let mut value = rest.as_bytes().to_vec();
            value.push(0);
            let _ = texture
                .kv_data_head
                .add_kv_pair(KTX_WRITER_SCPARAMS_KEY, &value);
        }

        // Save output file
        output.write_ktx2(&mut texture, report)
    }
}
