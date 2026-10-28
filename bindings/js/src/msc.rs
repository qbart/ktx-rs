// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of `transcoder_wrapper.cpp` (`msc_basis_transcoder.js`): the container independent
//! BasisLZ/ETC1S and UASTC image transcoders. Deprecated upstream in favour of Binomial's own
//! JS transcoder, kept for existing users.

use js_sys::{Object, Reflect, Uint8Array};
use ktx::basisu::etc1s::{LowlevelEtc1sTranscoder, TranscodeStatus};
use ktx::basisu::uastc_ldr::LowlevelUastcLdr4x4Transcoder;
use ktx::basisu::{
    BasisTexFormat, TranscoderState, TranscoderTextureFormat as F, basis_get_block_height,
    basis_get_block_width, basis_get_bytes_per_block_or_pixel, basis_is_format_supported,
    basis_transcoder_format_is_uncompressed, decode_flags,
};
use wasm_bindgen::prelude::*;

/// `basis_tex_format` from its numeric value.
fn tex_format(v: u32) -> Option<BasisTexFormat> {
    Some(match v {
        0 => BasisTexFormat::Etc1s,
        1 => BasisTexFormat::UastcLdr4x4,
        2 => BasisTexFormat::UastcHdr4x4,
        3 => BasisTexFormat::AstcHdr6x6,
        4 => BasisTexFormat::UastcHdr6x6Intermediate,
        _ => return None,
    })
}

/// The `TranscodeTarget`, `TextureFormat` and `TranscodeFlagBits` enums (`enum_<...>` in
/// `EMSCRIPTEN_BINDINGS`).
#[wasm_bindgen(js_name = __mscEnums)]
pub fn enums() -> JsValue {
    let obj = Object::new();
    let targets = super::enum_object(&[
        ("ETC1_RGB", F::ETC1_RGB.0),
        ("BC1_RGB", F::BC1_RGB.0),
        ("BC4_R", F::BC4_R.0),
        ("BC5_RG", F::BC5_RG.0),
        ("BC3_RGBA", F::BC3_RGBA.0),
        ("PVRTC1_4_RGB", F::PVRTC1_4_RGB.0),
        ("PVRTC1_4_RGBA", F::PVRTC1_4_RGBA.0),
        ("BC6HU_RGB", F::BC6H.0),
        ("BC7_RGBA", F::BC7_RGBA.0),
        // Deprecated. Use BC7_RGBA (cTFBC7_M6_RGB / cTFBC7_M5_RGBA alias cTFBC7_RGBA).
        ("BC7_M6_RGB", F::BC7_RGBA.0),
        ("BC7_M5_RGBA", F::BC7_RGBA.0),
        ("ETC2_RGBA", F::ETC2_RGBA.0),
        ("ASTC_4x4_RGBA", F::ASTC_LDR_4x4_RGBA.0),
        ("ASTC_LDR_4x4_RGBA", F::ASTC_LDR_4x4_RGBA.0),
        ("ASTC_HDR_4x4_RGBA", F::ASTC_HDR_4x4_RGBA.0),
        ("ASTC_HDR_6x6_RGBA", F::ASTC_HDR_6x6_RGBA.0),
        ("RGBA32", F::RGBA32.0),
        ("RGB565", F::RGB565.0),
        ("BGR565", F::BGR565.0),
        ("RGBA4444", F::RGBA4444.0),
        ("RGB16F", F::RGB_HALF.0),
        ("RGBA16F", F::RGBA_HALF.0),
        ("RGB9E5", F::RGB_9E5.0),
        ("PVRTC2_4_RGB", F::PVRTC2_4_RGB.0),
        ("PVRTC2_4_RGBA", F::PVRTC2_4_RGBA.0),
        ("EAC_R11", F::ETC2_EAC_R11.0),
        ("EAC_RG11", F::ETC2_EAC_RG11.0),
    ]);
    let tex_formats = super::enum_object(&[
        ("ETC1S", 0),
        ("UASTC_LDR_4x4", 1),
        ("UASTC_HDR_4x4", 2),
        ("UASTC_HDR_6x6_INTERMEDIATE", 4),
    ]);
    let flags = super::enum_object(&[
        (
            "TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS",
            decode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS,
        ),
        ("HIGH_QUALITY", decode_flags::HIGH_QUALITY),
    ]);
    for (name, value) in [
        ("TranscodeTarget", targets),
        ("TextureFormat", tex_formats),
        ("TranscodeFlagBits", flags),
    ] {
        let _ = Reflect::set(&obj, &JsValue::from_str(name), &value);
    }
    obj.into()
}

/// `initTranscoders` (`basisu_transcoder_init`): the transcoder tables are built on first use.
#[wasm_bindgen(js_name = initTranscoders)]
pub fn init_transcoders() {}

/// `isFormatSupported` (`basis_is_format_supported`).
#[wasm_bindgen(js_name = isFormatSupported)]
pub fn is_format_supported(target_format: u32, tex_format_value: u32) -> bool {
    tex_format(tex_format_value).is_some_and(|f| basis_is_format_supported(F(target_format), f))
}

/// `msc::basisu_image_desc` (`ImageInfo`).
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageInfo {
    pub flags: u32,
    #[wasm_bindgen(js_name = rgbByteOffset)]
    pub rgb_byte_offset: u32,
    #[wasm_bindgen(js_name = rgbByteLength)]
    pub rgb_byte_length: u32,
    #[wasm_bindgen(js_name = alphaByteOffset)]
    pub alpha_byte_offset: u32,
    #[wasm_bindgen(js_name = alphaByteLength)]
    pub alpha_byte_length: u32,
    pub width: u32,
    pub height: u32,
    #[wasm_bindgen(js_name = numBlocksX)]
    pub num_blocks_x: u32,
    #[wasm_bindgen(js_name = numBlocksY)]
    pub num_blocks_y: u32,
    pub level: u32,
}

#[wasm_bindgen]
impl ImageInfo {
    /// `basisu_image_desc(basis_tex_format, width, height, level)`: all formats are 4x4 blocks,
    /// so the format is ignored.
    #[wasm_bindgen(constructor)]
    pub fn new(_tex_format: u32, width: u32, height: u32, level: u32) -> ImageInfo {
        let (bw, bh) = (4u32, 4u32);
        ImageInfo {
            width,
            height,
            num_blocks_x: width.wrapping_add(bw - 1) / bw,
            num_blocks_y: height.wrapping_add(bh - 1) / bh,
            level,
            ..Default::default()
        }
    }
}

/// `BasisTranscoderState` (`basisu_transcoder_state`): constructible, unused by the API.
#[wasm_bindgen]
#[derive(Debug, Default)]
pub struct BasisTranscoderState {
    _state: TranscoderState,
}

#[wasm_bindgen]
impl BasisTranscoderState {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BasisTranscoderState {
        BasisTranscoderState::default()
    }
}

/// `msc::TranscodedImage`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct TranscodedImage {
    image: Vec<u8>,
}

#[wasm_bindgen]
impl TranscodedImage {
    /// The image data (a copy; Emscripten returns a view into its heap).
    pub fn get_typed_memory_view(&self) -> Uint8Array {
        Uint8Array::from(self.image.as_slice())
    }
}

/// `ImageTranscoderHelper::getTranscodedImageByteLength` (`uint32_t` arithmetic like C++).
fn transcoded_image_byte_length(format: F, width: u32, height: u32) -> u32 {
    let block_byte_length = basis_get_bytes_per_block_or_pixel(format);
    if basis_transcoder_format_is_uncompressed(format) {
        width.wrapping_mul(height).wrapping_mul(block_byte_length)
    } else if format == F::PVRTC1_4_RGB || format == F::PVRTC1_4_RGBA {
        // GL requires padding for very small PVRTC1 textures (IMG_texture_compression_pvrtc).
        let padded_width = width.wrapping_add(3) & !3;
        let padded_height = height.wrapping_add(3) & !3;
        8u32.max(padded_width)
            .wrapping_mul(8u32.max(padded_height))
            .wrapping_mul(4)
            .wrapping_add(7)
            / 8
    } else {
        let bw = basis_get_block_width(format);
        let bh = basis_get_block_height(format);
        let blocks_x = width.wrapping_add(bw - 1) / bw;
        let blocks_y = height.wrapping_add(bh - 1) / bh;
        blocks_x
            .wrapping_mul(blocks_y)
            .wrapping_mul(block_byte_length)
    }
}

/// `msc::BasisLzEtc1sImageTranscoder`.
#[wasm_bindgen]
#[derive(Debug, Default)]
pub struct BasisLzEtc1sImageTranscoder {
    transcoder: LowlevelEtc1sTranscoder,
    /// The C++ passes no state, so the transcoder's `m_def_state` is used.
    state: TranscoderState,
}

#[wasm_bindgen]
impl BasisLzEtc1sImageTranscoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BasisLzEtc1sImageTranscoder {
        BasisLzEtc1sImageTranscoder::default()
    }

    /// `getBytesPerBlock` (`basis_get_bytes_per_block_or_pixel`).
    #[wasm_bindgen(js_name = getBytesPerBlock)]
    pub fn get_bytes_per_block(format: u32) -> u32 {
        basis_get_bytes_per_block_or_pixel(F(format))
    }

    #[wasm_bindgen(js_name = decodePalettes)]
    pub fn decode_palettes(
        &mut self,
        num_endpoints: u32,
        endpoints: &[u8],
        num_selectors: u32,
        selectors: &[u8],
    ) -> bool {
        self.transcoder
            .decode_palettes(num_endpoints, endpoints, num_selectors, selectors)
    }

    #[wasm_bindgen(js_name = decodeTables)]
    pub fn decode_tables(&mut self, table_data: &[u8]) -> bool {
        self.transcoder.decode_tables(table_data)
    }

    /// Transcodes one BasisLZ/ETC1S image; `slices` starts at the RGB slice, an alpha slice
    /// follows it. `None` (no `transcodedImage`) on failure.
    #[wasm_bindgen(js_name = transcodeImage)]
    pub fn transcode_image(
        &mut self,
        target_format: u32,
        slices: &[u8],
        image_desc: &ImageInfo,
        decode_flags: u32,
        is_video: bool,
    ) -> Option<TranscodedImage> {
        let format = F(target_format);
        let size = transcoded_image_byte_length(format, image_desc.width, image_desc.height);
        let mut image = vec![0u8; size as usize];
        let (bw, bh) = (4u32, 4u32);
        let num_blocks_x = image_desc.width.wrapping_add(bw - 1) / bw;
        let num_blocks_y = image_desc.height.wrapping_add(bh - 1) / bh;
        let status = self.transcoder.transcode_image(
            format,
            &mut image,
            // C++ passes the byte size as the size in blocks or pixels.
            size,
            slices,
            num_blocks_x,
            num_blocks_y,
            image_desc.width,
            image_desc.height,
            image_desc.level,
            u64::from(image_desc.rgb_byte_offset),
            image_desc.rgb_byte_length,
            u64::from(image_desc.alpha_byte_offset),
            image_desc.alpha_byte_length,
            decode_flags,
            image_desc.alpha_byte_length != 0,
            is_video,
            0,
            &mut self.state,
            0,
        );
        (status == TranscodeStatus::Ok).then_some(TranscodedImage { image })
    }
}

/// `msc::UastcImageTranscoder`.
#[wasm_bindgen]
#[derive(Debug, Default)]
pub struct UastcImageTranscoder {
    transcoder: LowlevelUastcLdr4x4Transcoder,
    state: TranscoderState,
}

#[wasm_bindgen]
impl UastcImageTranscoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> UastcImageTranscoder {
        UastcImageTranscoder::default()
    }

    /// `getBytesPerBlock` (`basis_get_bytes_per_block_or_pixel`).
    #[wasm_bindgen(js_name = getBytesPerBlock)]
    pub fn get_bytes_per_block(format: u32) -> u32 {
        basis_get_bytes_per_block_or_pixel(F(format))
    }

    /// Transcodes one UASTC LDR 4x4 image. `None` (no `transcodedImage`) on failure.
    #[wasm_bindgen(js_name = transcodeImage)]
    pub fn transcode_image(
        &mut self,
        target_format: u32,
        image: &[u8],
        image_desc: &ImageInfo,
        decode_flags: u32,
        has_alpha: bool,
        is_video: bool,
    ) -> Option<TranscodedImage> {
        let format = F(target_format);
        let size = transcoded_image_byte_length(format, image_desc.width, image_desc.height);
        let mut out = vec![0u8; size as usize];
        let (bw, bh) = (4u32, 4u32);
        let num_blocks_x = image_desc.width.wrapping_add(bw - 1) / bw;
        let num_blocks_y = image_desc.height.wrapping_add(bh - 1) / bh;
        let status = self.transcoder.transcode_image(
            format,
            &mut out,
            size,
            image,
            num_blocks_x,
            num_blocks_y,
            image_desc.width,
            image_desc.height,
            image_desc.level,
            u64::from(image_desc.rgb_byte_offset),
            image_desc.rgb_byte_length,
            decode_flags,
            has_alpha,
            is_video,
            0,
            &mut self.state,
            0,
            -1,
            -1,
        );
        (status == TranscodeStatus::Ok).then_some(TranscodedImage { image: out })
    }
}
