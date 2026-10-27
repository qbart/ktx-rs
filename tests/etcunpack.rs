// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `_ktxUnpackETC` vs. the C++ original with etcdec on random ETC1 / ETC2 / EAC images
//! (`scripts/golden/gen_etcunpack_golden.sh`): every source format, odd sizes (cropping),
//! every R16 / sRGB context capability.
#![allow(clippy::panic)]

use ktx::etcunpack::ktx_unpack_etc;
use ktx::gl_format::*;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

#[test]
fn unpack_etc_matches_etcdec() {
    let formats: [(u32, u32); 11] = [
        (GL_ETC1_RGB8_OES, 8),
        (GL_COMPRESSED_RGB8_ETC2, 8),
        (GL_COMPRESSED_SRGB8_ETC2, 8),
        (GL_COMPRESSED_RGBA8_ETC2_EAC, 16),
        (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16),
        (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8),
        (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8),
        (GL_COMPRESSED_R11_EAC, 8),
        (GL_COMPRESSED_SIGNED_R11_EAC, 8),
        (GL_COMPRESSED_RG11_EAC, 16),
        (GL_COMPRESSED_SIGNED_RG11_EAC, 16),
    ];
    let sizes = [(4u32, 4u32), (8, 8), (5, 7), (1, 1), (13, 6), (16, 12)];
    let golden: Vec<&str> = include_str!("resources/golden/etcunpack.txt")
        .lines()
        .collect();
    let mut c = 0u32;
    let mut fails = Vec::new();
    for (fmt, block_bytes) in formats {
        for (w, h) in sizes {
            for r16 in [3u32, 1, 2, 0] {
                for srgb in [false, true] {
                    let mut l = Lcg(c.wrapping_mul(7919).wrapping_add(13));
                    let nb = w.div_ceil(4) * h.div_ceil(4);
                    let src: Vec<u8> = (0..nb * block_bytes).map(|_| l.next() as u8).collect();
                    let got = match ktx_unpack_etc(&src, fmt, w, h, r16, srgb) {
                        Ok(u) => {
                            let mut hash = 0xcbf29ce484222325u64;
                            for b in &u.image {
                                hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
                            }
                            format!(
                                "c{c}\t0 {:#x} {:#x} {:#x} {hash:016x}",
                                u.format, u.internal_format, u.r#type
                            )
                        }
                        Err(e) => format!("c{c}\t{} 0 0 0 cbf29ce484222325", e as i32),
                    };
                    if golden.get(c as usize) != Some(&got.as_str()) {
                        fails.push(format!(
                            "fmt {fmt:#x} {w}x{h} r16 {r16} srgb {srgb}: want {:?} got {got}",
                            golden.get(c as usize)
                        ));
                    }
                    c += 1;
                }
            }
        }
    }
    // Hand-built blocks: H mode with equal base colors (ordering bit), signed EAC base -128.
    let extra: [(u32, &str); 16] = [
        (GL_COMPRESSED_RGB8_ETC2, "2b07ab3a5c8cc1ab"),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "2b07ab3a5c8cc1ab",
        ),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "2b07ab385c8cc1ab",
        ),
        (GL_COMPRESSED_RGB8_ETC2, "49fa49e21b591d75"),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "49fa49e21b591d75",
        ),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "49fa49e01b591d75",
        ),
        (GL_COMPRESSED_RGB8_ETC2, "8714878e85a0bcc1"),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "8714878e85a0bcc1",
        ),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "8714878c85a0bcc1",
        ),
        (GL_COMPRESSED_RGB8_ETC2, "660e66625c8cc1ab"),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "660e66625c8cc1ab",
        ),
        (
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            "660e66605c8cc1ab",
        ),
        (GL_COMPRESSED_SIGNED_R11_EAC, "803a5c8cc1ab2b07"),
        (GL_COMPRESSED_SIGNED_R11_EAC, "80055c8cc1ab2b07"),
        (GL_COMPRESSED_SIGNED_R11_EAC, "81f15c8cc1ab2b07"),
        (
            GL_COMPRESSED_SIGNED_RG11_EAC,
            "8023ffffffffffff7f2a000000000000",
        ),
    ];
    for (i, (fmt, hex)) in extra.iter().enumerate() {
        let src: Vec<u8> = (0..hex.len() / 2)
            .map(|k| u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).unwrap_or(0))
            .collect();
        let got = match ktx_unpack_etc(&src, *fmt, 4, 4, 3, true) {
            Ok(u) => {
                let mut hash = 0xcbf29ce484222325u64;
                for b in &u.image {
                    hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
                }
                format!(
                    "x{i}\t0 {:#x} {:#x} {:#x} {hash:016x}",
                    u.format, u.internal_format, u.r#type
                )
            }
            Err(e) => format!("x{i}\t{} 0 0 0 cbf29ce484222325", e as i32),
        };
        if golden.get(c as usize) != Some(&got.as_str()) {
            fails.push(format!(
                "extra {i}: want {:?} got {got}",
                golden.get(c as usize)
            ));
        }
        c += 1;
    }
    assert_eq!(c as usize, golden.len());
    assert!(
        fails.is_empty(),
        "{} of {c} mismatches:\n{}",
        fails.len(),
        fails[..fails.len().min(20)].join("\n")
    );
}
