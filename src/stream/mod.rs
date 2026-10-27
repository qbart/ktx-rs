// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of the `ktxStream` interface (ktx.h), `memstream.c` and `filestream.c`.
//!
//! The C function-pointer table becomes the [`Stream`] trait. `destruct` is `Drop`.
//! Custom streams (`eStreamTypeCustom`) are any user type implementing [`Stream`].

mod filestream;
mod memstream;

pub use filestream::{FileStream, ReaderStream};
pub use memstream::MemStream;

use crate::error::KtxResult;

/// Port of `enum streamType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StreamType {
    File = 1,
    Memory = 2,
    Custom = 3,
}

/// Port of `struct ktxStream`.
pub trait Stream {
    /// `ktxStream_read`: read exactly `dst.len()` bytes.
    fn read(&mut self, dst: &mut [u8]) -> KtxResult<()>;
    /// `ktxStream_skip`: skip `count` bytes.
    fn skip(&mut self, count: u64) -> KtxResult<()>;
    /// `ktxStream_write`: write `count` items of `size` bytes from `src`.
    fn write(&mut self, src: &[u8], size: usize, count: usize) -> KtxResult<()>;
    /// `ktxStream_getpos`.
    fn getpos(&mut self) -> KtxResult<u64>;
    /// `ktxStream_setpos`.
    fn setpos(&mut self, pos: u64) -> KtxResult<()>;
    /// `ktxStream_getsize`.
    fn getsize(&mut self) -> KtxResult<u64>;
    /// Stream kind. Defaults to [`StreamType::Custom`].
    fn stream_type(&self) -> StreamType {
        StreamType::Custom
    }
}

/// Convenience: write a whole byte slice (`write(src, 1, len)`).
pub fn write_all(stream: &mut dyn Stream, src: &[u8]) -> KtxResult<()> {
    stream.write(src, 1, src.len())
}

/// Bytes `size * count` checked against `src` length.
pub(crate) fn write_len(src: &[u8], size: usize, count: usize) -> KtxResult<usize> {
    let n = size
        .checked_mul(count)
        .ok_or(crate::ErrorCode::FileOverflow)?;
    if n > src.len() {
        return Err(crate::ErrorCode::InvalidValue);
    }
    Ok(n)
}
