# dfd (dfdutils)

Source: `external/dfdutils/*` (`KHR/khr_df.h`, `dfd.h`, `createdfd.c`, `interpretdfd.c`,
`queries.c`, `vk2dfd.*`, `dfd2vk.*`, `printdfd.c`, `colourspaces.c`).

- DFD is `Vec<u32>`: `dfd[0]` = total bytes, basic descriptor block (BDB) = `dfd[1..]`.
- `khr_df.rs` generated (`scripts/gen_khr_df.py`): all `KHR_DF_*` as `u32`.
- Macros -> fns: `dfdval/dfdsetval(bdb, FIELD)`, `dfdsval/dfdsetsval(bdb, s, FIELD)`,
  `dfdsamplecount`, `dfdsizewords`. Out-of-range read = 0, write ignored.
- `create_dfd_{unpacked,packed,packed_shifted,422,compressed,depth_stencil,alpha}`
  return `Option<Vec<u32>>`; `None` for inputs C asserts on / UB. `(1<<32)-1` saturates.
- `VkSuffix`, `VkCompScheme` enums (C `s_*`, `c_*`).

Status: createdfd done. Todo: vk2dfd/dfd2vk, interpret, queries, print, colourspaces.
Tests: `tests/unittests.rs::create_dfd_test` (15 C tests).
