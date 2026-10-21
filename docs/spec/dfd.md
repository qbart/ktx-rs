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

- `interpret_dfd(dfd, &mut r, &mut g, &mut b, &mut a, &mut word_bytes) -> u32` (`I_*` flags / errors).
  Shifts by >= 32 wrap like x86 (C UB).
- queries: `get_dfd_component_info_unpacked -> (n, len)`, `get_dfd_num_components`,
  `reconstruct_dfd_bytes_plane0_from_samples`, `reconstruct_dfd_bytes_planes_from_samples`.
- colourspaces: `Primaries`, `find_mapping`, `get_primaries -> Option`.
- `vk2dfd(VkFormat) -> Option<Vec<u32>>`, `dfd2vk(&[u32]) -> VkFormat` generated from the
  `.inl` files by `scripts/gen_dfd_tables.py` (C subset translator). Vulkan `_EXT` alias
  names map to core names.

Status: done except printdfd.
Tests: `tests/unittests.rs::{create_dfd_test, dfd_vk_format_list_test}`.
