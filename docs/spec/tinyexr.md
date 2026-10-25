# tinyexr (basis_universal encoder/3rdparty/tinyexr.h)

Module `ktx::tinyexr` (`mod.rs` writer + constants, `load.rs` reader, `piz.rs` PIZ decoder).
Configuration of the KTX build: miniz (basisu `mz_uncompress`), PIZ on, ZFP / threads / OpenMP
off, little endian.

## Writer

Source: `SaveEXRImageToMemory` -> `SaveEXRNPartImageToMemory`, `EncodeChunk`, `EncodePixelData`,
`WriteChannelInfo`, `WriteAttributeToMemory`, `half_to_float`, `float_to_half_full`.

- `save_exr_image_to_memory(width, height, channels, custom_attributes)`: single part,
  scanline, `TINYEXR_COMPRESSIONTYPE_NONE` (what `ktx extract` uses).
  Header: magic + version `2,0,0,0`, `channels` (requested type, p_linear 0, sampling 1x1),
  `compression`, `dataWindow` = `displayWindow`, `lineOrder` 0, `pixelAspectRatio` 1,
  `screenWindowCenter` (0,0), `screenWindowWidth` 1, custom attributes, end byte; offset table;
  one chunk per scanline (y, size, channels in header order).
- Pixel conversions: HALF->FLOAT, HALF->HALF, FLOAT->HALF (`float_to_half_full`, round half
  up, NaN -> 0x7e00), FLOAT->FLOAT, UINT->UINT; other pairs give "Invalid requested_pixel_type."
- Not ported: tiles, multi-part, deep images, ZIP/RLE/PIZ/ZFP compression on the writer side.

## Reader

Source: `ParseEXRVersionFromMemory`, `ParseEXRHeaderFromMemory` (`ParseEXRHeader`,
`ReadAttribute`, `ReadChannelInfo`, `ConvertHeader`), `LoadEXRImageFromMemory`
(`DecodeEXRImage`, offsets / `ReconstructLineOffsets` / `ReconstructTileOffsets`, `DecodeChunk`,
`DecodeTiledLevel`, `DecodeTiledPixelData`, `DecodePixelData`, `AllocateImage`,
`DecompressZip`, `DecompressRle` / `rleUncompress`, `DecompressPiz` with `hufUncompress`,
`wav2Decode`, LUT), `InitEXRHeader/Image`, `FreeEXRHeader/Image/ErrorMessage`,
`EXRSetNameAttr`, `EXRNumLevels`.

- Structs `ExrVersion`, `ExrHeader`, `ExrChannelInfo`, `ExrImage`, `ExrTile`, `ExrBox2i`,
  `ExrAttribute` mirror the C structs; arrays are `Vec`s (`num_channels()` /
  `num_custom_attributes()` are their lengths), C strings are byte vectors (truncated to 255),
  pixel data is little endian bytes per channel (HALF 2 bytes, else 4). `NULL` pointers are
  `None`; `free_exr_image` drops levels, tiles and images. `err` is `&mut Option<String>`
  (`SetErrorMessage` replaces it, untouched when C leaves it untouched).
- Kept C behavior: `ConvertHeader` runs after a failed `ParseEXRHeader` (partial header,
  parse error returned); duplicate `channels` attributes append; `hufUncompress` ignores the
  results of table unpacking / building / decoding; the uncompressed path writes rows by block
  index; the HALF->HALF uncompressed path has no chunk bounds check (reads following file
  bytes); `DecodeChunk` writes the requested types into `pixel_types`; scanline blocks keep
  decoding after an invalid one; on failure the whole image is freed.
- Divergences (all undefined behavior or aborts in C): reads past the end of the file give 0;
  writes outside an image are dropped; a zero tile size (division by zero) fails tile info;
  allocations that fail or exceed 4 GiB fail like the C allocation checks (C would use a
  `NULL` buffer or throw `bad_alloc`); the offset table is read before it is allocated, so a
  corrupted chunk count never allocates more than the file (same results). Pixels the file
  does not cover are 0 (C leaves `malloc` memory uninitialized).
- Not ported: file variants, multi-part (`LoadEXRMultipartImageFromMemory`), deep images,
  layers / `LoadEXR*` RGBA helpers, ZFP.

Tests: `tests/tinyexr.rs` (writer, 122 cases vs `scripts/golden/gen_tinyexr_golden.sh`);
`tests/tinyexr_load.rs` (reader, 8350 cases vs `gen_tinyexr_load_golden.sh`: 619 synthetic
files written by tinyexr — scanline and tiled one level / mipmap / ripmap, all compressions,
HALF / FLOAT / UINT, line order 1, zeroed offsets — plus Desk.exr (PIZ) and the vendored CTS
EXR inputs, in 3-4 requested type / line order variants, byte flips and truncations; the C runs
each case in a forked ASan child, the 3 cases it crashes on are only checked for no panic;
random corruptions never panic).
