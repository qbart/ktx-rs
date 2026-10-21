//! Pure Rust port of [KTX-Software](https://github.com/KhronosGroup/KTX-Software) `libktx`.
//!
//! Fallible functions return [`KtxResult`]; `KTX_SUCCESS` maps to `Ok`.
//! The library never panics on malformed input.

pub mod basis_transcode;
pub mod basisu;
pub mod defs;
pub mod dfd;
pub mod error;
pub mod formatsize;
pub mod gl_format;
pub mod hashlist;
pub mod ktxint;
pub mod stream;
pub mod strings;
pub mod swap;
pub mod texture;
pub mod texture1;
pub mod texture2;
pub mod texture_any;
pub mod version;
pub mod vk_format;
pub mod vkformat;
pub mod writer1;
pub mod writer2;
pub mod zlib;

pub use defs::*;
pub use error::{ErrorCode, KtxResult, ktx_error_string};
pub use hashlist::{HashList, HashListEntry};
pub use texture::{
    ClassId, FormatVersion, KtxTexture, Texture, TextureCreateFlags, TextureCreateInfo,
    TextureCreateStorage, create_flags::*,
};
pub use texture_any::AnyTexture;
pub use texture1::Texture1;
pub use texture2::Texture2;
pub use vkformat::VkFormat;
