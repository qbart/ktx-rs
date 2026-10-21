//! Port of tests/unittests/unittests.cc (sections are added as the library grows).

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
