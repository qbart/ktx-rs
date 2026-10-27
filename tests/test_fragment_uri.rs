// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of tests/unittests/test_fragment_uri.cc (`FragmentURITest`).
#![allow(clippy::panic)]

use ktx::cli::fragment_uri::{FragmentURI, RANGE_END, SelectorRange, parse_fragment_uri};

fn p(s: &str) -> FragmentURI {
    parse_fragment_uri(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn r(begin: u32, end: u32) -> SelectorRange {
    SelectorRange::new(begin, end)
}

#[test]
fn parse_whole_range() {
    let none = SelectorRange::default();
    let all = SelectorRange::all();
    assert_eq!(p("").mip, none);
    assert_eq!(p("").facial, none);
    assert_eq!(p("").stratal, none);

    assert_eq!(p("m").mip, all);
    assert_eq!(p("m").stratal, none);
    assert_eq!(p("m").facial, none);
    assert_eq!(p("a").mip, none);
    assert_eq!(p("a").stratal, all);
    assert_eq!(p("a").facial, none);
    assert_eq!(p("f").mip, none);
    assert_eq!(p("f").stratal, none);
    assert_eq!(p("f").facial, all);

    assert_eq!(p("m&a").mip, all);
    assert_eq!(p("m&a").stratal, all);
    assert_eq!(p("a&f").stratal, all);
    assert_eq!(p("a&f").facial, all);
    assert_eq!(p("f&m").mip, all);
    assert_eq!(p("f&m").facial, all);
}

#[test]
fn parse_range_empty() {
    let all = SelectorRange::all();
    assert_eq!(p("m=").mip, all);
    assert_eq!(p("a=").stratal, all);
    assert_eq!(p("f=").facial, all);

    assert_eq!(p("m=,").mip, all);
    assert_eq!(p("a=,").stratal, all);
    assert_eq!(p("f=,").facial, all);
}

#[test]
fn parse_range_begin() {
    assert_eq!(p("m=0").mip, r(0, RANGE_END));
    assert_eq!(p("a=0").stratal, r(0, RANGE_END));
    assert_eq!(p("f=0").facial, r(0, RANGE_END));

    assert_eq!(p("m=1").mip, r(1, RANGE_END));
    assert_eq!(p("a=1").stratal, r(1, RANGE_END));
    assert_eq!(p("f=1").facial, r(1, RANGE_END));
}

#[test]
fn parse_range_end() {
    assert_eq!(p("m=,0").mip, r(0, 1));
    assert_eq!(p("a=,0").stratal, r(0, 1));
    assert_eq!(p("f=,0").facial, r(0, 1));

    assert_eq!(p("m=,1").mip, r(0, 2));
    assert_eq!(p("a=,1").stratal, r(0, 2));
    assert_eq!(p("f=,1").facial, r(0, 2));
}

#[test]
fn parse_range_begin_end() {
    assert_eq!(p("m=0,0").mip, r(0, 1));
    assert_eq!(p("a=0,0").stratal, r(0, 1));
    assert_eq!(p("f=0,0").facial, r(0, 1));

    assert_eq!(p("m=0,1").mip, r(0, 2));
    assert_eq!(p("a=0,1").stratal, r(0, 2));
    assert_eq!(p("f=0,1").facial, r(0, 2));

    assert_eq!(p("m=1,3").mip, r(1, 4));
    assert_eq!(p("a=1,3").stratal, r(1, 4));
    assert_eq!(p("f=1,3").facial, r(1, 4));
}

#[test]
fn parse_multiple_range() {
    assert_eq!(p("m=0,0&a=1,1").mip, r(0, 1));
    assert_eq!(p("m=0,0&a=1,1").stratal, r(1, 2));
    assert_eq!(p("a=0,0&f=1,1").stratal, r(0, 1));
    assert_eq!(p("a=0,0&f=1,1").facial, r(1, 2));
    assert_eq!(p("f=0,0&m=1,1").facial, r(0, 1));
    assert_eq!(p("f=0,0&m=1,1").mip, r(1, 2));
}

#[test]
fn parse_multi_range() {
    assert_eq!(p("m=10,15&m=20,").mip.to_string(), "10..15,20..last");
    assert_eq!(
        p("m=0,0&m=1,1&m=10,15&m=20,").mip.to_string(),
        "0,1,10..15,20..last"
    );
}

#[test]
fn validate() {
    assert!(p("m=0,0&a=1,1").validate(1, 2, 1));
    assert!(!p("m=0,0&a=1,1").validate(1, 1, 1));
    assert!(!p("m=0,0&a=1,1").validate(0, 0, 0));
}

#[test]
fn selector_range_to_string() {
    assert_eq!(r(0, 0).to_string(), "none");
    assert_eq!(r(0, 1).to_string(), "0");
    assert_eq!(r(10, 11).to_string(), "10");
    assert_eq!(r(0, 2).to_string(), "0..1");
    assert_eq!(r(10, 12).to_string(), "10..11");
    assert_eq!(r(0, RANGE_END).to_string(), "all");
}
