# vkformat

Source: `lib/src/vkformat_{enum.h,str.c,check.c,check_variant.c,typesize.c}`.
Generated into `src/vkformat/generated.rs` by `scripts/gen_vkformat.py`.

| C | Rust |
|---|---|
| `VkFormat` | `VkFormat(u32)` + consts |
| `vkFormatString` | `vk_format_string` (unknown: `VK_UNKNOWN_FORMAT`) |
| `stringToVkFormat` | `string_to_vk_format` (prefix optional, case-insensitive) |
| `isProhibitedFormat`/`isValidFormat`/`isSrgbFormat` | same, snake_case |
| `isNotSrgbFormatButHasSrgbVariant` | same |
| `vkFormatTypeSize` | `vk_format_type_size` (unknown: 0) |

Status: complete. Tests: `tests/test_string_to_vkformat.rs`.
