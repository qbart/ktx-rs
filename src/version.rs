//! Port of the generated `version.h`.

/// Port of `LIBKTX_VERSION`: version of this library.
pub const LIBKTX_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// Port of `LIBKTX_DEFAULT_VERSION` (`$DEF_VER.__default__`, upstream API version v5.0).
pub const LIBKTX_DEFAULT_VERSION: &str = "v5.0.__default__";
