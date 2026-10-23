# imageio (tools/imageio, subset)

Scope: only what `ktx create` needs for the common input formats, PNG, EXR and JPEG, behind an
imageio-like API (`ktx::cli::imageio`). Not ported: the netpbm reader, output plugins,
`image.hpp` / `imagecodec.hpp` image classes (separate), the stream overloads of `open`.

- `formatdesc.h` -> `cli::format_descriptor::FormatDescriptor` (flat basic block fields +
  `channel_count`, `same_unit_all_channels`, `oe_gamma`, ICC name / profile, `Sample`).
  Constructors `new` (channelCount, bits, dt, t, p, m, f), `with_bounds` (custom lower / upper,
  error for float), `from_basic_and_samples`, `from_dfd` (`createFormatDescriptor`); bit field
  widths are masked; `Sample::new` keeps the x86 `1U << 32` result for 32-bit unorm.
  `setTransfer` flips the alpha linear qualifier, `channelBitLength()` / `channelUpper()` /
  `channelBitLength(c)` return the C++ exception texts as `Err`. `operator==` compares basic,
  extended and samples. Not ported: shared exponent and varying-bit custom-bound constructors,
  `updateSample*`, `updateChannelCount`.
- `imageio.h` / `imageinput.cc` / `imageio.cc`: `ImageSpec`, `Origin`, `ImageInputFormatType`,
  `ImageInput` trait with `ImageInputBase` (members), `open(filename, wcb)` with the extension
  plugin first and then every plugin in `std::map` key order, `specificError` only from the
  first try, "No image plugin recognized ..." otherwise; stdin (`-`) is read whole.
  `readImage` / `readScanline` defaults with `convert`, `rescale` (float `roundf`, x86
  `cvttss2si` truncation), `rechannel`, `luminance`. Exceptions -> `ImageError`
  (`DifferentFormat`, `Runtime(what)`, `Stream` = `ios_base::failure`).
- `pnginput.cc`: signature check, `lodepng_decode_chunks` at open, color type -> channel count /
  model (tRNS search from byte 33 to size-1, paletted < 8 bit expanded with a warning), iCCP
  (profile followed by as many zeros, like the C++ `resize` + `insert`), sRGB, gAMA (`oeGamma`),
  cHRM (`findMapping` 0.002); `readImage`: 8 / 16 bit unorm 1-4 channel requests,
  `lodepng_finish_decode`, 16-bit byte swap, sBIT rescale (x86 shift count masking).
- `exrinput.cc`: version / header via the tinyexr port, bit depth / qualifiers / format type
  from the channel types, chromaticities attribute, `readImage` to half / float / uint with the
  R/G/B/A channel lookup (warning for others, defaults 0,0,0,1), scanline and tiled copy, the
  envmap / multi-level / size checks. A null tinyexr error string gives fmt's "string pointer
  is null" like `fmt::format`.

- `jpginput.cc` (`jpg.rs`): `myjpgdstream` over the input stream, header via the jpgd port
  (`JPGD_NOT_JPEG` -> different format, other codes by name), sRGB / BT.709 8-bit spec;
  `readImage` begins decoding and reads scan lines sequentially: [GREY, RGB] -> [GREY, RGB,
  RGBA] (luma with the jpgd weights, alpha 255), 2 channels and non-8-bit requests rejected.

Tests: `tests/imageio.rs`, 3895 cases vs `scripts/golden/gen_imageio_golden.sh` (KTX-Software
imageio built with PNG, EXR and JPEG, ASan child per case): every vendored CTS PNG / EXR input,
Desk.exr, 44 tinyexr-written EXR files, the 23 JPEG test files (scanline / tiled / multi-level, all compressions,
envmap and chromaticities attributes) x 19 target formats; spec, format descriptor, warnings,
errors and pixel hashes match; the 40 C crash cases (tiled EXR into an incompatible type) only
need to not panic.
