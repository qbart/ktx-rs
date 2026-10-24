# ktx convert

Source: `tools/ktx/command_convert.cpp`. Port: `src/cli/command_convert.rs`.

- `--input-type ktx` (required), `-l/--map-astc-rgba-to-ldr` (only kept for
  `GL_COMPRESSED_RGBA_ASTC_*_KHR`), `-d/--drop-bad-orientation`; optional output path
  (missing or directory → input name with `.ktx2`, `std::filesystem::replace_extension` rules).
- KTX1 load (`ktxTexture1_CreateFromStream`), unsupported formats report
  `glInternalformatString`. Metadata keys containing `ktx`/`KTX` other than KTXorientation /
  KTXwriter are dropped with a warning; `KTXOrientation` is rewritten unless `-d`. Like the
  C++ loop the entry after a deleted one is not examined.
- KTXwriter added, `ktxTexture1_WriteKTX2ToStream`; written files are re-validated and a
  failed validation exits with the validator's code and its messages.

Tests: CTS `tests/convert` (82 subcases pass).
