# lodepng encoder (external/lodepng)

Source: `external/lodepng/lodepng.cpp` / `lodepng.h` (version 20250506), encoder side only, as
used by the ktx tools (`lodepng::encode(out, image, w, h, state)`).

- `lodepng::zlib`: `CompressSettings` (default btype 2, LZ77, window 2048, minmatch 3,
  nicematch 128, lazy), boundary package-merge code lengths (`lodepng_huffman_code_lengths`, node
  pool with the same garbage collection, indices instead of pointers), hash chains with the zero
  run second chain, lazy matching, stored / fixed / dynamic blocks (block size
  `clamp(n/8+8, 64K, 256K)`, one hash across blocks), zlib header `78 01` + Adler-32.
- `lodepng`: `ColorMode` (palette is empty or 1024 bytes like the C allocation), `lodepng_convert`
  (the bulk RGBA8/RGB8 converters give the same values as the per-pixel path, so one path is
  used), `ColorStats` + `auto_choose_color` (palette / grey / bit depth / color key), the sBIT
  `allow_convert` rules, all filter strategies (MinSum default, palette / < 8 bit -> zero), Adam7,
  chunks IHDR, sRGB, gAMA, cHRM, sBIT, PLTE, tRNS, IDAT (2^31-1 splits), IEND, CRC-32,
  `lodepng_error_text` (generated). `ColorTree` is a `HashMap<[u8; 4], i32>` (same get / add
  semantics, later adds overwrite).
- Not ported (not used by the tools): decoder, iCCP, cICP, mDCV, cLLI, eXIf, bKGD, pHYs, tIME,
  text chunks, unknown chunks, custom zlib/deflate hooks.
- Divergence: C divides by zero for btype 1 with empty input; the port emits one empty fixed block.

Tests: `tests/lodepng.rs`, 3463 cases vs `scripts/golden/gen_lodepng_golden.sh` (8 image kinds x
11 raw modes x 5 sizes, 17 option variants, zlib streams of the shared synthetic inputs).
