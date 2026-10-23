# tinyexr writer (basis_universal encoder/3rdparty/tinyexr.h)

Source: `SaveEXRImageToMemory` -> `SaveEXRNPartImageToMemory`, `EncodeChunk`, `EncodePixelData`,
`WriteChannelInfo`, `WriteAttributeToMemory`, `half_to_float`, `float_to_half_full`.

- `tinyexr::save_exr_image_to_memory(width, height, channels, custom_attributes)`: single part,
  scanline, `TINYEXR_COMPRESSIONTYPE_NONE` (what `ktx extract` uses), little endian like x86.
  Header: magic + version `2,0,0,0`, `channels` (requested type, p_linear 0, sampling 1x1),
  `compression`, `dataWindow` = `displayWindow`, `lineOrder` 0, `pixelAspectRatio` 1,
  `screenWindowCenter` (0,0), `screenWindowWidth` 1, custom attributes, end byte; offset table;
  one chunk per scanline (y, size, channels in header order).
- Pixel conversions: HALF->FLOAT, HALF->HALF, FLOAT->HALF (`float_to_half_full`, round half
  up, NaN -> 0x7e00), FLOAT->FLOAT, UINT->UINT; other pairs give "Invalid requested_pixel_type."
- Not ported: reading, tiles, multi-part, deep images, ZIP/RLE/PIZ/ZFP compression.

Tests: `tests/tinyexr.rs`, 122 cases vs `scripts/golden/gen_tinyexr_golden.sh`.
