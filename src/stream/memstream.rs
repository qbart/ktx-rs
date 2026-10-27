// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of lib/src/memstream.c.

use std::borrow::Cow;

use super::{Stream, StreamType, write_len};
use crate::error::{ErrorCode, KtxResult};

/// Default allocation size for a writable [`MemStream`] (`KTX_MEM_DEFAULT_ALLOCATED_SIZE`).
pub const KTX_MEM_DEFAULT_ALLOCATED_SIZE: usize = 256;

#[derive(Debug)]
enum MemData<'a> {
    /// `robytes`: read-only data (borrowed or owned).
    Ro(Cow<'a, [u8]>),
    /// `bytes`: writable buffer; `len()` is `alloc_size`.
    Rw(Vec<u8>),
}

/// Port of `ktxMem` + memory `ktxStream`.
#[derive(Debug)]
pub struct MemStream<'a> {
    data: MemData<'a>,
    alloc_size: usize,
    used_size: usize,
    pos: usize,
}

impl<'a> MemStream<'a> {
    /// Port of `ktxMemStream_construct`: empty, writable, growable stream.
    pub fn construct() -> KtxResult<MemStream<'static>> {
        let mut s = MemStream {
            data: MemData::Rw(Vec::new()),
            alloc_size: 0,
            used_size: 0,
            pos: 0,
        };
        s.expand(KTX_MEM_DEFAULT_ALLOCATED_SIZE)?;
        Ok(s)
    }

    /// Port of `ktxMemStream_construct_ro`: read-only stream over `bytes`.
    pub fn construct_ro(bytes: &'a [u8]) -> KtxResult<MemStream<'a>> {
        if bytes.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        Ok(MemStream {
            alloc_size: bytes.len(),
            used_size: bytes.len(),
            data: MemData::Ro(Cow::Borrowed(bytes)),
            pos: 0,
        })
    }

    /// Read-only stream taking ownership of `bytes`.
    pub fn construct_ro_owned(bytes: Vec<u8>) -> KtxResult<MemStream<'static>> {
        if bytes.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        Ok(MemStream {
            alloc_size: bytes.len(),
            used_size: bytes.len(),
            data: MemData::Ro(Cow::Owned(bytes)),
            pos: 0,
        })
    }

    /// Converts a borrowed read-only stream into one that owns its data.
    pub fn into_owned(self) -> MemStream<'static> {
        let data = match self.data {
            MemData::Ro(c) => MemData::Ro(Cow::Owned(c.into_owned())),
            MemData::Rw(v) => MemData::Rw(v),
        };
        MemStream {
            data,
            alloc_size: self.alloc_size,
            used_size: self.used_size,
            pos: self.pos,
        }
    }

    fn bytes(&self) -> &[u8] {
        match &self.data {
            MemData::Ro(c) => c,
            MemData::Rw(v) => v,
        }
    }

    /// Port of `ktxMemStream_getdata`: the used bytes of the stream.
    pub fn getdata(&self) -> &[u8] {
        let b = self.bytes();
        b.get(..self.used_size).unwrap_or(b)
    }

    /// Takes the written bytes out of the stream (truncated to used size).
    pub fn into_data(self) -> Vec<u8> {
        let used = self.used_size;
        let mut v = match self.data {
            MemData::Ro(c) => c.into_owned(),
            MemData::Rw(v) => v,
        };
        v.truncate(used);
        v
    }

    /// Port of `ktxMem_expand`.
    fn expand(&mut self, newsize: usize) -> KtxResult<()> {
        let mut new_alloc_size = if self.alloc_size == 0 {
            KTX_MEM_DEFAULT_ALLOCATED_SIZE
        } else {
            self.alloc_size
        };
        while new_alloc_size < newsize {
            new_alloc_size = new_alloc_size
                .checked_shl(1)
                .filter(|&n| n > new_alloc_size)
                .unwrap_or(usize::MAX);
        }
        if new_alloc_size == self.alloc_size {
            return Ok(());
        }
        match &mut self.data {
            MemData::Rw(v) => {
                if v.try_reserve_exact(new_alloc_size.saturating_sub(v.len()))
                    .is_err()
                {
                    self.alloc_size = 0;
                    self.used_size = 0;
                    return Err(ErrorCode::OutOfMemory);
                }
                v.resize(new_alloc_size, 0);
            }
            MemData::Ro(_) => return Err(ErrorCode::InvalidOperation),
        }
        self.alloc_size = new_alloc_size;
        Ok(())
    }
}

impl Stream for MemStream<'_> {
    fn read(&mut self, dst: &mut [u8]) -> KtxResult<()> {
        let newpos = self
            .pos
            .checked_add(dst.len())
            .ok_or(ErrorCode::FileUnexpectedEof)?;
        if newpos > self.used_size {
            return Err(ErrorCode::FileUnexpectedEof);
        }
        let src = self
            .bytes()
            .get(self.pos..newpos)
            .ok_or(ErrorCode::FileUnexpectedEof)?;
        dst.copy_from_slice(src);
        self.pos = newpos;
        Ok(())
    }

    fn skip(&mut self, count: u64) -> KtxResult<()> {
        let count = usize::try_from(count).map_err(|_| ErrorCode::FileUnexpectedEof)?;
        let newpos = self
            .pos
            .checked_add(count)
            .ok_or(ErrorCode::FileUnexpectedEof)?;
        if newpos > self.used_size {
            return Err(ErrorCode::FileUnexpectedEof);
        }
        self.pos = newpos;
        Ok(())
    }

    fn write(&mut self, src: &[u8], size: usize, count: usize) -> KtxResult<()> {
        if matches!(self.data, MemData::Ro(_)) {
            return Err(ErrorCode::InvalidOperation);
        }
        let n = write_len(src, size, count)?;
        let new_size = self.pos.checked_add(n).ok_or(ErrorCode::FileOverflow)?;
        if self.alloc_size < new_size {
            self.expand(new_size)?;
        }
        let pos = self.pos;
        if let MemData::Rw(v) = &mut self.data {
            let dst = v.get_mut(pos..new_size).ok_or(ErrorCode::OutOfMemory)?;
            dst.copy_from_slice(src.get(..n).ok_or(ErrorCode::InvalidValue)?);
        }
        self.pos = new_size;
        if self.pos > self.used_size {
            self.used_size = self.pos;
        }
        Ok(())
    }

    fn getpos(&mut self) -> KtxResult<u64> {
        Ok(self.pos as u64)
    }

    fn setpos(&mut self, pos: u64) -> KtxResult<()> {
        let pos = usize::try_from(pos).map_err(|_| ErrorCode::InvalidOperation)?;
        if pos > self.alloc_size {
            return Err(ErrorCode::InvalidOperation);
        }
        self.pos = pos;
        Ok(())
    }

    fn getsize(&mut self) -> KtxResult<u64> {
        Ok(self.used_size as u64)
    }

    fn stream_type(&self) -> StreamType {
        StreamType::Memory
    }
}
