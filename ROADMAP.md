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
- [ ] `formatsize`, GL format tables (`gl_format.h`, `vkFormat2gl*`)
- [ ] `ktxTexture` base (texture.c): sizes, offsets, iterate, kv data
- [ ] `ktxTexture1` read/create/write (texture1.c, writer1.c)
- [ ] `ktxTexture2` read/create/write (texture2.c, writer2.c)
- [ ] Texture1 -> Texture2 conversion on write
- [ ] Supercompression: Zstd (pure Rust)
- [ ] Supercompression: Zlib (pure Rust)
- [ ] `info.c` (ktxPrintInfo, JSON)

## Basis Universal (pure Rust port)
- [ ] BasisLZ/ETC1S transcoder (all target formats)
- [ ] UASTC LDR 4x4 transcoder (all target formats)
- [ ] UASTC HDR 4x4 / 6x6 intermediate transcoder
- [ ] `ktxTexture2_TranscodeBasis`
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
- [ ] unittests: Swizzle, WriterTestHelper
- [ ] texturetests
- [ ] transcodetests
- [ ] CLI tests (tests/cts)
