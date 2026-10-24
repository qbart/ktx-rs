# imageio image.hpp

Source: `tools/imageio/image.hpp` (+ glm `packF2x11_1x10` / `packF3x9_E1x5`, imageio_utility
`convertUNORM/UINT/SINT`, `float_to_half`). Port: `src/cli/imageio/image.rs`.

- Transfer functions: Linear, Gamma, SRGB, ITU (Netpbm continuous BT.709), BT2100 PQ EOTF
  (`powf` like the libstdc++ float overloads).
- `ColorPrimaries` (11 constructors, `transform_to`), `ColorPrimaryTransform`.
- `trait Image` (object safe, `Box<dyn Image>` = `std::unique_ptr<Image>`) and generic
  `ImageT<T, N>` with `pixels: Vec<[T; N]>` for u8/u16/u32/i8/i16/i32/f32 × 1..4 (all aliases).
  `operator uint8_t*` → `as_bytes` / `as_bytes_mut` (bytemuck, pure Rust).
- Getters `get_unorm(_packed)`, `get_sfloat`, `get_b10g11r11`, `get_e5b9g9r9`, `get_(u|s)int(_packed)`;
  `resample` (basisu resampler, sRGB-aware, alpha linear), `yflip`, `transform_color_space`,
  `normalize`, `swizzle`, `copy_to_{r,rg,rgb,rgba}`, `premultiply_alpha`, `map_range`.
- C++ casts emulated as x86-64 GCC (`cvttss2si` 32/64-bit, GCC float→u64), `color::operator[]`
  index clamping, `numeric_limits<float>::min()` clamp in `transformColorSpace`.
  NaN + NaN in the primaries matrix keeps the accumulator's payload like `addss`.
- Deviations: exceptions → `ImageResult` (`resample`); `copyTo*` into an image of a different
  component type (UB cast in C++) returns an error. `getUNORM*` on 32-bit component images
  (C++ shifts by 32, UB) uses the full mask; not exercised by `ktx create`.

Tests: `tests/image.rs` vs `tests/resources/golden/image.txt` (`scripts/golden/gen_image_golden.sh`):
5677 cases over 28 image types incl. NaN/Inf/extreme values — all getters, 11 color space
transforms, normalize, swizzles, copyTo, premultiply, mapRange, yflip, 100 resample variants +
errors, transfer function and primaries tables. All match.
