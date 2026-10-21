//! Port of tests/texturetests/texturetests.cc (sections are added as the library grows).
#![allow(clippy::panic, clippy::type_complexity)]

mod common;

use ktx::{
    ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KTX_TEXTURE_CREATE_NO_FLAGS, KtxTexture,
    SupercmpScheme, Texture2, ktx_error_string,
};

////////////////////////////////////////////
// Unicode file name tests
///////////////////////////////////////////

#[test]
fn unicode_file_names_create_from() {
    let ktx_file_set = [
        "hűtő.ktx",
        "نَسِيج.ktx",
        "テクスチャ.ktx",
        "质地.ktx",
        "조직.ktx",
    ];
    let ktx2_file_set = [
        "hűtő.ktx2",
        "نَسِيج.ktx2",
        "テクスチャ.ktx2",
        "质地.ktx2",
        "조직.ktx2",
        "hűtő_zstd.ktx2",
        "نَسِيج_zstd.ktx2",
        "テクスチャ_zstd.ktx2",
        "质地_zstd.ktx2",
        "조직_zstd.ktx2",
    ];
    let check_creation = |name: &str, ktx: bool| {
        let path = common::unicode_path().join(name);
        if let Err(e) = ktx::AnyTexture::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS)
        {
            panic!(
                "ktxTexture_CreateFromNamedFile \"{}\" failed: {}",
                path.display(),
                ktx_error_string(e)
            );
        }
        let result = if ktx {
            ktx::Texture1::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS).map(|_| ())
        } else {
            Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS).map(|_| ())
        };
        if let Err(e) = result {
            panic!(
                "ktxTexture{}_CreateFromNamedFile \"{}\" failed: {}",
                if ktx { 1 } else { 2 },
                path.display(),
                ktx_error_string(e)
            );
        }
    };
    for name in ktx_file_set {
        check_creation(name, true);
    }
    for name in ktx2_file_set {
        check_creation(name, false);
    }
}

/////////////////////////////////////////
// ktxTexture2_GetDataSizeUncompressed
/////////////////////////////////////////

#[test]
fn ktx_texture2_get_data_size_uncompressed_compare_with_get_data_size_after_inflate() {
    let path = common::ktx2_path().join("invalid_zstd_level0_padding.ktx2");
    let mut texture = match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS) {
        Ok(t) => t,
        Err(e) => panic!(
            "ktxTexture2_CreateFromNamedFile failed: {}",
            ktx_error_string(e)
        ),
    };
    let reported_uncompressed_size = texture.get_data_size_uncompressed();
    assert_eq!(texture.load_image_data(None), Ok(()));
    assert!(texture.get_data().is_some(), "Image data not loaded");
    assert_eq!(reported_uncompressed_size, texture.data_size);
}

/////////////////////////////////////////
// ktxTexture2 invalid creation params tests.
/////////////////////////////////////////

#[test]
fn ktx_texture2_invalid_create_info_params_invalid_width() {
    let create_info = ktx::TextureCreateInfo {
        gl_internalformat: 0,
        vk_format: ktx::VkFormat::R8G8B8A8_SRGB,
        dfd: None,
        base_width: 0, // Invalid width
        base_height: 1,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        is_array: false,
        generate_mipmaps: false,
    };
    let result = Texture2::create(&create_info, ktx::TextureCreateStorage::NoStorage);
    assert_eq!(result.err(), Some(ErrorCode::InvalidValue));
}

/////////////////////////////////////////
// ktxTexture2_IterateLoadLevelFaces with supercompression
/////////////////////////////////////////

#[derive(Default)]
struct SupercompressionIterCbData {
    calls: u32,
    miplevel: u32,
    face: u32,
    width: u32,
    height: u32,
    depth: u32,
    face_lod_size: u64,
    pixel: u8,
}

fn r8_1x1_create_info() -> ktx::TextureCreateInfo {
    ktx::TextureCreateInfo {
        gl_internalformat: 0,
        vk_format: ktx::VkFormat::R8_UNORM,
        dfd: None,
        base_width: 1,
        base_height: 1,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        is_array: false,
        generate_mipmaps: false,
    }
}

fn iterate_load_level_faces_supercompressed(zlib: bool) {
    let pixel: u8 = 7;
    let mut texture = match Texture2::create(
        &r8_1x1_create_info(),
        ktx::TextureCreateStorage::AllocStorage,
    ) {
        Ok(t) => t,
        Err(e) => panic!("ktxTexture2_Create failed: {}", ktx_error_string(e)),
    };
    assert_eq!(texture.set_image_from_memory(0, 0, 0, &[pixel]), Ok(()));
    if zlib {
        assert_eq!(texture.deflate_zlib(9), Ok(()));
    } else {
        assert_eq!(texture.deflate_zstd(3), Ok(()));
    }
    let ktx_mem_file = match texture.write_to_memory() {
        Ok(m) => m,
        Err(e) => panic!("ktxTexture2_WriteToMemory failed: {}", ktx_error_string(e)),
    };

    // Control: a normal load inflates the level.
    let texture =
        match Texture2::create_from_memory(&ktx_mem_file, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => t,
            Err(e) => panic!(
                "ktxTexture2_CreateFromMemory failed: {}",
                ktx_error_string(e)
            ),
        };
    let data = match texture.get_data() {
        Some(d) => d,
        None => panic!("Image data not loaded"),
    };
    assert_eq!(data[0], pixel);

    let mut texture = match Texture2::create_from_memory(&ktx_mem_file, KTX_TEXTURE_CREATE_NO_FLAGS)
    {
        Ok(t) => t,
        Err(e) => panic!(
            "ktxTexture2_CreateFromMemory failed: {}",
            ktx_error_string(e)
        ),
    };
    let expected_scheme = if zlib {
        SupercmpScheme::ZLIB
    } else {
        SupercmpScheme::ZSTD
    };
    assert_eq!(texture.supercompression_scheme, expected_scheme);

    let mut data = SupercompressionIterCbData::default();
    let result = texture.iterate_load_level_faces(
        &mut |miplevel, face, width, height, depth, face_lod_size, pixels| {
            data.calls += 1;
            data.miplevel = miplevel;
            data.face = face;
            data.width = width;
            data.height = height;
            data.depth = depth;
            data.face_lod_size = face_lod_size;
            data.pixel = pixels.first().copied().unwrap_or_default();
            Ok(())
        },
    );
    assert_eq!(result, Ok(()));
    assert_eq!(data.calls, 1);
    assert_eq!(data.miplevel, 0);
    assert_eq!(data.face, 0);
    assert_eq!(data.width, 1);
    assert_eq!(data.height, 1);
    assert_eq!(data.depth, 1);
    assert_eq!(data.face_lod_size, 1);
    assert_eq!(data.pixel, pixel);
}

#[test]
fn ktx_texture2_iterate_load_level_faces_zlib_iterate_image() {
    iterate_load_level_faces_supercompressed(true);
}

#[test]
fn ktx_texture2_iterate_load_level_faces_zstd_iterate_image() {
    iterate_load_level_faces_supercompressed(false);
}

// ---------------------------------------------------------------------------
// Fixtures (port of ktxTextureTestBase & friends)
// ---------------------------------------------------------------------------

use common::wthelper::{E_ARRAY, E_MIPMAPPED, E_NONE, WriterTestHelper};
use ktx::defs::{KTX_GL_UNPACK_ALIGNMENT, KTX_IDENTIFIER_REF, KTX_ORIENTATION_KEY};
use ktx::gl_format::{GL_RG16, GL_RGB8, GL_RGBA8, GL_SRGB8};
use ktx::ktxint::KtxHeader;
use ktx::vk_format::{RgbaAstcMapping, vk_get_format_from_opengl_internal_format};
use ktx::{
    AnyTexture, ClassId, FormatVersion, KTX_TEXTURE_CREATE_RAW_KVDATA_BIT,
    KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT,
};
use ktx::{Texture1, TextureCreateInfo, TextureCreateStorage, VkFormat};

/// `ROUNDING(x)` macro.
fn rounding(x: u32) -> u32 {
    3 - ((x + KTX_GL_UNPACK_ALIGNMENT - 1) % KTX_GL_UNPACK_ALIGNMENT)
}

fn ok<T>(r: ktx::KtxResult<T>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{what} failed: {}", ktx_error_string(e)),
    }
}

/// Port of `ktxTextureTestBase<GLubyte, 4, GL_RGBA8>`.
struct TextureTestBase {
    helper: WriterTestHelper<u8, 4, GL_RGBA8>,
    kv_data: Vec<u8>,
    kv_data_len: u32,
    ktx_mem_file: Vec<u8>,
    pixel_size: u32,
    mip_levels: u32,
    iter_cb_calls: u32,
    padded_image_data_size: usize,
}

impl TextureTestBase {
    fn new(fv: FormatVersion) -> TextureTestBase {
        let mut helper = WriterTestHelper::<u8, 4, GL_RGBA8>::new();
        helper.resize(E_MIPMAPPED, 1, 1, 2, 16, 16, 1);
        let mip_levels = helper.num_levels;
        let (kv_data, kv_data_len, padded, mem) = if fv == FormatVersion::One {
            let mut texture = ok(
                Texture1::create(&helper.texinfo.ci, TextureCreateStorage::AllocStorage),
                "ktxTexture1_Create",
            );
            Self::set_images(&helper, &mut |l, ly, f, d| {
                texture.set_image_from_memory(l, ly, f, d)
            });
            let padded = texture.data_size;
            texture.kv_data = Some(helper.kv_data.clone());
            texture.kv_data_len = helper.kv_data_len;
            let mem = ok(texture.write_to_memory(), "ktxTexture_WriteToMemory");
            (helper.kv_data.clone(), helper.kv_data_len, padded, mem)
        } else {
            let mut ci = helper.texinfo.ci.clone();
            ci.vk_format = vk_get_format_from_opengl_internal_format(
                ci.gl_internalformat,
                RgbaAstcMapping::ToHdr,
            );
            helper.texinfo.ci.vk_format = ci.vk_format;
            let mut texture = ok(
                Texture2::create(&ci, TextureCreateStorage::AllocStorage),
                "ktxTexture2_Create",
            );
            texture.kv_data_head = ktx::HashList::construct_copy(&helper.kv_hash_ktx2);
            Self::set_images(&helper, &mut |l, ly, f, d| {
                texture.set_image_from_memory(l, ly, f, d)
            });
            let padded = texture.data_size;
            let mem = ok(texture.write_to_memory(), "ktxTexture_WriteToMemory");
            (
                helper.kv_data_writer_ktx2.clone(),
                helper.kv_data_len_writer_ktx2,
                padded,
                mem,
            )
        };
        TextureTestBase {
            helper,
            kv_data,
            kv_data_len,
            ktx_mem_file: mem,
            pixel_size: 16,
            mip_levels,
            iter_cb_calls: 0,
            padded_image_data_size: padded,
        }
    }

    fn set_images(
        helper: &WriterTestHelper<u8, 4, GL_RGBA8>,
        set: &mut dyn FnMut(u32, u32, u32, &[u8]) -> ktx::KtxResult<()>,
    ) {
        let ci = &helper.texinfo.ci;
        for level in 0..ci.num_levels {
            let level_depth = (ci.base_depth >> level).max(1);
            let image = &helper.image_list[level as usize];
            for layer in 0..ci.num_layers {
                let num_images = if ci.num_faces == 6 {
                    ci.num_faces
                } else {
                    level_depth
                };
                for face_slice in 0..num_images {
                    let _ = set(level, layer, face_slice, &image.data);
                }
            }
        }
    }

    /// Port of `iterCallback`.
    fn iter_callback(
        &mut self,
        miplevel: u32,
        width: u32,
        face_lod_size: u64,
        pixels: &[u8],
    ) -> ktx::KtxResult<()> {
        let expected_width = self.pixel_size >> miplevel;
        assert_eq!(width, expected_width);
        assert_eq!(face_lod_size, (expected_width * expected_width * 4) as u64);
        let img = &self.helper.image_list[miplevel as usize];
        assert_eq!(pixels.get(..img.size), Some(&img.data[..]));
        self.iter_cb_calls += 1;
        Ok(())
    }

    /// Port of `ktxTexture1TestBase::compareTexture`.
    fn compare_texture1(&self, t: &Texture1) -> bool {
        let ti = &self.helper.texinfo;
        t.gl_internalformat == ti.ci.gl_internalformat
            && t.gl_base_internalformat == ti.gl_base_internalformat
            && t.gl_format == ti.gl_format
            && t.gl_type == ti.gl_type
            && t.gl_type_size() == ti.gl_type_size
            && t.base_width == ti.ci.base_width
            && (if ti.ci.base_height == 0 {
                t.base_height == 1
            } else {
                t.base_height == ti.ci.base_height
            })
            && (if ti.ci.base_depth == 0 {
                t.base_depth == 1
            } else {
                t.base_depth == ti.ci.base_depth
            })
            && t.num_faces == ti.ci.num_faces
            && t.num_levels == ti.ci.num_levels
    }

    /// Port of `ktxTexture2TestBase::compareTexture`.
    fn compare_texture2(&self, t: &Texture2) -> bool {
        let ti = &self.helper.texinfo;
        t.vk_format
            == vk_get_format_from_opengl_internal_format(
                ti.ci.gl_internalformat,
                RgbaAstcMapping::ToHdr,
            )
            && t.base_width == ti.ci.base_width
            && (if ti.ci.base_height == 0 {
                t.base_height == 1
            } else {
                t.base_height == ti.ci.base_height
            })
            && (if ti.ci.base_depth == 0 {
                t.base_depth == 1
            } else {
                t.base_depth == ti.ci.base_depth
            })
            && t.num_faces == ti.ci.num_faces
            && t.num_levels == ti.ci.num_levels
    }

    fn create_any(&self, flags: u32) -> AnyTexture {
        ok(
            AnyTexture::create_from_memory(&self.ktx_mem_file, flags),
            "ktxTexture_CreateFromMemory",
        )
    }
}

fn ktx1_fixture() -> TextureTestBase {
    TextureTestBase::new(FormatVersion::One)
}

fn ktx2_fixture() -> TextureTestBase {
    TextureTestBase::new(FormatVersion::Two)
}

// Not portable: tests passing NULL pointers or NULL callbacks
// (InvalidValueOnNullParams, *_InvalidValueOnNullCallback). Rust references and
// closures cannot be null, so these error paths do not exist.

/////////////////////////////////////////
// ktxTexture_Create tests
/////////////////////////////////////////

#[test]
fn ktx_texture_create_test_construct_from_memory() {
    let f = ktx1_fixture();
    let texture = f.create_any(0);
    assert_eq!(texture.class_id(), ClassId::Texture1);
    let AnyTexture::Ktx1(t1) = &texture else {
        panic!("not ktx1")
    };
    assert!(f.compare_texture1(t1));
    assert!(!texture.is_compressed);
    assert!(!texture.generate_mipmaps);
    assert_eq!(texture.num_dimensions, 2);
    assert_eq!(texture.num_layers, 1);
    assert!(!texture.is_array);
}

#[test]
fn ktx_texture1_create_test_construct_from_memory() {
    let f = ktx1_fixture();
    let texture = ok(
        Texture1::create_from_memory(&f.ktx_mem_file, 0),
        "ktxTexture1_CreateFromMemory",
    );
    assert!(f.compare_texture1(&texture));
    assert!(!texture.is_compressed);
    assert!(!texture.generate_mipmaps);
    assert_eq!(texture.num_dimensions, 2);
    assert_eq!(texture.num_layers, 1);
    assert!(!texture.is_array);
}

#[test]
fn ktx_texture1_create_test_create_empty() {
    let f = ktx1_fixture();
    ok(
        Texture1::create(&f.helper.create_info, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
}

#[test]
fn ktx_texture1_create_test_invalid_value_too_many_mip_levels() {
    let f = ktx1_fixture();
    let mut ci = f.helper.create_info.clone();
    ci.num_levels += 1;
    assert_eq!(
        Texture1::create(&ci, TextureCreateStorage::NoStorage).err(),
        Some(ErrorCode::InvalidOperation)
    );
}

#[test]
fn ktx_texture1_create_test_invalid_op_on_set_images_no_storage() {
    let f = ktx1_fixture();
    let mut texture = ok(
        Texture1::create(&f.helper.create_info, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    let image = &f.helper.images[0][0][0];
    assert_eq!(
        texture.set_image_from_memory(0, 0, 0, image),
        Err(ErrorCode::InvalidOperation)
    );
}

#[test]
fn ktx_texture1_create_test_create_empty_and_set_images() {
    let f = ktx1_fixture();
    let mut texture = ok(
        Texture1::create(&f.helper.create_info, TextureCreateStorage::AllocStorage),
        "ktxTexture1_Create",
    );
    assert_eq!(f.helper.copy_images_to_texture1(&mut texture), Ok(()));
    assert!(
        f.helper
            .compare_texture1_images(texture.get_data().unwrap_or_default())
    );
}

#[test]
fn ktx_texture1_create_test_create_empty_set_images_write_to_memory() {
    let f = ktx1_fixture();
    let mut texture = ok(
        Texture1::create(&f.helper.create_info, TextureCreateStorage::AllocStorage),
        "ktxTexture1_Create",
    );
    assert_eq!(
        texture
            .kv_data_head
            .add_kv_pair(KTX_ORIENTATION_KEY, b"S=r,T=d\0"),
        Ok(())
    );
    assert_eq!(f.helper.copy_images_to_texture1(&mut texture), Ok(()));
    assert!(
        f.helper
            .compare_texture1_images(texture.get_data().unwrap_or_default())
    );
    let test_mem_file = ok(texture.write_to_memory(), "ktxTexture1_WriteToMemory");
    assert_eq!(test_mem_file.len(), f.ktx_mem_file.len());
    assert!(test_mem_file == f.ktx_mem_file);
}

/////////////////////////////////////////
// ktxTexture2_Create tests
/////////////////////////////////////////

/// Port of `ktxTexture2_CreateTest::create` with defaults.
fn ktx2_create(format: VkFormat) -> ktx::KtxResult<Texture2> {
    let ci = TextureCreateInfo {
        vk_format: format,
        base_width: 16,
        base_height: 16,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    Texture2::create(&ci, TextureCreateStorage::AllocStorage)
}

#[test]
fn ktx_texture2_create_test_e5b9g9r9() {
    assert!(ktx2_create(VkFormat::E5B9G9R9_UFLOAT_PACK32).is_ok());
}

/////////////////////////////////////////
// ktxTexture_KVData tests
/////////////////////////////////////////

#[test]
fn ktx_texture_kv_data_test_kv_data_deserialized() {
    let f = ktx1_fixture();
    let texture = f.create_any(0);
    assert!(texture.kv_data.is_none(), "Raw KVData should not be loaded");
    assert!(!texture.kv_data_head.is_empty(), "KVData not deserialized");
    let value = ok(
        texture.kv_data_head.find_value(KTX_ORIENTATION_KEY),
        "find orientation",
    );
    assert_eq!(value.get(..7), Some(&b"S=r,T=d"[..]));
}

#[test]
fn ktx_texture_kv_data_test_load_raw_kv_data() {
    let f = ktx1_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_RAW_KVDATA_BIT);
    assert!(texture.kv_data.is_some(), "Raw KVData not loaded");
    assert!(
        texture.kv_data_head.is_empty(),
        "KVData should not be deserialized"
    );
    assert_eq!(
        texture.kv_data_len, f.kv_data_len,
        "Length of KV data incorrect"
    );
    assert_eq!(texture.kv_data.as_deref(), Some(&f.kv_data[..]));
}

#[test]
fn ktx_texture_kv_data_test_skip_kv_data() {
    let f = ktx1_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT);
    assert!(texture.kv_data.is_none(), "Raw KVData should not be loaded");
    assert!(
        texture.kv_data_head.is_empty(),
        "KVData should not be deserialized"
    );
}

/////////////////////////////////////////
// ktxTexture_IterateLoadLevelFaces tests
/////////////////////////////////////////

#[test]
fn ktx_texture1_iterate_load_level_faces_test_invalid_op_when_data_already_loaded() {
    let mut f = ktx1_fixture();
    let mut texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    assert!(texture.p_data.is_some(), "Image data not loaded");
    let r =
        texture.iterate_load_level_faces(&mut |l, _f, w, _h, _d, s, p| f.iter_callback(l, w, s, p));
    assert_eq!(r, Err(ErrorCode::InvalidOperation));
}

#[test]
fn ktx_texture1_iterate_load_level_faces_test_iterate_images() {
    let mut f = ktx1_fixture();
    let mut texture = f.create_any(0);
    let r =
        texture.iterate_load_level_faces(&mut |l, _f, w, _h, _d, s, p| f.iter_callback(l, w, s, p));
    assert_eq!(r, Ok(()));
    assert_eq!(
        f.iter_cb_calls, f.mip_levels,
        "No. of calls to iterCallback differs from number of mip levels"
    );
}

/////////////////////////////////////////
// ktxTexture_IterateLevelFaces tests
/////////////////////////////////////////

#[test]
fn ktx_texture1_iterate_level_faces_test_iterate_images() {
    let mut f = ktx1_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    let r = texture.iterate_level_faces(&mut |l, _f, w, _h, _d, s, p| f.iter_callback(l, w, s, p));
    assert_eq!(r, Ok(()));
    assert_eq!(f.iter_cb_calls, f.mip_levels);
}

#[test]
fn ktx_texture2_iterate_level_faces_test_iterate_images() {
    let mut f = ktx2_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    let r = texture.iterate_level_faces(&mut |l, _f, w, _h, _d, s, p| f.iter_callback(l, w, s, p));
    assert_eq!(r, Ok(()));
    assert_eq!(f.iter_cb_calls, f.mip_levels);
}

/////////////////////////////////////////
// ktxTexture_IterateLevels tests
/////////////////////////////////////////

#[test]
fn ktx_texture2_iterate_levels_test_iterate_levels() {
    // The C test calls ktxTexture_IterateLevelFaces here too.
    let mut f = ktx2_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    let r = texture.iterate_level_faces(&mut |l, _f, w, _h, _d, s, p| f.iter_callback(l, w, s, p));
    assert_eq!(r, Ok(()));
    assert_eq!(f.iter_cb_calls, f.mip_levels);
}

/////////////////////////////////////////
// ktxTexture_LoadImageData tests
/////////////////////////////////////////

fn load_image_data_invalid_op_when_loaded(f: TextureTestBase) {
    let mut texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    assert!(texture.p_data.is_some(), "Image data not loaded");
    let mut buf = vec![0u8; f.padded_image_data_size];
    assert_eq!(
        texture.load_image_data(Some(&mut buf)),
        Err(ErrorCode::InvalidOperation)
    );
}

fn load_image_data_invalid_op_when_loaded_to_external(f: TextureTestBase) {
    let mut texture = f.create_any(0);
    assert!(texture.p_data.is_none(), "Image data must not be loaded");
    let mut buf = vec![0u8; f.padded_image_data_size];
    assert_eq!(texture.load_image_data(Some(&mut buf)), Ok(()));
    assert_eq!(
        texture.load_image_data(Some(&mut buf)),
        Err(ErrorCode::InvalidOperation)
    );
}

#[test]
fn ktx_texture1_load_image_data_test_invalid_op_when_data_already_loaded() {
    load_image_data_invalid_op_when_loaded(ktx1_fixture());
}

#[test]
fn ktx_texture1_load_image_data_test_invalid_op_when_data_already_loaded_to_external() {
    load_image_data_invalid_op_when_loaded_to_external(ktx1_fixture());
}

#[test]
fn ktx_texture1_load_image_data_test_load_image_data_internal() {
    let f = ktx1_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    assert_eq!(f.padded_image_data_size, texture.get_data_size());
    assert!(
        f.helper
            .compare_texture1_images(texture.get_data().unwrap_or_default())
    );
}

#[test]
fn ktx_texture1_load_image_data_test_load_image_data_external() {
    let f = ktx1_fixture();
    let mut texture = f.create_any(0);
    let mut buf = vec![0u8; f.padded_image_data_size];
    assert_eq!(texture.load_image_data(Some(&mut buf)), Ok(()));
    assert_eq!(f.padded_image_data_size, texture.get_data_size());
    assert!(f.helper.compare_texture1_images(&buf));
}

#[test]
fn ktx_texture2_load_image_data_test_invalid_op_when_data_already_loaded() {
    load_image_data_invalid_op_when_loaded(ktx2_fixture());
}

#[test]
fn ktx_texture2_load_image_data_test_invalid_op_when_data_already_loaded_to_external() {
    load_image_data_invalid_op_when_loaded_to_external(ktx2_fixture());
}

#[test]
fn ktx_texture2_load_image_data_test_load_image_data_internal() {
    let f = ktx2_fixture();
    let texture = f.create_any(KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT);
    assert_eq!(f.padded_image_data_size, texture.get_data_size());
    assert!(
        f.helper
            .compare_texture2_images(texture.get_data().unwrap_or_default())
    );
}

#[test]
fn ktx_texture2_load_image_data_test_load_image_data_external() {
    let f = ktx2_fixture();
    let mut texture = f.create_any(0);
    let mut buf = vec![0u8; f.padded_image_data_size];
    assert_eq!(texture.load_image_data(Some(&mut buf)), Ok(()));
    assert_eq!(f.padded_image_data_size, texture.get_data_size());
    assert!(f.helper.compare_texture2_images(&buf));
}

/////////////////////////////////////////
// ktxTexture_GetRowPitch vs. ktxTexture_GetImageSize
/////////////////////////////////////////

#[test]
fn ktx_texture_get_row_pitch_get_row_pitch_vs_get_image_size() {
    let level = 0;
    {
        // KTXv1 texture (padding to KTX_GL_UNPACK_ALIGNMENT required)
        let ci = TextureCreateInfo {
            gl_internalformat: GL_SRGB8,
            base_width: 3,
            base_height: 3,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            ..Default::default()
        };
        let texture1 = ok(
            Texture1::create(&ci, TextureCreateStorage::NoStorage),
            "ktxTexture1_Create",
        );
        // 3 * 3 == 9 => should pad to 12 for KTXv1
        assert_eq!(texture1.get_row_pitch(level) % KTX_GL_UNPACK_ALIGNMENT, 0);
        let image_size_0 = texture1.get_row_pitch(level) as usize
            * (texture1.base_height >> level).max(1) as usize;
        let image_size_1 = texture1.calc_image_size(level, FormatVersion::One);
        assert_eq!(image_size_0, image_size_1);
    }
    {
        // KTXv2 texture (no padding required)
        let ci = TextureCreateInfo {
            vk_format: VkFormat::R8G8B8_SRGB,
            base_width: 3,
            base_height: 3,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            ..Default::default()
        };
        let texture2 = ok(
            Texture2::create(&ci, TextureCreateStorage::NoStorage),
            "ktxTexture2_Create",
        );
        let image_size_0 = texture2.get_row_pitch(level) as usize
            * (texture2.base_height >> level).max(1) as usize;
        let image_size_1 = texture2.calc_image_size(level, FormatVersion::Two);
        assert_eq!(image_size_0, image_size_1);
    }
}

/////////////////////////////////////////
// ktxTexture2_CreateCopyTest
/////////////////////////////////////////

#[test]
fn ktx_texture2_create_copy_test_create_copy() {
    let f = ktx2_fixture();
    let mut texture = ok(
        Texture2::create_from_memory(&f.ktx_mem_file, 0),
        "ktxTexture_CreateFromMemory",
    );
    let copy_texture = ok(
        Texture2::create_copy(&mut texture),
        "ktxTexture2_CreateCopy",
    );
    assert!(f.compare_texture2(&copy_texture));
    assert_eq!(texture.get_data(), copy_texture.get_data());
    assert_eq!(texture.format_size(), copy_texture.format_size());
    assert_eq!(texture.type_size(), copy_texture.type_size());
    assert_eq!(texture.level_index(), copy_texture.level_index());
    assert_eq!(
        texture.supercompression_global_data(),
        copy_texture.supercompression_global_data()
    );
    assert_eq!(
        texture.private().required_level_alignment,
        copy_texture.private().required_level_alignment
    );
    assert_eq!(
        texture.private().first_level_file_offset,
        copy_texture.private().first_level_file_offset
    );
}

/////////////////////////////////////////
// TestCreateInfo for size and offset tests.
/////////////////////////////////////////

#[allow(clippy::too_many_arguments)]
fn test_create_info_full(
    width: u32,
    height: u32,
    depth: u32,
    dimensions: u32,
    internalformat: u32,
    vkformat: VkFormat,
    array: bool,
    faces: u32,
    layers: u32,
) -> TextureCreateInfo {
    TextureCreateInfo {
        base_width: width,
        base_height: height,
        base_depth: depth,
        num_dimensions: dimensions,
        generate_mipmaps: false,
        gl_internalformat: internalformat,
        vk_format: vkformat,
        is_array: array,
        num_faces: faces,
        num_layers: layers,
        num_levels: WriterTestHelper::<u8, 4, GL_RGBA8>::levels_from_size(width, height, depth),
        dfd: None,
    }
}

fn test_create_info() -> TextureCreateInfo {
    test_create_info_full(
        16,
        16,
        1,
        2,
        GL_RGBA8,
        VkFormat::R8G8B8A8_UNORM,
        false,
        1,
        1,
    )
}

/////////////////////////////////////////
// ktxTexture_calcImageSize tests
/////////////////////////////////////////

fn check_sizes(ci: &TextureCreateInfo, ktx1sizes: &[usize], ktx2sizes: &[usize], level_size: bool) {
    let texture = ok(
        Texture1::create(ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    for i in 0..ci.num_levels {
        let (a, b) = if level_size {
            (
                texture.calc_level_size(i, FormatVersion::One),
                texture.calc_level_size(i, FormatVersion::Two),
            )
        } else {
            (
                texture.calc_image_size(i, FormatVersion::One),
                texture.calc_image_size(i, FormatVersion::Two),
            )
        };
        assert_eq!(a, ktx1sizes[i as usize]);
        assert_eq!(b, ktx2sizes[i as usize]);
    }
}

#[test]
fn ktx_texture_calc_image_size_image_size_at_each_level_rgba_2d() {
    check_sizes(
        &test_create_info(),
        &[1024, 256, 64, 16, 4],
        &[1024, 256, 64, 16, 4],
        false,
    );
}

#[test]
fn ktx_texture_calc_image_size_image_size_at_each_level_rgb_2d() {
    let ci = test_create_info_full(9, 9, 1, 2, GL_RGB8, VkFormat::R8G8B8_UNORM, false, 1, 1);
    check_sizes(
        &ci,
        &[28 * 9, 12 * 4, 8 * 2, 4],
        &[27 * 9, 12 * 4, 6 * 2, 3],
        false,
    );
}

/////////////////////////////////////////
// ktxTexture_calcLevelSize tests
/////////////////////////////////////////

#[test]
fn ktx_texture_calc_level_size_size_of_each_level_rgba_2d() {
    check_sizes(
        &test_create_info(),
        &[1024, 256, 64, 16, 4],
        &[1024, 256, 64, 16, 4],
        true,
    );
}

#[test]
fn ktx_texture_calc_level_size_size_of_each_level_rgb_2d() {
    let ci = test_create_info_full(9, 9, 1, 2, GL_RGB8, VkFormat::R8G8B8_UNORM, false, 1, 1);
    check_sizes(
        &ci,
        &[28 * 9, 12 * 4, 8 * 2, 4],
        &[27 * 9, 12 * 4, 6 * 2, 3],
        true,
    );
}

/////////////////////////////////////////
// ktxTexture_calcLevelOffset tests
/////////////////////////////////////////

fn check_offsets(ci: &TextureCreateInfo, ktx1offsets: Option<&[usize]>, ktx2offsets: &[usize]) {
    let t1 = ktx1offsets.map(|_| {
        ok(
            Texture1::create(ci, TextureCreateStorage::NoStorage),
            "ktxTexture1_Create",
        )
    });
    let t2 = ok(
        Texture2::create(ci, TextureCreateStorage::NoStorage),
        "ktxTexture2_Create",
    );
    for i in 0..ci.num_levels {
        if let (Some(t1), Some(o1)) = (&t1, ktx1offsets) {
            assert_eq!(t1.calc_level_offset(i), o1[i as usize]);
        }
        assert_eq!(t2.calc_level_offset(i), ktx2offsets[i as usize]);
    }
}

#[test]
fn ktx_texture_calc_level_offset_offset_of_each_level_rgba_2d() {
    check_offsets(
        &test_create_info(),
        Some(&[0, 1024, 1024 + 256, 1024 + 256 + 64, 1024 + 256 + 64 + 16]),
        &[4 + 16 + 64 + 256, 4 + 16 + 64, 4 + 16, 4, 0],
    );
}

#[test]
fn ktx_texture_calc_level_offset_offset_of_each_level_rgb_2d() {
    let ci = test_create_info_full(9, 9, 1, 2, GL_RGB8, VkFormat::R8G8B8_UNORM, false, 1, 1);
    check_offsets(
        &ci,
        Some(&[0, 28 * 9, 28 * 9 + 12 * 4, 28 * 9 + 12 * 4 + 8 * 2]),
        &[12 * 4 + 24, 6 * 2 + 12, 3 + 9, 0],
    );
}

#[test]
fn ktx_texture_calc_level_offset_offset_of_each_level_d16_unorm_s8_uint() {
    let ci = test_create_info_full(9, 9, 1, 2, 0, VkFormat::D16_UNORM_S8_UINT, false, 1, 1);
    check_offsets(&ci, None, &[4 + 16 + 64, 4 + 16, 4, 0]);
}

#[test]
fn ktx_texture_calc_level_offset_offset_of_each_level_d32_sfloat_s8_uint() {
    let ci = test_create_info_full(9, 9, 1, 2, 0, VkFormat::D32_SFLOAT_S8_UINT, false, 1, 1);
    check_offsets(&ci, None, &[8 + 32 + 128, 8 + 32, 8, 0]);
}

/////////////////////////////////////////
// ktxTexture_GetImageOffset tests
/////////////////////////////////////////

#[test]
fn ktx_texture_get_image_offset_test_invalid_op_on_level_face_layer_too_big() {
    let ci = test_create_info();
    let texture = ok(
        Texture1::create(&ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    assert_eq!(
        texture.get_image_offset(ci.num_levels, 0, 0),
        Err(ErrorCode::InvalidOperation)
    );
    assert_eq!(
        texture.get_image_offset(0, ci.num_layers, 0),
        Err(ErrorCode::InvalidOperation)
    );
    assert_eq!(
        texture.get_image_offset(0, 0, ci.num_faces),
        Err(ErrorCode::InvalidOperation)
    );
}

#[test]
fn ktx_texture_get_image_offset_test_image_offset_level() {
    let mut helper = WriterTestHelper::<u8, 4, GL_RGBA8>::new();
    helper.resize(E_MIPMAPPED, 1, 1, 2, 16, 16, 1);
    let texture = ok(
        Texture1::create(&helper.create_info, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    assert_eq!(texture.get_image_offset(0, 0, 0), Ok(0));
    // GL_RGBA8 is 1 x 4 bytes.
    let image_size = (helper.create_info.base_width * helper.create_info.base_height * 4) as usize;
    assert_eq!(texture.get_image_offset(1, 0, 0), Ok(image_size));
    let expected_offset = helper.image_data_size - 4;
    assert_eq!(
        texture.get_image_offset(helper.create_info.num_levels - 1, 0, 0),
        Ok(expected_offset)
    );
}

#[test]
fn ktx_texture_get_image_offset_test_image_offset_with_row_padding() {
    let mut ci = test_create_info();
    ci.gl_internalformat = GL_RGB8;
    ci.base_width = 9;
    let texture = ok(
        Texture1::create(&ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    let row_bytes = 9 * 3;
    let image_size = ((row_bytes + rounding(row_bytes)) * texture.base_height) as usize;
    assert_eq!(texture.get_image_offset(1, 0, 0), Ok(image_size));
    let mut expected_offset = 0usize;
    for i in 0..texture.num_levels - 1 {
        let lw = (texture.base_width >> i).max(1);
        let lh = (texture.base_height >> i).max(1);
        let mut lrb = lw * 3;
        lrb += rounding(lrb);
        expected_offset += (lrb * lh) as usize;
    }
    assert_eq!(
        texture.get_image_offset(ci.num_levels - 1, 0, 0),
        Ok(expected_offset)
    );
}

fn level1_image_size(texture: &Texture1) -> usize {
    let lw = (texture.base_width >> 1).max(1);
    let lh = (texture.base_height >> 1).max(1);
    let mut lrb = lw * 3;
    lrb += rounding(lrb);
    (lrb * lh) as usize
}

#[test]
fn ktx_texture_get_image_offset_test_image_offset_array() {
    let mut ci = test_create_info();
    ci.gl_internalformat = GL_RGB8;
    ci.base_width = 9;
    ci.num_layers = 3;
    let texture = ok(
        Texture1::create(&ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    let row_bytes = 9 * 3;
    let image_size = ((row_bytes + rounding(row_bytes)) * ci.base_height) as usize;
    let layer_size = image_size * texture.num_faces as usize;
    let mut expected_offset = layer_size * texture.num_layers as usize;
    assert_eq!(texture.get_image_offset(1, 0, 0), Ok(expected_offset));
    expected_offset += level1_image_size(&texture) * 2;
    assert_eq!(texture.get_image_offset(1, 2, 0), Ok(expected_offset));
}

#[test]
fn ktx_texture_get_image_offset_test_image_offset_face() {
    let mut ci = test_create_info();
    ci.gl_internalformat = GL_RGB8;
    ci.base_width = 9;
    ci.base_height = 9;
    ci.num_levels = 4;
    ci.num_layers = 1;
    ci.num_faces = 6;
    let texture = ok(
        Texture1::create(&ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    let row_bytes = 9 * 3;
    let image_size = ((row_bytes + rounding(row_bytes)) * texture.base_height) as usize;
    let layer_size = image_size * texture.num_faces as usize;
    assert_eq!(texture.get_image_offset(0, 0, 4), Ok(image_size * 4));
    let expected_offset = layer_size + level1_image_size(&texture) * 3;
    assert_eq!(texture.get_image_offset(1, 0, 3), Ok(expected_offset));
}

#[test]
fn ktx_texture_get_image_offset_test_image_offset_array_face() {
    let mut ci = test_create_info();
    ci.gl_internalformat = GL_RGB8;
    ci.base_width = 9;
    ci.base_height = 9;
    ci.num_levels = 4;
    ci.num_layers = 3;
    ci.num_faces = 6;
    let texture = ok(
        Texture1::create(&ci, TextureCreateStorage::NoStorage),
        "ktxTexture1_Create",
    );
    let row_bytes = 9 * 3;
    let image_size = ((row_bytes + rounding(row_bytes)) * ci.base_height) as usize;
    let layer_size = image_size * texture.num_faces as usize;
    let mut expected_offset = layer_size * ci.num_layers as usize;
    assert_eq!(texture.get_image_offset(1, 0, 0), Ok(expected_offset));
    let level_image_size = level1_image_size(&texture);
    expected_offset += level_image_size * texture.num_faces as usize * 2;
    assert_eq!(texture.get_image_offset(1, 2, 0), Ok(expected_offset));
    expected_offset += level_image_size * 3; // 3 faces
    assert_eq!(texture.get_image_offset(1, 2, 3), Ok(expected_offset));
}

/////////////////////////////////////////
// ktxTexture_Write tests
/////////////////////////////////////////

/// Port of `ktxTexture1WriteTestBase::runTest`.
fn ktx1_write_run_test<C: common::wthelper::Component, const NC: u32, const IF: u32>(
    helper: &WriterTestHelper<C, NC, IF>,
    write_metadata: bool,
) {
    let mut texture = ok(
        Texture1::create(&helper.create_info, TextureCreateStorage::AllocStorage),
        "ktxTexture1_Create",
    );
    if write_metadata {
        let mut o = helper.orientation.as_bytes().to_vec();
        o.push(0);
        assert_eq!(
            texture.kv_data_head.add_kv_pair(KTX_ORIENTATION_KEY, &o),
            Ok(())
        );
    }
    assert_eq!(helper.copy_images_to_texture1(&mut texture), Ok(()));
    assert!(helper.compare_texture1_images(texture.get_data().unwrap_or_default()));
    let mem = ok(texture.write_to_memory(), "ktxTexture_WriteToMemory");
    assert_eq!(mem.get(..12), Some(&KTX_IDENTIFIER_REF[..]));
    let mut hb = [0u8; 64];
    hb.copy_from_slice(&mem[..64]);
    assert!(helper.texinfo.compare_header1(&KtxHeader::from_bytes(&hb)));
    // Check the metadata.
    let mut file_ptr = 64usize;
    if write_metadata {
        assert_eq!(
            mem.get(file_ptr..file_ptr + helper.kv_data_len as usize),
            Some(&helper.kv_data[..])
        );
        file_ptr += helper.kv_data_len as usize;
    }
    // Check data pointer is properly aligned.
    assert_eq!(file_ptr & 0x3, 0);
    assert!(helper.compare_raw_images(&mem[file_ptr..]));
}

macro_rules! ktx1_write_test {
    ($name:ident, $c:ty, $nc:expr, $if:expr, $flags:expr, $layers:expr, $faces:expr, $dims:expr, $w:expr, $h:expr, $d:expr, $md:expr) => {
        #[test]
        fn $name() {
            let mut helper = WriterTestHelper::<$c, $nc, { $if }>::new();
            helper.resize($flags, $layers, $faces, $dims, $w, $h, $d);
            ktx1_write_run_test(&helper, $md);
        }
    };
}

ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_1d,
    u8,
    3,
    GL_RGB8,
    E_NONE,
    1,
    1,
    1,
    32,
    1,
    1,
    false
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_1d_needs_padding,
    u8,
    3,
    GL_RGB8,
    E_NONE,
    1,
    1,
    1,
    9,
    1,
    1,
    false
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_1d_mipmap,
    u8,
    4,
    GL_RGBA8,
    E_MIPMAPPED,
    1,
    1,
    1,
    32,
    1,
    1,
    false
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_1d_array,
    u8,
    3,
    GL_RGB8,
    E_ARRAY,
    4,
    1,
    1,
    32,
    1,
    1,
    false
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_1d_array_mipmap,
    u8,
    4,
    GL_RGBA8,
    E_MIPMAPPED | E_ARRAY,
    4,
    1,
    1,
    32,
    1,
    1,
    false
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_2d,
    u8,
    3,
    GL_RGB8,
    E_NONE,
    1,
    1,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_2d_mipmap,
    u8,
    3,
    GL_RGB8,
    E_MIPMAPPED,
    1,
    1,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_2d_array,
    u8,
    4,
    GL_RGBA8,
    E_ARRAY,
    4,
    1,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_2d_array_mipmap,
    u8,
    4,
    GL_RGBA8,
    E_ARRAY | E_MIPMAPPED,
    4,
    1,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_3d,
    u8,
    3,
    GL_RGB8,
    E_NONE,
    1,
    1,
    3,
    32,
    32,
    32,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_3d_mipmap,
    u8,
    3,
    GL_RGB8,
    E_MIPMAPPED,
    1,
    1,
    3,
    8,
    8,
    2,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgb8_write_cubemap,
    u8,
    3,
    GL_RGB8,
    E_NONE,
    1,
    6,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_cubemap_mipmap,
    u8,
    4,
    GL_RGBA8,
    E_MIPMAPPED,
    1,
    6,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rgba8_write_cubemap_array_mipmap,
    u8,
    4,
    GL_RGBA8,
    E_MIPMAPPED | E_ARRAY,
    4,
    6,
    2,
    32,
    32,
    1,
    true
);
ktx1_write_test!(
    ktx_texture1_write_test_rg16_write_2d_mipmap,
    u16,
    2,
    GL_RG16,
    E_MIPMAPPED,
    1,
    1,
    2,
    32,
    32,
    1,
    true
);
