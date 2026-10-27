# Latest reference

The port tracks this upstream revision (cloned at `./ref/ktx`, not versioned).

- Repository: https://github.com/KhronosGroup/KTX-Software
- Commit: `2e690dd27c7ffaee0fc88966367df47e022e1b5e` (2026-10-21, "Build Windows pyktx wheels with libktx bundled (#1298)")
- Submodule `tests/cts`: `59f0a653d83f8e48d599d7b169fc49bcbf1572e8`

When syncing with a newer upstream, port the diff and update the SHAs here.

## Versioning

- `LIBKTX_VERSION` (`src/version.rs`, shown by `ktx --version` and written to `KTXwriter`)
  follows upstream: `v4.4.2` for the upstream release, `v4.4.2-1`, `v4.4.2-2`, ... for port
  patches of the same release.
- The crate version (`Cargo.toml`, `macros/Cargo.toml`) is plain SemVer for crates.io: it starts
  at the upstream version (`4.4.2`) and port patches bump the patch number (`4.4.3`, ...).
  On a new upstream release, reset both to it.
