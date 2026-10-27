// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of tests/unittests/test_string_to_vkformat.cc

use ktx::vkformat::{VkFormat, string_to_vk_format};

#[test]
fn string_to_vk_format_test() {
    assert_eq!(string_to_vk_format("UNDEFINED"), VkFormat::UNDEFINED);
    assert_eq!(
        string_to_vk_format("VK_FORMAT_UNDEFINED"),
        VkFormat::UNDEFINED
    );
    assert_eq!(string_to_vk_format("Not a format"), VkFormat::UNDEFINED);

    assert_eq!(
        string_to_vk_format("R4G4_UNORM_PACK8"),
        VkFormat::R4G4_UNORM_PACK8
    );
    assert_eq!(
        string_to_vk_format("VK_FORMAT_R4G4_UNORM_PACK8"),
        VkFormat::R4G4_UNORM_PACK8
    );

    assert_eq!(string_to_vk_format("R8G8B8_UNORM"), VkFormat::R8G8B8_UNORM);
    assert_eq!(
        string_to_vk_format("VK_FORMAT_R8G8B8_UNORM"),
        VkFormat::R8G8B8_UNORM
    );
    assert_eq!(string_to_vk_format("R8G8B8_SNORM"), VkFormat::R8G8B8_SNORM);
    assert_eq!(
        string_to_vk_format("VK_FORMAT_R8G8B8_SNORM"),
        VkFormat::R8G8B8_SNORM
    );

    assert_eq!(
        string_to_vk_format("ASTC_6x6_UNORM_BLOCK"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("ASTC_6X6_UNORM_BLOCK"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("astc_6x6_unorm_block"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("VK_FORMAT_ASTC_6x6_UNORM_BLOCK"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("VK_FORMAT_ASTC_6X6_UNORM_BLOCK"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("VK_FORMAT_ASTC_6X6_UNORM_BLOCK"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );
    assert_eq!(
        string_to_vk_format("vk_format_astc_6x6_unorm_block"),
        VkFormat::ASTC_6x6_UNORM_BLOCK
    );

    assert_eq!(
        string_to_vk_format("VK_FORMAT_ASTC_6x6_UNORM"),
        VkFormat::UNDEFINED
    );
    assert_eq!(
        string_to_vk_format("VK_FORMAT_ASTC_6x6_UNORM_BLOC"),
        VkFormat::UNDEFINED
    );
    assert_eq!(
        string_to_vk_format("K_FORMAT_ASTC_6x6_UNORM_BLOCK"),
        VkFormat::UNDEFINED
    );
    assert_eq!(
        string_to_vk_format("_ASTC_6x6_UNORM_BLOCK"),
        VkFormat::UNDEFINED
    );
    assert_eq!(
        string_to_vk_format("STC_6x6_UNORM_BLOCK"),
        VkFormat::UNDEFINED
    );
}
