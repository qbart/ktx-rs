//! Pure Rust port of [KTX-Software](https://github.com/KhronosGroup/KTX-Software) `libktx`.
//!
//! Fallible functions return [`KtxResult`]; `KTX_SUCCESS` maps to `Ok`.
//! The library never panics on malformed input.

pub mod defs;
pub mod error;
pub mod strings;
pub mod vkformat;

pub use defs::*;
pub use error::{ErrorCode, KtxResult, ktx_error_string};
pub use vkformat::VkFormat;
