// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of `MetricsCalculator` (tools/ktx/metrics_utils.h): SSIM / PSNR of an encoded texture
//! against the uncompressed input images.

use super::command::*;
use super::encode_utils::OptionsMetrics;
use super::transcode_utils::{TranscodeSwizzleInfo, determine_transcode_swizzle};
use crate::basisu::ssim::{Image, ImageMetrics, compute_ssim};
use crate::dfd::khr_df::KHR_DF_MODEL_ASTC;
use crate::dfd::{MODEL, bdb, dfdval};
use crate::texture::KtxTexture;
use crate::{Texture2, TranscodeFmt, ktx_error_string};

/// `fmt::format("{:+7.6f}", v)`.
fn signed6(v: f32) -> String {
    format!("{v:+7.6}")
}

/// `rgba8image::swizzle(swizzle)` of one pixel.
fn swizzle_pixel(p: [u8; 4], swizzle: &str) -> [u8; 4] {
    let s = swizzle.as_bytes();
    std::array::from_fn(|i| match s.get(i) {
        Some(b'r') => p[0],
        Some(b'g') => p[1],
        Some(b'b') => p[2],
        Some(b'a') => p[3],
        Some(b'0') => 0,
        Some(b'1') => 255,
        _ => p[i],
    })
}

/// Port of `MetricsCalculator`.
#[derive(Debug, Default)]
pub struct MetricsCalculator {
    reference_num_channels: u32,
    reference_images: Vec<Image>,
}

impl MetricsCalculator {
    /// `saveReferenceImages`: the input is R/RG/RGB/RGBA 8-bit UNORM / SRGB.
    pub fn save_reference_images(&mut self, texture: &Texture2, opts: &OptionsMetrics) {
        if !opts.compare_ssim && !opts.compare_psnr {
            return;
        }

        let num_channels = texture.get_num_components();
        self.reference_num_channels = num_channels;
        let data = texture.p_data.as_deref().unwrap_or_default();

        for level_index in 0..texture.num_levels {
            let image_width = (texture.base_width >> level_index).max(1);
            let image_height = (texture.base_height >> level_index).max(1);
            let image_depths = (texture.base_depth >> level_index).max(1);

            for layer_index in 0..texture.num_layers {
                for face_index in 0..texture.num_faces {
                    for depth_slice_index in 0..image_depths {
                        let image_offset = texture
                            .get_image_offset(
                                level_index,
                                layer_index,
                                face_index + depth_slice_index,
                            )
                            .unwrap_or(0);
                        let image_data = data.get(image_offset..).unwrap_or_default();

                        let mut reference = Image::new(image_width, image_height);
                        for y in 0..image_height {
                            for x in 0..image_width {
                                if let Some(p) = reference.get_mut(x, y) {
                                    *p = [0, 0, 0, 255];
                                    for c in 0..num_channels.min(4) {
                                        let i =
                                            (y * image_width * num_channels + x * num_channels + c)
                                                as usize;
                                        p[c as usize] = image_data.get(i).copied().unwrap_or(0);
                                    }
                                }
                            }
                        }
                        self.reference_images.push(reference);
                    }
                }
            }
        }
    }

    /// `decodeAndCalculateMetrics`: prints to stderr when `to_stderr` (stdout is the KTX file).
    pub fn decode_and_calculate_metrics(
        &self,
        encoded_texture: &mut Texture2,
        opts: &OptionsMetrics,
        report: &Reporter,
        to_stderr: bool,
    ) -> CmdResult {
        if !opts.compare_ssim && !opts.compare_psnr {
            return Ok(());
        }

        let mut texture = match Texture2::create_copy(encoded_texture) {
            Ok(t) => t,
            Err(e) => {
                return Err(report.fatal(
                    ReturnCode::RuntimeError,
                    &format!(
                        "Failed to create KTX2 texture copy: {}",
                        ktx_error_string(e)
                    ),
                ));
            }
        };

        // Start with a default swizzle
        let mut t_swizzle_info = TranscodeSwizzleInfo {
            default_num_components: 4,
            swizzle: "rgba".to_string(),
        };

        // Decode the encoded texture to observe the compression losses
        let result = if dfdval(bdb(&texture.p_dfd), MODEL) == KHR_DF_MODEL_ASTC {
            texture.decode_astc()
        } else {
            t_swizzle_info = determine_transcode_swizzle(&texture, report)?;
            texture.transcode_basis(TranscodeFmt::RGBA32, 0)
        };
        if let Err(e) = result {
            return Err(report.fatal(
                ReturnCode::RuntimeError,
                &format!(
                    "Failed to transcode KTX2 texture to calculate error metrics: {}",
                    ktx_error_string(e)
                ),
            ));
        }

        let mut out = String::new();
        let mut overall_ssim = [0.0f32; 4];
        let mut overall_psnr = 0.0f64;
        let multi = self.reference_images.len() != 1;
        let data = texture.p_data.as_deref().unwrap_or_default();

        let mut ref_it = self.reference_images.iter();
        for level_index in 0..texture.num_levels {
            let image_width = (texture.base_width >> level_index).max(1);
            let image_height = (texture.base_height >> level_index).max(1);
            let image_depth = (texture.base_depth >> level_index).max(1);

            for layer_index in 0..texture.num_layers {
                for face_index in 0..texture.num_faces {
                    for depth_slice_index in 0..image_depth {
                        let Some(reference) = ref_it.next() else {
                            return Err(report.fatal(ReturnCode::RuntimeError, "Internal error"));
                        };

                        let image_offset = texture
                            .get_image_offset(
                                level_index,
                                layer_index,
                                face_index + depth_slice_index,
                            )
                            .unwrap_or(0);
                        let image_data = data.get(image_offset..).unwrap_or_default();

                        let mut basisu_image = Image::new(image_width, image_height);
                        for y in 0..image_height {
                            for x in 0..image_width {
                                let i = ((y * image_width + x) * 4) as usize;
                                let px = std::array::from_fn(|c| {
                                    image_data.get(i + c).copied().unwrap_or(0)
                                });
                                let px = swizzle_pixel(px, &t_swizzle_info.swizzle);
                                if let Some(p) = basisu_image.get_mut(x, y) {
                                    *p = [0, 0, 0, 255];
                                    let n = self.reference_num_channels.min(4) as usize;
                                    p[..n].copy_from_slice(&px[..n]);
                                }
                            }
                        }

                        if multi {
                            out += &format!(
                                "Level {}{}{}{}:\n",
                                level_index,
                                if texture.is_array {
                                    format!(" Layer {layer_index}")
                                } else {
                                    String::new()
                                },
                                if texture.is_cubemap {
                                    format!(" Face {face_index}")
                                } else {
                                    String::new()
                                },
                                if texture.num_dimensions == 3 {
                                    format!(" Depth {depth_slice_index}")
                                } else {
                                    String::new()
                                }
                            );
                        }

                        if opts.compare_ssim {
                            let ssim = compute_ssim(reference, &basisu_image, false, false);
                            if multi {
                                let n = self.reference_num_channels;
                                let names = ["R", "G", "B", "A"];
                                if n > 0 {
                                    let parts: Vec<String> = (0..n.min(4) as usize)
                                        .map(|c| format!("{}: {}", names[c], signed6(ssim[c])))
                                        .collect();
                                    out += &format!("    SSIM {}\n", parts.join(", "));
                                }
                            }
                            for i in 0..4 {
                                overall_ssim[i] += ssim[i];
                            }
                        }

                        if opts.compare_psnr {
                            let mut im = ImageMetrics::default();
                            im.calc(reference, &basisu_image, 0, 0, true, false);
                            if multi {
                                out += &format!("    PSNR: {:9.6}\n", im.psnr);
                            }
                            overall_psnr = overall_psnr.max(im.psnr);
                        }
                    }
                }
            }
        }

        out += &format!("{}Overall:\n", if multi { "\n" } else { "" });

        if opts.compare_ssim {
            let num_if = self.reference_images.len() as f32;
            let n = self.reference_num_channels.clamp(1, 4) as usize;
            let names = ["R", "G", "B", "A"];
            let parts: Vec<String> = (0..n)
                .map(|c| format!("{}: {}", names[c], signed6(overall_ssim[c] / num_if)))
                .collect();
            out += &format!("    SSIM Avg {}\n", parts.join(", "));
        }

        if opts.compare_psnr {
            out += &format!("    PSNR Max: {overall_psnr:9.6}\n");
        }

        if to_stderr {
            print_err(out.as_bytes());
        } else {
            print_out(out.as_bytes());
        }
        Ok(())
    }
}
