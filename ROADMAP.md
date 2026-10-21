# ROADMAP

Pure Rust port of KTX-Software (`libktx` + `ktx` CLI). Reference: `./ref/ktx` (not versioned).
Legend: [x] done, [~] partial, [ ] todo.

## Foundation
- [x] Crate scaffold, no-panic lints (`unwrap`/`expect`/`panic` denied)
- [x] Error codes + `ktxErrorString` (`error.rs`)
- [x] `ktx.h` constants, orientation, supercompression, transcode enums (`defs.rs`)
- [x] `strings.c` helpers + `strnstr`
- [x] VkFormat enum, string, parse, check, typesize (generated, `scripts/gen_vkformat.py`)
- [x] Hash list (key/value data) — `hashlist.c`
- [x] Streams: memory, file, custom — `memstream.c`, `filestream.c`
- [x] Header checks KTX1/KTX2 — `checkheader.c`, header structs
- [x] Endian swap — `swap.c`

## Data Format Descriptor (external/dfdutils)
- [x] khr_df constants + accessors, createdfd
- [x] interpretdfd, queries
- [x] vk2dfd / dfd2vk tables (generated)
- [ ] printdfd
- [x] colourspaces

## Texture core
- [x] `formatsize` (`ktxFormatSize_initFromDfd`)
- [x] GL/Vk format tables (`gl_format.h`, `vk_format.h`, generated)
- [ ] `vkFormat2gl*` tables (vk2gl.h, GL loader only)
- [x] `ktxTexture` base (texture.c): sizes, offsets, iterate, kv data
- [x] `ktxTexture1` read/create/write (texture1.c, writer1.c)
- [x] `ktxTexture2` read/create (texture2.c)
- [x] `ktxTexture2` write (writer2.c), SetImageFromMemory
- [x] Texture1 -> Texture2 conversion on write
- [x] `ktxTexture_CreateFrom*` (`AnyTexture`)
- [~] Supercompression: Zstd (pure Rust, `ruzstd`): inflate + deflate; deflate ignores level (ruzstd only has fastest)
- [x] Supercompression: Zlib (pure Rust, `miniz_oxide`)
- [ ] `info.c` (ktxPrintInfo, JSON)

## Basis Universal (pure Rust port)
- [~] BasisLZ/ETC1S transcoder: core + ETC1, ETC2, EAC R11/RG11, BC1, BC3, BC7, BC4, BC5, RGBA32, RGB565, BGR565, RGBA4444, ASTC 4x4, PVRTC1 RGB/RGBA (byte exact vs C++);
  todo PVRTC2, ATC, FXT1
- [ ] UASTC LDR 4x4 transcoder (all target formats)
- [ ] UASTC HDR 4x4 / 6x6 intermediate transcoder
- [~] `ktxTexture2_TranscodeBasis` (ETC1S path)
- [ ] ETC1S encoder (`ktxTexture2_CompressBasis`)
- [ ] UASTC encoder + RDO

## Other codecs
- [ ] ASTC encoder/decoder (astc_codec.cpp port)
- [ ] ETC unpack (etcunpack.cxx)

## GPU upload
- [ ] Vulkan loader (`vkloader.c`) via Vulkan bindings
- [ ] OpenGL loader (out of scope unless requested)

## CLI (`ktx` tool)
- [ ] info, validate, extract, create, encode, transcode, compare, deflate, help

## Tests ported
- [x] unittests: strnstr, test_string_to_vkformat
- [x] unittests: HashList, MemStream, CheckHeader1, createDFD, DFDVkFormatList
- [x] unittests: WriterTestHelper
- [ ] unittests: Swizzle (needs basis_encode), BadVulkanAlloc (needs Vulkan loader)
- [~] texturetests: 96 ported (create, KV data, iterate, load, sizes/offsets, image offsets,
  KTX1 write, KTX1->KTX2 write, KTX2 read, deflate, GetNumComponents uncompressed, metadata,
  unicode, supercompressed iterate). Todo: BasisCompress/Transcode, ASTC encode/decode.
  Null-pointer tests are not applicable in Rust.
- [~] transcodetests: Basic (implemented formats, golden SHA-256 from C++ basisu)
- [ ] CLI tests (tests/cts)
