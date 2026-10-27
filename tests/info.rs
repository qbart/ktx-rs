// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `ktxPrintInfo*` (lib/src/info.c) vs the original C implementation.
//!
//! Not a C test port (libktx has no info test): the reference output comes
//! from `scripts/golden/gen_info_golden.sh`, which runs the C print functions
//! on every KTX/KTX2 test file (KTX2 also as text, indented JSON, minified JSON
//! and truncated copies) and is stored in `tests/resources/golden/info.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::KtxResult;
use ktx::info::{
    ktx_write_info_for_stream, ktx_write_ktx1_info_text_for_stream,
    ktx_write_ktx2_info_json_for_stream, ktx_write_ktx2_info_text_for_stream,
};
use ktx::stream::{MemStream, Stream};

fn rc(out: &mut Vec<u8>, r: KtxResult<()>) {
    let code = match r {
        Ok(()) => 0,
        Err(e) => e as u32,
    };
    out.extend_from_slice(format!("-- rc {code}\n").as_bytes());
}

fn on_memory(
    out: &mut Vec<u8>,
    d: &[u8],
    f: impl FnOnce(&mut Vec<u8>, &mut dyn Stream) -> KtxResult<()>,
) {
    let r = MemStream::construct_ro(d).and_then(|mut s| f(out, &mut s));
    rc(out, r);
}

fn ktx2(out: &mut Vec<u8>, d: &[u8]) {
    out.extend_from_slice(b"-- text\n");
    on_memory(out, d, ktx_write_ktx2_info_text_for_stream);
    out.extend_from_slice(b"-- json\n");
    on_memory(out, d, |o, s| {
        ktx_write_ktx2_info_json_for_stream(o, s, 1, 4, false)
    });
    out.extend_from_slice(b"-- minified\n");
    on_memory(out, d, |o, s| {
        ktx_write_ktx2_info_json_for_stream(o, s, 1, 4, true)
    });
}

/// Mirrors scripts/golden/info_golden.c.
fn render(name: &str, d: &[u8]) -> Vec<u8> {
    let mut out = format!("== {name}\n").into_bytes();
    on_memory(&mut out, d, ktx_write_info_for_stream);
    if d.len() > 12 && d[5] == b'2' {
        ktx2(&mut out, d);
        for cut in [100, 200, d.len() / 2, d.len() - 1] {
            if cut >= d.len() {
                continue;
            }
            out.extend_from_slice(format!("-- truncated {cut}\n").as_bytes());
            let t = d.get(..cut).unwrap_or_default();
            on_memory(&mut out, t, ktx_write_info_for_stream);
            ktx2(&mut out, t);
        }
    } else {
        out.extend_from_slice(b"-- ktx1\n");
        on_memory(&mut out, d, ktx_write_ktx1_info_text_for_stream);
    }
    out
}

fn read(path: &std::path::Path) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

/// Corrupt variants, as built by `synthetic()` in info_golden.c.
fn synthetic(out: &mut Vec<u8>) {
    let res = common::resources_path();
    let mut d = read(&res.join("ktx").join("r8g8b8a8_srgb.ktx"));
    for o in (12..=64).step_by(4) {
        d[o..o + 4].reverse(); // header + imageSize
    }
    out.extend(render("synthetic_big_endian.ktx", &d));
    let mut d = read(&res.join("ktx").join("orient_up_metadata.ktx"));
    d[64] = 0xFF; // first key/value size
    d[65] = 0xFF;
    out.extend(render("synthetic_bad_kvd.ktx", &d));
    let mut d = read(&common::ktx2_path().join("r8g8b8a8_srgb.ktx2"));
    let dfd_ofs = u32::from_le_bytes([d[48], d[49], d[50], d[51]]) as usize;
    d[dfd_ofs] ^= 4;
    out.extend(render("synthetic_bad_dfd_size.ktx2", &d));
    d[0] ^= 1;
    out.extend(render("synthetic_bad_identifier.ktx2", &d));
}

fn list(dir: &std::path::Path, ext: &str) -> Vec<std::path::PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(d) => d
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .collect(),
        Err(e) => panic!("{}: {e}", dir.display()),
    }
}

#[test]
fn print_info_matches_libktx() {
    let golden_path = common::resources_path().join("golden").join("info.txt");
    let golden = match std::fs::read(&golden_path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", golden_path.display()),
    };
    let mut files = list(&common::resources_path().join("ktx"), "ktx");
    files.extend(list(&common::ktx2_path(), "ktx2"));
    files.sort();
    assert!(files.len() > 50);

    let mut all = Vec::new();
    synthetic(&mut all);
    for path in &files {
        let d = read(path);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        all.extend(render(name, &d));
    }
    if all != golden {
        let a = String::from_utf8_lossy(&all);
        let g = String::from_utf8_lossy(&golden);
        let (n, (x, y)) = a
            .lines()
            .zip(g.lines())
            .enumerate()
            .find(|(_, (x, y))| x != y)
            .unwrap_or((0, ("<length>", "<length>")));
        panic!("line {}: got {x:?}, want {y:?}", n + 1);
    }
}

/// Garbage after a valid identifier never panics.
#[test]
fn print_info_handles_garbage() {
    let mut seed = 0x1234_5678u32;
    for len in [12usize, 64, 80, 81, 100, 200, 400] {
        for ident in [ktx::KTX_IDENTIFIER_REF, ktx::KTX2_IDENTIFIER_REF] {
            for _ in 0..50 {
                let mut d: Vec<u8> = (0..len)
                    .map(|_| {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        (seed >> 24) as u8
                    })
                    .collect();
                d[..12].copy_from_slice(&ident);
                let mut out = Vec::new();
                on_memory(&mut out, &d, ktx_write_info_for_stream);
                on_memory(&mut out, &d, |o, s| {
                    ktx_write_ktx2_info_json_for_stream(o, s, 0, 2, false)
                });
                assert!(!out.is_empty());
            }
        }
    }
}
