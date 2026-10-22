//! `ktx validate` validator robustness: corrupted files never panic.
//!
//! The CTS (tests/cts.rs) checks the validator output against the original
//! tool; this test feeds byte-flipped copies of every KTX2 test file through
//! `validate_memory` in-process (debug builds also catch arithmetic overflow).
#![allow(clippy::panic)]

mod common;

use ktx::cli::validate::validate_memory;

#[test]
fn validate_corrupted_files_never_panics() {
    let mut files: Vec<_> = match std::fs::read_dir(common::ktx2_path()) {
        Ok(d) => d
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ktx2"))
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 300_000))
            .collect(),
        Err(e) => panic!("{e}"),
    };
    files.sort();
    let mut seed = 0x9E37_79B9u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut runs = 0;
    for path in &files {
        let data = std::fs::read(path).unwrap_or_default();
        let mut issues = 0usize;
        let rc = validate_memory(&data, false, true, &mut |_| issues += 1);
        assert!(matches!(rc, Ok(0 | 3)), "{}: {rc:?}", path.display());
        for _ in 0..15 {
            let mut d = data.clone();
            // Corrupt the header/index/DFD/KVD region where parsing happens.
            let span = d.len().min(1024);
            for _ in 0..(1 + rnd() % 8) {
                let i = rnd() as usize % span;
                d[i] = rnd() as u8;
            }
            if rnd() % 4 == 0 {
                d.truncate(rnd() as usize % d.len().max(1));
            }
            let _ = validate_memory(&d, rnd() % 2 == 0, rnd() % 2 == 0, &mut |_| {});
            runs += 1;
        }
    }
    assert!(runs > 200);
}
