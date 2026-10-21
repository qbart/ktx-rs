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
