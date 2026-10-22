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
- [x] printdfd (text + JSON, byte exact vs C)
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
- [x] Supercompression: Zstd: deflate = libzstd 1.5.7 translation `zstd-pure-rs` (byte exact, all
  levels), inflate = `ruzstd`
- [x] Supercompression: Zlib: deflate = port of basisu miniz `tdefl` (byte exact, x86-64 config),
  inflate = `miniz_oxide`
- [x] `info.c` (ktxPrintInfo text + KTX2 JSON, byte exact vs C) + `glformat_str.c`

## Basis Universal (pure Rust port)
- [~] BasisLZ/ETC1S transcoder: core + ETC1, ETC2, EAC R11/RG11, BC1, BC3, BC7, BC4, BC5, RGBA32, RGB565, BGR565, RGBA4444, ASTC 4x4, PVRTC1 RGB/RGBA, PVRTC2 RGB/RGBA (byte exact vs C++);
  all KTX transcode targets done; todo ATC, FXT1 (basisu-only formats)
- [x] UASTC LDR 4x4 transcoder: all KTX targets (ASTC, BC1/3/4/5/7, ETC1/ETC2, EAC R11/RG11,
  PVRTC1, RGBA32/565/4444) byte exact vs C++ on every level of 4 UASTC files; PVRTC2 rejected like C
- [x] UASTC HDR 4x4 transcoder: ASTC HDR 4x4, BC6H, RGB/RGBA half, RGB9E5 byte exact vs C++
- [x] UASTC HDR 6x6 intermediate transcoder: ASTC HDR 6x6, BC6H (incl. HQ 2-subset), RGB/RGBA half,
  RGB9E5 byte exact vs C++
- [x] `ktxTexture2_TranscodeBasis`: ETC1S, UASTC LDR 4x4, UASTC HDR 4x4, UASTC HDR 6x6i
- [ ] ETC1S encoder (`ktxTexture2_CompressBasis`)
- [ ] UASTC encoder + RDO

## Other codecs
- [ ] ASTC encoder/decoder (astc_codec.cpp port)
- [ ] ETC unpack (etcunpack.cxx)

## GPU upload
- [ ] Vulkan loader (`vkloader.c`) via Vulkan bindings
- [ ] OpenGL loader (out of scope unless requested)

## CLI (`ktx` tool)
- [x] framework: cxxopts port (parsing, errors, help layout), Reporter, option groups
- [x] validate (validator + text/JSON output, all CTS validate cases pass)
- [x] info (text/JSON, KTX1 + KTX2, all CTS info cases pass)
- [x] transcode (all targets, swizzle/channel conversion, --zstd/--zlib), help (man pages)
- [ ] extract, create, encode, compare, deflate, convert

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
- [~] CLI tests (tests/cts): runner ported (`tests/cts.rs`); validate, info, help, transcode done
