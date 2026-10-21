# texture1 / writer1 (ktxTexture1)

Source: `lib/src/texture1.c`, `writer1.c`.

- `Texture1 { gl_format, gl_internalformat, gl_base_internalformat, gl_type, private }`,
  derefs to `Texture`. Private: `need_swap`, `rgba_astc_mapping` (default HDR), `data_offset`.
- Create: `create`, `create_from_stream/stdio_stream/named_file/memory`.
  Unknown/legacy GL formats -> `UnsupportedTextureType`.
- Loading reads `imageSize` + images sequentially from `data_offset` (C relies on the
  stream position; recorded so deferred loads from an owned copy work). The stream is
  consumed by `load_image_data` / `iterate_load_level_faces` even on error (as C).
- Orientation parsed by emulating `sscanf("S=%c,T=%c,R=%c")`.
- Writer: `set_image_from_*` (pads rows to 4), `write_to_*` (KTX1),
  `write_ktx2_to_*` (convert to KTX2: VkFormat from GL, DFD via vk2dfd, orientation
  rewritten, lib id appended, rows unpadded).
- `AnyTexture` = `ktxTexture_CreateFrom*` result (enum of both classes).
- Divergences: KVD count swap for opposite-endian files advances past the size field
  (C bug); `IterateLoadLevelFaces` stops at the first callback error (C ignores it);
  KTX2 conversion advances the source by the padded KTX1 image size (C uses unpadded).

Status: complete. Tests: `tests/texturetests.rs` (Unicode, KTX1 round trip + convert).
