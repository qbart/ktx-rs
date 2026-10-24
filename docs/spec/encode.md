# ktx encode / libktx ASTC codec

Ports `tools/ktx/command_encode.cpp` (`src/cli/command_encode.rs`) and `lib/src/astc_codec.cpp`
(`src/astc_codec.rs`).

## astc_codec

- `Texture2::compress_astc_ex` (`ktxTexture2_CompressAstcEx`): 8-bit uncompressed, non packed,
  sRGB or linear inputs; 1-4 components expanded to RGBA (L -> LLL1, LA -> LLLA); levels encoded
  smallest first into a prototype texture, which then replaces format, DFD (alpha flags kept),
  level index and data. Single threaded: the astcenc context is created for `threadCount` and the
  calling thread compresses every block (same output).
- C++ quirks kept: `astcProfile(sRGB, mode)` receives the mode enum as its `ldr` flag, so HDR
  mode still uses an LDR profile; invalid block dimensions fall back to 6x6 (sRGB format).
- `compress_astc` (`ktxTexture2_CompressAstc`): default options, quality snapped down to a preset.
- `decode_astc` (`ktxTexture2_DecodeAstc`): ASTC to RGBA8 / SRGBA8 / RGBA16F per image.

## ktx encode

- Options `--format` (ASTC only) / `--codec`, the ASTC, Basis, common, metrics, deflate and generic
  groups; validation order and messages as in C++.
- C++ quirk kept: `--format` only selects ASTC encoding, `blockDimension` is never set, so every
  ASTC output is 4x4 UNORM / sRGB (the CTS goldens are 4x4).
- `--compare-ssim/psnr`: `MetricsCalculator` (`src/cli/metrics_utils.rs`) saves the 8-bit input
  images, decodes the encoded copy (`decode_astc` or `transcode_basis` RGBA32 + transcode swizzle)
  and prints per image and overall SSIM (`basisu::compute_ssim`, 11x11 Gaussian, sigma 1.5) and
  PSNR (`image_metrics::calc`, default args: BT.709 luma) to stdout, or stderr when the KTX file
  goes to stdout. Shared with `ktx create`.
- Not yet: Basis Universal codecs (NOT_IMPLEMENTED).

Tests: CTS `tests/encode` (800 subcases pass incl. 24 metrics cases; Basis subcases pending),
texturetests ASTC LDR encode/decode and decode (9 tests).
