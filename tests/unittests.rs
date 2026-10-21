//! Port of tests/unittests/unittests.cc (sections are added as the library grows).
#![allow(clippy::panic)]

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
        assert_eq!(head.add_kv_pair(KTX_ORIENTATION_KEY, &c_str(ORIENTATION_VAL)), Ok(()));
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
        assert_eq!(stream.read(&mut [0u8; 1]), Err(ktx::ErrorCode::FileUnexpectedEof));
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

    const KTX_ID: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];

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
        assert_eq!(ktx_check_header1(&mut test_header), Err(ErrorCode::UnknownFileFormat));
    }

    #[test]
    fn disallows_invalid_endianness() {
        let mut test_header = test_header();
        test_header.endianness = 0;
        assert_eq!(ktx_check_header1(&mut test_header), Err(ErrorCode::FileDataError));
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
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::UnsupportedFeature));

        let mut h = header();
        h.pixel_depth = 4;
        h.layer_count = 2;
        assert_eq!(ktx_check_header2(&mut h), Err(ErrorCode::UnsupportedFeature));
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
