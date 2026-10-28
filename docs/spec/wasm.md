# wasm32

Targets: `wasm32-unknown-unknown` (library), `wasm32-wasip1` (library + tests under wasmtime).
`make wasm` builds both feature sets, `make test-wasm` runs the suite (`WASMTIME=` runner,
`ARGS=` cargo flags).

- GPU loaders: `ash` / `libloading` are non-wasm target dependencies; `vkloader` / `glloader`
  are compiled out on wasm, so `vulkan` / `opengl` are no-ops and the default features build.
- Threads: `scoped.rs` replaces `std::thread::scope` for the encoder workers. On wasm each
  "spawned" job runs inline in spawn order; no job blocks on another, so the output equals a
  threaded run with the same thread count (thread count dependent paths such as the threaded VQ
  stay identical to native).
- SIMD: astcenc runs the scalar lane loops (x86-64 kernels are not compiled).
- `tinyexr` `MAX_ALLOC` is `u64` (`1 << 32` overflowed 32-bit `usize`).

Bit exactness vs native (glibc) builds:
- Decoding / transcoding: identical (no libm calls).
- astcenc: identical (embedded sin/cos tables, see astcenc.md).
- Other encoders and colour conversions call libm (`powf`, `log2`, `exp`, `sin`, `cos`) like
  upstream: Basis HDR encoders, quality mapping, Huffman entropy estimate, resampler, transfer
  functions, SSIM. Output is exact per libm, as upstream on each platform; the test data has no
  differences on wasm.
- lodepng: `lodepng_pixel_overflow` works in `size_t`, so corrupt headers with huge dimensions
  fail with 92 on 32-bit, like 32-bit C (`tests/lodepng.rs` accepts that).

Tests: everything except `tests/cts.rs` (WASI cannot spawn `ktx`) and the GPU loader tests runs
under wasmtime; tests use `CARGO_TARGET_TMPDIR` instead of `temp_dir()` / `process::id()`.
