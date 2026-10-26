# OpenGL loader (`glloader.c`, `gl_funcs.c`, `gl_funclist.inl`) -> `src/glloader.rs`

Feature `opengl` (opt-in; upstream `LIBKTX_FEATURE_GL_UPLOAD`). OpenGL is the second allowed C
binding (AGENTS.md): plain `extern "system"` function pointer types, no GL crate; `libloading`
only to search the process. `unsafe` is allowed only here and in `vkloader.rs`.

- `ktx_load_opengl(Option<&mut dyn FnMut(&CStr) -> *const c_void>)` = `ktxLoadOpenGL`: the
  `gl_funclist.inl` commands (required ones -> `KTX_NOT_FOUND` + message), loaded once per
  process. `None`: `ktxLoadOpenGLLibrary` = `dlopen(NULL)` / Windows already loaded
  `libGLESv2.dll`, `libGLES_CM.dll`, `opengl32.dll` + `wglGetProcAddress`
  (`KTX_LIBRARY_NOT_LINKED`). The C "statically linked to GL" check does not apply to Rust.
- `discoverContextCapabilities` (ES / desktop, version via `GL_MAJOR_VERSION` or `sscanf` of
  `GL_VERSION`, sized formats, swizzle, R16 / SNORM, sRGB, cube map arrays, max level, ES 2
  pointer zeroing, `hasExtension` via `glGetStringi` or `GL_EXTENSIONS`), process global like C.
- `ktxTexture_GLUploadPrivate`: target selection (1D/2D/3D, arrays, cube map, cube map array),
  1D / 3D / cube map array rejection, `GL_GENERATE_MIPMAP` vs `glGenerateMipmap`,
  `GL_TEXTURE_MAX_LEVEL`, base internal format substitution for unsized / legacy formats,
  the six image callbacks (layers as height / depth for arrays), ETC software fallback through
  `ktx_unpack_etc` on `INVALID_ENUM` / `INVALID_VALUE` (RGB8 / RGBA8 -> unsized without sized
  formats), GL error reporting (`KTX_GL_ERROR` + `p_glerror`), name deletion on failure.
- `Texture1::gl_upload` (unpack alignment 4), `Texture2::gl_upload` (vk2gl mapping, Basis ->
  `KTX_INVALID_OPERATION`, unpack alignment 1), `AnyTexture::gl_upload`; the previous unpack
  alignment is restored.

Deviations: a missing optional command needed by a texture returns
`KTX_UNSUPPORTED_TEXTURE_TYPE` instead of the C assert; null `glGetString` reads as "".

Tests (Mesa llvmpipe via surfaceless EGL, `tests/gl_common`): `tests/glloader.rs` desktop core
profile with `eglGetProcAddress`: 1D, 1D array, 2D mips, cube map, 2D array, 3D, KTX1 padded R8,
ETC1 software fallback (vs `ktx_unpack_etc`), native ETC2 (compressed bytes read back), generated
mips, user texture name, GL error path, Basis rejection; read back with `glGetTexImage`.
`tests/glloader_es.rs` GL ES 3.0 with the `dlopen(NULL)` lookup: 2D mips and cube map read back
through a framebuffer. Mutant not observable: deleting the generated name on failure.
