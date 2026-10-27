# ktx (Rust port of KTX-Software)

An **unofficial Rust port** of [Khronos KTX-Software](https://github.com/KhronosGroup/KTX-Software):
the `libktx` library and the `ktx` command line tool, including the Basis Universal encoders and
transcoders. It is not affiliated with or endorsed by The Khronos Group.

The port follows the C/C++ API and behaviour closely: output is byte identical to the upstream
tools, and every case of the upstream CLI conformance test suite (CTS) passes. The upstream revision
it tracks is recorded in [LATEST_REFERENCE.md](LATEST_REFERENCE.md).

## What is included
- KTX1 / KTX2 reading, writing, validation; zstd and zlib supercompression.
- Basis Universal: ETC1S (Basis-LZ), UASTC, UASTC HDR 4x4 and 6x6i encoders; transcoding to every
  target libktx supports.
- ASTC encoding and decoding (astcenc port), ETC unpack, PNG / JPEG / EXR input.
- GPU upload: Vulkan loader (feature `vulkan`, default) and OpenGL / OpenGL ES loader (feature
  `opengl`, opt in).
- The `ktx` CLI: `create`, `convert`, `encode`, `deflate`, `transcode`, `extract`, `info`,
  `validate`, `compare`, `help`.

Not ported: the legacy tools (`toktx`, `ktx2check`, ...), language bindings, OpenCL encoding, and
ASTC HDR encoding (broken upstream; deferred). See [ROADMAP.md](ROADMAP.md) for the full status.

## Building and installing the CLI

```sh
make build      # ./bin/ktx and its man pages in ./share/man/man1
make install    # ~/bin/ktx and ~/share/man/man1 (PREFIX=/usr/local to change)
```

Or with Cargo: `cargo build --release` (the binary is `target/release/ktx`).

```sh
ktx create --format R8G8B8A8_SRGB --generate-mipmap albedo.png albedo.ktx2
ktx encode --codec basis-lz albedo.ktx2 albedo_blz.ktx2
ktx info albedo_blz.ktx2
ktx help create
```

## Using the library

```rust
use ktx::texture::KtxTexture;
use ktx::{KtxResult, Texture2, TranscodeFmt};

fn transcode_to_bc7(src: &str, dst: &str) -> KtxResult<()> {
    let mut texture =
        Texture2::create_from_named_file(src, ktx::KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)?;
    if texture.needs_transcoding() {
        texture.transcode_basis(TranscodeFmt::BC7_RGBA, 0)?;
    }
    texture.write_to_named_file(dst)
}
```

Functions keep their libktx names in snake case (`ktxTexture2_TranscodeBasis` is
`Texture2::transcode_basis`) and return `KtxResult<T>`, whose error is the libktx `ErrorCode`.
The library never panics on bad input (`unwrap`, `expect` and `panic!` are denied).

## Design notes
- Pure Rust except the allowed C bindings: Vulkan (`ash`) and, behind `opengl`, OpenGL.
- `unsafe` is denied outside the GPU loaders and the astcenc SIMD code.
- Encoders use multiple threads (`--threads`, default: all cores). The ASTC encoder has SSE2,
  SSE4.1 and AVX2 builds and picks one at runtime, so no `target-cpu` flags are needed.
- Implementation notes per component live in [docs/spec](docs/spec).

## Testing

```sh
cargo test --release                    # unit, golden and CTS tests
cargo test --release --features opengl  # plus the OpenGL loader (needs Mesa / EGL)
scripts/bench/bench.sh <path-to-C++-ktx> # speed and output comparison with upstream
```

Test data under `tests/resources` is copied from KTX-Software and its CTS.

## License
Each translated source file keeps the copyright and license of the code it was ported from:
mostly Apache-2.0 (KTX-Software, Basis Universal, astc-encoder), plus Zlib (LodePNG),
BSD-3-Clause (tinyexr) and MIT (cxxopts). The crate license is therefore
`Apache-2.0 AND BSD-3-Clause AND MIT AND Zlib`. See [NOTICE.md](NOTICE.md), [LICENSE](LICENSE),
[LICENSES/](LICENSES) and [REUSE.toml](REUSE.toml).

Test data under `tests/resources` comes from KTX-Software and keeps its original licenses
(recorded in `REUSE.toml`); it is not part of the published crate. The HI Corporation trademark
images (`hi_mark*.ktx`) that upstream ships under a non-open license are left out.
