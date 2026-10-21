# errors & strings

Source: `ktx.h` (`ktx_error_code_e`), `lib/src/strings.c`, `strnstr` from `writer2.c`.

- `ErrorCode` values 1..=20 equal the C values; `ktx_error_string_code(0)` = success text.
- `ktx_transcode_format_string`, `ktx_supercompression_scheme_string`,
  `ktx_bu_image_flags_bit_string` (returns `Option`).
- `strnstr(hay, needle, len) -> Option<offset>`.

Status: complete. Tests: `tests/unittests.rs` (strnstr).
