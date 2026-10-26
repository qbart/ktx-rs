# Vulkan loader (`vkloader.c`, `vk_funcs.c`, `ktxvulkan.h`) -> `src/vkloader.rs`

Feature `vulkan` (default on, like `LIBKTX_FEATURE_VK_UPLOAD`); off = `KTX_OMIT_VULKAN`.
Vulkan is the only allowed C binding: `ash` (types only, no loader) + `libloading`. The crate
denies `unsafe_code`; `src/vkloader.rs` is the only module allowing it.

- `VulkanFunctions` (`ktxVulkanFunctions`, `Option<PFN_*>`), `VulkanTexture`,
  `SubAllocatorCallbacks` (plain `fn` pointers like the C typedefs; partial set ->
  `KTX_INVALID_VALUE`), `VulkanDeviceInfo` (`p_allocator` raw pointer as in C).
- `VulkanDeviceInfo::{create, create_ex, construct, construct_ex, destruct, destroy}`: same
  loading order (`LOAD_EXT_FUNC` / `LOAD_INST_FUNC` / `LOAD_DEVICE_FUNC`, `KTX_NOT_FOUND`),
  `vkGetPhysicalDeviceMemoryProperties`, one primary command buffer (`KTX_OUT_OF_MEMORY`).
- `ktx_load_vulkan_library` / `ktx_load_vulkan_function`: `dlopen(NULL)` via
  `libloading::os::unix::Library::this()`, Windows `GetModuleHandleEx("vulkan-1.dll")` via
  `open_already_loaded`; the app must have linked / globally loaded Vulkan (same messages).
- `ktx_texture_vk_upload_ex_with_suballocator` (+ `_ex`, plain; methods `vk_upload*` on
  `Texture1`, `Texture2`, `AnyTexture`): image / view type, format checks, mip count for
  `generateMipmaps` (`floor(log2(max dim)) + 1`), faster-path rule, optimal tiling via staging
  buffer (`optimalTilingCallback` / `optimalTilingPadCallback`, `lcm4` region rounding), linear
  tiling via mapped image (`linearTilingCallback` / `linearTilingPadCallback`), `setImageLayout`
  access masks, `generateMipmaps` blits, fence / queue idle waits.
- `KtxTexture::get_vk_format` (`ktxTexture{,1,2}_GetVkFormat`, KTX1 via GL internal format then
  format/type).
- `VulkanTexture::{destruct, destruct_with_suballocator}`.

Deviations (no runtime exceptions): `VK_CHECK_RESULT` ignored as in release C; an unset
function pointer gives `KTX_NOT_FOUND` instead of a crash; mapped memory writes are bounds
checked and short source images return `KTX_FILE_DATA_ERROR` (C reads past them); the
`setImageLayout` asserts on unused layouts leave the access mask empty.

Tests: `tests/vkloader.rs` on the first CPU (lavapipe) or any Vulkan device: optimal 2D mips,
cubemap, array, 3D, KTX1 R8 padded rows, linear fast + padded, generated mipmaps, suballocator
(staging + image ids freed), errors, `dlopen(NULL)` device info. Images are read back and compared.
`unittests.rs`: `BadVulkanAllocExceptionTest` (the load tests' `bad_vulkan_alloc` message).
Mutants not observable on lavapipe: linear `copySize` min (row pitch never smaller) and the
staging region `lcm4` rounding (unaligned `bufferOffset` accepted by the driver).
