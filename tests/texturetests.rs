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
