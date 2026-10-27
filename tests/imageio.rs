// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! imageio PNG / EXR reader port vs KTX-Software's tools/imageio.
//!
//! Not a C test port: `scripts/golden/gen_imageio_golden.sh` runs
//! `ImageInput::open` + `readImage` for every vendored PNG / EXR input and
//! 19 target formats (native, the `ktx create` load formats and unsupported
//! ones) and records the spec, format descriptor, warnings and the result
//! (`tests/resources/golden/imageio.txt`). "CRASH" cases (undefined behavior
//! in C) only have to not panic.
#![allow(clippy::panic)]

mod common;

use common::synthetic::fnv1a;
use ktx::VkFormat;
use ktx::cli::format_descriptor::FormatDescriptor;
use ktx::cli::imageio::{ImageInput, WarningCallback, format_type_to_string, open};
use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

fn esc(s: &[u8]) -> String {
    let mut o = String::new();
    for &c in s {
        match c {
            b'\n' => o.push_str("\\n"),
            b'\\' => o.push_str("\\\\"),
            b'\t' => o.push_str("\\t"),
            0x20..0x7f => o.push(c as char),
            _ => {
                let _ = write!(o, "\\x{c:02x}");
            }
        }
    }
    o
}

fn fd_summary(f: &FormatDescriptor) -> String {
    let mut o = String::new();
    let t = f.texel_block_dimension;
    let b = f.bytes_plane;
    let _ = write!(
        o,
        "fd:{},{},{},{} tb:{},{},{},{} bp:{},{},{},{},{},{},{},{}",
        f.model,
        f.primaries,
        f.transfer,
        f.flags,
        t[0],
        t[1],
        t[2],
        t[3],
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7]
    );
    let _ = write!(
        o,
        " ext:{},{},{:08x},{},{},{:x}",
        f.channel_count,
        u8::from(f.same_unit_all_channels),
        f.oe_gamma.to_bits(),
        esc(&f.icc_profile_name),
        f.icc_profile.len(),
        fnv1a(&f.icc_profile)
    );
    let _ = write!(o, " s:{}", f.samples.len());
    for s in &f.samples {
        let _ = write!(
            o,
            " [{},{},{},{}{}{}{},{},{},{},{},{},{}]",
            s.bit_offset,
            s.bit_length,
            s.channel_id,
            u8::from(s.qualifier_linear),
            u8::from(s.qualifier_exponent),
            u8::from(s.qualifier_signed),
            u8::from(s.qualifier_float),
            s.sample_position[0],
            s.sample_position[1],
            s.sample_position[2],
            s.sample_position[3],
            s.lower,
            s.upper
        );
    }
    o
}

fn run_case(path: &str, vk: u32) -> String {
    let warnings = Rc::new(RefCell::new(String::new()));
    let w = Rc::clone(&warnings);
    let wcb: WarningCallback = Rc::new(move |m: &str| {
        w.borrow_mut()
            .push_str(&format!("{{{}}}", esc(m.as_bytes())));
    });
    let mut input: Box<dyn ImageInput> = match open(path, Some(wcb)) {
        Ok(i) => i,
        Err(e) => return format!("OPEN {} W{}", esc(e.what().as_bytes()), warnings.borrow()),
    };
    let spec = input.spec().clone();
    let o = spec.origin();
    let mut out = format!(
        "{} {} {} {}x{}x{} {},{},{} {}",
        input.format_name(),
        format_type_to_string(input.format_type()),
        input.subimage_count(),
        spec.width(),
        spec.height(),
        spec.depth(),
        o.x,
        o.y,
        o.z,
        fd_summary(spec.format())
    );
    let target = if vk == 0 {
        FormatDescriptor::default()
    } else {
        let dfd = ktx::dfd::vk2dfd(VkFormat(vk)).unwrap_or_else(|| panic!("vk2dfd {vk}"));
        FormatDescriptor::from_dfd(&dfd)
    };
    let size = if target.is_unknown() {
        spec.image_byte_count()
    } else {
        spec.width() as usize * spec.height() as usize * target.pixel_byte_count() as usize
    };
    let mut buf = vec![0u8; size];
    match input.read_image(&mut buf, 0, 0, &target) {
        Ok(()) => {
            let _ = write!(out, "\tREAD {size} {:016x}", fnv1a(&buf));
        }
        Err(e) => {
            let _ = write!(out, "\tERR {}", esc(e.what().as_bytes()));
        }
    }
    let _ = write!(out, " W{}", warnings.borrow());
    out
}

#[test]
fn imageio_matches_ktx_software() {
    let golden = std::fs::read_to_string("tests/resources/golden/imageio.txt")
        .unwrap_or_else(|e| panic!("golden: {e}"));
    let mut failures = Vec::new();
    let mut n = 0;
    for line in golden.lines() {
        let mut it = line.splitn(3, '\t');
        let (Some(path), Some(vk), Some(want)) = (it.next(), it.next(), it.next()) else {
            panic!("bad line {line}");
        };
        let vk: u32 = vk.parse().unwrap_or_else(|e| panic!("{line}: {e}"));
        let got = run_case(path, vk);
        n += 1;
        if want != "CRASH" && got != want {
            failures.push(format!("{path} {vk}\n  want {want}\n  got  {got}"));
        }
    }
    assert!(n > 3000, "only {n} cases");
    assert!(
        failures.is_empty(),
        "{} of {n} cases differ:\n{}",
        failures.len(),
        failures[..failures.len().min(6)].join("\n")
    );
}
