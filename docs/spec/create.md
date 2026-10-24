# ktx create

Source: `tools/ktx/command_create.cpp`, `encode_utils_{common,astc,basis}.h`, `metrics_utils.h`
(`OptionsMetrics`), `command.h` (`OptionsMultiInSingleOut`). Port: `src/cli/command_create.rs`,
`src/cli/encode_utils.rs`, `src/encode_params.rs` (`ktxBasisParams`, `ktxAstcParams`, enums).

- Options, help text, validation order (`Combine` processes right to left, then
  `processOptions`) and all error messages as in C++. `cxxopts::value<float>()` parsing emulates
  `istringstream >> float` (`options::parse_f32`).
- Raw path: `--raw` image size checks, `ktxTexture_SetImageFromMemory`.
- Image path: imageio PNG / EXR / JPEG readers → `ImageT` (rgba8/16/32/32f) → color space
  (assign/convert TF and primaries, gamma, ICC, PNG defaults, warnings) → `--scale` / `--width` /
  `--height` resample → texcoord origin y-flip → normalize → map range (`--map-range-*`, auto) →
  input swizzle → premultiply → `convert` for every convertible VkFormat → `--generate-mipmap`.
- Metadata: KTXwriter, KTXswizzle, KTXmapRange, KTXorientation, KTXwriterScParams; DFD
  primaries / transfer / premultiplied flag / linear alpha qualifier; Zstd / ZLIB.
- C++ quirk kept: `--*-texcoord-origin ...-front` maps to back (`compare()` inverted).
- ASTC (`--format ASTC_*`) encoding through `ktxTexture2_CompressAstcEx`.
- Not yet: Basis Universal (`--encode`) encoding and `--compare-ssim/psnr` metrics: parsed and
  validated, then fail with NOT_IMPLEMENTED ("... not implemented").

Tests: CTS `tests/create` (4482 subcases pass; the rest skip non-vendored inputs or are
pending on the Basis encoders). `outputTolerance` cases use `tests/common/ktxdiff.rs` (port of
`tests/ktxdiff`) like clitest.py on non-primary platforms: the ZLIB level 1 goldens come from
arm64 miniz, which has no x86-64 `tdefl_compress_fast` path.
