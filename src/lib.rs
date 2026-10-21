//! Pure Rust port of [KTX-Software](https://github.com/KhronosGroup/KTX-Software) `libktx`.
//!
//! Fallible functions return [`KtxResult`]; `KTX_SUCCESS` maps to `Ok`.
//! The library never panics on malformed input.

pub mod defs;
pub mod dfd;
pub mod error;
pub mod formatsize;
pub mod hashlist;
pub mod ktxint;
pub mod stream;
pub mod strings;
pub mod swap;
pub mod texture;
pub mod texture2;
pub mod vkformat;
pub mod zlib;

pub use defs::*;
pub use error::{ErrorCode, KtxResult, ktx_error_string};
pub use hashlist::{HashList, HashListEntry};
pub use vkformat::VkFormat;
pub use texture::{
    ClassId, FormatVersion, KtxTexture, Texture, TextureCreateFlags, TextureCreateInfo, TextureCreateStorage,
    create_flags::*,
};
pub use texture2::Texture2;
