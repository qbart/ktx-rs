// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu resampler port vs. golden output of the C++ original
//! (`scripts/golden/gen_resampler_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::resample_filters::find_resample_filter;
use ktx::basisu::resampler::{BoundaryOp, ContribList, Resampler};

const FNV0: u64 = 0xcbf29ce484222325;

fn fnv(mut h: u64, b: &[u8]) -> u64 {
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
    fn sample(&mut self) -> f32 {
        (self.next() & 0xffff) as f32 / 65535.0 * 1.5 - 0.25
    }
}

const SIZES: [(i32, i32, i32, i32); 10] = [
    (1, 1, 1, 1),
    (7, 5, 3, 2),
    (5, 7, 13, 11),
    (16, 16, 8, 8),
    (33, 17, 16, 8),
    (9, 9, 9, 9),
    (64, 3, 5, 40),
    (2, 2, 1, 1),
    (17, 1, 4, 1),
    (3, 30, 1, 2),
];
const PARAMS: [(f32, f32, f32, f32, f32); 4] = [
    (1.0, 0.0, 0.0, 0.0, 1.0),
    (0.75, 0.25, -0.5, 0.0, 0.0),
    (2.0, 0.0, 1.5, -0.5, 0.25),
    (0.1, 0.0, 0.0, 0.0, 1.0),
];
const OPS: [BoundaryOp; 3] = [BoundaryOp::Wrap, BoundaryOp::Reflect, BoundaryOp::Clamp];

fn hash_clist(l: Option<&Vec<ContribList>>, n: i32) -> u64 {
    let Some(l) = l else {
        return 0;
    };
    let mut h = FNV0;
    for c in l.iter().take(n as usize) {
        h = fnv(h, &c.n.to_le_bytes());
        for p in &c.p {
            h = fnv(h, &p.weight.to_le_bytes());
            h = fnv(h, &p.pixel.to_le_bytes());
        }
    }
    h
}

fn drive(r: &mut Resampler, sx: i32, sy: i32, seed: u32) -> (i32, i32, u64) {
    let mut lcg = Lcg(seed);
    let (mut puts, mut gets, mut h) = (0, 0, FNV0);
    let mut line = vec![0f32; sx as usize];
    for _ in 0..sy + 1 {
        for v in line.iter_mut() {
            *v = lcg.sample();
        }
        if r.put_line(&line) {
            puts += 1;
        }
        while let Some(o) = r.get_line() {
            for v in o {
                h = fnv(h, &v.to_le_bytes());
            }
            gets += 1;
        }
    }
    (puts, gets, h)
}

fn golden() -> BTreeMap<String, String> {
    let text = include_str!("resources/golden/resampler.txt");
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn check(g: &BTreeMap<String, String>, key: &str, got: &str, fails: &mut Vec<String>) {
    match g.get(key) {
        Some(want) if want == got => {}
        Some(want) => fails.push(format!("{key}: want {want} got {got}")),
        None => fails.push(format!("{key}: missing golden")),
    }
}

#[test]
fn resampler_matches_basisu() {
    let g = golden();
    let mut fails = Vec::new();
    let nf = Resampler::get_filter_num();
    let mut names = format!("{nf}");
    for i in -1..=nf {
        names.push(' ');
        names.push_str(Resampler::get_filter_name(i).unwrap_or("(null)"));
    }
    check(&g, "filters", &names, &mut fails);

    let mut cases = 0;
    for f in 0..nf {
        let name = Resampler::get_filter_name(f);
        for (z, &(sx, sy, dx, dy)) in SIZES.iter().enumerate() {
            for (b, &op) in OPS.iter().enumerate() {
                for (p, &(scale, ox, oy, lo, hi)) in PARAMS.iter().enumerate() {
                    let mut r = Resampler::new(
                        sx, sy, dx, dy, op, lo, hi, name, None, None, scale, scale, ox, oy,
                    );
                    let mut got = format!("S{}", r.status() as i32);
                    if r.status() as i32 == 0 {
                        let seed = (f as usize * 1000 + z * 10 + b + p * 3) as u32;
                        let (puts, gets, h) = drive(&mut r, sx, sy, seed);
                        got.push_str(&format!(
                            " X{:016x} Y{:016x} P{puts} G{gets} H{h:016x}",
                            hash_clist(r.get_clist_x().as_deref(), dx),
                            hash_clist(r.get_clist_y().as_deref(), dy)
                        ));
                    }
                    check(&g, &format!("f{f} z{z} b{b} p{p}"), &got, &mut fails);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 16 * 10 * 3 * 4);

    for (z, &(sx, sy, dx, dy)) in SIZES.iter().enumerate() {
        let mut rs: Vec<Resampler> = Vec::new();
        for c in 0..3 {
            let (cx, cy) = match rs.first() {
                Some(r0) if c > 0 => (r0.get_clist_x(), r0.get_clist_y()),
                _ => (None, None),
            };
            rs.push(Resampler::new(
                sx,
                sy,
                dx,
                dy,
                BoundaryOp::Clamp,
                0.0,
                1.0,
                Some("kaiser"),
                cx,
                cy,
                1.0,
                1.0,
                0.0,
                0.0,
            ));
        }
        let mut got = String::new();
        for round in 0..2u32 {
            for (c, r) in rs.iter_mut().enumerate() {
                let (puts, gets, h) = drive(r, sx, sy, (z * 7 + c) as u32 + round * 100);
                got.push_str(&format!(
                    "S{} P{puts} G{gets} H{h:016x} ",
                    r.status() as i32
                ));
                r.restart();
            }
        }
        check(&g, &format!("shared z{z}"), &got, &mut fails);
        let forced = rs.iter().map(|r| r.clists_forced()).collect::<Vec<_>>();
        assert_eq!(forced, vec![(false, false), (true, true), (true, true)]);
    }

    for n in ["lanczos4", "Lanczos4", "", "box ", "quadratic_mix", "none"] {
        let r = Resampler::new(
            4,
            4,
            2,
            2,
            BoundaryOp::Clamp,
            0.0,
            1.0,
            Some(n),
            None,
            None,
            1.0,
            1.0,
            0.0,
            0.0,
        );
        let got = format!("S{} F{}", r.status() as i32, find_resample_filter(n));
        check(&g, &format!("name '{n}'"), &got, &mut fails);
    }
    let mut r = Resampler::new(
        6,
        5,
        3,
        9,
        BoundaryOp::Reflect,
        0.0,
        1.0,
        None,
        None,
        None,
        1.0,
        1.0,
        0.0,
        0.0,
    );
    let (puts, gets, h) = drive(&mut r, 6, 5, 77);
    let got = format!("S{} P{puts} G{gets} H{h:016x}", r.status() as i32);
    check(&g, "default", &got, &mut fails);

    if !fails.is_empty() {
        panic!(
            "{} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(20)].join("\n")
        );
    }
}

#[test]
fn degenerate_inputs_never_panic() {
    for &(sx, sy, dx, dy) in &[
        (0, 4, 4, 4),
        (4, 0, 4, 4),
        (4, 4, 0, 4),
        (4, 4, 4, 0),
        (-3, 2, 2, 2),
    ] {
        for op in OPS {
            let mut r = Resampler::new(
                sx,
                sy,
                dx,
                dy,
                op,
                0.0,
                1.0,
                Some("tent"),
                None,
                None,
                1.0,
                1.0,
                0.0,
                0.0,
            );
            let line = vec![0.5f32; 8];
            for _ in 0..6 {
                r.put_line(&line);
                while r.get_line().is_some() {}
            }
        }
    }
    // Mismatched shared contributor lists and short input lines.
    let r0 = Resampler::new(
        32,
        32,
        64,
        64,
        BoundaryOp::Wrap,
        0.0,
        0.0,
        Some("lanczos12"),
        None,
        None,
        1.0,
        1.0,
        0.0,
        0.0,
    );
    let mut r = Resampler::new(
        4,
        4,
        2,
        2,
        BoundaryOp::Wrap,
        0.0,
        0.0,
        None,
        r0.get_clist_x(),
        r0.get_clist_y(),
        1.0,
        1.0,
        0.0,
        0.0,
    );
    for _ in 0..8 {
        r.put_line(&[1.0]);
        while r.get_line().is_some() {}
    }
}
