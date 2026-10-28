# JS binding (interface/js_binding)

Crate `ktx-rs-wasm` (`bindings/js`), wasm-bindgen instead of Emscripten embind. One crate, one
cargo feature per upstream module, each built as its own `.wasm`:

| Feature | Upstream target | Output | Factory |
|---|---|---|---|
| `msc` | `msc_basis_transcoder_js` (`transcoder_wrapper.cpp`) | `msc_basis_transcoder.{js,wasm}` | `MSC_TRANSCODER` |
| `read` | `ktx_js_read` (`ktx_wrapper.cpp`) | `libktx_read.{js,wasm}` | `createKtxReadModule` |
| `write` | `ktx_js` (`ktx_wrapper.cpp`, `KTX_FEATURE_WRITE`) | `libktx.{js,wasm}` | `createKtxModule` |

`make js` builds `bindings/js/dist/`, `make test-js` runs the node tests.

## JS layer (embind compatibility)

wasm-bindgen's `--target no-modules` glue is concatenated with `js/embind.js` and the module's
`js/<module>.js`, so existing users of the Emscripten build keep their code:
- Factory `NAME(moduleArg)` returns a Promise of the module object (Emscripten `MODULARIZE`);
  `moduleArg.wasmBinary` / `moduleArg.locateFile` are honoured, default is `<name>.wasm` next
  to the script (browser) or `__dirname` (node). CommonJS `module.exports = NAME`.
- Enums are objects of frozen `{value}` entries plus `.values[n]` (embind `enum_`); functions
  taking enums accept the entry or a plain number.
- Objects have `delete()` / `isDeleted()` (embind) on top of wasm-bindgen's `free()`.
- Returned memory views are copies (`Uint8Array`) instead of views into the wasm heap; they stay
  valid after `delete()`.

## msc_basis_transcoder

- `initTranscoders()`: no-op (the Rust transcoder tables are built lazily).
- `isFormatSupported(TranscodeTarget, TextureFormat)` -> `basis_is_format_supported`.
- `ImageInfo(texFormat, width, height, level)`: `basisu_image_desc` (4x4 blocks), all fields
  read / write properties.
- `BasisLzEtc1sImageTranscoder`: `decodePalettes`, `decodeTables`,
  `transcodeImage(target, slices, imageInfo, decodeFlags = 0, isVideo = false)`.
- `UastcImageTranscoder`: `transcodeImage(target, image, imageInfo, decodeFlags = 0,
  hasAlpha = false, isVideo = false)`.
- Both: static `getBytesPerBlock(target)`; `transcodeImage` returns `{transcodedImage}` or `{}`.
- `BasisTranscoderState` (empty), `TranscodedImage.get_typed_memory_view()`.
- Kept C++ behaviour: output size from `getTranscodedImageByteLength` in `uint32_t` (PVRTC1
  padded to 8x8); the byte size is passed as `output_blocks_buf_size_in_blocks_or_pixels`; each
  transcoder object keeps its own default `basisu_transcoder_state`.
