// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Error codes. Port of `ktx_error_code_e` and `ktxErrorString` (lib/src/strings.c).
//!
//! The C API returns `KTX_error_code`; here every fallible call returns
//! [`KtxResult<T>`]. `KTX_SUCCESS` is represented by `Ok(..)`.

use core::fmt;

/// Port of `ktx_error_code_e` without `KTX_SUCCESS` (which is `Ok`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ErrorCode {
    /// The data in the file is inconsistent with the spec.
    FileDataError = 1,
    /// The file is a pipe or named pipe.
    FileIsPipe = 2,
    /// The target file could not be opened.
    FileOpenFailed = 3,
    /// The operation would exceed the max file size.
    FileOverflow = 4,
    /// An error occurred while reading from the file.
    FileReadError = 5,
    /// An error occurred while seeking in the file.
    FileSeekError = 6,
    /// File does not have enough data to satisfy request.
    FileUnexpectedEof = 7,
    /// An error occurred while writing to the file.
    FileWriteError = 8,
    /// GL operations resulted in an error.
    GlError = 9,
    /// The operation is not allowed in the current state.
    InvalidOperation = 10,
    /// A parameter value was not valid.
    InvalidValue = 11,
    /// Requested metadata key or required dynamically loaded GPU function was not found.
    NotFound = 12,
    /// Not enough memory to complete the operation.
    OutOfMemory = 13,
    /// Transcoding of block compressed texture failed.
    TranscodeFailed = 14,
    /// The file not a KTX file.
    UnknownFileFormat = 15,
    /// The KTX file specifies an unsupported texture type.
    UnsupportedTextureType = 16,
    /// Feature not included in in-use library or not yet implemented.
    UnsupportedFeature = 17,
    /// Library dependency (OpenGL or Vulkan) not linked into application.
    LibraryNotLinked = 18,
    /// Decompressed byte count does not match expected byte size.
    DecompressLengthError = 19,
    /// Checksum mismatch when decompressing.
    DecompressChecksumError = 20,
}

/// Port of `KTX_ERROR_MAX_ENUM`.
pub const KTX_ERROR_MAX_ENUM: u32 = ErrorCode::DecompressChecksumError as u32;

/// Result type used across the library.
pub type KtxResult<T> = Result<T, ErrorCode>;

const ERROR_STRINGS: [&str; KTX_ERROR_MAX_ENUM as usize + 1] = [
    "Operation succeeded.",
    "File data is inconsistent with KTX spec.",
    "File is a pipe; seek operations not possible.",
    "File open failed.",
    "Operation would exceed the max file size.",
    "File read error.",
    "File seek error.",
    "File does not have enough data for request.",
    "File write error.",
    "GL error occurred.",
    "Operation not allowed in the current state.",
    "Invalid parameter value.",
    "Metadata key or loader-required GPU function not found.",
    "Out of memory.",
    "Transcoding of block compressed texture failed.",
    "Not a KTX file.",
    "Texture type not supported.",
    "Feature not included in in-use library or not yet implemented.",
    "Library dependency (OpenGL or Vulkan) not linked into application.",
    "Decompressed byte count does not match expected byte size",
    "Checksum mismatch when decompressing",
];

impl ErrorCode {
    /// Numeric value as in the C enum.
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Converts a C error code value. `0` (`KTX_SUCCESS`) and unknown values give `None`.
    pub fn from_u32(v: u32) -> Option<ErrorCode> {
        use ErrorCode::*;
        Some(match v {
            1 => FileDataError,
            2 => FileIsPipe,
            3 => FileOpenFailed,
            4 => FileOverflow,
            5 => FileReadError,
            6 => FileSeekError,
            7 => FileUnexpectedEof,
            8 => FileWriteError,
            9 => GlError,
            10 => InvalidOperation,
            11 => InvalidValue,
            12 => NotFound,
            13 => OutOfMemory,
            14 => TranscodeFailed,
            15 => UnknownFileFormat,
            16 => UnsupportedTextureType,
            17 => UnsupportedFeature,
            18 => LibraryNotLinked,
            19 => DecompressLengthError,
            20 => DecompressChecksumError,
            _ => return None,
        })
    }
}

/// Port of `ktxErrorString` taking the raw numeric code (0 = success).
pub fn ktx_error_string_code(code: u32) -> &'static str {
    match ERROR_STRINGS.get(code as usize) {
        Some(s) => s,
        None => "Unrecognized error code",
    }
}

/// Port of `ktxErrorString`.
pub fn ktx_error_string(error: ErrorCode) -> &'static str {
    ktx_error_string_code(error as u32)
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ktx_error_string(*self))
    }
}

impl std::error::Error for ErrorCode {}
