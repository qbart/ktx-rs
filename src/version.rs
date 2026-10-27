// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of the generated `version.h`.

/// Port of `LIBKTX_VERSION`: version of this library.
pub const LIBKTX_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// Port of `LIBKTX_DEFAULT_VERSION` (`$DEF_VER.__default__`, upstream API version v5.0).
pub const LIBKTX_DEFAULT_VERSION: &str = "v5.0.__default__";
