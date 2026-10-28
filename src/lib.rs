// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Rust port of [KTX-Software](https://github.com/KhronosGroup/KTX-Software) `libktx`.
//!
//! Fallible functions return [`KtxResult`]; `KTX_SUCCESS` maps to `Ok`.
//! The library never panics on malformed input.

pub mod astc_codec;
pub mod astcenc;
pub mod basis_encode;
pub mod basis_transcode;
pub mod basisu;
pub mod cli;
pub mod defs;
pub mod dfd;
pub mod encode_params;
pub mod error;
pub mod etcunpack;
pub mod formatsize;
pub mod gl_format;
pub mod glformat_str;
#[cfg(all(feature = "opengl", not(target_family = "wasm")))]
pub mod glloader;
pub mod glupload;
pub mod hashlist;
pub mod info;
pub mod jpgd;
pub mod ktxint;
pub mod lodepng;
mod scoped;
pub mod stream;
pub mod strings;
pub mod swap;
pub mod texture;
pub mod texture1;
pub mod texture2;
pub mod texture_any;
pub mod tinyexr;
pub mod version;
pub mod vk2gl;
pub mod vk_format;
pub mod vkformat;
#[cfg(all(feature = "vulkan", not(target_family = "wasm")))]
pub mod vkloader;
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
