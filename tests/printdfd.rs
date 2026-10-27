// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `printDFD` / `printDFDJSON` vs the original dfdutils printdfd.c.
//!
//! Not a C test port (dfdutils has no print test): the reference text comes
//! from `scripts/golden/gen_printdfd_golden.sh`, which runs the C functions on
//! the DFD of every KTX2 test file (text, indented JSON, minified JSON and a
//! truncated DFD) and is stored in `tests/resources/golden/printdfd.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::dfd::printdfd::{write_dfd, write_dfd_json};

fn read_dfd(path: &std::path::Path) -> Vec<u32> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let u32_at = |o: usize| {
        data.get(o..o + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (ofs, len) = (u32_at(48) as usize, u32_at(52) as usize);
    (0..len / 4).map(|i| u32_at(ofs + i * 4)).collect()
}

#[test]
fn print_dfd_matches_dfdutils() {
    let golden_path = common::resources_path().join("golden").join("printdfd.txt");
    let golden = match std::fs::read_to_string(&golden_path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", golden_path.display()),
    };
    let mut files: Vec<_> = match std::fs::read_dir(common::ktx2_path()) {
        Ok(d) => d
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ktx2"))
            .collect(),
        Err(e) => panic!("{e}"),
    };
    files.sort();
    assert!(!files.is_empty());

    let mut out = String::new();
    for path in &files {
        let dfd = read_dfd(path);
        let len = (dfd.len() * 4) as u32;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        out.push_str(&format!("== {name}\n"));
        write_dfd(&mut out, &dfd, len);
        out.push_str("-- json\n");
        write_dfd_json(&mut out, &dfd, len, 1, 4, false);
        out.push_str("-- minified\n");
        write_dfd_json(&mut out, &dfd, len, 1, 4, true);
        out.push_str("\n-- truncated\n");
        let truncated = len.saturating_sub(20);
        write_dfd(&mut out, &dfd, truncated);
        write_dfd_json(&mut out, &dfd, truncated, 0, 2, false);
    }
    for (i, (a, b)) in out.lines().zip(golden.lines()).enumerate() {
        assert_eq!(a, b, "line {}", i + 1);
    }
    assert_eq!(out, golden);
}

#[test]
fn print_dfd_handles_garbage() {
    // Huge block size, unknown vendor and short buffers must not panic.
    let mut out = String::new();
    for dfd in [
        vec![],
        vec![4],
        vec![0xFFFF_FFFF, 0xFFFF_FFFF, 0x0000_0000],
        vec![200, 0, 0x00FF_0000, 0, 0, 0, 0],
        vec![40, 0x1234, 8 << 16, 1, 2, 3],
    ] {
        let len = (dfd.len() * 4) as u32;
        write_dfd(&mut out, &dfd, len);
        write_dfd(&mut out, &dfd, u32::MAX);
        write_dfd_json(&mut out, &dfd, len, 0, 4, false);
        write_dfd_json(&mut out, &dfd, u32::MAX, 0, 0, true);
    }
    assert!(out.contains("Unknown block"));
}
