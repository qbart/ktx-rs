// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port-specific (no upstream test): a KTX2 level index whose ranges exceed a
//! 32-bit `usize`, read from a pipe so the file size check cannot catch it.

use ktx::stream::ReaderStream;
use ktx::{ErrorCode, KTX_TEXTURE_CREATE_NO_FLAGS, Texture2};

const LEVEL_INDEX: usize = 80;
const ENTRY: usize = 24;

fn with_level1_lengths_over_4g() -> Vec<u8> {
    let mut file = std::fs::read("tests/resources/ktx2/r8g8b8a8_srgb_mip.ktx2").unwrap_or_default();
    // Level 1 entry: byteOffset, byteLength, uncompressedByteLength (u64 LE).
    for field in [1, 2] {
        let at = LEVEL_INDEX + ENTRY + field * 8;
        if let Some(b) = file.get_mut(at..at + 8) {
            let mut v = [0u8; 8];
            v.copy_from_slice(b);
            b.copy_from_slice(&(u64::from_le_bytes(v) + (1 << 32)).to_le_bytes());
        }
    }
    file
}

#[test]
fn level_index_beyond_32bit_usize_from_pipe() {
    let file = with_level1_lengths_over_4g();
    assert!(file.len() > LEVEL_INDEX + 2 * ENTRY);
    let stream = ReaderStream::construct(std::io::Cursor::new(file));
    let result =
        stream.and_then(|s| Texture2::create_from_stream(Box::new(s), KTX_TEXTURE_CREATE_NO_FLAGS));
    if cfg!(target_pointer_width = "32") {
        assert_eq!(result.err(), Some(ErrorCode::FileOverflow));
    } else {
        // 64-bit: unchanged from C, the header is accepted (no file size for pipes).
        assert!(result.is_ok());
    }
}
