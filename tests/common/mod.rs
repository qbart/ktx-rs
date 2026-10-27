// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Shared helpers for ported tests.
#![allow(dead_code)]

pub mod astcenc_gen;
pub mod ktxdiff;
pub mod synthetic;
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
