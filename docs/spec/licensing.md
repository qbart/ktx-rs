# Licensing

The port is a translation, so each file keeps the copyright and license of its source.

- `LICENSE`: Apache-2.0. `LICENSES/`: the texts used in the repository (Apache-2.0,
  BSD-3-Clause, MIT, Zlib, CC0-1.0, CC-BY-3.0, LicenseRef-Kodak), copied from upstream.
- Every `.rs` file starts with `SPDX-FileCopyrightText` / `SPDX-License-Identifier` lines and
  a note that it was translated and modified (Apache-2.0 §4(b), Basis Universal NOTICE, Zlib
  "plainly marked"). Mapping:
  - `src/astcenc`: Arm, Apache-2.0
  - `src/basisu` and `src/jpgd.rs`: Binomial, Apache-2.0
  - `src/lodepng`: Lode Vandevenne, Zlib
  - `src/tinyexr`: Syoyo Fujita and ILM, BSD-3-Clause
  - `src/cli/options.rs`: Jarryd Beck (cxxopts), MIT
  - `src/dfd`: Khronos, Apache-2.0
  - everything else: Mark Callow, Khronos and KTX-Software contributors, Apache-2.0
- `REUSE.toml` annotates files without headers. Rust files rewritten by the `scripts/gen_*.py`
  generators lose their header but stay covered by the path entries. Test data entries mirror
  upstream's `REUSE.toml`.
- `NOTICE.md` lists the components and carries the Basis Universal NOTICE (§4(d)).
- `Cargo.toml`: `license = "Apache-2.0 AND BSD-3-Clause AND MIT AND Zlib"`. `include` keeps
  `tests/`, `scripts/` and `docs/` out of the package. `repository` is
  github.com/qbart/ktx-rs, not the Khronos repository.
- Left out: `hi_mark.ktx` and `hi_mark_sq.ktx` (LicenseRef-HI-Trademark, not open source).
  Their sections were removed from `tests/resources/golden/info.txt`. Ericsson `etcdec` is never
  vendored; `scripts/golden/gen_etcunpack_golden.sh` compiles it from `./ref` only to create
  goldens locally.
