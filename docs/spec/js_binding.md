# JS binding (interface/js_binding)

Crate `ktx-rs-wasm` (`bindings/js`), wasm-bindgen instead of Emscripten embind. One crate, one
cargo feature per upstream module, each built as its own `.wasm`:

| Feature | Upstream target | Output | Factory |
|---|---|---|---|
| `msc` | `msc_basis_transcoder_js` (`transcoder_wrapper.cpp`) | `msc_basis_transcoder.{js,wasm}` | `MSC_TRANSCODER` |
| `read` | `ktx_js_read` (`ktx_wrapper.cpp`) | `libktx_read.{js,wasm}` | `createKtxReadModule` |
| `write` | `ktx_js` (`ktx_wrapper.cpp`, `KTX_FEATURE_WRITE`) | `libktx.{js,wasm}` | `createKtxModule` |

`make js` builds `bindings/js/dist/`, `make test-js` runs the node tests. Upstream's
`class_compat.js` (aliases set from `onRuntimeInitialized`, replacing the user's callback) and
`module_create_compat.js` (`var LIBKTX`) are reproduced for the libktx builds.

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
  valid after `delete()`. Rust `None` is mapped back to `null` where C++ returns `val::null()`.
- Enum entries sharing a value: `values[n]` is the last registered entry, as in embind (e.g.
  `khr_df_primaries.values[1]` is `SRGB`, not `BT709`).

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

## libktx_read / libktx (ktx_wrapper.cpp)

- `texture(data)` (`ktxTexture_CreateFromMemory`, image data loaded), `texture(createInfo,
  storage)` (write build, `ktxTexture2_Create`); overload picked by argument count like embind.
- Properties: `dataSize`, `baseWidth`, `baseHeight`, `colorModel`, `transferFunction`, `oetf`,
  `primaries` (setters in the write build, results ignored), `isSrgb`, `isHDR`,
  `isPremultiplied`, `isTranscodable`, `needsTranscoding`, `numComponents`, `orientation`
  (value_object of enum entries), `supercompressScheme` (upstream spelling), `vkFormat`.
- Methods: `createCopy`, `findKeyValue`, `getImage`, `transcodeBasis`, `decodeAstc`; write
  build: `compressAstc` (1 thread), `compressBasis` (1 thread, `noSSE`), `deflateZstd`,
  `deflateZLIB`, `addKVPairString` (value stored with its NUL), `addKVPairByte`,
  `deleteKVPair`, `setImageFromMemory`, `writeToMemory`. Results are `error_code` entries.
- Write build: `textureCreateInfo`, `astcParams`, `basisParams` (zeroed like embind's
  `constructor<>()`; enum typed fields set with entries, read as numbers; `inputSwizzle` as a
  4 character string), the write enums, `VkFormat` (230 names, generated from `vk_format.inl` by
  `scripts/gen_js_vkformat.py`) and the constants (`FACESLICE_WHOLE_lEVEL` spelling kept).
- Kept C++ behaviour: `error_code.FILE_UNEXPECTED_ERROR` is `KTX_FILE_UNEXPECTED_EOF`; enum
  getters give `undefined` for unregistered values (`colorModel` of ETC1S); `numComponents` /
  `vkFormat` log and return 0 for KTX1; `ERROR: ...` messages go to `console.log` (stdout).
- Deviation: a texture whose creation failed holds no `ktxTexture` (C++ dereferences a null
  pointer); its getters return 0 / false and its methods `INVALID_OPERATION` / `null`.
  `decodeAstc` / `compressAstc` / `compressBasis` on KTX1 return `INVALID_OPERATION` (C++
  casts to `ktxTexture2`).
- `glUpload()`: `ktxTexture_GLUpload` (the `glupload` core shared with the native loader) on
  `js/gl.js`, a port of the part of Emscripten's `library_webgl.js` libktx uses (MIT, see
  NOTICE.md): `Module.GL` (`createContext` honouring `preinitializedWebGLContext`,
  `registerContext`, `makeContextCurrent`, `getContext`, `deleteContext`, the shared
  `getNewId` name counter, `GL.textures`), `glGetString(GL_VERSION)` = `OpenGL ES 3.0 (...)`,
  `GL_MAJOR/MINOR_VERSION` = 3 / 0 (`INVALID_ENUM` on WebGL 1), extensions listed plain and
  `GL_` prefixed, pixel data as the typed array `heapObjectForWebGLType` picks, every GLES
  command present except the 1D ones. Returns `{object: GL.textures[name], target, error}`;
  the capability statics live once per module instance.
- glUpload deviations: without a current context it throws a descriptive `Error` (Emscripten
  fails on the null `GLctx`); a WebGL exception is held until the upload returns and then
  rethrown, so it never unwinds through the wasm frames.

Tests (`bindings/js/tests`, node): API surface of all three modules, `transcodeBasis` /
`getImage` of every image vs `golden/transcode.tsv`, and the write scenarios vs
`golden/write.tsv`, both written by `cargo run -p ktx-rs-wasm --example js_golden [write]`.
`tests/browser` (`make test-js-browser`, chromedriver + headless Chrome, WebGL 2 on
SwiftShader): `glUpload` of 2D mipmapped, 2D array, 3D, cube map (built with the write API),
transcoded ETC1S (`libktx_read.js`) and KTX1 textures, every image read back through a
framebuffer and compared with `getImage`; ETC2 without the WebGL extension (software unpack
fallback); unpack alignment restored; untranscoded BasisLZ refused.
