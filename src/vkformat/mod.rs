// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Vulkan format enum and helpers. Port of `vkformat_enum.h`, `vkformat_str.c`,
//! `vkformat_check.c`, `vkformat_check_variant.c` and `vkformat_typesize.c`.

mod generated;

pub use generated::{
    VK_FORMAT_ENTRIES, VK_FORMAT_MAX_STANDARD_ENUM, is_not_srgb_format_but_has_srgb_variant,
    is_prohibited_format, is_srgb_format, is_valid_format, vk_format_string, vk_format_type_size,
};

/// `VkFormat`. A transparent `u32` so any value read from a file is representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[repr(transparent)]
pub struct VkFormat(pub u32);

impl From<u32> for VkFormat {
    fn from(v: u32) -> Self {
        VkFormat(v)
    }
}

impl From<VkFormat> for u32 {
    fn from(v: VkFormat) -> Self {
        v.0
    }
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Port of `stringToVkFormat`. `VK_FORMAT_` prefix is optional, case insensitive.
/// Returns [`VkFormat::UNDEFINED`] when not recognised.
pub fn string_to_vk_format(s: &str) -> VkFormat {
    let mut b = s.as_bytes();
    const PREFIX: &[u8] = b"VK_FORMAT_";
    if b.len() >= PREFIX.len() && eq_ignore_case(&b[..PREFIX.len()], PREFIX) {
        b = &b[PREFIX.len()..];
    }
    for (name, fmt) in generated::VK_FORMAT_PARSE_TABLE {
        if eq_ignore_case(b, name.as_bytes()) {
            return *fmt;
        }
    }
    VkFormat::UNDEFINED
}
