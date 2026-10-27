// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/transcode_utils.h / transcode_utils.cpp: transcode target
//! selection, channel swizzles and the `transcode` helper shared by
//! `ktx transcode` and `ktx extract`.

use super::command::*;
use super::formats::to_string_channel;
use super::options::{Options, ParseResult};
use crate::dfd::khr_df::*;
use crate::dfd::{CHANNELID, MODEL, bdb, dfdsamplecount, dfdsval, dfdval};
use crate::texture::{KtxTexture, TextureCreateInfo, TextureCreateStorage};
use crate::{Texture2, TranscodeFmt, VkFormat, ktx_error_string};

/// Port of `TranscodeSwizzleInfo`.
#[derive(Debug, Clone, Default)]
pub struct TranscodeSwizzleInfo {
    pub default_num_components: u32,
    pub swizzle: String,
}

/// Port of `getChannelType`.
pub fn get_channel_type(texture: &Texture2, index: u32) -> Option<u32> {
    let bdfd = bdb(&texture.p_dfd);
    if dfdsamplecount(bdfd) <= index {
        return None;
    }
    Some(dfdsval(bdfd, index, CHANNELID))
}

fn channel_str(model: u32, channel: Option<u32>) -> String {
    channel.map_or_else(|| "-".to_string(), |c| to_string_channel(model, c))
}

/// Port of `determineTranscodeSwizzle`.
pub fn determine_transcode_swizzle(
    texture: &Texture2,
    report: &Reporter,
) -> CmdResult<TranscodeSwizzleInfo> {
    let model = dfdval(bdb(&texture.p_dfd), MODEL);
    let sample0 = get_channel_type(texture, 0);
    let sample1 = get_channel_type(texture, 1);
    let info = |n: u32, s: &str| {
        Ok(TranscodeSwizzleInfo {
            default_num_components: n,
            swizzle: s.to_string(),
        })
    };

    if texture.supercompression_scheme == crate::SupercmpScheme::BASIS_LZ {
        if sample0 == Some(KHR_DF_CHANNEL_ETC1S_RGB) && sample1 == Some(KHR_DF_CHANNEL_ETC1S_AAA) {
            info(4, "rgba")
        } else if sample0 == Some(KHR_DF_CHANNEL_ETC1S_RGB) {
            info(3, "rgb1")
        } else if sample0 == Some(KHR_DF_CHANNEL_ETC1S_RRR)
            && sample1 == Some(KHR_DF_CHANNEL_ETC1S_GGG)
        {
            info(2, "ra01")
        } else if sample0 == Some(KHR_DF_CHANNEL_ETC1S_RRR) {
            info(1, "r001")
        } else {
            Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "Unsupported channel types for Basis-LZ transcoding: {}, {}",
                    channel_str(KHR_DF_MODEL_ETC1S, sample0),
                    channel_str(KHR_DF_MODEL_ETC1S, sample1)
                ),
            ))
        }
    } else if model == KHR_DF_MODEL_UASTC {
        match sample0 {
            Some(KHR_DF_CHANNEL_UASTC_RGBA) => info(4, "rgba"),
            Some(KHR_DF_CHANNEL_UASTC_RGB) => info(3, "rgb1"),
            Some(KHR_DF_CHANNEL_UASTC_RRRG) => info(2, "ra01"),
            Some(KHR_DF_CHANNEL_UASTC_RG) => info(2, "rg01"),
            Some(KHR_DF_CHANNEL_UASTC_RRR) => info(1, "r001"),
            _ => Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "Unsupported channel type for UASTC transcoding: {}",
                    channel_str(KHR_DF_MODEL_UASTC, sample0)
                ),
            )),
        }
    } else if model == KHR_DF_MODEL_UASTC_HDR_4x4 || model == KHR_DF_MODEL_UASTC_HDR_6x6 {
        info(4, "rgba")
    } else {
        Err(report.fatal(
            ReturnCode::InvalidFile,
            "Requested transcoding but input file is neither BasisLZ, nor UASTC",
        ))
    }
}

/// Port of `OptionsTranscodeTarget<TRANSCODE_CMD>`.
#[derive(Debug, Clone, Default)]
pub struct OptionsTranscodeTarget<const TRANSCODE_CMD: bool> {
    pub transcode_target: Option<TranscodeFmt>,
    pub transcode_target_name: String,
    pub transcode_swizzle_components: u32,
    pub transcode_swizzle: String,
}

/// Target name -> (format, swizzle components).
const TARGETS: [(&str, TranscodeFmt, u32); 20] = [
    ("etc-rgb", TranscodeFmt::ETC1_RGB, 0),
    ("etc-rgba", TranscodeFmt::ETC2_RGBA, 0),
    ("eac-r11", TranscodeFmt::ETC2_EAC_R11, 0),
    ("eac-rg11", TranscodeFmt::ETC2_EAC_RG11, 0),
    ("bc1", TranscodeFmt::BC1_RGB, 0),
    ("bc3", TranscodeFmt::BC3_RGBA, 0),
    ("bc4", TranscodeFmt::BC4_R, 0),
    ("bc5", TranscodeFmt::BC5_RG, 0),
    ("bc7", TranscodeFmt::BC7_RGBA, 0),
    ("astc", TranscodeFmt::ASTC_4x4_RGBA, 0),
    ("r8", TranscodeFmt::RGBA32, 1),
    ("rg8", TranscodeFmt::RGBA32, 2),
    ("rgb8", TranscodeFmt::RGBA32, 3),
    ("rgba8", TranscodeFmt::RGBA32, 4),
    ("rgb16f", TranscodeFmt::RGB_HALF, 0),
    ("rgba16f", TranscodeFmt::RGBA_HALF, 0),
    ("rgb9e5", TranscodeFmt::RGB_9E5, 0),
    ("astc-hdr-4x4", TranscodeFmt::ASTC_HDR_4x4_RGBA, 0),
    ("astc-hdr-6x6", TranscodeFmt::ASTC_HDR_6x6_RGBA, 0),
    ("bc6hu", TranscodeFmt::BC6HU_RGB, 0),
];

impl<const TRANSCODE_CMD: bool> OptionGroup for OptionsTranscodeTarget<TRANSCODE_CMD> {
    fn init(&mut self, _opts: &mut Options) {}

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        // "transcode" command - optional "target" argument
        // "extract" command - optional "transcode" argument
        let arg_name = if TRANSCODE_CMD { "target" } else { "transcode" };
        if args.count(arg_name) > 0 {
            let arg_str = args.get(arg_name).unwrap_or_default().to_lowercase();
            let Some(&(_, fmt, components)) = TARGETS.iter().find(|t| t.0 == arg_str) else {
                return Err(
                    report.fatal_usage(&format!("Invalid transcode target: \"{arg_str}\"."))
                );
            };
            self.transcode_target = Some(fmt);
            self.transcode_target_name = arg_str;
            self.transcode_swizzle_components = components;
        }
        Ok(())
    }
}

impl<const TRANSCODE_CMD: bool> OptionsTranscodeTarget<TRANSCODE_CMD> {
    /// `isTargetHDR`.
    pub fn is_target_hdr(target: TranscodeFmt) -> bool {
        matches!(
            target,
            TranscodeFmt::RGB_HALF
                | TranscodeFmt::RGBA_HALF
                | TranscodeFmt::RGB_9E5
                | TranscodeFmt::ASTC_HDR_4x4_RGBA
                | TranscodeFmt::ASTC_HDR_6x6_RGBA
                | TranscodeFmt::BC6HU_RGB
        )
    }

    /// `validateTextureTranscode`.
    pub fn validate_texture_transcode(
        &mut self,
        texture: &Texture2,
        report: &Reporter,
    ) -> CmdResult {
        let tswizzle = determine_transcode_swizzle(texture, report)?;
        let model = dfdval(bdb(&texture.p_dfd), MODEL);
        let is_src_hdr = model == KHR_DF_MODEL_UASTC_HDR_6x6 || model == KHR_DF_MODEL_UASTC_HDR_4x4;
        match self.transcode_target {
            None => {
                let (fmt, name) = if is_src_hdr {
                    (TranscodeFmt::RGBA_HALF, "rgba16f")
                } else {
                    (TranscodeFmt::RGBA32, "rgba8")
                };
                self.transcode_target = Some(fmt);
                self.transcode_target_name = name.to_string();
                self.transcode_swizzle_components = tswizzle.default_num_components;
            }
            Some(target) if is_src_hdr && !Self::is_target_hdr(target) => {
                return Err(
                    report.fatal_usage("UASTC HDR textures can only be transcoded to HDR formats.")
                );
            }
            Some(target) if !is_src_hdr && Self::is_target_hdr(target) => {
                return Err(report.fatal_usage(
                    "BasisLZ and UASTC LDR textures can only be transcoded to LDR formats.",
                ));
            }
            Some(_) => {}
        }
        self.transcode_swizzle = tswizzle.swizzle;
        Ok(())
    }
}

/// `swizzlePixel` for 8-bit components.
fn swizzle_pixel(src: &[u8], swizzle: u8) -> u8 {
    match swizzle {
        b'r' => src.first().copied().unwrap_or(0),
        b'g' => src.get(1).copied().unwrap_or(0),
        b'b' => src.get(2).copied().unwrap_or(0),
        b'a' => src.get(3).copied().unwrap_or(0),
        b'1' => 255,
        _ => 0,
    }
}

/// `rgba8image::copyTo{R,RG,RGB}` (`dst_comps` < 4) or in-place `swizzle`
/// (`dst_comps` == 4) over `pixels` RGBA8 texels.
fn copy_swizzled(src: &[u8], dst: &mut [u8], dst_comps: usize, swizzle: &[u8]) {
    for (px, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.chunks_exact_mut(dst_comps))
    {
        for (c, v) in d.iter_mut().enumerate() {
            *v = swizzle_pixel(px, swizzle.get(c).copied().unwrap_or(b'0'));
        }
    }
}

/// Port of `transcode`: transcodes `texture` to the selected target, then
/// converts RGBA8 output to fewer channels or swizzles it when needed.
pub fn transcode<const TRANSCODE_CMD: bool>(
    mut texture: Texture2,
    options: &mut OptionsTranscodeTarget<TRANSCODE_CMD>,
    report: &Reporter,
) -> CmdResult<Texture2> {
    options.validate_texture_transcode(&texture, report)?;

    let target = options.transcode_target.unwrap_or(TranscodeFmt::RGBA32);
    if let Err(e) = texture.transcode_basis(target, 0) {
        return Err(report.fatal(
            ReturnCode::InvalidFile,
            &format!("Failed to transcode KTX2 texture: {}", ktx_error_string(e)),
        ));
    }

    // Need to perform format conversion and swizzling if needed
    let mut need_format_conversion = false;
    let mut need_swizzle = false;
    let components = options.transcode_swizzle_components;
    if components != 0 {
        if components == 4 {
            if options.transcode_swizzle != "rgba" {
                need_swizzle = true;
            }
        } else {
            need_format_conversion = true;
            need_swizzle = true;
        }
    }

    let mut converted = None;
    if need_format_conversion {
        let srgb = texture.vk_format == VkFormat::R8G8B8A8_SRGB;
        let vk_format = match (components, srgb) {
            (1, true) => VkFormat::R8_SRGB,
            (1, false) => VkFormat::R8_UNORM,
            (2, true) => VkFormat::R8G8_SRGB,
            (2, false) => VkFormat::R8G8_UNORM,
            (_, true) => VkFormat::R8G8B8_SRGB,
            (_, false) => VkFormat::R8G8B8_UNORM,
        };
        let create_info = TextureCreateInfo {
            gl_internalformat: 0,
            vk_format,
            dfd: None,
            base_width: texture.base_width,
            base_height: texture.base_height,
            base_depth: texture.base_depth,
            num_dimensions: texture.num_dimensions,
            num_levels: texture.num_levels,
            num_layers: texture.num_layers,
            num_faces: texture.num_faces,
            is_array: texture.is_array,
            generate_mipmaps: texture.generate_mipmaps,
        };
        match Texture2::create(&create_info, TextureCreateStorage::AllocStorage) {
            Ok(t) => converted = Some(t),
            Err(e) => {
                return Err(report.fatal(
                    ReturnCode::IoFailure,
                    &format!("Failed to create output texture: {}", ktx_error_string(e)),
                ));
            }
        }
    }

    if need_format_conversion || need_swizzle {
        let swizzle = options.transcode_swizzle.as_bytes();
        for level in 0..texture.num_levels {
            let width = (texture.base_width >> level).max(1) as usize;
            let height = (texture.base_height >> level).max(1) as usize;
            let depth = (texture.base_depth >> level).max(1);
            let pixels = width * height;
            for face in 0..texture.num_faces {
                for layer in 0..texture.num_layers {
                    for depth_index in 0..depth {
                        let face_slice = face + depth_index;
                        let Ok(src_ofs) = texture.get_image_offset(level, layer, face_slice) else {
                            continue;
                        };
                        match converted.as_mut() {
                            Some(out) => {
                                let Ok(dst_ofs) = out.get_image_offset(level, layer, face_slice)
                                else {
                                    continue;
                                };
                                let comps = components as usize;
                                let (Some(src), Some(dst)) = (
                                    texture
                                        .p_data
                                        .as_deref()
                                        .and_then(|d| d.get(src_ofs..src_ofs + pixels * 4)),
                                    out.p_data
                                        .as_deref_mut()
                                        .and_then(|d| d.get_mut(dst_ofs..dst_ofs + pixels * comps)),
                                ) else {
                                    continue;
                                };
                                copy_swizzled(src, dst, comps, swizzle);
                            }
                            None => {
                                // Swizzle in-place
                                let Some(img) = texture
                                    .p_data
                                    .as_deref_mut()
                                    .and_then(|d| d.get_mut(src_ofs..src_ofs + pixels * 4))
                                else {
                                    continue;
                                };
                                let src = img.to_vec();
                                copy_swizzled(&src, img, 4, swizzle);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(converted.unwrap_or(texture))
}
