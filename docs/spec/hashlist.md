# hashlist

Source: `lib/src/hashlist.c` (uthash based).

- `HashList` = `Vec<HashListEntry>` in insertion order; key stored without NUL.
- Duplicate keys allowed (as uthash). `find_*` returns the most recently added.
- `add_kv_pair(key, value)`: key truncated at first NUL; empty key -> `InvalidValue`.
- `delete_kv_pair` succeeds when key absent. `delete_entry(index)`.
- `serialize`: `u32 keyAndValueByteSize | key\0 | value | pad4`.
- `deserialize`: list must be empty (`InvalidOperation`), empty input `InvalidValue`;
  truncated entry, missing NUL, or BOM-prefixed key -> `FileDataError`; list cleared on error.
- `sort`: stable byte-wise key order (`strcmp`).

Status: complete. Tests: `tests/unittests.rs::hash_list_test`.
