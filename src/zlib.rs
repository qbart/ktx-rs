//! Port of lib/src/miniz_wrapper.cpp (deflate: the basisu miniz port in
//! `basisu::miniz`; inflate: the pure Rust `miniz_oxide`) and
//! the zstd helpers used by texture2.c (via the pure Rust `ruzstd`).

use crate::basisu::miniz::{MZ_BUF_ERROR, MZ_PARAM_ERROR, mz_compress_bound, mz_compress2};
use crate::error::{ErrorCode, KtxResult};

/// Port of `ktxCompressZLIBBounds` (`mz_compressBound`).
pub fn ktx_compress_zlib_bounds(src_length: usize) -> usize {
    usize::try_from(mz_compress_bound(src_length as u64)).unwrap_or(usize::MAX)
}

/// Port of `ktxCompressZLIBInt` (basisu miniz `mz_compress2`). Returns the
/// compressed bytes. `dest_capacity` mirrors `*pDestLength` on input.
pub fn ktx_compress_zlib_int(src: &[u8], dest_capacity: usize, level: u32) -> KtxResult<Vec<u8>> {
    if (src.len() as u64 | dest_capacity as u64) > 0xFFFF_FFFF {
        return Err(ErrorCode::InvalidValue);
    }
    match mz_compress2(src, dest_capacity as u64, level as i32) {
        Ok(out) => Ok(out),
        Err(MZ_PARAM_ERROR) => Err(ErrorCode::InvalidValue),
        Err(MZ_BUF_ERROR) => Err(ErrorCode::OutOfMemory),
        Err(_) => Err(ErrorCode::InvalidOperation),
    }
}

/// Port of `ktxUncompressZLIBInt`. Inflates `src` into `dest`, returning the
/// number of bytes written.
pub fn ktx_uncompress_zlib_int(dest: &mut [u8], src: &[u8]) -> KtxResult<usize> {
    if (src.len() as u64 | dest.len() as u64) > 0xFFFF_FFFF {
        return Err(ErrorCode::InvalidValue);
    }
    use miniz_oxide::inflate::TINFLStatus;
    use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
    let mut state = DecompressorOxide::new();
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    let (status, _in_consumed, out_written) = decompress(&mut state, src, dest, 0, flags);
    match status {
        TINFLStatus::Done => Ok(out_written),
        TINFLStatus::HasMoreOutput => Err(ErrorCode::DecompressLengthError),
        _ => Err(ErrorCode::FileDataError),
    }
}

/// Zstd decompression of one or more frames into `dest`. Returns bytes written.
/// Error mapping follows texture2.c: too-small destination is
/// `DecompressLengthError`, anything else `FileDataError`.
pub fn ktx_zstd_decompress(dest: &mut [u8], src: &[u8]) -> KtxResult<usize> {
    use ruzstd::decoding::FrameDecoder;
    use ruzstd::decoding::errors::FrameDecoderError;
    let mut dec = FrameDecoder::new();
    match dec.decode_all(src, dest) {
        Ok(n) => Ok(n),
        Err(FrameDecoderError::TargetTooSmall) => Err(ErrorCode::DecompressLengthError),
        Err(_) => Err(ErrorCode::FileDataError),
    }
}
