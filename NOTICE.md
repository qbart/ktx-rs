<!--
SPDX-FileCopyrightText: 2026 KTX Rust port contributors
SPDX-License-Identifier: Apache-2.0
-->

# NOTICE

This is an unofficial Rust port of [KTX-Software](https://github.com/KhronosGroup/KTX-Software)
(libktx and the `ktx` tool), translated from upstream commit
`2e690dd27c7ffaee0fc88966367df47e022e1b5e`. It is not affiliated with or endorsed by The
Khronos Group Inc. KTX and Khronos are trademarks of The Khronos Group Inc.

The Rust sources are translations of the C and C++ sources listed below and have been
modified from the originals. Each translated file carries the copyright and SPDX license
identifier of the code it was translated from. Files without a header are annotated in
`REUSE.toml`. License texts are in [LICENSE](LICENSE) and [LICENSES/](LICENSES).

| Rust port | Translated from | Copyright | License |
|---|---|---|---|
| everything not listed below | KTX-Software (libktx, ktx tool, imageio) | Mark Callow, The Khronos Group Inc. and KTX-Software contributors | Apache-2.0 |
| `src/dfd` | KTX-Software dfdutils | The Khronos Group Inc. | Apache-2.0 |
| `src/astcenc` | astc-encoder | Arm Limited | Apache-2.0 |
| `src/basisu`, `src/jpgd.rs` | Basis Universal (including its jpgd and miniz forks) | Binomial LLC | Apache-2.0 |
| `src/lodepng` | LodePNG | Lode Vandevenne | Zlib |
| `src/tinyexr` | tinyexr (with OpenEXR code) | Syoyo Fujita and contributors; Industrial Light & Magic | BSD-3-Clause |
| `src/cli/options.rs` | cxxopts | Jarryd Beck | MIT |

`src/etcunpack.rs` decodes ETC2/EAC following the Khronos Data Format Specification. It does
not contain code from `etcdec.cxx` (Ericsson AB), which is not open source and is not part of
this repository.

Zstandard and Deflate are provided by the `ruzstd`, `zstd-pure-rs` and `miniz_oxide` crates,
under their own licenses.

## Basis Universal NOTICE

Basis Universal™ Supercompressed GPU Texture Compression Library

Copyright © 2016–2026 Binomial LLC.
All rights reserved except as granted under the
[Apache 2.0 license](https://github.com/BinomialLLC/basis_universal/blob/master/LICENSE).
"Basis Universal" is a trademark of Binomial LLC.

## Test data

`tests/resources` contains test images and expected outputs from KTX-Software, with the
licenses recorded in `REUSE.toml`. These include images under BSD-3-Clause (Industrial Light &
Magic), the Kodak test image license, CC0-1.0 and CC-BY-3.0 (Emil Persson, see the
`readme.txt` next to those images). The test data is not part of the published crate.
