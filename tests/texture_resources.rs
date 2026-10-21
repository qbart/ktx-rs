//! Additional (non-C) coverage over the KTX-Software test resources.
#![allow(clippy::panic)]

mod common;

use ktx::{
    ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KTX_TEXTURE_CREATE_NO_FLAGS, KtxTexture,
    SupercmpScheme, Texture2, ktx_error_string,
};

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
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".ktx2") || name.starts_with("invalid_") {
            continue;
        }
        let tex =
            match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
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
    let loaded = match Texture2::create_from_memory(&bytes, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
    {
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
    assert_eq!(
        deferred.load_image_data(None),
        Err(ErrorCode::InvalidOperation)
    );
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
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".ktx2") || name.starts_with("invalid_") {
            continue;
        }
        let mut tex =
            match Texture2::create_from_named_file(&path, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
                Ok(t) => t,
                Err(e) => panic!("{name}: {}", ktx_error_string(e)),
            };
        let bytes = match tex.write_to_memory() {
            Ok(b) => b,
            Err(e) => panic!("{name}: write {}", ktx_error_string(e)),
        };
        let back =
            match Texture2::create_from_memory(&bytes, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT) {
                Ok(t) => t,
                Err(e) => panic!("{name}: reload {}", ktx_error_string(e)),
            };
        assert_eq!(back.get_data(), tex.get_data(), "{name}");
        assert_eq!(back.vk_format, tex.vk_format, "{name}");
        assert_eq!(back.p_dfd, tex.p_dfd, "{name}");
        assert_eq!(
            back.supercompression_scheme, tex.supercompression_scheme,
            "{name}"
        );
        assert_eq!(
            back.supercompression_global_data(),
            tex.supercompression_global_data(),
            "{name}"
        );
        assert_eq!(back.kv_data_head, tex.kv_data_head, "{name}");
        assert_eq!(back.level_index(), tex.level_index(), "{name}");
        assert_eq!(
            (
                back.base_width,
                back.base_height,
                back.base_depth,
                back.num_levels,
                back.num_layers,
                back.num_faces
            ),
            (
                tex.base_width,
                tex.base_height,
                tex.base_depth,
                tex.num_levels,
                tex.num_layers,
                tex.num_faces
            ),
            "{name}"
        );
    }
}

///////////////////////////////////////////////////////////
// Additional KTX1 coverage: load, rewrite as KTX1 and convert to KTX2.
///////////////////////////////////////////////////////////

#[test]
fn ktx1_resources_round_trip_and_convert() {
    let mut entries: Vec<_> = match std::fs::read_dir(common::resources_path().join("ktx")) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => panic!("cannot read resources: {e}"),
    };
    entries.sort();
    let mut converted = 0;
    for path in entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".ktx") {
            continue;
        }
        let mut tex = match ktx::Texture1::create_from_named_file(
            &path,
            KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
        ) {
            Ok(t) => t,
            // Legacy (e.g. luminance) GL formats are not supported by libktx.
            Err(ErrorCode::UnsupportedTextureType) => continue,
            Err(e) => panic!("{name}: {}", ktx_error_string(e)),
        };
        assert_eq!(
            tex.get_data().map(|d| d.len()),
            Some(tex.data_size),
            "{name}"
        );
        assert_eq!(tex.data_size, tex.calc_data_size_texture(), "{name}");
        let bytes = match tex.write_to_memory() {
            Ok(b) => b,
            Err(e) => panic!("{name}: write {}", ktx_error_string(e)),
        };
        let back =
            match ktx::Texture1::create_from_memory(&bytes, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
            {
                Ok(t) => t,
                Err(e) => panic!("{name}: reload {}", ktx_error_string(e)),
            };
        assert_eq!(back.get_data(), tex.get_data(), "{name}");
        assert_eq!(back.kv_data_head, tex.kv_data_head, "{name}");
        assert_eq!(back.gl_internalformat, tex.gl_internalformat, "{name}");

        match tex.write_ktx2_to_memory() {
            Ok(k2) => {
                let t2 =
                    match Texture2::create_from_memory(&k2, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
                    {
                        Ok(t) => t,
                        Err(e) => panic!("{name}: converted KTX2 reload {}", ktx_error_string(e)),
                    };
                assert_eq!(t2.base_width, tex.base_width, "{name}");
                assert_eq!(t2.num_levels, tex.num_levels, "{name}");
                // Unpadded size equals KTX1 size whenever rows need no padding.
                {
                    let d1 = tex.get_data().unwrap_or_default();
                    let d2 = t2.get_data().unwrap_or_default();
                    for level in 0..tex.num_levels {
                        let (rows, row_bytes, pad) = if tex.is_compressed {
                            (0, 0, 0)
                        } else {
                            tex.row_info(level)
                        };
                        if pad != 0 {
                            // Compare row by row, dropping KTX1 row padding.
                            let packed = (row_bytes - pad) as usize;
                            let images =
                                (tex.get_level_size(level) / tex.get_image_size(level)) as usize;
                            let mut o1 = tex.calc_level_offset(level);
                            let mut o2 = t2.level_data_offset(level) as usize;
                            for _ in 0..images * rows as usize {
                                assert!(
                                    d1.get(o1..o1 + packed) == d2.get(o2..o2 + packed),
                                    "{name} level {level}"
                                );
                                o1 += row_bytes as usize;
                                o2 += packed;
                            }
                            continue;
                        }
                        let size = tex.get_level_size(level);
                        let o1 = tex.calc_level_offset(level);
                        let o2 = t2.level_data_offset(level) as usize;
                        assert!(
                            d1.get(o1..o1 + size) == d2.get(o2..o2 + size),
                            "{name} level {level}"
                        );
                    }
                }
                converted += 1;
            }
            Err(ErrorCode::UnsupportedTextureType) => {}
            Err(e) => panic!("{name}: convert {}", ktx_error_string(e)),
        }
    }
    assert!(converted > 10);
}
