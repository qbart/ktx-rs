// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! tinyexr reader port vs the tinyexr bundled with basis_universal.
//!
//! Not a C test port: `scripts/golden/gen_tinyexr_load_golden.sh` writes the
//! synthetic corpus (`tests/resources/golden/tinyexr_load_corpus.bin`: scanline
//! and tiled one level / mipmap / ripmap images for every compression, pixel
//! type and line order, zeroed offset tables) and the expected result of
//! every case (`tinyexr_load.txt`): the corpus files and the vendored real EXR
//! files in 3-4 requested type / line order variants plus byte flips and
//! truncations, comparing return codes, error messages, the parsed header
//! and hashes of the decoded pixels. Cases where the C code crashes ("CRASH")
//! only have to not panic.
#![allow(clippy::panic)]

mod common;

use common::synthetic::fnv1a;
use ktx::tinyexr::*;
use std::collections::HashMap;
use std::fmt::Write;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

fn fnv_more(data: &[u8], mut h: u64) -> u64 {
    for &b in data {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

/// `q` / `qs` of the harness.
fn q(s: &[u8]) -> String {
    let mut o = String::new();
    for &c in s {
        match c {
            b'\n' => o.push_str("\\n"),
            b'\\' => o.push_str("\\\\"),
            0x20..0x7f => o.push(c as char),
            _ => {
                let _ = write!(o, "\\x{c:02x}");
            }
        }
    }
    o
}

fn qs(s: &Option<String>) -> String {
    s.as_ref().map_or("-".to_string(), |s| q(s.as_bytes()))
}

fn header_summary(h: &ExrHeader) -> String {
    let mut o = String::new();
    let (d, p) = (h.data_window, h.display_window);
    let _ = write!(
        o,
        "dw:{},{},{},{} dp:{},{},{},{} lo:{} c:{} t:{},{},{},{},{} cc:{} hl:{}",
        d.min_x,
        d.min_y,
        d.max_x,
        d.max_y,
        p.min_x,
        p.min_y,
        p.max_x,
        p.max_y,
        h.line_order,
        h.compression_type,
        h.tiled,
        h.tile_size_x,
        h.tile_size_y,
        h.tile_level_mode,
        h.tile_rounding_mode,
        h.chunk_count,
        h.header_len
    );
    let _ = write!(
        o,
        " f:{:08x},{:08x},{:08x},{:08x}",
        h.pixel_aspect_ratio.to_bits(),
        h.screen_window_center[0].to_bits(),
        h.screen_window_center[1].to_bits(),
        h.screen_window_width.to_bits()
    );
    let _ = write!(
        o,
        " v:{},{},{} n:{} ch:{}",
        h.long_name,
        h.non_image,
        h.multipart,
        q(&h.name),
        h.num_channels()
    );
    for (c, ch) in h.channels.iter().enumerate() {
        let _ = write!(
            o,
            " {}:{}:{}:{}:{}:{}:{}",
            q(&ch.name),
            ch.pixel_type,
            ch.x_sampling,
            ch.y_sampling,
            ch.p_linear,
            h.pixel_types[c],
            h.requested_pixel_types[c]
        );
    }
    let _ = write!(o, " at:{}", h.num_custom_attributes());
    for a in &h.custom_attributes {
        let _ = write!(
            o,
            " {}:{}:{}:{:x}",
            q(&a.name),
            q(&a.attr_type),
            a.value.len(),
            fnv1a(&a.value)
        );
    }
    o
}

fn image_summary(img: &ExrImage, h: &ExrHeader) -> String {
    let mut o = String::new();
    let pt: Vec<String> = h.pixel_types.iter().map(|p| p.to_string()).collect();
    let _ = write!(o, " pt:{} nl:{}", pt.join(","), exr_num_levels(img));
    let mut level = Some(img);
    while let Some(l) = level {
        let _ = write!(
            o,
            " [{}x{} {},{} {} {}",
            l.width, l.height, l.level_x, l.level_y, l.num_channels, l.num_tiles
        );
        let mut hash = 0xcbf29ce484222325u64;
        if let Some(images) = &l.images {
            o.push_str(" S");
            for c in images {
                hash = fnv_more(c, hash);
            }
        }
        if let Some(tiles) = &l.tiles {
            o.push_str(" T");
            for t in tiles {
                let mut v = Vec::new();
                for x in [
                    t.offset_x, t.offset_y, t.level_x, t.level_y, t.width, t.height,
                ] {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                hash = fnv_more(&v, hash);
                for c in t.images.iter().flatten() {
                    hash = fnv_more(c, hash);
                }
            }
        }
        let _ = write!(o, " {hash:016x}]");
        level = l.next_level.as_deref();
    }
    o
}

/// `too_big` of the harness.
fn too_big(h: &ExrHeader) -> bool {
    const T: i64 = 1024 * 8192;
    const LIMIT: i64 = 1 << 26;
    let w = i64::from(h.data_window.max_x) - i64::from(h.data_window.min_x) + 1;
    let ht = i64::from(h.data_window.max_y) - i64::from(h.data_window.min_y) + 1;
    let nc = i64::from(h.num_channels());
    if w > 0 && ht > 0 && w <= T && ht <= T && w * ht * nc * 4 > LIMIT {
        return true;
    }
    if h.tiled != 0 {
        let (tx, ty) = (i64::from(h.tile_size_x), i64::from(h.tile_size_y));
        if tx >= 0 && ty >= 0 && tx <= T && ty <= T && tx * ty * nc * 4 > LIMIT {
            return true;
        }
        if tx > 0
            && ty > 0
            && w > 0
            && ht > 0
            && w <= T
            && ht <= T
            && ((w + tx - 1) / tx) * ((ht + ty - 1) / ty) > (1 << 16)
        {
            return true;
        }
    } else if h.chunk_count > (1 << 23) {
        return true;
    }
    false
}

fn run_case(file: &[u8], variant: i32) -> String {
    let mut o = String::new();
    let mut version = ExrVersion::default();
    let vr = parse_exr_version_from_memory(&mut version, file);
    let _ = write!(o, "V{vr}");
    if vr != 0 {
        return o;
    }
    let _ = write!(
        o,
        " {},{},{},{},{}",
        version.version, version.tiled, version.long_name, version.non_image, version.multipart
    );

    let mut header = ExrHeader::default();
    init_exr_header(&mut header);
    let mut err = None;
    let hr = parse_exr_header_from_memory(&mut header, &version, file, &mut err);
    let _ = write!(o, "\tH{hr} {} {}", qs(&err), header_summary(&header));
    free_exr_error_message(&mut err);
    if hr != 0 {
        free_exr_header(&mut header);
        return o;
    }
    if too_big(&header) {
        return o + "\tBIG";
    }

    for c in 0..header.channels.len() {
        if (1..=2).contains(&variant) && header.pixel_types[c] == TINYEXR_PIXELTYPE_HALF {
            header.requested_pixel_types[c] = TINYEXR_PIXELTYPE_FLOAT;
        }
        if variant == 3 {
            header.requested_pixel_types[c] = TINYEXR_PIXELTYPE_UINT;
        }
    }
    if variant == 2 {
        header.line_order = 0;
    }

    let mut image = ExrImage::default();
    init_exr_image(&mut image);
    let lr = load_exr_image_from_memory(&mut image, &mut header, file, &mut err);
    let _ = write!(o, "\tL{lr} {}", qs(&err));
    if lr == 0 {
        o.push_str(&image_summary(&image, &header));
        free_exr_image(&mut image);
    }
    free_exr_header(&mut header);
    o
}

fn read_corpus(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u32_at =
        |p: usize| u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
    let mut out = Vec::new();
    let mut p = 0;
    while p + 4 <= data.len() {
        let nl = u32_at(p);
        p += 4;
        let name = String::from_utf8_lossy(&data[p..p + nl]).into_owned();
        p += nl;
        let dl = u32_at(p);
        p += 4;
        out.push((name, data[p..p + dl].to_vec()));
        p += dl;
    }
    out
}

fn external_files() -> Vec<String> {
    let mut files = vec!["tests/resources/input/exr/Desk.exr".to_string()];
    for dir in [
        "tests/resources/cts/input/exr",
        "tests/resources/cts/input/exr_sample",
    ] {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .filter_map(|e| e.ok())
            .map(|e| format!("{dir}/{}", e.file_name().to_string_lossy()))
            .filter(|n| n.ends_with(".exr"))
            .collect();
        names.sort();
        files.extend(names);
    }
    files
}

/// Every case of the harness as (name, variant, file).
fn cases() -> Vec<(String, i32, Vec<u8>)> {
    let corpus = std::fs::read("tests/resources/golden/tinyexr_load_corpus.bin")
        .unwrap_or_else(|e| panic!("corpus: {e}"));
    let mut entries: Vec<(String, Vec<u8>, bool)> = read_corpus(&corpus)
        .into_iter()
        .map(|(n, d)| (n, d, true))
        .collect();
    for f in external_files() {
        let d = std::fs::read(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        entries.push((f, d, false));
    }
    let mut out = Vec::new();
    for (name, data, synthetic) in entries {
        let nvar = if synthetic { 4 } else { 3 };
        for v in 0..nvar {
            out.push((name.clone(), v, data.clone()));
        }
        if data.is_empty() || data.len() > 65536 {
            continue;
        }
        let mut rng = Lcg(fnv1a(name.as_bytes()) as u32);
        let size = data.len();
        let nflip = if synthetic { 6 } else { 12 };
        for k in 0..nflip {
            let r = rng.next() as usize;
            let pos = if k % 2 == 0 {
                r % size.min(400)
            } else {
                r % size
            };
            let val = rng.next() as u8;
            let mut d = data.clone();
            d[pos] = val;
            out.push((format!("{name}#f{k}"), 0, d));
        }
        for k in 0..2 {
            let len = rng.next() as usize % size;
            out.push((format!("{name}#t{k}"), 0, data[..len].to_vec()));
        }
    }
    out
}

#[test]
fn load_matches_tinyexr() {
    let golden = std::fs::read_to_string("tests/resources/golden/tinyexr_load.txt")
        .unwrap_or_else(|e| panic!("golden: {e}"));
    let mut expected: HashMap<(String, i32), String> = HashMap::new();
    for line in golden.lines() {
        let mut it = line.splitn(3, '\t');
        let (Some(name), Some(v), Some(rest)) = (it.next(), it.next(), it.next()) else {
            panic!("bad golden line {line}");
        };
        let v: i32 = v.parse().unwrap_or_else(|e| panic!("{line}: {e}"));
        expected.insert((name.to_string(), v), rest.to_string());
    }

    let cases = cases();
    assert_eq!(cases.len(), expected.len(), "case count");
    let mut failures = Vec::new();
    let mut crashes = 0;
    for (name, variant, data) in &cases {
        let got = run_case(data, *variant);
        let Some(want) = expected.get(&(name.clone(), *variant)) else {
            panic!("{name} {variant}: no golden line");
        };
        if want == "CRASH" {
            crashes += 1;
            continue;
        }
        if &got != want {
            failures.push(format!("{name}\t{variant}\n  want {want}\n  got  {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ ({crashes} C crashes skipped):\n{}",
        failures.len(),
        cases.len(),
        failures[..failures.len().min(8)].join("\n")
    );
}

#[test]
fn load_random_corruptions_never_panic() {
    let corpus = std::fs::read("tests/resources/golden/tinyexr_load_corpus.bin")
        .unwrap_or_else(|e| panic!("corpus: {e}"));
    let entries = read_corpus(&corpus);
    let mut rng = Lcg(12345);
    for (i, (_, data)) in entries.iter().enumerate().step_by(3) {
        for _ in 0..20 {
            let mut d = data.clone();
            let n = 1 + rng.next() % 4;
            for _ in 0..n {
                let pos = rng.next() as usize % d.len();
                d[pos] = rng.next() as u8;
            }
            if i % 2 == 0 {
                let len = rng.next() as usize % d.len();
                d.truncate(len.max(d.len() / 2));
            }
            for v in 0..4 {
                let _ = run_case(&d, v);
            }
        }
    }
}
