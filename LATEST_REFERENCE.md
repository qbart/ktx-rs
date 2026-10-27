# Latest reference

The port tracks this upstream revision (cloned at `./ref/ktx`, not versioned).

- Repository: https://github.com/KhronosGroup/KTX-Software
- Commit: `2e690dd27c7ffaee0fc88966367df47e022e1b5e` (2026-10-21, "Build Windows pyktx wheels with libktx bundled (#1298)")
- Submodule `tests/cts`: `59f0a653d83f8e48d599d7b169fc49bcbf1572e8`

When syncing with a newer upstream, port the diff and update the SHAs here.

## Versioning

The crate version mirrors the upstream KTX-Software release (`version` in `Cargo.toml` and
`macros/Cargo.toml`); `LIBKTX_VERSION` is `v` + that version (e.g. `v4.4.2`), as upstream's
`git describe` tag. Bump it together with the upstream release the port tracks.
