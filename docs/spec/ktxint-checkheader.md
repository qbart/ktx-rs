# ktxint / checkheader / swap

Source: `lib/src/ktxint.h`, `lib/src/checkheader.c`, `lib/src/swap.c`.

- `KtxHeader` (64 B), `KtxHeader2` (80 B), `IndexEntry32/64`, `LevelIndexEntry` (24 B):
  `from_bytes`/`to_bytes`, little-endian.
- `ktx_check_header1(&mut h) -> KtxResult<SupplementalInfo>`: identifier, endianness
  (swaps reversed), glType/glFormat pairing, glFormat != glInternalformat, dims, faces,
  level count (0 -> 1 + generateMipmaps), max levels.
- `ktx_check_header2`: identifier, prohibited (`FileDataError`) / invalid (`UnsupportedFeature`)
  format, BasisLZ/UASTC-HDR-6x6 require `UNDEFINED`, dims, cube rules, supercompression range.
- `swap`: 16/32/64 + raw byte groups. Divergence: correct 64-bit swap (C has a shift bug).

Status: complete. Tests: `tests/unittests.rs::{check_header1_test,check_header2_test,swap_test}`.
