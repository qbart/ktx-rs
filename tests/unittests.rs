//! Port of tests/unittests/unittests.cc (sections are added as the library grows).
#![allow(clippy::panic)]

mod common;

use ktx::strings::strnstr;

// sizeof(haystack) in the C test is the size of a pointer (8).
const HAYSTACK: &[u8] = b"abcdefabc";
const PTR_SIZE: usize = 8;

#[test]
fn strnstr_zero_length_needle() {
    assert_eq!(strnstr(HAYSTACK, b"", PTR_SIZE), Some(0));
}

#[test]
fn strnstr_match_at_start() {
    assert_eq!(strnstr(HAYSTACK, b"abc", PTR_SIZE), Some(0));
}

#[test]
fn strnstr_match_at_end() {
    assert_eq!(strnstr(HAYSTACK, b"def", PTR_SIZE), Some(3));
}

#[test]
fn strnstr_match_in_middle() {
    assert_eq!(strnstr(HAYSTACK, b"fa", PTR_SIZE), Some(5));
}

#[test]
fn strnstr_no_match_before_given_length() {
    assert_eq!(strnstr(HAYSTACK, b"cde", 2), None);
}

#[test]
fn strnstr_needle_matches_haystack_but_longer() {
    assert_eq!(strnstr(HAYSTACK, b"abcdefg", PTR_SIZE), None);
}

#[test]
fn strnstr_no_match() {
    assert_eq!(strnstr(HAYSTACK, b"foo", PTR_SIZE), None);
}

//////////////////////////////
// HashListTest
//////////////////////////////

mod hash_list_test {
    use ktx::{HashList, KTX_ORIENTATION_KEY, KTX_WRITER_KEY};

    const WRITER_VAL: &str = "HashListTest";
    const ORIENTATION_VAL: &str = "ruo";

    fn c_str(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    fn construct_list(sort: bool) -> HashList {
        let mut head = HashList::new();
        assert_eq!(head.add_kv_pair(KTX_WRITER_KEY, &c_str(WRITER_VAL)), Ok(()));
        assert_eq!(
            head.add_kv_pair(KTX_ORIENTATION_KEY, &c_str(ORIENTATION_VAL)),
            Ok(())
        );
        if sort {
            assert_eq!(head.sort(), Ok(()));
        }
        head
    }

    fn compare_list(list: &HashList, is_sorted: bool) {
        let mut entry_count = 0u32;
        for entry in list {
            entry_count += 1;
            let (_, key) = entry.get_key();
            let (_, value) = entry.get_value();
            if is_sorted {
                match entry_count {
                    1 => assert_eq!(key, KTX_ORIENTATION_KEY.as_bytes()),
                    2 => assert_eq!(key, KTX_WRITER_KEY.as_bytes()),
                    _ => {}
                }
            }
            if key == KTX_ORIENTATION_KEY.as_bytes() {
                assert_eq!(value, c_str(ORIENTATION_VAL).as_slice());
            } else if key == KTX_WRITER_KEY.as_bytes() {
                assert_eq!(value, c_str(WRITER_VAL).as_slice());
            } else {
                panic!("unexpected key");
            }
        }
        assert_eq!(entry_count, 2);
    }

    #[test]
    fn construct_sorted() {
        let head = construct_list(true);
        compare_list(&head, true);
    }

    #[test]
    fn construct_copy() {
        let head = construct_list(true);
        let copy_head = HashList::construct_copy(&head);
        compare_list(&copy_head, true);
    }

    // Additional coverage of serialize/deserialize round-trip semantics.
    #[test]
    fn serialize_deserialize_round_trip() {
        let head = construct_list(true);
        let kvd = head.serialize();
        assert!(kvd.is_ok());
        let kvd = kvd.unwrap_or_default();
        assert_eq!(kvd.len() % 4, 0);
        let mut copy = HashList::new();
        assert_eq!(copy.deserialize(&kvd), Ok(()));
        compare_list(&copy, true);
    }
}

//////////////////////////////
// MemStreamTest
//////////////////////////////

mod mem_stream_test {
    use ktx::stream::{MemStream, Stream};

    #[test]
    fn read() {
        let data: &[u8] = b"28 bytes of rubbish to read.";
        let size = 28;
        let mut read_buf = vec![0u8; size];
        let mut stream = match MemStream::construct_ro(data) {
            Ok(s) => s,
            Err(e) => panic!("construct_ro failed: {e}"),
        };
        assert_eq!(stream.read(&mut read_buf), Ok(()));
        assert_eq!(&data[..size], read_buf.as_slice());
    }

    #[test]
    fn write() {
        let data: &[u8] = b"29 bytes of rubbish to write.";
        let count = 29;
        let mut stream = match MemStream::construct() {
            Ok(s) => s,
            Err(e) => panic!("construct failed: {e}"),
        };
        assert_eq!(stream.write(data, 1, count), Ok(()));
        let returned_count = stream.getsize();
        assert_eq!(returned_count, Ok(count as u64));
        assert_eq!(&stream.getdata()[..count], &data[..count]);
    }

    #[test]
    fn write_expand() {
        let data: &[u8] = b"29 bytes of rubbish to write.";
        let data2: &[u8] = b" 26 more bytes of rubbish.";
        let count = 29;
        let count2 = 26;
        let mut stream = match MemStream::construct() {
            Ok(s) => s,
            Err(e) => panic!("construct failed: {e}"),
        };
        assert_eq!(stream.write(data, 1, count), Ok(()));
        assert_eq!(stream.write(data2, 1, count2), Ok(()));
        assert_eq!(stream.getsize(), Ok((count + count2) as u64));
        let returned = stream.getdata();
        assert_eq!(&returned[..count], data);
        assert_eq!(&returned[count..count + count2], data2);
    }

    // Not in the C suite: writing beyond the default allocation forces expansion.
    #[test]
    fn write_expand_beyond_default_allocation() {
        let mut stream = match MemStream::construct() {
            Ok(s) => s,
            Err(e) => panic!("construct failed: {e}"),
        };
        let big = vec![7u8; 1000];
        assert_eq!(stream.write(&big, 1, big.len()), Ok(()));
        assert_eq!(stream.getdata(), big.as_slice());
        assert_eq!(
            stream.read(&mut [0u8; 1]),
            Err(ktx::ErrorCode::FileUnexpectedEof)
        );
    }
}

mod file_stream_test {
    use ktx::stream::{FileStream, Stream};

    #[test]
    fn write_then_read() {
        let dir = std::env::temp_dir().join(format!("ktx-rs-fs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("stream.bin");
        {
            let mut s = match FileStream::create(&path) {
                Ok(s) => s,
                Err(e) => panic!("create failed: {e}"),
            };
            assert_eq!(s.write(b"abcdefgh", 4, 2), Ok(()));
            assert_eq!(s.getsize(), Ok(8));
        }
        let mut s = match FileStream::open(&path) {
            Ok(s) => s,
            Err(e) => panic!("open failed: {e}"),
        };
        let mut buf = [0u8; 3];
        assert_eq!(s.skip(2), Ok(()));
        assert_eq!(s.read(&mut buf), Ok(()));
        assert_eq!(&buf, b"cde");
        assert_eq!(s.getpos(), Ok(5));
        assert_eq!(s.setpos(9), Err(ktx::ErrorCode::InvalidOperation));
        assert_eq!(s.skip(10), Err(ktx::ErrorCode::FileUnexpectedEof));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//////////////////////////////
// CheckHeaderTest
//////////////////////////////

mod check_header1_test {
    use ktx::ErrorCode;
    use ktx::ktxint::{KtxHeader, ktx_check_header1};

    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_RGBA: u32 = 0x1908;
    const GL_RGBA8: u32 = 0x8058;

    const KTX_ID: [u8; 12] = [
        0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
    ];

    fn test_header() -> KtxHeader {
        KtxHeader {
            identifier: KTX_ID,
            endianness: 0x04030201,
            gl_type: GL_UNSIGNED_BYTE,
            gl_type_size: 1,
            gl_format: GL_RGBA,
            gl_internalformat: GL_RGBA8,
            gl_base_internalformat: GL_RGBA8,
            pixel_width: 16,
            pixel_height: 16,
            pixel_depth: 16,
            number_of_array_elements: 0,
            number_of_faces: 1,
            number_of_mip_levels: 5,
            bytes_of_key_value_data: 0,
        }
    }

    #[test]
    fn validates_identifier() {
        let mut test_header = test_header();
        assert!(ktx_check_header1(&mut test_header).is_ok());

        test_header.identifier[9] = 0;
        assert_eq!(
            ktx_check_header1(&mut test_header),
            Err(ErrorCode::UnknownFileFormat)
        );
    }

    #[test]
    fn disallows_invalid_endianness() {
        let mut test_header = test_header();
        test_header.endianness = 0;
        assert_eq!(
            ktx_check_header1(&mut test_header),
            Err(ErrorCode::FileDataError)
        );
    }

    // Not in the C suite: opposite endianness is swapped and accepted.
    #[test]
    fn swaps_reversed_endianness() {
        let orig = test_header();
        let mut h = orig;
        h.endianness = 0x01020304;
        h.gl_type = h.gl_type.swap_bytes();
        h.gl_type_size = h.gl_type_size.swap_bytes();
        h.gl_format = h.gl_format.swap_bytes();
        h.gl_internalformat = h.gl_internalformat.swap_bytes();
        h.gl_base_internalformat = h.gl_base_internalformat.swap_bytes();
        h.pixel_width = h.pixel_width.swap_bytes();
        h.pixel_height = h.pixel_height.swap_bytes();
        h.pixel_depth = h.pixel_depth.swap_bytes();
        h.number_of_faces = h.number_of_faces.swap_bytes();
        h.number_of_mip_levels = h.number_of_mip_levels.swap_bytes();
        assert!(ktx_check_header1(&mut h).is_ok());
        assert_eq!(h.pixel_width, 16);
        assert_eq!(h.number_of_mip_levels, 5);
    }

    #[test]
    fn header_bytes_round_trip() {
        let h = test_header();
        assert_eq!(KtxHeader::from_bytes(&h.to_bytes()), h);
    }
}

mod check_header2_test {
    use ktx::ktxint::{KtxHeader2, ktx_check_header2};
    use ktx::{ErrorCode, KTX2_IDENTIFIER_REF, SupercmpScheme, VkFormat};

    fn header() -> KtxHeader2 {
        KtxHeader2 {
            identifier: KTX2_IDENTIFIER_REF,
            vk_format: VkFormat::R8G8B8A8_UNORM,
            type_size: 1,
            pixel_width: 16,
            pixel_height: 16,
            pixel_depth: 0,
            layer_count: 0,
            face_count: 1,
            level_count: 5,
            ..Default::default()
        }
    }

    #[test]
    fn accepts_valid() {
        let mut h = header();
        let supp = ktx_check_header2(&mut h);
        assert_eq!(supp.map(|s| s.texture_dimension), Ok(2));
        assert_eq!(KtxHeader2::from_bytes(&h.to_bytes()), h);
    }

    #[test]
    fn rejects_bad_values() {
        let mut h = header();
        h.vk_format = VkFormat::R8_USCALED;
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::FileDataError));

        let mut h = header();
        h.supercompression_scheme = SupercmpScheme::BASIS_LZ;
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::FileDataError));

        let mut h = header();
        h.face_count = 6;
        h.pixel_height = 8;
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::FileDataError));

        let mut h = header();
        h.level_count = 6;
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::FileDataError));

        let mut h = header();
        h.supercompression_scheme = SupercmpScheme(0x10000);
        assert_eq!(
            ktx_check_header2(&mut h),
            Err(ErrorCode::UnsupportedFeature)
        );

        let mut h = header();
        h.pixel_depth = 4;
        h.layer_count = 2;
        assert_eq!(
            ktx_check_header2(&mut h),
            Err(ErrorCode::UnsupportedFeature)
        );
    }

    #[test]
    fn zero_levels_means_generate_mipmaps() {
        let mut h = header();
        h.level_count = 0;
        let supp = ktx_check_header2(&mut h);
        assert_eq!(supp.map(|s| s.generate_mipmaps), Ok(1));
        assert_eq!(h.level_count, 1);
    }
}

mod swap_test {
    use ktx::swap::*;

    #[test]
    fn swaps() {
        let mut a = [0x0102u16];
        ktx_swap_endian16(&mut a);
        assert_eq!(a, [0x0201]);
        let mut b = [0x01020304u32];
        ktx_swap_endian32(&mut b);
        assert_eq!(b, [0x04030201]);
        let mut c = [0x0102030405060708u64];
        ktx_swap_endian64(&mut c);
        assert_eq!(c, [0x0807060504030201]);
        let mut d = [1u8, 2, 3, 4, 5];
        ktx_swap_endian_bytes(&mut d, 2);
        assert_eq!(d, [2, 1, 4, 3, 5]);
    }
}

//////////////////////////////
// createDFD tests.
//////////////////////////////

mod create_dfd_test {
    use ktx::dfd::khr_df::*;
    use ktx::dfd::*;

    /// Mirrors `sampleType` of the C fixture.
    struct S(u32, u32, u32, u32, u32, u32, u32, u32, u32);

    /// Builds the expected DFD words (including the leading size word).
    #[allow(clippy::too_many_arguments)]
    fn expected(
        bytes_plane: u32,
        model: u32,
        primaries: u32,
        transfer: u32,
        flags: u32,
        dims: [u32; 4],
        samples: &[S],
    ) -> Vec<u32> {
        let block_size = 4 * (6 + 4 * samples.len() as u32);
        let mut v = vec![block_size + 4];
        v.push(KHR_DF_VENDORID_KHRONOS | (KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT << 17));
        v.push(KHR_DF_VERSIONNUMBER_1_3 | (block_size << 16));
        v.push(model | (primaries << 8) | (transfer << 16) | (flags << 24));
        v.push(dims[0] | (dims[1] << 8) | (dims[2] << 16) | (dims[3] << 24));
        v.push(bytes_plane);
        v.push(0);
        for s in samples {
            v.push(s.0 | (s.1 << 16) | (s.2 << 24));
            v.push(s.3 | (s.4 << 8) | (s.5 << 16) | (s.6 << 24));
            v.push(s.7);
            v.push(s.8);
        }
        v
    }

    fn df_channel_type(id: u32, q: u32) -> u32 {
        id | q
    }

    fn check(dfd: Option<Vec<u32>>, exp: Vec<u32>) {
        let dfd = match dfd {
            Some(d) => d,
            None => panic!("createDFD returned None"),
        };
        assert_eq!(dfd[0], exp[0]);
        assert_eq!(dfd, exp);
    }

    #[test]
    fn unpacked_test4_format_srgba8() {
        let exp = expected(
            4,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_SRGB,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [0; 4],
            &[
                S(0, 7, KHR_DF_CHANNEL_RGBSDA_RED, 0, 0, 0, 0, 0, 255),
                S(8, 7, KHR_DF_CHANNEL_RGBSDA_GREEN, 0, 0, 0, 0, 0, 255),
                S(16, 7, KHR_DF_CHANNEL_RGBSDA_BLUE, 0, 0, 0, 0, 0, 255),
                S(
                    24,
                    7,
                    df_channel_type(KHR_DF_CHANNEL_RGBSDA_ALPHA, KHR_DF_SAMPLE_DATATYPE_LINEAR),
                    0,
                    0,
                    0,
                    0,
                    0,
                    255,
                ),
            ],
        );
        check(create_dfd_unpacked(false, 4, 1, false, VkSuffix::Srgb), exp);
    }

    #[test]
    fn unpacked_test4_format_sbgra8() {
        let exp = expected(
            4,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_SRGB,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [0; 4],
            &[
                S(0, 7, KHR_DF_CHANNEL_RGBSDA_BLUE, 0, 0, 0, 0, 0, 255),
                S(8, 7, KHR_DF_CHANNEL_RGBSDA_GREEN, 0, 0, 0, 0, 0, 255),
                S(16, 7, KHR_DF_CHANNEL_RGBSDA_RED, 0, 0, 0, 0, 0, 255),
                S(
                    24,
                    7,
                    df_channel_type(KHR_DF_CHANNEL_RGBSDA_ALPHA, KHR_DF_SAMPLE_DATATYPE_LINEAR),
                    0,
                    0,
                    0,
                    0,
                    0,
                    255,
                ),
            ],
        );
        check(create_dfd_unpacked(false, 4, 1, true, VkSuffix::Srgb), exp);
    }

    #[test]
    fn unpacked_test4_format_rgba8() {
        let exp = expected(
            4,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_LINEAR,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [0; 4],
            &[
                S(0, 7, KHR_DF_CHANNEL_RGBSDA_RED, 0, 0, 0, 0, 0, 255),
                S(8, 7, KHR_DF_CHANNEL_RGBSDA_GREEN, 0, 0, 0, 0, 0, 255),
                S(16, 7, KHR_DF_CHANNEL_RGBSDA_BLUE, 0, 0, 0, 0, 0, 255),
                S(24, 7, KHR_DF_CHANNEL_RGBSDA_ALPHA, 0, 0, 0, 0, 0, 255),
            ],
        );
        check(
            create_dfd_unpacked(false, 4, 1, false, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn unpacked_test3_format_srgb8() {
        let exp = expected(
            3,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_SRGB,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [0; 4],
            &[
                S(0, 7, KHR_DF_CHANNEL_RGBSDA_RED, 0, 0, 0, 0, 0, 255),
                S(8, 7, KHR_DF_CHANNEL_RGBSDA_GREEN, 0, 0, 0, 0, 0, 255),
                S(16, 7, KHR_DF_CHANNEL_RGBSDA_BLUE, 0, 0, 0, 0, 0, 255),
            ],
        );
        check(create_dfd_unpacked(false, 3, 1, false, VkSuffix::Srgb), exp);
    }

    #[test]
    fn packed_test3_format_rgb565() {
        let exp = expected(
            2,
            KHR_DF_MODEL_RGBSDA,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_LINEAR,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [0; 4],
            &[
                S(0, 4, KHR_DF_CHANNEL_RGBSDA_BLUE, 0, 0, 0, 0, 0, 31),
                S(5, 5, KHR_DF_CHANNEL_RGBSDA_GREEN, 0, 0, 0, 0, 0, 63),
                S(11, 4, KHR_DF_CHANNEL_RGBSDA_RED, 0, 0, 0, 0, 0, 31),
            ],
        );
        // In order from LSB.
        let bits = [5, 6, 5, 0];
        let channels = [
            KHR_DF_CHANNEL_RGBSDA_BLUE,
            KHR_DF_CHANNEL_RGBSDA_GREEN,
            KHR_DF_CHANNEL_RGBSDA_RED,
            0,
        ];
        check(
            create_dfd_packed(false, 3, &bits, &channels, VkSuffix::Unorm),
            exp,
        );
    }

    fn comp1(
        model: u32,
        transfer: u32,
        dims: [u32; 4],
        ch: u32,
        bits: u32,
        bytes: u32,
    ) -> Vec<u32> {
        expected(
            bytes,
            model,
            KHR_DF_PRIMARIES_BT709,
            transfer,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            dims,
            &[S(0, bits, ch, 0, 0, 0, 0, 0, 0xFFFFFFFF)],
        )
    }

    #[test]
    fn compressed_test1_format_etc1s_r8b8g8() {
        let exp = comp1(
            KHR_DF_MODEL_ETC1S,
            KHR_DF_TRANSFER_LINEAR,
            [3, 3, 0, 0],
            KHR_DF_CHANNEL_ETC1S_RGB,
            63,
            8,
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc1s, 4, 4, 1, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test1_format_etc1s_sr8b8g8() {
        let exp = comp1(
            KHR_DF_MODEL_ETC1S,
            KHR_DF_TRANSFER_SRGB,
            [3, 3, 0, 0],
            KHR_DF_CHANNEL_ETC1S_RGB,
            63,
            8,
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc1s, 4, 4, 1, VkSuffix::Srgb),
            exp,
        );
    }

    #[test]
    fn compressed_test1_format_etc2_r8b8g8() {
        let exp = comp1(
            KHR_DF_MODEL_ETC2,
            KHR_DF_TRANSFER_LINEAR,
            [3, 3, 0, 0],
            KHR_DF_CHANNEL_ETC2_COLOR,
            63,
            8,
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc2R8G8B8, 4, 4, 1, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test2_format_etc2_r8g8b8a8() {
        let exp = expected(
            16,
            KHR_DF_MODEL_ETC2,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_LINEAR,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [3, 3, 0, 0],
            &[
                S(0, 63, KHR_DF_CHANNEL_ETC2_ALPHA, 0, 0, 0, 0, 0, 0xFFFFFFFF),
                S(64, 63, KHR_DF_CHANNEL_ETC2_COLOR, 0, 0, 0, 0, 0, 0xFFFFFFFF),
            ],
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc2R8G8B8A8, 4, 4, 1, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test1_format_etc2_sr8b8g8() {
        let exp = comp1(
            KHR_DF_MODEL_ETC2,
            KHR_DF_TRANSFER_SRGB,
            [3, 3, 0, 0],
            KHR_DF_CHANNEL_ETC2_COLOR,
            63,
            8,
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc2R8G8B8, 4, 4, 1, VkSuffix::Srgb),
            exp,
        );
    }

    #[test]
    fn compressed_test2_format_etc2_sr8g8b8a8() {
        let exp = expected(
            16,
            KHR_DF_MODEL_ETC2,
            KHR_DF_PRIMARIES_BT709,
            KHR_DF_TRANSFER_SRGB,
            KHR_DF_FLAG_ALPHA_STRAIGHT,
            [3, 3, 0, 0],
            &[
                S(
                    0,
                    63,
                    df_channel_type(KHR_DF_CHANNEL_RGBSDA_ALPHA, KHR_DF_SAMPLE_DATATYPE_LINEAR),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0xFFFFFFFF,
                ),
                S(64, 63, KHR_DF_CHANNEL_ETC2_COLOR, 0, 0, 0, 0, 0, 0xFFFFFFFF),
            ],
        );
        check(
            create_dfd_compressed(VkCompScheme::Etc2R8G8B8A8, 4, 4, 1, VkSuffix::Srgb),
            exp,
        );
    }

    #[test]
    fn compressed_test1x16_format_astc_12x12_srgb() {
        let exp = comp1(
            KHR_DF_MODEL_ASTC,
            KHR_DF_TRANSFER_SRGB,
            [11, 11, 0, 0],
            KHR_DF_CHANNEL_ASTC_DATA,
            127,
            16,
        );
        check(
            create_dfd_compressed(VkCompScheme::Astc, 12, 12, 1, VkSuffix::Srgb),
            exp,
        );
    }

    #[test]
    fn compressed_test1x16_format_astc_10x5_srgb() {
        let exp = comp1(
            KHR_DF_MODEL_ASTC,
            KHR_DF_TRANSFER_SRGB,
            [9, 4, 0, 0],
            KHR_DF_CHANNEL_ASTC_DATA,
            127,
            16,
        );
        check(
            create_dfd_compressed(VkCompScheme::Astc, 10, 5, 1, VkSuffix::Srgb),
            exp,
        );
    }

    #[test]
    fn compressed_test1x16_format_astc_5x4() {
        let exp = comp1(
            KHR_DF_MODEL_ASTC,
            KHR_DF_TRANSFER_LINEAR,
            [4, 3, 0, 0],
            KHR_DF_CHANNEL_ASTC_DATA,
            127,
            16,
        );
        check(
            create_dfd_compressed(VkCompScheme::Astc, 5, 4, 1, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test1x16_format_astc_10x8() {
        let exp = comp1(
            KHR_DF_MODEL_ASTC,
            KHR_DF_TRANSFER_LINEAR,
            [9, 7, 0, 0],
            KHR_DF_CHANNEL_ASTC_DATA,
            127,
            16,
        );
        check(
            create_dfd_compressed(VkCompScheme::Astc, 10, 8, 1, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test1x16_format_astc_3x3x3() {
        let exp = comp1(
            KHR_DF_MODEL_ASTC,
            KHR_DF_TRANSFER_LINEAR,
            [2, 2, 2, 0],
            KHR_DF_CHANNEL_ASTC_DATA,
            127,
            16,
        );
        check(
            create_dfd_compressed(VkCompScheme::Astc, 3, 3, 3, VkSuffix::Unorm),
            exp,
        );
    }

    #[test]
    fn compressed_test1_format_bc1() {
        let exp = comp1(
            KHR_DF_MODEL_BC1A,
            KHR_DF_TRANSFER_LINEAR,
            [3, 3, 0, 0],
            KHR_DF_CHANNEL_BC1A_COLOR,
            63,
            8,
        );
        check(
            create_dfd_compressed(VkCompScheme::Bc1Rgb, 4, 4, 1, VkSuffix::Unorm),
            exp,
        );
    }
}

//////////////////////////////
// DFDVkFormatListTest
//////////////////////////////

mod dfd_vk_format_list_test {
    use super::common::vkformat_list::FORMATS;
    use ktx::VkFormat;
    use ktx::dfd::khr_df::*;
    use ktx::dfd::*;
    use ktx::vkformat::vk_format_string;

    #[test]
    fn reconstruct_dfd_bytes_planes() {
        for &format in FORMATS {
            let mut dfd = match vk2dfd(format) {
                Some(d) => d,
                None => panic!(
                    "vk2dfd failed to produce DFD for {}",
                    vk_format_string(format)
                ),
            };
            let bp0 = KHR_DF_WORD_BYTESPLANE0 as usize + 1;
            let bp4 = KHR_DF_WORD_BYTESPLANE4 as usize + 1;
            // There are 4 bytesPlane values in one word. Capture all at once.
            let orig_bytes_plane03 = dfd[bp0];
            let orig_bytes_plane47 = dfd[bp4];
            dfd[bp0] = 0;
            dfd[KHR_DF_WORD_BYTESPLANE1 as usize + 1] = 0;
            reconstruct_dfd_bytes_planes_from_samples(&mut dfd);
            assert_eq!(orig_bytes_plane03, dfd[bp0], "{}", vk_format_string(format));
            assert_eq!(orig_bytes_plane47, dfd[bp4], "{}", vk_format_string(format));
        }

        // Make ETC1S DFDs and test reconstruction.
        for sample_count in 1..3u32 {
            let bdb_size = (KHR_DF_WORD_SAMPLESTART + sample_count * KHR_DF_WORD_SAMPLEWORDS) * 4;
            let dfd_size = bdb_size + 4;
            let mut dfd = vec![0u32; dfd_size as usize];
            {
                let b = bdb_mut(&mut dfd);
                dfdsetval(b, VENDORID, KHR_DF_VENDORID_KHRONOS);
                dfdsetval(b, DESCRIPTORTYPE, KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT);
                dfdsetval(b, VERSIONNUMBER, KHR_DF_VERSIONNUMBER_LATEST);
                dfdsetval(b, DESCRIPTORBLOCKSIZE, bdb_size);
                dfdsetval(b, MODEL, KHR_DF_MODEL_ETC1S);
                dfdsetval(b, PRIMARIES, KHR_DF_PRIMARIES_BT709);
                dfdsetval(b, TRANSFER, KHR_DF_TRANSFER_SRGB);
                dfdsetval(b, FLAGS, 0);
                b[KHR_DF_WORD_TEXELBLOCKDIMENSION0 as usize] =
                    3 | (3 << KHR_DF_SHIFT_TEXELBLOCKDIMENSION1);
                b[KHR_DF_WORD_BYTESPLANE0 as usize] = 0;
                b[KHR_DF_WORD_BYTESPLANE4 as usize] = 0;
                for sample in 0..sample_count {
                    let (bit_offset, channel_id) = if sample == 0 {
                        (0, KHR_DF_CHANNEL_ETC1S_RGB)
                    } else {
                        (64, KHR_DF_CHANNEL_ETC1S_AAA)
                    };
                    dfdsetsval(b, sample, CHANNELID, channel_id);
                    dfdsetsval(b, sample, QUALIFIERS, 0);
                    dfdsetsval(b, sample, SAMPLEPOSITION_ALL, 0);
                    dfdsetsval(b, sample, BITOFFSET, bit_offset);
                    dfdsetsval(b, sample, BITLENGTH, 63);
                    dfdsetsval(b, sample, SAMPLELOWER, 0);
                    dfdsetsval(b, sample, SAMPLEUPPER, u32::MAX);
                }
            }
            reconstruct_dfd_bytes_planes_from_samples(&mut dfd);
            let b = bdb(&dfd);
            assert_eq!(8, dfdval(b, BYTESPLANE0));
            if sample_count == 2 {
                assert_eq!(8, dfdval(b, BYTESPLANE1));
            } else {
                assert_eq!(0, dfdval(b, BYTESPLANE1));
            }
            assert_eq!(0, dfdval(b, BYTESPLANE2));
            assert_eq!(0, dfdval(b, BYTESPLANE3));
            assert_eq!(0, b[KHR_DF_WORD_BYTESPLANE4 as usize]);
        }
    }

    #[test]
    fn bidirectional_vk2dfd_test() {
        for &format in FORMATS {
            let dfd = match vk2dfd(format) {
                Some(d) => d,
                None => panic!(
                    "vk2dfd failed to produce DFD for {}",
                    vk_format_string(format)
                ),
            };
            let format_out = dfd2vk(&dfd);
            // SCALED formats are indistinguishable from INT formats and the
            // A8B8G8R8_*_PACK32 formats from R8G8B8A8*; dfd2vk returns the common one.
            let expected = match format {
                VkFormat::R8_USCALED => VkFormat::R8_UINT,
                VkFormat::R8_SSCALED => VkFormat::R8_SINT,
                VkFormat::R8G8_USCALED => VkFormat::R8G8_UINT,
                VkFormat::R8G8_SSCALED => VkFormat::R8G8_SINT,
                VkFormat::B8G8R8_USCALED => VkFormat::B8G8R8_UINT,
                VkFormat::B8G8R8_SSCALED => VkFormat::B8G8R8_SINT,
                VkFormat::R8G8B8_USCALED => VkFormat::R8G8B8_UINT,
                VkFormat::R8G8B8_SSCALED => VkFormat::R8G8B8_SINT,
                VkFormat::R8G8B8A8_USCALED => VkFormat::R8G8B8A8_UINT,
                VkFormat::R8G8B8A8_SSCALED => VkFormat::R8G8B8A8_SINT,
                VkFormat::B8G8R8A8_USCALED => VkFormat::B8G8R8A8_UINT,
                VkFormat::B8G8R8A8_SSCALED => VkFormat::B8G8R8A8_SINT,
                VkFormat::A8B8G8R8_UNORM_PACK32 => VkFormat::R8G8B8A8_UNORM,
                VkFormat::A8B8G8R8_SNORM_PACK32 => VkFormat::R8G8B8A8_SNORM,
                VkFormat::A8B8G8R8_USCALED_PACK32 => VkFormat::R8G8B8A8_UINT,
                VkFormat::A8B8G8R8_SSCALED_PACK32 => VkFormat::R8G8B8A8_SINT,
                VkFormat::A8B8G8R8_UINT_PACK32 => VkFormat::R8G8B8A8_UINT,
                VkFormat::A8B8G8R8_SINT_PACK32 => VkFormat::R8G8B8A8_SINT,
                VkFormat::A8B8G8R8_SRGB_PACK32 => VkFormat::R8G8B8A8_SRGB,
                VkFormat::A2R10G10B10_USCALED_PACK32 => VkFormat::A2R10G10B10_UINT_PACK32,
                VkFormat::A2R10G10B10_SSCALED_PACK32 => VkFormat::A2R10G10B10_SINT_PACK32,
                VkFormat::A2B10G10R10_USCALED_PACK32 => VkFormat::A2B10G10R10_UINT_PACK32,
                VkFormat::A2B10G10R10_SSCALED_PACK32 => VkFormat::A2B10G10R10_SINT_PACK32,
                VkFormat::R16_USCALED => VkFormat::R16_UINT,
                VkFormat::R16_SSCALED => VkFormat::R16_SINT,
                VkFormat::R16G16_USCALED => VkFormat::R16G16_UINT,
                VkFormat::R16G16_SSCALED => VkFormat::R16G16_SINT,
                VkFormat::R16G16B16_USCALED => VkFormat::R16G16B16_UINT,
                VkFormat::R16G16B16_SSCALED => VkFormat::R16G16B16_SINT,
                VkFormat::R16G16B16A16_USCALED => VkFormat::R16G16B16A16_UINT,
                VkFormat::R16G16B16A16_SSCALED => VkFormat::R16G16B16A16_SINT,
                f => f,
            };
            assert_eq!(
                format_out,
                expected,
                "{} -> {}",
                vk_format_string(format),
                vk_format_string(format_out)
            );
        }
    }
}

//////////////////////////////
// WriterTestHelper tests.
//////////////////////////////

mod writer_test_helper_rgb8_test {
    use super::common::wthelper::{E_NONE, WriterTestHelper};
    use ktx::gl_format::GL_RGB8;

    const NUM_COMPONENTS: u32 = 3;

    #[test]
    fn construct_2d() {
        let mut helper = WriterTestHelper::<u8, NUM_COMPONENTS, GL_RGB8>::new();
        helper.resize(E_NONE, 1, 1, 2, 32, 32, 1);
        assert_eq!(helper.images.len(), 1);
        assert_eq!(helper.images[0].len(), 1);
        assert_eq!(helper.images[0][0].len(), 1);
        // u8 components: element count equals byte count.
        assert_eq!(helper.images[0][0][0].len(), 32 * 32 * 3);
        assert_eq!(NUM_COMPONENTS, 3);
    }

    #[test]
    fn construct_3d() {
        let mut helper = WriterTestHelper::<u8, NUM_COMPONENTS, GL_RGB8>::new();
        helper.resize(E_NONE, 1, 1, 3, 32, 32, 32);
        assert_eq!(helper.images.len(), 1);
        assert_eq!(helper.images[0].len(), 1);
        assert_eq!(helper.images[0][0].len(), 32);
        assert_eq!(helper.images[0][0][0].len(), 32 * 32 * 3);
        assert_eq!(NUM_COMPONENTS, 3);
    }
}
