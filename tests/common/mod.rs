//! Shared helpers for ported tests.
#![allow(dead_code)]

pub mod vkformat_list;
pub mod wthelper;

use std::path::PathBuf;

/// `tests/resources` directory (port of the test resources path argument).
pub fn resources_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("resources")
}

/// `tests/resources/ktx2`.
pub fn ktx2_path() -> PathBuf {
    resources_path().join("ktx2")
}

/// `tests/resources/unicode`.
pub fn unicode_path() -> PathBuf {
    resources_path().join("unicode")
}
