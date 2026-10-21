//! Port of lib/src/filestream.c.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};

use super::{Stream, StreamType, write_len};
use crate::error::{ErrorCode, KtxResult};

fn read_err(e: &std::io::Error) -> ErrorCode {
    if e.kind() == ErrorKind::UnexpectedEof {
        ErrorCode::FileUnexpectedEof
    } else {
        ErrorCode::FileReadError
    }
}

fn write_err(e: &std::io::Error) -> ErrorCode {
    match e.kind() {
        ErrorKind::FileTooLarge | ErrorKind::StorageFull => ErrorCode::FileOverflow,
        _ => ErrorCode::FileWriteError,
    }
}

/// Port of the file `ktxStream` over a seekable [`File`].
#[derive(Debug)]
pub struct FileStream {
    file: File,
}

impl FileStream {
    /// Port of `ktxFileStream_construct`. The file is closed when the stream is dropped.
    pub fn construct(file: File) -> KtxResult<FileStream> {
        Ok(FileStream { file })
    }

    /// Opens `path` for reading.
    pub fn open(path: impl AsRef<std::path::Path>) -> KtxResult<FileStream> {
        File::open(path)
            .map(|file| FileStream { file })
            .map_err(|_| ErrorCode::FileOpenFailed)
    }

    /// Creates/truncates `path` for writing.
    pub fn create(path: impl AsRef<std::path::Path>) -> KtxResult<FileStream> {
        File::create(path)
            .map(|file| FileStream { file })
            .map_err(|_| ErrorCode::FileOpenFailed)
    }

    /// Releases the underlying file.
    pub fn into_inner(self) -> File {
        self.file
    }
}

impl Stream for FileStream {
    fn read(&mut self, dst: &mut [u8]) -> KtxResult<()> {
        self.file.read_exact(dst).map_err(|e| read_err(&e))
    }

    fn skip(&mut self, count: u64) -> KtxResult<()> {
        // C skips with getc so EOF is detected; emulate by checking size.
        let pos = self.getpos()?;
        let size = self.getsize()?;
        let newpos = pos.checked_add(count).ok_or(ErrorCode::FileUnexpectedEof)?;
        if newpos > size {
            // Move to end like getc reaching EOF.
            let _ = self.file.seek(SeekFrom::End(0));
            return Err(ErrorCode::FileUnexpectedEof);
        }
        self.file
            .seek(SeekFrom::Start(newpos))
            .map(|_| ())
            .map_err(|_| ErrorCode::FileReadError)
    }

    fn write(&mut self, src: &[u8], size: usize, count: usize) -> KtxResult<()> {
        let n = write_len(src, size, count)?;
        self.file
            .write_all(src.get(..n).ok_or(ErrorCode::InvalidValue)?)
            .map_err(|e| write_err(&e))
    }

    fn getpos(&mut self) -> KtxResult<u64> {
        self.file
            .stream_position()
            .map_err(|e| match e.raw_os_error() {
                Some(29) => ErrorCode::FileIsPipe,   // ESPIPE
                Some(75) => ErrorCode::FileOverflow, // EOVERFLOW
                _ => ErrorCode::FileSeekError,
            })
    }

    fn setpos(&mut self, pos: u64) -> KtxResult<()> {
        let file_size = self.getsize()?;
        if pos > file_size {
            return Err(ErrorCode::InvalidOperation);
        }
        self.file
            .seek(SeekFrom::Start(pos))
            .map(|_| ())
            .map_err(|_| ErrorCode::FileSeekError)
    }

    fn getsize(&mut self) -> KtxResult<u64> {
        let _ = self.file.flush();
        let md = self.file.metadata().map_err(|_| ErrorCode::FileReadError)?;
        let ft = md.file_type();
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if ft.is_fifo() || ft.is_socket() {
                return Err(ErrorCode::FileIsPipe);
            }
            if ft.is_char_device() {
                return Err(ErrorCode::InvalidOperation);
            }
        }
        if ft.is_dir() {
            return Err(ErrorCode::InvalidOperation);
        }
        Ok(md.len())
    }

    fn stream_type(&self) -> StreamType {
        StreamType::File
    }
}

/// Port of the stdin behaviour of the file stream: a forward-only stream over
/// any [`Read`] (pipes, stdin). `setpos` may only move forward; `getsize` fails.
#[derive(Debug)]
pub struct ReaderStream<R: Read> {
    reader: R,
    readpos: u64,
}

impl<R: Read> ReaderStream<R> {
    /// Wraps `reader`.
    pub fn construct(reader: R) -> KtxResult<ReaderStream<R>> {
        Ok(ReaderStream { reader, readpos: 0 })
    }
}

impl<R: Read> Stream for ReaderStream<R> {
    fn read(&mut self, dst: &mut [u8]) -> KtxResult<()> {
        self.reader.read_exact(dst).map_err(|e| read_err(&e))?;
        self.readpos += dst.len() as u64;
        Ok(())
    }

    fn skip(&mut self, count: u64) -> KtxResult<()> {
        let copied = std::io::copy(&mut (&mut self.reader).take(count), &mut std::io::sink())
            .map_err(|_| ErrorCode::FileReadError)?;
        self.readpos += copied;
        if copied != count {
            return Err(ErrorCode::FileUnexpectedEof);
        }
        Ok(())
    }

    fn write(&mut self, _src: &[u8], _size: usize, _count: usize) -> KtxResult<()> {
        Err(ErrorCode::FileWriteError)
    }

    fn getpos(&mut self) -> KtxResult<u64> {
        Ok(self.readpos)
    }

    fn setpos(&mut self, pos: u64) -> KtxResult<()> {
        if pos > self.readpos {
            self.skip(pos - self.readpos)
        } else {
            Err(ErrorCode::FileIsPipe)
        }
    }

    fn getsize(&mut self) -> KtxResult<u64> {
        Err(ErrorCode::FileIsPipe)
    }

    fn stream_type(&self) -> StreamType {
        StreamType::File
    }
}
