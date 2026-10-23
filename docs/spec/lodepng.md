# lodepng (external/lodepng)

Source: `external/lodepng/lodepng.cpp` / `lodepng.h` (version 20250506, KTX-Software's altered
copy), as used by the ktx tools: `lodepng::encode(out, image, w, h, state)` (extract) and the
split decoder `lodepng_decode_chunks` + `lodepng_finish_decode` (png input of create).

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
- Decoder (`lodepng::decode`, `zlib` inflater): bit reader (`ensureBits9/17/25/32` incl. the
  32-bit shift drop), two-level Huffman tables (`FIRSTBITS` 9, `INVALIDSYMBOL`, error 55 for
  over-subscribed trees), stored / fixed / dynamic blocks, zlib header + Adler-32 (read from the
  last 4 input bytes), `DecompressSettings` / `DecoderSettings` (no custom hooks). PNG:
  `lodepng_inspect` (resets `info_png`), `lodepng_decode_chunks` (all chunk readers: PLTE, tRNS,
  bKGD, tEXt, zTXt, iTXt, tIME, pHYs, gAMA, cHRM, sRGB, iCCP, cICP, mDCV, cLLI, eXIf, sBIT; unknown
  chunk checks 121 / 122 / 69, `remember_unknown_chunks`, CRC only on known chunks, the
  `chunk_next` jump over an embedded PNG signature), `inflateIdat` (expected size, error 91),
  unfilter (the unrolled C predictors give the standard results), padding removal, Adam7,
  `lodepng_finish_decode`, `lodepng_decode`, `decode_memory/32/24`, `lodepng_inspect_chunk`,
  `lodepng_chunk_find_const` / `next_const` on offsets. A PLTE resets all 256 entries to opaque
  black. In-place C buffer reuse is done with copies (same bytes). The IDAT buffer is not
  pre-reserved (C ignores a failed reservation). `lodepng_decode` returns the image, `w`, `h`.
- Not ported: custom zlib/deflate hooks; encoder side of iCCP, cICP, mDCV, cLLI, eXIf, bKGD,
  pHYs, tIME, text and unknown chunks (not used by the tools).
- Divergences: C divides by zero for btype 1 with empty input; the port emits one empty fixed block.
  `lodepng_finish_decode` with conversion converts and frees an unset buffer when inflating fails
  (undefined behavior); the port returns the inflate error after the 56 / 123 checks.

Tests: `tests/lodepng.rs`: encoder 3463 cases vs `scripts/golden/gen_lodepng_golden.sh` (8 image
kinds x 11 raw modes x 5 sizes, 17 option variants, zlib streams of the shared synthetic inputs);
decoder 20800 cases (`lodepng_decode_golden.cpp`: the encoder images in 6 decode modes incl. the
pnginput.cc split path, 61 hand-built extra chunk sets at 2 places x 4 settings, 2112 byte-flip /
truncation corruptions x 4 settings of ignore_crc / adler32 / end / critical), comparing error, size,
output hash and a summary of the decoder state; random corruptions never panic.
