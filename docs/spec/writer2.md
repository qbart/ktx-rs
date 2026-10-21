# writer2

Source: `lib/src/writer2.c`.

- `append_lib_id(&mut HashList)`: appends ` / libktx <LIBKTX_VERSION>` to `KTXwriter`
  (`LIBKTX_DEFAULT_VERSION` when the value contains `__default__`); replaces an older lib id.
  Version = `v<crate version>`, default = `v5.0.__default__` (upstream DEF_VER).
- `set_image_from_stream/_memory/_stdio_stream`: whole level with `KTX_FACESLICE_WHOLE_LEVEL`.
- `write_to_stream(&mut self, &mut dyn Stream)` (mutates metadata like C: lib id + sort),
  `write_to_stdio_stream(File)`, `write_to_named_file`, `write_to_memory() -> Vec<u8>`.
  Rejects lowercase `ktx*` keys and unknown `KTX*` keys (`InvalidOperation`).
- `deflate_zstd(level)` (> 22 -> `InvalidValue`), `deflate_zlib(level)`; alignment becomes 1.
- Gap: `ruzstd` encodes only at its fastest level; levels are accepted but not honoured.
- Test hook `set_disable_writer_metadata` is thread-local (`__disableWriterMetadata__`).

Status: done. Tests: `tests/texturetests.rs` (supercompressed iterate, round trip).
