# ROADMAP

Rust port of KTX-Software (`libktx` + `ktx` CLI). Reference: `./ref/ktx` (not versioned).
Legend: [x] done, [~] partial, [ ] todo.

## Status
Feature complete for the scope (libktx, `ktx` CLI, Basis Universal): every CTS subcase passes,
codecs are byte exact vs the C++ originals, upstream unit / texture / transcode tests are ported.
Open: ASTC HDR encoding (deferred, see below), optional performance work, and the gaps under
"Not covered".

## Licensing
- [x] `LICENSE`, `LICENSES/`, `NOTICE.md`, `REUSE.toml`; SPDX headers on every Rust file
  (`docs/spec/licensing.md`)
- [x] Non-open HI trademark test images dropped; crates.io package excludes test data
- [ ] Run `reuse lint` (tool not installed)

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
- [x] `vkFormat2gl*` tables (vk2gl.h, generated, all VkFormat values vs C)
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
- [x] BasisLZ/ETC1S transcoder: core + ETC1, ETC2, EAC R11/RG11, BC1, BC3, BC7, BC4, BC5, RGBA32, RGB565, BGR565, RGBA4444, ASTC 4x4, PVRTC1 RGB/RGBA, PVRTC2 RGB/RGBA (byte exact vs C++);
  all KTX transcode targets done; ATC RGB/RGBA (basisu-only, byte exact); FXT1 compiled out like
  libktx (`BASISD_SUPPORT_FXT1=0`)
- [x] UASTC LDR 4x4 transcoder: all KTX targets (ASTC, BC1/3/4/5/7, ETC1/ETC2, EAC R11/RG11,
  PVRTC1, RGBA32/565/4444) byte exact vs C++ on every level of 4 UASTC files; PVRTC2 rejected like C
- [x] UASTC HDR 4x4 transcoder: ASTC HDR 4x4, BC6H, RGB/RGBA half, RGB9E5 byte exact vs C++
- [x] UASTC HDR 6x6 intermediate transcoder: ASTC HDR 6x6, BC6H (incl. HQ 2-subset), RGB/RGBA half,
  RGB9E5 byte exact vs C++
- [x] `ktxTexture2_TranscodeBasis`: ETC1S, UASTC LDR 4x4, UASTC HDR 4x4, UASTC HDR 6x6i
- [x] basisu SSIM (`basisu_ssim.cpp`) and 8-bit `image_metrics::calc` (CTS exact output)
- [x] basisu resampler + 16 filters (`encoder/basisu_resampler`, bit exact vs C++, used by image scaling)
- [x] ETC1 block encoder `basisu_etc.cpp` (etc_block, etc1_optimizer, solid color, EAC A8; bit exact vs SSE build)
- [x] ETC1S encoder: TSVQ vector quantizer, frontend, Huffman encoder, backend, `basis_compressor`
  + `.basis` writer (bit exact)
- [x] `ktxTexture2_CompressBasisEx` / `CompressBasis` (`basis_encode.cpp`): ETC1S/BasisLZ, UASTC
  LDR 4x4, UASTC HDR 4x4 and UASTC HDR 6x6i byte exact vs CTS (docs/spec/basis_encode.md)
- [x] UASTC HDR 4x4 encoder (`basisu_uastc_hdr_4x4_enc.cpp` + CEM 7/11 parts of
  `basisu_astc_hdr_common.cpp`, bit exact)
- [x] ASTC HDR 6x6 / UASTC HDR 6x6i encoder (`basisu_astc_hdr_6x6_enc.cpp` + dual plane,
  downsampled grid and refinement parts of `basisu_astc_hdr_common.cpp`, bit exact incl. RDO)
- [x] UASTC encoder + RDO: bc7enc color cell compressor, `encode_uastc` block encoder (all
  levels / flags, BC1/ETC1/EAC hints) and `uastc_rdo` post-pass (incl. job split) bit exact

## Other codecs
- [x] lodepng PNG encoder + its zlib encoder (byte exact vs C, `src/lodepng`)
- [x] lodepng PNG decoder incl. KTX's split decode API and its inflater (byte exact vs C, 20800 cases)
- [x] tinyexr EXR writer (scanline, uncompressed; byte exact vs C++, `src/tinyexr.rs`)
- [x] tinyexr EXR reader: version / header parsing, scanline + tiled (one level, mipmap, ripmap),
  NONE / RLE / ZIPS / ZIP / PIZ (byte exact vs C++, 8350 cases)
- [x] astcenc decompressor (byte exact vs AVX2/F16C astcenc, `src/astcenc`)
- [x] astcenc compressor: `astcenc_compress_image` bit exact vs the AVX2 C++ build (all block
  sizes, profiles, presets, flags); alpha scale RDO (`a_scale_radius`) not ported (unused by KTX)
- [x] libktx `astc_codec.cpp`: `ktxTexture2_CompressAstcEx`, `ktxTexture2_CompressAstc`, `ktxTexture2_DecodeAstc`
- [ ] DEFERRED: ASTC HDR encoding. Upstream is broken (`astcProfile(sRGB, mode)` always yields an LDR
  profile, 8-bit input only, `ktx encode` forces LDR); kept as-is for CTS parity, fix later
- [x] jpgd JPEG decoder (baseline + progressive, byte exact vs C++, `src/jpgd.rs`)
- [x] ETC unpack (`etcunpack.cxx`; block decoder written from the spec since etcdec's license
  forbids modified copies, byte exact vs etcdec)

## GPU upload
- [x] Vulkan loader (`vkloader.c`, `vk_funcs.c`): `ash` + `libloading`, feature `vulkan` (default),
  all upload paths incl. suballocator, tested on lavapipe (docs/spec/vkloader.md)
- [x] OpenGL loader (`glloader.c`, `gl_funcs.c`): opt-in feature `opengl`, desktop + ES context
  capabilities, ETC software fallback, tested on Mesa llvmpipe via EGL (docs/spec/glloader.md)

## Performance (docs/spec/performance.md)
- [x] benchmark vs the C++ `ktx` (`scripts/bench/bench.sh`, outputs compared byte for byte)
- [x] basisu job pool (`job_pool.rs`); UASTC LDR block encode threaded
- [x] Basis-LZ frontend job sites
- [ ] optional: KTX2 level supercompression jobs (zstd 20 is 1.18x C++), basisu SSE
  `find_lowest_error` kernels (Basis-LZ `--clevel 5` 1.4x), AVX2 hardware gathers in astcenc
- [x] astcenc worker threads (`ParallelManager`, shared context)
- [x] astcenc SIMD: SSE2/SSE4.1/AVX2 kernels, compressor compiled per instruction set with
  runtime CPU dispatch (`#[isa]`, `ktx-macros`), scalar on other targets
- OpenCL encoding: not ported (C binding)

## CLI (`ktx` tool)
- [x] framework: cxxopts port (parsing, errors, help layout), Reporter, option groups
- [x] validate (validator + text/JSON output, all CTS validate cases pass)
- [x] info (text/JSON, KTX1 + KTX2, all CTS info cases pass)
- [x] transcode (all targets, swizzle/channel conversion, --zstd/--zlib), help (man pages)
- [x] deflate (KTXwriter / KTXwriterScParams rewrite rules, all CTS deflate cases pass)
- [x] extract: options, fragment URIs, selection checks, raw, PNG, EXR, ASTC decode, 4:2:2
  unpack (all CTS extract cases pass)
- [x] compare (header, level index, DFD, KVD, SGD, raw / per texel block image content;
  `ImageCodec` port; all CTS compare cases pass)
- [x] imageio subset for create: FormatDescriptor, ImageSpec, ImageInput, PNG + EXR + JPEG
  readers (netpbm reader and output plugins intentionally not ported)
- [x] imageio `image.hpp`: transfer functions, primaries, `ImageT<T, N>` conversions, color space
  transform, resample, swizzle/copy/normalize/premultiply (bit exact vs C++, 5677 cases)
- [x] create: all options, raw + PNG/EXR/JPEG input, color space conversion, resize, mipmap
  generation, all convertible formats, ASTC and all Basis encoders, `--compare-ssim/psnr`
  metrics (all 4784 CTS create subcases pass)
- [x] convert (KTX1 → KTX2, all CTS convert cases pass)
- [x] encode: ASTC and all Basis encoders + SSIM / PSNR metrics (all 1213 CTS encode subcases
  pass)

## Tests ported
- [x] unittests: strnstr, test_string_to_vkformat
- [x] unittests: HashList, MemStream, CheckHeader1, createDFD, DFDVkFormatList
- [x] unittests: WriterTestHelper
- [x] unittests: Swizzle (`swizzle_to_rgba`)
- [x] unittests: FragmentURI (`test_fragment_uri.cc`), NormaliseColor + ImagePacking E5B9G9R9
  (`image_unittests.cc`)
- [x] unittests: BadVulkanAlloc
- [x] texturetests: 115 ported (create, KV data, iterate, load, sizes/offsets, image offsets,
  KTX1 write, KTX1->KTX2 write, KTX2 read, deflate, GetNumComponents uncompressed, metadata,
  unicode, supercompressed iterate, ASTC LDR encode/decode, ASTC decode, BasisCompress,
  TranscodeBasis MissingAlphaSlices, GetNumComponents BasisLZ/UASTC).
  Null-pointer tests are not applicable in Rust (references cannot be null), so not ported.
- [x] transcodetests: Basic (all formats, golden SHA-256 from C++ basisu), TranscodeUastcHdr6x6i
  RoundTrip2D / RoundTrip3DMipLevels
- [x] CLI tests (tests/cts): runner ported (`tests/cts.rs`), all CTS data vendored, every
  subcase of every command runs (no skips) and passes

## Not covered
- Vulkan / OpenGL loaders tested only on Mesa software drivers (lavapipe, llvmpipe), not on GPUs.
- Tested on Linux x86-64 only (aarch64 compiles); Windows and macOS untested.
- BasisLZ video (P-frame) transcoding ported but untested: no video files in the test data.
- No fuzzing of the parsers (KTX1/KTX2, BasisLZ, zstd/zlib, PNG/EXR/JPEG); relevant only for
  untrusted input.
