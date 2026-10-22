# texture2 (ktxTexture2)

Source: `lib/src/texture2.c`, `texture2.h`, `miniz_wrapper.cpp`.

- `Texture2 { vk_format, p_dfd, supercompression_scheme, is_video, duration,
  timescale, loopcount, private: Texture2Private }`.
- Create: `create`, `create_copy(&mut orig)`, `create_from_stream(Box<dyn Stream>)`,
  `create_from_stdio_stream(File)`, `create_from_named_file`, `create_from_memory`
  (keeps an owned copy only when image data is deferred).
- Reading validates as C: level/DFD/KVD/SGD bounds vs file size, DFD via
  `FormatSize::init_from_dfd`, transfer/sRGB, BasisLZ/UASTC rules, glTF basisu flag.
- `LOAD_IMAGE_DATA`, `load_image_data`, `load_deflated_image_data`,
  `iterate_load_level_faces`: stream is consumed on success, restored on error.
- Zstd: `ruzstd` decoder. Zlib: deflate via `basisu::miniz` (tdefl port), inflate via `miniz_oxide` (`zlib.rs` =
  miniz_wrapper port).
- Allocations sized from file data use `try_reserve` -> `OutOfMemory`, no abort.
- Divergence: `KTXanimData` parsed as 3 LE u32 (C passes the array as `void**`).
- Queries: components, transfer/primaries/model, premultiplied, transcodable, HDR,
  setters for transfer/OETF/primaries.

Status: read path done. Todo: writer2, deflate, SetImageFromMemory, transcode, compress.
