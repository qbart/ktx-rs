//! Port of tests/texturetests/texturetests.cc (sections are added as the library grows).
#![allow(clippy::panic)]

mod common;

use ktx::{
    ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KTX_TEXTURE_CREATE_NO_FLAGS, KtxTexture, SupercmpScheme,
    Texture2, ktx_error_string,
};

////////////////////////////////////////////
// Unicode file name tests
///////////////////////////////////////////

#[test]
fn unicode_file_names_create_from() {
    // TODO(texture1): add the KTX 1 file set once ktxTexture1 is ported.
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
    for name in ktx2_file_set {
        let path = common::unicode_path().join(name);
        let result = Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS);
        if let Err(e) = &result {
            panic!("ktxTexture2_CreateFromNamedFile \"{}\" failed: {}", path.display(), ktx_error_string(*e));
        }
    }
}

///////////////////////////////////////////////////////////
// Additional read coverage of every KTX2 test resource.
///////////////////////////////////////////////////////////

#[test]
fn ktx2_resources_load() {
    let mut count = 0;
    let mut entries: Vec<_> = match std::fs::read_dir(common::ktx2_path()) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => panic!("cannot read resources: {e}"),
    };
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !name.ends_with(".ktx2") || name.starts_with("invalid_") {
            continue;
        }
        let tex = match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => t,
            Err(e) => panic!("{name}: {}", ktx_error_string(e)),
        };
        let data = tex.get_data().unwrap_or_default();
        assert_eq!(data.len(), tex.get_data_size(), "{name}");
        // Zstd/ZLIB textures are inflated on load.
        assert!(
            tex.supercompression_scheme != SupercmpScheme::ZSTD
                && tex.supercompression_scheme != SupercmpScheme::ZLIB,
            "{name}"
        );
        if tex.supercompression_scheme == SupercmpScheme::NONE {
            assert_eq!(tex.get_data_size(), tex.calc_data_size_texture(), "{name}");
        }
        count += 1;
    }
    assert!(count > 20);
}

#[test]
fn ktx2_memory_and_deferred_load_agree() {
    let path = common::ktx2_path().join("color_grid_zstd_5.ktx2");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => panic!("{e}"),
    };
    let loaded = match Texture2::create_from_memory(&bytes, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
        Ok(t) => t,
        Err(e) => panic!("{}", ktx_error_string(e)),
    };
    let mut deferred = match Texture2::create_from_memory(&bytes, KTX_TEXTURE_CREATE_NO_FLAGS) {
        Ok(t) => t,
        Err(e) => panic!("{}", ktx_error_string(e)),
    };
    assert_eq!(deferred.supercompression_scheme, SupercmpScheme::ZSTD);
    assert!(deferred.is_active_stream());
    assert_eq!(deferred.load_image_data(None), Ok(()));
    assert!(!deferred.is_active_stream());
    assert_eq!(deferred.get_data(), loaded.get_data());
    assert_eq!(deferred.load_image_data(None), Err(ErrorCode::InvalidOperation));
}

/////////////////////////////////////////
// ktxTexture2_GetDataSizeUncompressed
/////////////////////////////////////////

#[test]
fn ktx_texture2_get_data_size_uncompressed_compare_with_get_data_size_after_inflate() {
    let path = common::ktx2_path().join("invalid_zstd_level0_padding.ktx2");
    let mut texture = match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_NO_FLAGS) {
        Ok(t) => t,
        Err(e) => panic!("ktxTexture2_CreateFromNamedFile failed: {}", ktx_error_string(e)),
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
    let mut texture = match Texture2::create(&r8_1x1_create_info(), ktx::TextureCreateStorage::AllocStorage) {
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
    let texture = match Texture2::create_from_memory(&ktx_mem_file, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
        Ok(t) => t,
        Err(e) => panic!("ktxTexture2_CreateFromMemory failed: {}", ktx_error_string(e)),
    };
    let data = match texture.get_data() {
        Some(d) => d,
        None => panic!("Image data not loaded"),
    };
    assert_eq!(data[0], pixel);

    let mut texture = match Texture2::create_from_memory(&ktx_mem_file, KTX_TEXTURE_CREATE_NO_FLAGS) {
        Ok(t) => t,
        Err(e) => panic!("ktxTexture2_CreateFromMemory failed: {}", ktx_error_string(e)),
    };
    let expected_scheme = if zlib { SupercmpScheme::ZLIB } else { SupercmpScheme::ZSTD };
    assert_eq!(texture.supercompression_scheme, expected_scheme);

    let mut data = SupercompressionIterCbData::default();
    let result = texture.iterate_load_level_faces(&mut |miplevel, face, width, height, depth, face_lod_size, pixels| {
        data.calls += 1;
        data.miplevel = miplevel;
        data.face = face;
        data.width = width;
        data.height = height;
        data.depth = depth;
        data.face_lod_size = face_lod_size;
        data.pixel = pixels.first().copied().unwrap_or_default();
        Ok(())
    });
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

///////////////////////////////////////////////////////////
// Additional writer coverage: load -> write -> reload round trip.
///////////////////////////////////////////////////////////

#[test]
fn ktx2_resources_write_round_trip() {
    let mut entries: Vec<_> = match std::fs::read_dir(common::ktx2_path()) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => panic!("cannot read resources: {e}"),
    };
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !name.ends_with(".ktx2") || name.starts_with("invalid_") {
            continue;
        }
        let mut tex = match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => t,
            Err(e) => panic!("{name}: {}", ktx_error_string(e)),
        };
        let bytes = match tex.write_to_memory() {
            Ok(b) => b,
            Err(e) => panic!("{name}: write {}", ktx_error_string(e)),
        };
        let back = match Texture2::create_from_memory(&bytes, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
            Ok(t) => t,
            Err(e) => panic!("{name}: reload {}", ktx_error_string(e)),
        };
        assert_eq!(back.get_data(), tex.get_data(), "{name}");
        assert_eq!(back.vk_format, tex.vk_format, "{name}");
        assert_eq!(back.p_dfd, tex.p_dfd, "{name}");
        assert_eq!(back.supercompression_scheme, tex.supercompression_scheme, "{name}");
        assert_eq!(back.supercompression_global_data(), tex.supercompression_global_data(), "{name}");
        assert_eq!(back.kv_data_head, tex.kv_data_head, "{name}");
        assert_eq!(back.level_index(), tex.level_index(), "{name}");
        assert_eq!(
            (back.base_width, back.base_height, back.base_depth, back.num_levels, back.num_layers, back.num_faces),
            (tex.base_width, tex.base_height, tex.base_depth, tex.num_levels, tex.num_layers, tex.num_faces),
            "{name}"
        );
    }
}
