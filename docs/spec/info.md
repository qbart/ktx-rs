# info (ktxPrintInfo)

Source: `lib/src/info.c`, `lib/src/glformat_str.c`.

- `glformat_str.rs` generated (`scripts/gen_glformat_str.py`, GL values from
  `other_include/GL*`): `gl_format_string`, `gl_internalformat_string`, `gl_type_string`;
  unknown values print as C `%#x` (`strings::c_hash_x`).
- `info.rs`: every `ktxPrint*` function writes to stdout; each has a `ktx_write_*` twin
  appending the same bytes to a `Vec<u8>` (keys/values are raw bytes, not UTF-8).
  - `ktx_{print,write}_info_for_stream` (KTX1 or KTX2), `..._ktx1_info_text_for_stream`,
    `..._ktx2_info_text_for_stream`, `..._ktx2_info_json_for_stream(base_indent, indent_width,
    minified)`; `_for_memory`, `_for_stdio_stream(&File)` (cursor shared, file kept open),
    `_for_named_file` print variants.
  - `ktx_print_info_for_named_file` is a no-op returning `Ok`, like C (TODO there).
  - Helpers kept public: `write_kv_data[_json]`, `write_identifier`, `write_ktx[2]_header`,
    `write_level_index`, `write_basis_sgd_info`, `write_uastc_hdr_6x6_intermediate_sgd_info`,
    `is_known_key_value*`.
- C quirks kept: `%d` of signed `char` value bytes, `PRId64` (signed) for u64 lengths, `%#x`
  of 0 is `0`, 32-bit `24 * numLevels` and SGD image-count math wrap, shift >= 32 masks like
  x86, at most 100 KV entries / 64 levels, KTX1 header printed before byte swap, KTX1 read
  errors ignored (missing bytes = 0, unreadable KVD = parse failure), JSON with a BasisLZ
  SGD shorter than its header closes that object and returns early.
- Reads grow buffers chunk-wise so garbage lengths cannot force huge allocations.

Status: done.
Tests: `tests/info.rs` — output and return codes for every KTX/KTX2 test file (KTX2 also
JSON, minified, 4 truncations) plus synthetic big-endian KTX1, bad KVD, bad DFD size and bad
identifier, byte exact vs `tests/resources/golden/info.txt` (`scripts/golden/gen_info_golden.sh`);
random garbage after valid identifiers never panics.
