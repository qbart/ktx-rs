# stream

Source: `ktx.h` (`ktxStream`), `lib/src/memstream.c`, `lib/src/filestream.c`.

- `trait Stream { read, skip, write(src,size,count), getpos, setpos, getsize, stream_type }`.
  `destruct` = `Drop`. Custom streams implement the trait (`StreamType::Custom`).
- `MemStream<'a>`: `construct()` writable (alloc 256, doubling growth),
  `construct_ro(&[u8])` borrowed / `construct_ro_owned(Vec)`; empty ro input -> `InvalidValue`.
  Write on ro -> `InvalidOperation`. Read past used size -> `FileUnexpectedEof`.
  `setpos` > alloc size -> `InvalidOperation`. `getdata()` = used bytes.
- `FileStream` over `std::fs::File`: setpos beyond size -> `InvalidOperation`;
  fifo/socket size -> `FileIsPipe`; char device -> `InvalidOperation`.
- `ReaderStream<R: Read>`: stdin/pipe behaviour (forward-only setpos, `getsize` -> `FileIsPipe`).

Status: complete. Tests: `tests/unittests.rs::{mem_stream_test,file_stream_test}`.
