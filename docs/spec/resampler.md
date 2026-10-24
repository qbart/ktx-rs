# basisu resampler

Source: `external/basis_universal/encoder/basisu_resampler.{h,cpp}`, `basisu_resample_filters.cpp`.
Port: `src/basisu/resampler.rs`, `src/basisu/resample_filters.rs`. Used by `ImageT::resample`
(imageio `image.hpp`) for `ktx create` scaling and mipmap generation.

- All 16 filters with identical float/double promotion (float literals rounded to `f32` before
  widening; `exp(float)` resolves to `expf` like libstdc++'s `<math.h>` overloads).
- `make_clist`, `reflect` (wrap/reflect/clamp), X-Y vs Y-X order choice, 16384-slot scan buffer,
  `put_line`/`get_line`/`restart`, sample clamping, status codes.
- Contributor lists are `Rc<Vec<ContribList>>`; passing `get_clist_x/y()` of another resampler
  is the C "forced" (shared, not freed) list. `destroy()` / `free_clist()` mirror manual frees.
- C `NULL`/`calloc` failure → `None` / `STATUS_OUT_OF_MEMORY` (via `try_reserve`). Out-of-range
  reads the C code would perform (mismatched shared lists, short lines) read 0 instead of UB.

Tests: `tests/resampler.rs` vs `tests/resources/golden/resampler.txt`
(`scripts/golden/gen_resampler_golden.sh`): 16 filters × 10 size pairs × 3 boundary ops × 4
parameter sets (scale, offsets, clamp range; incl. failing clist builds), shared lists + restart,
filter lookup and default filter — contributor weights and output sample bits match exactly.
