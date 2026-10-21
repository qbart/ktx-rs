# texture (ktxTexture base)

Source: `lib/src/texture.c`, `texture.h`, `formatsize.h`, `ktx.h`.

- `Texture` = `KTXTEXTURECLASSDEFN` fields (snake_case) + `protected`
  (`FormatSize`, `type_size`, `stream: Option<Box<dyn Stream>>`).
  `p_data: Option<Vec<u8>>`, `data_size` kept separately as in C.
- Subclasses (`Texture2`, later `Texture1`) `Deref` to `Texture`.
- Virtual table -> `trait KtxTexture` (`calc_*`, `get_image_offset`, `get_*_size`,
  `iterate_levels`, `iterate_load_level_faces`, `load_image_data`, `needs_transcoding`);
  `iterate_level_faces`, `calc_data_size_texture` are default methods.
- Iteration callback `PFNKTXITERCB` -> `&mut dyn FnMut(level, face, w, h, d, size, &[u8]) -> KtxResult<()>`.
- `calc_image_size` uses integer `div_ceil` instead of `ceilf` (exact for all u32).
- `TextureCreateInfo.dfd: Option<Vec<u32>>` replaces `pDfd`.
- Create flags/storage enums mirror C names.

Status: base done (no writer/GL). Tests: `tests/texturetests.rs`.
