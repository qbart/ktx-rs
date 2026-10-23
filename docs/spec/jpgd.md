# jpgd (basis_universal encoder/jpgd.cpp)

Module `ktx::jpgd`: Rich Geldreich's baseline + progressive JPEG decoder, as used by the
`ktx create` JPEG input (`jpginput.cc`).

- API: `JpegDecoder::new(stream, flags)` (constructor reads up to SOF, check
  `get_error_code`), `begin_decoding`, `decode` -> `(status, scan line)` (RGBA or Y),
  `get_width/height/num_components/bytes_per_pixel/total_bytes_read`;
  `JpegDecoderStream` trait, `JpegDecoderMemStream` (`close`), `jpgd_status` constants and
  `jpgd_status_name`; `decompress_jpeg_image_from_stream / _from_memory` (req comps 1/3/4).
- Errors: `stop_decoding` + `setjmp/longjmp` -> `Result` propagation, the status is stored
  like `m_error_code` and the stream dropped (`free_all_blocks`).
- Ported in full: marker parsing (DHT, DQT 8/16-bit, SOF0/1/2, DRI, SOS, skipping, SOI search
  over 4096 bytes), input buffer with EOI padding and `stuff_char` before the buffer,
  bit readers with and without markers, Huffman `look_up` / `look_up2` / tree, sequential
  decoding with restarts, progressive DC / AC first and refinement scans (1000 scan limit),
  fast-path IDCT (row / column tables), YCbCr conversion for gray, H1V1, H2V1, H1V2, H2V2 with
  and without linear chroma filtering.
- Memory layout quirks kept: `huff_tables` is one word array, so a tree index before the tree
  writes into `code_size` / `look_up2` like the C struct; `m_mcu_org[10]` overflows into the six
  `int` members after it (a scan listing the same component several times).
- Divergences (UB in C): sample / scan-line accesses out of range read 0 / are dropped, pool
  allocations are zeroed `Vec`s, a NULL quantization table reads as zeros, `stuff_char` at the
  start of the padding does not move further back. Integer overflow wraps, shift counts are
  masked like x86.

Tests: `tests/jpgd.rs`, 1012 cases vs `scripts/golden/gen_jpgd_golden.sh` (ASan child per
case): 21 synthetic files from `scripts/golden/make_jpegs.py` (no JPEG encoder is available
here; baseline / progressive with DC successive approximation and AC spectral selection, all
sampling factors, restarts, 16-bit tables, extra markers, 1x1 to 47x5) and two progressive
samples with AC refinement (Iron_Bars), byte flips and truncations, both flag values; status,
sizes, bytes read and the hash of every scan line match; random corruptions never panic.
