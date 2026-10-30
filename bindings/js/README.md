# ktx-rs JS binding

Drop-in replacements for KTX-Software's Emscripten-built JS modules (`interface/js_binding`),
built from the Rust port with wasm-bindgen:

| Files | Factory | Upstream equivalent |
|---|---|---|
| `libktx.js` + `libktx.wasm` | `createKtxModule` | read-write libktx |
| `libktx_read.js` + `libktx_read.wasm` | `createKtxReadModule` | read-only libktx |
| `msc_basis_transcoder.js` + `msc_basis_transcoder.wasm` | `MSC_TRANSCODER` | Basis Universal transcoder |

Keep each `.wasm` next to its `.js` (or pass `wasmBinary` / `locateFile` to the factory), load
the script and call the factory as with upstream:

```html
<script src="libktx.js"></script>
<script>
  createKtxModule().then((ktx) => {
    const texture = new ktx.texture(ktx2Bytes);
    // ...
    texture.delete();
  });
</script>
```

In node: `const createKtxModule = require('./libktx.js');`. API, kept upstream behaviour and the
few deviations: `docs/spec/js_binding.md` in https://github.com/qbart/ktx-rs.

Build from source: `make js` (needs `wasm-bindgen-cli` matching the `wasm-bindgen` crate
version); `make js-zip` builds, tests and packages a release archive.

Licenses: see `NOTICE.md`; the code is Apache-2.0 with Zlib (LodePNG), BSD-3-Clause (tinyexr)
and MIT (cxxopts, `gl.js` from Emscripten) parts.
