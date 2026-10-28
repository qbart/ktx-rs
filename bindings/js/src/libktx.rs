// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of `ktx_wrapper.cpp` (`libktx_read.js`, and with the `write` feature `libktx.js`):
//! `ktx::texture` and the `EMSCRIPTEN_BINDINGS(ktx)` enums. Enum typed properties and results
//! are exported as numbers (`_name`) and turned into embind enum entries by `js/libktx.js`.
//!
//! A texture whose creation failed holds no `ktxTexture` (C++ keeps a null pointer and would
//! dereference it): getters return 0 / false, methods `KTX_INVALID_OPERATION` or `null`.

use js_sys::{Array, Object, Reflect, Uint8Array};
use ktx::texture::KtxTexture;
use ktx::texture_any::AnyTexture;
use ktx::{
    ClassId, ErrorCode, KTX_FACESLICE_WHOLE_LEVEL, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
    SupercmpScheme, Texture2, TranscodeFmt, ktx_error_string,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// `std::cout` (Emscripten prints it with `console.log`).
    #[wasm_bindgen(js_namespace = console)]
    pub(crate) fn log(s: &str);
}

/// `ktx_error_code_e` value of a result.
pub(crate) fn code(r: Result<(), ErrorCode>) -> u32 {
    match r {
        Ok(()) => 0,
        Err(e) => e as u32,
    }
}

const KTX_INVALID_OPERATION: u32 = ErrorCode::InvalidOperation as u32;

// khr_df_model_e / khr_df_transfer_e / khr_df_primaries_e values used by the wrapper.
const KHR_DF_MODEL_UNSPECIFIED: u32 = 0;
const KHR_DF_MODEL_UASTC_LDR_4X4: u32 = 166;
const KHR_DF_MODEL_UASTC_HDR_4X4: u32 = 167;
const KHR_DF_MODEL_UASTC_HDR_6X6: u32 = 168;
const KHR_DF_TRANSFER_UNSPECIFIED: u32 = 0;
const KHR_DF_TRANSFER_LINEAR: u32 = 1;
const KHR_DF_TRANSFER_SRGB: u32 = 2;
const KHR_DF_PRIMARIES_UNSPECIFIED: u32 = 0;
const KHR_DF_PRIMARIES_BT709: u32 = 1;
const KHR_DF_PRIMARIES_DISPLAYP3: u32 = 10;
const VK_FORMAT_UNDEFINED: u32 = 0;

/// The `EMSCRIPTEN_BINDINGS(ktx)` enums shared by both builds.
#[wasm_bindgen(js_name = __ktxEnums)]
pub fn enums() -> JsValue {
    use super::enum_object as e;
    let obj = Object::new();
    let error_code = e(&[
        ("SUCCESS", 0),
        ("FILE_DATA_ERROR", ErrorCode::FileDataError as u32),
        ("FILE_ISPIPE", ErrorCode::FileIsPipe as u32),
        ("FILE_OPEN_FAILED", ErrorCode::FileOpenFailed as u32),
        ("FILE_OVERFLOW", ErrorCode::FileOverflow as u32),
        ("FILE_READ_ERROR", ErrorCode::FileReadError as u32),
        ("FILE_SEEK_ERROR", ErrorCode::FileSeekError as u32),
        // Upstream names KTX_FILE_UNEXPECTED_EOF this way.
        ("FILE_UNEXPECTED_ERROR", ErrorCode::FileUnexpectedEof as u32),
        ("FILE_WRITE_ERROR", ErrorCode::FileWriteError as u32),
        ("GL_ERROR", ErrorCode::GlError as u32),
        ("INVALID_OPERATION", ErrorCode::InvalidOperation as u32),
        ("INVALID_VALUE", ErrorCode::InvalidValue as u32),
        ("NOT_FOUND", ErrorCode::NotFound as u32),
        ("OUT_OF_MEMORY", ErrorCode::OutOfMemory as u32),
        ("TRANSCODE_FAILED", ErrorCode::TranscodeFailed as u32),
        ("UNKNOWN_FILE_FORMAT", ErrorCode::UnknownFileFormat as u32),
        (
            "UNSUPPORTED_TEXTURE_TYPE",
            ErrorCode::UnsupportedTextureType as u32,
        ),
        ("UNSUPPORTED_FEATURE", ErrorCode::UnsupportedFeature as u32),
        ("LIBRARY_NOT_LINKED", ErrorCode::LibraryNotLinked as u32),
        (
            "DECOMPRESS_LENGTH_ERROR",
            ErrorCode::DecompressLengthError as u32,
        ),
        (
            "DECOMPRESS_CHECKSUM_ERROR",
            ErrorCode::DecompressChecksumError as u32,
        ),
    ]);
    let transcode_fmt = e(&[
        ("ETC1_RGB", TranscodeFmt::ETC1_RGB.0),
        ("BC1_RGB", TranscodeFmt::BC1_RGB.0),
        ("BC4_R", TranscodeFmt::BC4_R.0),
        ("BC5_RG", TranscodeFmt::BC5_RG.0),
        ("BC3_RGBA", TranscodeFmt::BC3_RGBA.0),
        ("BC1_OR_3", TranscodeFmt::BC1_OR_3.0),
        ("PVRTC1_4_RGB", TranscodeFmt::PVRTC1_4_RGB.0),
        ("PVRTC1_4_RGBA", TranscodeFmt::PVRTC1_4_RGBA.0),
        ("BC6HU_RGB", TranscodeFmt::BC6HU_RGB.0),
        ("BC7_RGBA", TranscodeFmt::BC7_RGBA.0),
        ("ETC2_RGBA", TranscodeFmt::ETC2_RGBA.0),
        ("ASTC_4x4_RGBA", TranscodeFmt::ASTC_LDR_4x4_RGBA.0),
        ("ASTC_LDR_4x4_RGBA", TranscodeFmt::ASTC_LDR_4x4_RGBA.0),
        ("ASTC_HDR_4x4_RGBA", TranscodeFmt::ASTC_HDR_4x4_RGBA.0),
        ("ASTC_HDR_6x6_RGBA", TranscodeFmt::ASTC_HDR_6x6_RGBA.0),
        ("RGBA32", TranscodeFmt::RGBA32.0),
        ("RGB565", TranscodeFmt::RGB565.0),
        ("BGR565", TranscodeFmt::BGR565.0),
        ("RGBA4444", TranscodeFmt::RGBA4444.0),
        ("RGBA8888", TranscodeFmt::RGBA32.0),
        ("RGB16F", TranscodeFmt::RGB_HALF.0),
        ("RGB9E5", TranscodeFmt::RGB_9E5.0),
        ("RGBA16F", TranscodeFmt::RGBA_HALF.0),
        ("PVRTC2_4_RGB", TranscodeFmt::PVRTC2_4_RGB.0),
        ("PVRTC2_4_RGBA", TranscodeFmt::PVRTC2_4_RGBA.0),
        ("ETC", TranscodeFmt::ETC.0),
        ("EAC_R11", TranscodeFmt::ETC2_EAC_R11.0),
        ("EAC_RG11", TranscodeFmt::ETC2_EAC_RG11.0),
    ]);
    let transcode_flag_bits = e(&[(
        "TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS",
        ktx::transcode_flags::TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS,
    )]);
    let basis_codec = e(&[
        ("NONE", 0),
        ("ETC1S", 1),
        ("UASTC_LDR_4x4", 2),
        ("UASTC_HDR_4x4", 3),
        ("UASTC_HDR_6x6_INTERMEDIATE", 4),
    ]);
    let supercmp = e(&[
        ("NONE", SupercmpScheme::NONE.0),
        ("BASIS_LZ", SupercmpScheme::BASIS_LZ.0),
        ("ZSTD", SupercmpScheme::ZSTD.0),
        ("ZLIB", SupercmpScheme::ZLIB.0),
    ]);
    let orient_x = e(&[("LEFT", u32::from(b'l')), ("RIGHT", u32::from(b'r'))]);
    let orient_y = e(&[("UP", u32::from(b'u')), ("DOWN", u32::from(b'd'))]);
    let orient_z = e(&[("IN", u32::from(b'i')), ("OUT", u32::from(b'o'))]);
    let model = e(&[
        ("KHR_DF_MODEL_UASTC_LDR_4x4", KHR_DF_MODEL_UASTC_LDR_4X4),
        ("KHR_DF_MODEL_UASTC_HDR_4x4", KHR_DF_MODEL_UASTC_HDR_4X4),
        ("KHR_DF_MODEL_UASTC_HDR_6x6", KHR_DF_MODEL_UASTC_HDR_6X6),
    ]);
    let primaries = e(&[
        ("UNSPECIFIED", KHR_DF_PRIMARIES_UNSPECIFIED),
        ("BT709", KHR_DF_PRIMARIES_BT709),
        // KHR_DF_PRIMARIES_SRGB is KHR_DF_PRIMARIES_BT709.
        ("SRGB", KHR_DF_PRIMARIES_BT709),
        ("DISPLAYP3", KHR_DF_PRIMARIES_DISPLAYP3),
    ]);
    let transfer = e(&[
        ("UNSPECIFIED", KHR_DF_TRANSFER_UNSPECIFIED),
        ("LINEAR", KHR_DF_TRANSFER_LINEAR),
        ("SRGB", KHR_DF_TRANSFER_SRGB),
    ]);
    for (name, value) in [
        ("error_code", error_code),
        ("transcode_fmt", transcode_fmt),
        ("transcode_flag_bits", transcode_flag_bits),
        ("basis_codec", basis_codec),
        ("SupercmpScheme", supercmp),
        ("OrientationX", orient_x),
        ("OrientationY", orient_y),
        ("OrientationZ", orient_z),
        ("khr_df_model", model),
        ("khr_df_primaries", primaries),
        ("khr_df_transfer", transfer),
    ] {
        let _ = Reflect::set(&obj, &JsValue::from_str(name), &value);
    }
    #[cfg(feature = "write")]
    super::libktx_write::write_enums(&obj);
    obj.into()
}

/// `ktx::texture`.
#[wasm_bindgen(js_name = texture)]
#[derive(Debug, Default)]
pub struct Texture {
    pub(crate) ptr: Option<AnyTexture>,
}

impl Texture {
    pub(crate) fn texture2(&self) -> Option<&Texture2> {
        match &self.ptr {
            Some(AnyTexture::Ktx2(t)) => Some(t),
            _ => None,
        }
    }

    pub(crate) fn texture2_mut(&mut self) -> Option<&mut Texture2> {
        match &mut self.ptr {
            Some(AnyTexture::Ktx2(t)) => Some(t),
            _ => None,
        }
    }

    fn is_texture2(&self) -> bool {
        self.ptr
            .as_ref()
            .is_some_and(|t| t.class_id() == ClassId::Texture2)
    }
}

#[wasm_bindgen(js_class = texture)]
impl Texture {
    /// Empty texture; `js/libktx.js` calls `_load` or `_create` (embind picks the overload
    /// by argument count).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Texture {
        Texture::default()
    }

    /// `texture(const val& data)`: `ktxTexture_CreateFromMemory` with the image data loaded.
    pub fn _load(&mut self, data: &[u8]) {
        match AnyTexture::create_from_memory(data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => self.ptr = Some(t),
            Err(e) => {
                log(&format!(
                    "ERROR: Failed to create from memory: {}",
                    ktx_error_string(e)
                ));
                self.ptr = None;
            }
        }
    }

    #[wasm_bindgen(js_name = findKeyValue)]
    pub fn find_key_value(&self, key: &str) -> Option<Uint8Array> {
        let t = self.ptr.as_ref()?;
        match t.kv_data_head.find_value(key) {
            Ok(v) => Some(Uint8Array::from(v)),
            Err(e) => {
                log(&format!(
                    "ERROR: failed to findKeyValue: {}",
                    ktx_error_string(e)
                ));
                None
            }
        }
    }

    #[wasm_bindgen(getter = dataSize)]
    pub fn data_size(&self) -> u32 {
        self.ptr.as_ref().map_or(0, |t| t.get_data_size() as u32)
    }

    #[wasm_bindgen(getter = baseWidth)]
    pub fn base_width(&self) -> u32 {
        self.ptr.as_ref().map_or(0, |t| t.base_width)
    }

    #[wasm_bindgen(getter = baseHeight)]
    pub fn base_height(&self) -> u32 {
        self.ptr.as_ref().map_or(0, |t| t.base_height)
    }

    #[wasm_bindgen(getter = needsTranscoding)]
    pub fn needs_transcoding(&self) -> bool {
        self.ptr.as_ref().is_some_and(|t| t.needs_transcoding())
    }

    #[wasm_bindgen(getter = isTranscodable)]
    pub fn is_transcodable(&self) -> bool {
        self.ptr.as_ref().is_some_and(|t| t.is_transcodable())
    }

    #[wasm_bindgen(getter = isHDR)]
    pub fn is_hdr(&self) -> bool {
        self.ptr.as_ref().is_some_and(|t| t.is_hdr())
    }

    /// `khr_df_model_e` (enum entry in JS).
    #[wasm_bindgen(getter = _colorModel)]
    pub fn color_model(&self) -> u32 {
        self.texture2()
            .map_or(KHR_DF_MODEL_UNSPECIFIED, |t| t.get_color_model_e())
    }

    /// `khr_df_transfer_e` (enum entry in JS).
    #[wasm_bindgen(getter = _transferFunction)]
    pub fn transfer_function(&self) -> u32 {
        self.texture2()
            .map_or(KHR_DF_TRANSFER_UNSPECIFIED, |t| t.get_transfer_function_e())
    }

    /// `khr_df_transfer_e` (enum entry in JS).
    #[wasm_bindgen(getter = _oetf)]
    pub fn oetf(&self) -> u32 {
        self.texture2()
            .map_or(KHR_DF_TRANSFER_UNSPECIFIED, |t| t.get_oetf_e())
    }

    /// `khr_df_primaries_e` (enum entry in JS).
    #[wasm_bindgen(getter = _primaries)]
    pub fn primaries(&self) -> u32 {
        self.texture2()
            .map_or(KHR_DF_PRIMARIES_UNSPECIFIED, |t| t.get_primaries_e())
    }

    #[wasm_bindgen(getter = isSrgb)]
    pub fn is_srgb(&self) -> bool {
        self.oetf() == KHR_DF_TRANSFER_SRGB
    }

    #[wasm_bindgen(getter = isPremultiplied)]
    pub fn is_premultiplied(&self) -> bool {
        self.texture2().is_some_and(|t| t.get_premultiplied_alpha())
    }

    #[wasm_bindgen(getter = numComponents)]
    pub fn num_components(&self) -> u32 {
        match self.texture2() {
            Some(t) => t.get_num_components(),
            None => {
                log("ERROR: numComponents is only supported for KTX2");
                0
            }
        }
    }

    /// `ktxSupercmpScheme` (enum entry in JS).
    #[wasm_bindgen(getter = _supercompressScheme)]
    pub fn supercompression_scheme(&self) -> u32 {
        self.texture2()
            .map_or(SupercmpScheme::NONE.0, |t| t.supercompression_scheme.0)
    }

    #[wasm_bindgen(getter = vkFormat)]
    pub fn vk_format(&self) -> u32 {
        match self.texture2() {
            Some(t) => t.vk_format.0,
            None => {
                log("ERROR: vkFormat is only supported for KTX2");
                VK_FORMAT_UNDEFINED
            }
        }
    }

    /// `ktxOrientation` as `[x, y, z]` (a value_object of enum entries in JS).
    #[wasm_bindgen(getter = _orientation)]
    pub fn orientation(&self) -> Array {
        let o = self.ptr.as_ref().map(|t| t.orientation).unwrap_or_default();
        [o.x.0, o.y.0, o.z.0]
            .iter()
            .map(|&c| JsValue::from(u32::from(c)))
            .collect()
    }

    /// The image (or with `KTX_FACESLICE_WHOLE_LEVEL` the whole level), a copy of the data.
    #[wasm_bindgen(js_name = getImage)]
    pub fn get_image(&self, level: u32, layer: u32, face_slice: u32) -> Option<Uint8Array> {
        let t = self.ptr.as_ref()?;
        let whole = face_slice == KTX_FACESLICE_WHOLE_LEVEL;
        let offset = match t.get_image_offset(level, layer, if whole { 0 } else { face_slice }) {
            Ok(o) => o,
            Err(e) => {
                log(&format!(
                    "ERROR: getImage.ktxTexture_GetImageOffset: {}",
                    ktx_error_string(e)
                ));
                return None;
            }
        };
        let length = if whole {
            t.get_level_size(level)
        } else {
            t.get_image_size(level)
        };
        if length > t.data_size {
            log("ERROR: getImage: not enough data in texture.");
            return None;
        }
        let data = t.p_data.as_deref().unwrap_or_default();
        data.get(offset..offset.checked_add(length)?)
            .map(Uint8Array::from)
    }

    /// `ktxTexture2_TranscodeBasis` (`ktx_error_code_e`, enum entry in JS).
    pub fn _transcode_basis(&mut self, target_format: u32, decode_flags: u32) -> u32 {
        let Some(t) = self.texture2_mut() else {
            log("ERROR: transcodeBasis is only supported for KTX2");
            return KTX_INVALID_OPERATION;
        };
        let result = code(t.transcode_basis(TranscodeFmt(target_format), decode_flags));
        if result != 0 {
            log(&format!(
                "ERROR: Failed to transcode: {}",
                ktx::error::ktx_error_string_code(result)
            ));
        }
        result
    }

    /// `ktxTexture2_DecodeAstc` (`ktx_error_code_e`, enum entry in JS).
    pub fn _decode_astc(&mut self) -> u32 {
        let result = match self.texture2_mut() {
            Some(t) => code(t.decode_astc()),
            None => KTX_INVALID_OPERATION,
        };
        if result != 0 {
            log(&format!(
                "ERROR: Failed to decodeAstc: {}",
                ktx::error::ktx_error_string_code(result)
            ));
        }
        result
    }

    /// `createCopy` (`ktxTexture2_CreateCopy`; KTX1 textures give an empty texture).
    #[wasm_bindgen(js_name = createCopy)]
    pub fn create_copy(&mut self) -> Texture {
        if !self.is_texture2() {
            return Texture::default();
        }
        let Some(t) = self.texture2_mut() else {
            return Texture::default();
        };
        match Texture2::create_copy(t) {
            Ok(copy) => Texture {
                ptr: Some(AnyTexture::Ktx2(copy)),
            },
            Err(e) => {
                log(&format!(
                    "ERROR: Failed to copy construct: {}",
                    ktx_error_string(e)
                ));
                Texture::default()
            }
        }
    }
}
