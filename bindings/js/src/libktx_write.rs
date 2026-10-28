// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! The `KTX_FEATURE_WRITE` part of `ktx_wrapper.cpp` (`libktx.js`): texture creation,
//! compression, key/value data and writing, `textureCreateInfo`, `astcParams`, `basisParams`
//! and their enums and constants. Structs start zeroed like embind's `constructor<>()`.

use js_sys::{Object, Reflect, Uint8Array};
use ktx::encode_params::{AstcParams, BasisParams, KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL};
use ktx::error::ktx_error_string_code;
use ktx::texture::KtxTexture;
use ktx::texture_any::AnyTexture;
use ktx::vkformat::VkFormat;
use ktx::{
    KTX_ANIMDATA_KEY, KTX_FACESLICE_WHOLE_LEVEL, KTX_ORIENTATION_KEY, KTX_SWIZZLE_KEY,
    KTX_WRITER_KEY, KTX_WRITER_SCPARAMS_KEY, Texture2, TextureCreateInfo as CreateInfo,
    TextureCreateStorage, ktx_error_string,
};
use wasm_bindgen::prelude::*;

use super::libktx::{Texture, code, log};
use super::vk_format::VK_FORMATS;

const KTX_INVALID_OPERATION: u32 = ktx::ErrorCode::InvalidOperation as u32;

/// The write-only enums of `EMSCRIPTEN_BINDINGS(ktx)`.
pub(crate) fn write_enums(obj: &Object) {
    use super::enum_object as e;
    let dims: Vec<(&str, u32)> = [
        "D4x4", "D5x4", "D5x5", "D6x5", "D6x6", "D8x5", "D8x6", "D10x5", "D10x6", "D8x8", "D10x8",
        "D10x10", "D12x10", "D12x12", "D3x3x3", "D4x3x3", "D4x4x3", "D4x4x4", "D5x4x4", "D5x5x4",
        "D5x5x5", "D6x5x5", "D6x6x5", "D6x6x6",
    ]
    .iter()
    .zip(0u32..)
    .map(|(&n, v)| (n, v))
    .collect();
    for (name, value) in [
        (
            "TextureCreateStorageEnum",
            e(&[("NO_STORAGE", 0), ("ALLOC_STORAGE", 1)]),
        ),
        ("VkFormat", e(VK_FORMATS)),
        (
            "pack_astc_quality_levels",
            e(&[
                ("FASTEST", 0),
                ("FAST", 10),
                ("MEDIUM", 60),
                ("THOROUGH", 98),
                ("EXHAUSTIVE", 100),
            ]),
        ),
        ("pack_astc_block_dimension", e(&dims)),
        (
            "pack_astc_encoder_mode",
            e(&[("DEFAULT", 0), ("LDR", 1), ("HDR", 2)]),
        ),
        (
            "pack_uastc_flag_bits",
            e(&[
                ("LEVEL_FASTEST", 0),
                ("LEVEL_FASTER", 1),
                ("LEVEL_DEFAULT", 2),
                ("LEVEL_SLOWER", 3),
                ("LEVEL_VERYSLOW", 4),
            ]),
        ),
    ] {
        let _ = Reflect::set(obj, &JsValue::from_str(name), &value);
    }
}

/// The `constant(...)`s (only in the write build upstream, spelling included).
#[wasm_bindgen(js_name = __constants)]
pub fn constants() -> JsValue {
    let obj = Object::new();
    let set = |k: &str, v: JsValue| {
        let _ = Reflect::set(&obj, &JsValue::from_str(k), &v);
    };
    set("ANIMDATA_KEY", KTX_ANIMDATA_KEY.into());
    set("ORIENTATION_KEY", KTX_ORIENTATION_KEY.into());
    set("SWIZZLE_KEY", KTX_SWIZZLE_KEY.into());
    set("WRITER_KEY", KTX_WRITER_KEY.into());
    set("WRITER_SCPARAMS_KEY", KTX_WRITER_SCPARAMS_KEY.into());
    set("FACESLICE_WHOLE_lEVEL", KTX_FACESLICE_WHOLE_LEVEL.into());
    set(
        "ETC1S_DEFAULT_COMPRESSION_LEVEL",
        KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL.into(),
    );
    obj.into()
}

/// `char[4]` swizzle as the 4 character string embind returns (NULs included).
fn swizzle_get(s: &[u8; 4]) -> String {
    s.iter().map(|&b| char::from(b)).collect()
}

/// The setter copies `s[0..4]`; positions past the end of `s` read its terminating NUL.
fn swizzle_set(s: &str) -> [u8; 4] {
    let b = s.as_bytes();
    std::array::from_fn(|i| b.get(i).copied().unwrap_or(0))
}

/// `ktxTextureCreateInfo` (`textureCreateInfo`).
#[wasm_bindgen(js_name = textureCreateInfo)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TextureCreateInfo {
    /// `vkFormat` (the JS setter takes a `VkFormat` entry).
    #[wasm_bindgen(js_name = _vkFormat)]
    pub vk_format: u32,
    #[wasm_bindgen(js_name = baseWidth)]
    pub base_width: u32,
    #[wasm_bindgen(js_name = baseHeight)]
    pub base_height: u32,
    #[wasm_bindgen(js_name = baseDepth)]
    pub base_depth: u32,
    #[wasm_bindgen(js_name = numDimensions)]
    pub num_dimensions: u32,
    #[wasm_bindgen(js_name = numLevels)]
    pub num_levels: u32,
    #[wasm_bindgen(js_name = numLayers)]
    pub num_layers: u32,
    #[wasm_bindgen(js_name = numFaces)]
    pub num_faces: u32,
    #[wasm_bindgen(js_name = isArray)]
    pub is_array: bool,
    #[wasm_bindgen(js_name = generateMipmaps)]
    pub generate_mipmaps: bool,
}

#[wasm_bindgen(js_class = textureCreateInfo)]
impl TextureCreateInfo {
    #[wasm_bindgen(constructor)]
    pub fn new() -> TextureCreateInfo {
        TextureCreateInfo::default()
    }
}

/// `ktxAstcParams` (`astcParams`). Enum typed properties are set with entries and read back
/// as numbers (embind lambda properties over `uint32_t` fields).
#[wasm_bindgen(js_name = astcParams)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AstcParamsJs {
    p: AstcParams,
}

#[wasm_bindgen(js_class = astcParams)]
impl AstcParamsJs {
    #[wasm_bindgen(constructor)]
    pub fn new() -> AstcParamsJs {
        AstcParamsJs::default()
    }
    #[wasm_bindgen(getter = structSize)]
    pub fn struct_size(&self) -> u32 {
        self.p.struct_size
    }
    #[wasm_bindgen(setter = structSize)]
    pub fn set_struct_size(&mut self, v: u32) {
        self.p.struct_size = v;
    }
    #[wasm_bindgen(getter)]
    pub fn verbose(&self) -> bool {
        self.p.verbose
    }
    #[wasm_bindgen(setter)]
    pub fn set_verbose(&mut self, v: bool) {
        self.p.verbose = v;
    }
    #[wasm_bindgen(getter = threadCount)]
    pub fn thread_count(&self) -> u32 {
        self.p.thread_count
    }
    #[wasm_bindgen(setter = threadCount)]
    pub fn set_thread_count(&mut self, v: u32) {
        self.p.thread_count = v;
    }
    #[wasm_bindgen(getter = blockDimension)]
    pub fn block_dimension(&self) -> u32 {
        self.p.block_dimension
    }
    #[wasm_bindgen(setter = _blockDimension)]
    pub fn set_block_dimension(&mut self, v: u32) {
        self.p.block_dimension = v;
    }
    #[wasm_bindgen(getter)]
    pub fn mode(&self) -> u32 {
        self.p.mode
    }
    #[wasm_bindgen(setter = _mode)]
    pub fn set_mode(&mut self, v: u32) {
        self.p.mode = v;
    }
    #[wasm_bindgen(getter = qualityLevel)]
    pub fn quality_level(&self) -> u32 {
        self.p.quality_level
    }
    #[wasm_bindgen(setter = _qualityLevel)]
    pub fn set_quality_level(&mut self, v: u32) {
        self.p.quality_level = v;
    }
    #[wasm_bindgen(getter = normalMap)]
    pub fn normal_map(&self) -> bool {
        self.p.normal_map
    }
    #[wasm_bindgen(setter = normalMap)]
    pub fn set_normal_map(&mut self, v: bool) {
        self.p.normal_map = v;
    }
    #[wasm_bindgen(getter = inputSwizzle)]
    pub fn input_swizzle(&self) -> String {
        swizzle_get(&self.p.input_swizzle)
    }
    #[wasm_bindgen(setter = inputSwizzle)]
    pub fn set_input_swizzle(&mut self, s: &str) {
        self.p.input_swizzle = swizzle_set(s);
    }
}

/// `ktxBasisParams` (`basisParams`).
#[wasm_bindgen(js_name = basisParams)]
#[derive(Debug, Clone, Copy, Default)]
pub struct BasisParamsJs {
    p: BasisParams,
}

/// Plain getter / setter pairs of `basisParams` fields.
macro_rules! props {
    ($($js:ident, $set_js:ident: $field:ident, $set:ident: $t:ty;)*) => {
        #[wasm_bindgen(js_class = basisParams)]
        impl BasisParamsJs {
            $(
                #[wasm_bindgen(getter = $js)]
                pub fn $field(&self) -> $t {
                    self.p.$field
                }
                #[wasm_bindgen(setter = $set_js)]
                pub fn $set(&mut self, v: $t) {
                    self.p.$field = v;
                }
            )*
        }
    };
}

props! {
    structSize, structSize: struct_size, set_struct_size: u32;
    codec, codec: codec, set_codec: u32;
    verbose, verbose: verbose, set_verbose: bool;
    noSSE, noSSE: no_sse, set_no_sse: bool;
    threadCount, threadCount: thread_count, set_thread_count: u32;
    preSwizzle, preSwizzle: pre_swizzle, set_pre_swizzle: bool;
    etc1sCompressionLevel, etc1sCompressionLevel: etc1s_compression_level,
        set_etc1s_compression_level: u32;
    qualityLevel, qualityLevel: quality_level, set_quality_level: u32;
    maxEndpoints, maxEndpoints: max_endpoints, set_max_endpoints: u32;
    endpointRDOThreshold, endpointRDOThreshold: endpoint_rdo_threshold,
        set_endpoint_rdo_threshold: f32;
    maxSelectors, maxSelectors: max_selectors, set_max_selectors: u32;
    selectorRDOThreshold, selectorRDOThreshold: selector_rdo_threshold,
        set_selector_rdo_threshold: f32;
    normalMap, normalMap: normal_map, set_normal_map: bool;
    noEndpointRDO, noEndpointRDO: no_endpoint_rdo, set_no_endpoint_rdo: bool;
    noSelectorRDO, noSelectorRDO: no_selector_rdo, set_no_selector_rdo: bool;
    uastcFlags, _uastcFlags: uastc_flags, set_uastc_flags: u32;
    uastcRDO, uastcRDO: uastc_rdo, set_uastc_rdo: bool;
    uastcRDOQualityScalar, uastcRDOQualityScalar: uastc_rdo_quality_scalar,
        set_uastc_rdo_quality_scalar: f32;
    uastcRDODictSize, uastcRDODictSize: uastc_rdo_dict_size, set_uastc_rdo_dict_size: u32;
    uastcRDOMaxSmoothBlockErrorScale, uastcRDOMaxSmoothBlockErrorScale:
        uastc_rdo_max_smooth_block_error_scale, set_uastc_rdo_max_smooth_block_error_scale: f32;
    uastcRDOMaxSmoothBlockStdDev, uastcRDOMaxSmoothBlockStdDev:
        uastc_rdo_max_smooth_block_std_dev, set_uastc_rdo_max_smooth_block_std_dev: f32;
    uastcRDODontFavorSimplerModes, uastcRDODontFavorSimplerModes:
        uastc_rdo_dont_favor_simpler_modes, set_uastc_rdo_dont_favor_simpler_modes: bool;
    uastcRDONoMultithreading, uastcRDONoMultithreading: uastc_rdo_no_multithreading,
        set_uastc_rdo_no_multithreading: bool;
    uastcHDRQuality, uastcHDRQuality: uastc_hdr_quality, set_uastc_hdr_quality: u32;
    uastcHDRUberMode, uastcHDRUberMode: uastc_hdr_uber_mode, set_uastc_hdr_uber_mode: bool;
    uastcHDRUltraQuant, uastcHDRUltraQuant: uastc_hdr_ultra_quant,
        set_uastc_hdr_ultra_quant: bool;
    uastcHDRFavorAstc, uastcHDRFavorAstc: uastc_hdr_favor_astc, set_uastc_hdr_favor_astc: bool;
    uastcHDRLambda, uastcHDRLambda: uastc_hdr_lambda, set_uastc_hdr_lambda: f32;
    uastcHDRLevel, uastcHDRLevel: uastc_hdr_level, set_uastc_hdr_level: u32;
}

#[wasm_bindgen(js_class = basisParams)]
impl BasisParamsJs {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BasisParamsJs {
        BasisParamsJs::default()
    }
    #[wasm_bindgen(getter = inputSwizzle)]
    pub fn input_swizzle(&self) -> String {
        swizzle_get(&self.p.input_swizzle)
    }
    #[wasm_bindgen(setter = inputSwizzle)]
    pub fn set_input_swizzle(&mut self, s: &str) {
        self.p.input_swizzle = swizzle_set(s);
    }
}

/// Logs `ERROR: <what>: <ktxErrorString>` for a failed result, like the C++ methods.
fn report(what: &str, result: u32) -> u32 {
    if result != 0 {
        log(&format!("ERROR: {what}: {}", ktx_error_string_code(result)));
    }
    result
}

#[wasm_bindgen(js_class = texture)]
impl Texture {
    /// `texture(const ktxTextureCreateInfo&, ktxTextureCreateStorageEnum)`
    /// (`ktxTexture2_Create`).
    pub fn _create(&mut self, info: &TextureCreateInfo, storage: u32) {
        let create_info = CreateInfo {
            vk_format: VkFormat(info.vk_format),
            base_width: info.base_width,
            base_height: info.base_height,
            base_depth: info.base_depth,
            num_dimensions: info.num_dimensions,
            num_levels: info.num_levels,
            num_layers: info.num_layers,
            num_faces: info.num_faces,
            is_array: info.is_array,
            generate_mipmaps: info.generate_mipmaps,
            ..Default::default()
        };
        let storage = if storage == TextureCreateStorage::AllocStorage as u32 {
            TextureCreateStorage::AllocStorage
        } else {
            TextureCreateStorage::NoStorage
        };
        match Texture2::create(&create_info, storage) {
            Ok(t) => self.ptr = Some(AnyTexture::Ktx2(t)),
            Err(e) => {
                log(&format!(
                    "ERROR: failed to create texture: {}",
                    ktx_error_string(e)
                ));
                self.ptr = None;
            }
        }
    }

    #[wasm_bindgen(js_name = setImageFromMemory)]
    pub fn _set_image_from_memory(
        &mut self,
        level: u32,
        layer: u32,
        face_slice: u32,
        image: &[u8],
    ) -> u32 {
        let result = match &mut self.ptr {
            Some(t) => code(t.set_image_from_memory(level, layer, face_slice, image)),
            None => KTX_INVALID_OPERATION,
        };
        report("Failed to setImageFromMemory", result)
    }

    /// `ktxTexture2_CompressAstcEx` with `threadCount = 1`, `verbose = false`.
    pub fn _compress_astc(&mut self, params: &AstcParamsJs) -> u32 {
        let mut p = params.p;
        p.struct_size = ktx::encode_params::KTX_ASTC_PARAMS_SIZE;
        p.thread_count = 1;
        p.verbose = false;
        let result = match self.texture2_mut() {
            Some(t) => code(t.compress_astc_ex(&p)),
            None => KTX_INVALID_OPERATION,
        };
        report("Failed to compressAstc", result)
    }

    /// `ktxTexture2_CompressBasisEx` with `threadCount = 1`, `verbose = false`,
    /// `noSSE = true`.
    pub fn _compress_basis(&mut self, params: &BasisParamsJs) -> u32 {
        let mut p = params.p;
        p.struct_size = ktx::encode_params::KTX_BASIS_PARAMS_SIZE;
        p.thread_count = 1;
        p.verbose = false;
        p.no_sse = true;
        let result = match self.texture2_mut() {
            Some(t) => code(t.compress_basis_ex(&p)),
            None => KTX_INVALID_OPERATION,
        };
        report("failed to compressBasis", result)
    }

    pub fn _deflate_zstd(&mut self, compression_level: i32) -> u32 {
        let Some(t) = self.texture2_mut() else {
            log("ERROR: deflateZstd is only supported for KTX2");
            return KTX_INVALID_OPERATION;
        };
        let result = code(t.deflate_zstd(compression_level as u32));
        report("Failed to deflateZstd", result)
    }

    pub fn _deflate_zlib(&mut self, compression_level: i32) -> u32 {
        let Some(t) = self.texture2_mut() else {
            log("ERROR: deflateZLIB is only supported for KTX2");
            return KTX_INVALID_OPERATION;
        };
        let result = code(t.deflate_zlib(compression_level as u32));
        report("Failed to deflateZLIB", result)
    }

    /// `addKVPair(key, std::string)`: the value is stored with its terminating NUL.
    pub fn _add_kv_pair_string(&mut self, key: &str, value: &str) -> u32 {
        let mut v = value.as_bytes().to_vec();
        v.push(0);
        let result = match &mut self.ptr {
            Some(t) => code(t.base_mut().kv_data_head.add_kv_pair(key, &v)),
            None => KTX_INVALID_OPERATION,
        };
        report("Failed to addKVPair (string)", result)
    }

    pub fn _add_kv_pair_byte(&mut self, key: &str, value: &[u8]) -> u32 {
        let result = match &mut self.ptr {
            Some(t) => code(t.base_mut().kv_data_head.add_kv_pair(key, value)),
            None => KTX_INVALID_OPERATION,
        };
        report("Failed to addKVPair (vector)", result)
    }

    pub fn _delete_kv_pair(&mut self, key: &str) -> u32 {
        let result = match &mut self.ptr {
            Some(t) => code(t.base_mut().kv_data_head.delete_kv_pair(key)),
            None => KTX_INVALID_OPERATION,
        };
        report("Failed to deleteKVPair", result)
    }

    pub fn _set_transfer_function(&mut self, tf: u32) -> u32 {
        self.texture2_mut()
            .map_or(KTX_INVALID_OPERATION, |t| code(t.set_transfer_function(tf)))
    }

    pub fn _set_oetf(&mut self, oetf: u32) -> u32 {
        self.texture2_mut()
            .map_or(KTX_INVALID_OPERATION, |t| code(t.set_oetf(oetf)))
    }

    pub fn _set_primaries(&mut self, primaries: u32) -> u32 {
        self.texture2_mut()
            .map_or(KTX_INVALID_OPERATION, |t| code(t.set_primaries(primaries)))
    }

    /// `ktxTexture_WriteToMemory`; `null` on failure.
    #[wasm_bindgen(js_name = writeToMemory)]
    pub fn write_to_memory(&mut self) -> Option<Uint8Array> {
        let t = self.ptr.as_mut()?;
        match t.write_to_memory() {
            Ok(data) => Some(Uint8Array::from(data.as_slice())),
            Err(e) => {
                log(&format!(
                    "ERROR: Failed to writeToMemory: {}",
                    ktx_error_string(e)
                ));
                None
            }
        }
    }
}
