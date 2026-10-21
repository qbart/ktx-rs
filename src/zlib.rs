//! Port of lib/src/miniz_wrapper.cpp (zlib via the pure Rust `miniz_oxide`) and
//! the zstd helpers used by texture2.c (via the pure Rust `ruzstd`).

use crate::error::{ErrorCode, KtxResult};

/// Port of `ktxCompressZLIBBounds` (`mz_compressBound`).
pub fn ktx_compress_zlib_bounds(src_length: usize) -> usize {
    let a = 128usize.saturating_add(src_length.saturating_mul(110) / 100);
    let b = 128usize
        .saturating_add(src_length)
        .saturating_add((src_length / (31 * 1024) + 1).saturating_mul(5));
    a.max(b)
}

/// Port of `ktxCompressZLIBInt`. Returns the compressed bytes.
/// `dest_capacity` mirrors `*pDestLength` on input.
pub fn ktx_compress_zlib_int(src: &[u8], dest_capacity: usize, level: u32) -> KtxResult<Vec<u8>> {
    if (src.len() as u64 | dest_capacity as u64) > 0xFFFF_FFFF {
        return Err(ErrorCode::InvalidValue);
    }
    if level > 10 {
        return Err(ErrorCode::InvalidValue);
    }
    let out = miniz_oxide::deflate::compress_to_vec_zlib(src, level as u8);
    if out.len() > dest_capacity {
        return Err(ErrorCode::OutOfMemory);
    }
    Ok(out)
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
