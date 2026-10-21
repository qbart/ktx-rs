# Conventions

- C `KTX_error_code` -> `KtxResult<T> = Result<T, ErrorCode>`; `KTX_SUCCESS` is `Ok`.
- No `unwrap`/`expect`/`panic` (clippy deny). Slice access on file data uses checked `get`.
- C names map to snake_case: `ktxHashList_AddKVPair` -> `HashList::add_kv_pair`.
- Open C enums read from files (`VkFormat`, `ktxSupercmpScheme`, transcode fmt)
  are `#[repr(transparent)]` `u32` newtypes with associated consts.
- Explicit destroy: objects expose `destroy(self)` mirroring `*_Destroy`; `Drop` also frees.
- Generated tables come from `scripts/gen_*.py` reading `./ref/ktx`.
- Ported tests live in `tests/<original_file_stem>.rs`, test names follow the gtest names.
