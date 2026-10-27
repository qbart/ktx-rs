// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu encoder Huffman coding (`basisu_enc.cpp`: `huffman_encoding_table`, `bitwise_coder`,
//! `palette_index_reorderer`) vs. golden output of the C++ original
//! (`scripts/golden/gen_basisu_huffman_enc_golden.sh`).
#![allow(clippy::panic, clippy::manual_is_multiple_of)]

use std::collections::BTreeMap;

use ktx::basisu::huffman_enc::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

struct H(u64);

impl H {
    fn new() -> Self {
        H(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn q(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

const SIZES: [u32; 11] = [1, 2, 3, 5, 21, 64, 257, 1000, 4096, 16383, 16385];

fn make_hist(c: u32) -> Histogram {
    let mut l = Lcg(c.wrapping_mul(7919).wrapping_add(17));
    let n = SIZES[(c % 11) as usize];
    let mut h = Histogram::new(n);
    let kind = (c / 11) % 8;
    for i in 0..n {
        let v = match kind {
            0 => l.next() % 50,
            1 => 1 + (100000 / (i + 1)) % 60000,
            2 => {
                let (mut a, mut b) = (1u32, 1u32);
                for _ in 0..i % 25 {
                    let t = a.wrapping_add(b);
                    a = b;
                    b = t;
                }
                a
            }
            3 => {
                if l.next() % 7 == 0 {
                    1 + l.next() % 300
                } else {
                    0
                }
            }
            4 => {
                if l.next() % 3 == 0 {
                    70000 + l.next() % 5000000
                } else {
                    l.next() % 4
                }
            }
            5 => {
                if i == c % n {
                    5
                } else {
                    0
                }
            }
            6 => 0,
            _ if i == 0 => 65535,
            _ => {
                if (i / 7) & 1 != 0 {
                    0
                } else {
                    1 + ((i / 3) % 4) * 1000
                }
            }
        };
        h.as_mut_slice()[i as usize] = v;
    }
    h
}

fn huff_case(c: u32) -> u64 {
    let hist = make_hist(c);
    let mut h = H::new();
    let mut mcs = 7;
    while mcs <= 17 {
        let mut t = HuffmanEncodingTable::new();
        let ok = t.init(&hist, mcs);
        h.u(ok as u32);
        if ok {
            h.u(t.get_total_used_codes());
            h.bytes(t.get_code_sizes());
            for &x in t.get_codes() {
                h.bytes(&x.to_le_bytes());
            }
            let mut coder = BitwiseCoder::new();
            coder.init(16);
            coder.put_bits((c * 5) & ((1u32 << (c % 8)) - 1), c % 8);
            let bits = coder.emit_huffman_table(&t);
            h.u(bits);
            for i in 0..hist.size().min(3000) {
                if t.get_code_sizes()[i as usize] != 0 {
                    let r = coder.put_code(i, &t);
                    h.u(r);
                }
            }
            let r = coder.flush();
            h.u(r);
            h.q(coder.get_total_bits());
            h.bytes(coder.get_bytes());
        }
        mcs += if c & 1 != 0 { 3 } else { 1 };
    }
    h.0
}

fn coder_case(c: u32) -> u64 {
    let mut l = Lcg(c.wrapping_mul(31).wrapping_add(5));
    let mut h = H::new();
    let (mut a, mut b) = (BitwiseCoder::new(), BitwiseCoder::new());
    a.init(4);
    b.init(4);
    let mut hist = Histogram::new(40);
    for i in 0..40 {
        hist.as_mut_slice()[i] = 1 + l.next() % 90;
    }
    let mut t = HuffmanEncodingTable::new();
    t.init(&hist, 12);
    let nops = 20 + c * 7;
    for _ in 0..nops {
        let use_a = l.next() & 3 != 0;
        let op = l.next() % 7;
        let r = if op == 6 {
            let bc = b.clone();
            a.append(&bc);
            a.get_total_bits() as u32
        } else {
            let d = if use_a { &mut a } else { &mut b };
            match op {
                0 => {
                    let nb = l.next() % 33;
                    let v = if nb == 32 {
                        (l.next() << 8) ^ l.next()
                    } else {
                        l.next() & ((1u32 << nb) - 1)
                    };
                    d.put_bits(v, nb)
                }
                1 => {
                    let m = 1 + l.next() % 8;
                    let v = l.next() % (65u32 << m);
                    d.put_rice(v, m)
                }
                2 => {
                    let cb = 1 + l.next() % 8;
                    let v = l.next();
                    let v = v >> (l.next() % 24);
                    d.put_vlc(v, cb)
                }
                3 => {
                    let n = 2 + l.next() % 1000;
                    let v = l.next() % n;
                    d.put_truncated_binary(v, n)
                }
                4 => d.put_code(l.next() % 40, &t),
                _ => d.flush(),
            }
        };
        h.u(r);
    }
    h.q(a.get_total_bits());
    h.q(b.get_total_bits());
    a.flush();
    b.flush();
    h.bytes(a.get_bytes());
    h.bytes(b.get_bytes());
    h.0
}

fn reorder_case(c: u32) -> u64 {
    let mut l = Lcg(c.wrapping_mul(131).wrapping_add(3));
    let num_syms = 1 + l.next() % if c % 3 == 0 { 8 } else { 300 };
    let num_indices = if c % 17 == 0 { 1 } else { 1 + l.next() % 3000 };
    let kind = c % 4;
    let mut idx = vec![0u32; num_indices as usize];
    let mut cur = l.next() % num_syms;
    for (i, x) in idx.iter_mut().enumerate() {
        let i = i as u32;
        match kind {
            0 => cur = l.next() % num_syms,
            1 => {
                if l.next() % 5 == 0 {
                    cur = l.next() % num_syms;
                }
            }
            2 => cur = (cur + num_syms + (l.next() % 5) - 2) % num_syms,
            _ => {
                cur = if l.next() % 4 == 0 {
                    l.next() % num_syms
                } else {
                    (i * 3) % num_syms
                }
            }
        }
        *x = cur;
    }
    let dist = |i: u32, j: u32| ((i * 7 + j * 13) % 101) as f32 / 100.0;
    let mut h = H::new();
    for m in 0..3 {
        let mut r = PaletteIndexReorderer::new();
        let w = [0.0f32, 0.5, 1.0][m];
        r.init(&idx, num_syms, if m != 0 { Some(&dist) } else { None }, w);
        let t = r.get_remap_table();
        h.u(t.len() as u32);
        for &x in t {
            h.u(x);
        }
    }
    h.0
}

#[test]
fn huffman_enc_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_huffman_enc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    let mut check = |k: String, got: u64| {
        let got = format!("{got:016x}");
        match golden.get(&k) {
            Some(w) if *w == got => {}
            Some(w) => fails.push(format!("{k}: want {w} got {got}")),
            None => fails.push(format!("{k}: missing")),
        }
    };
    for c in 0..88 * 3 {
        check(format!("h{c}"), huff_case(c));
    }
    for c in 0..200 {
        check(format!("c{c}"), coder_case(c));
    }
    for c in 0..120 {
        check(format!("r{c}"), reorder_case(c));
    }
    if !fails.is_empty() {
        panic!(
            "{} of {} mismatches:\n{}",
            fails.len(),
            golden.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
