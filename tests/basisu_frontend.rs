//! basisu ETC1S frontend (`basisu_frontend.cpp`) and libstdc++ `std::sort` tie order vs. golden
//! output of the C++ original (`scripts/golden/gen_basisu_frontend_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::enc::{color_rgba, indirect_sort};
use ktx::basisu::frontend::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

#[derive(Clone, Copy)]
struct H(u64);

impl H {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
}

fn make_image(img: u32, bw: u32, bh: u32) -> Vec<PixelBlock> {
    let mut l = Lcg(img.wrapping_mul(7919).wrapping_add(5));
    let mut blocks = vec![[[0u8; 4]; 16]; (bw * bh) as usize];
    let kind = if img >= 10 { 5 } else { img % 5 };
    for by in 0..bh {
        for bx in 0..bw {
            let bi = (by * bw + bx) as usize;
            let dup = kind == 3 && bx & 1 != 0 && bx > 0;
            for i in 0..16u32 {
                let (x, y) = ((bx * 4 + (i & 3)) as i32, (by * 4 + (i >> 2)) as i32);
                let (mut r, mut g, mut b) = match kind {
                    0 => (x * 3, y * 2, 128 + (x - y)),
                    1 => {
                        let c = if ((x >> 3) + (y >> 3)) & 1 != 0 {
                            220
                        } else {
                            30
                        };
                        (c, (c + x * 4) & 255, 255 - c)
                    }
                    2 => (
                        (l.next() & 255) as i32,
                        ((l.next() >> 4) & 255) as i32,
                        ((l.next() >> 8) & 255) as i32,
                    ),
                    3 => {
                        let s = ((x * x + y * y) >> 4) & 255;
                        (s, (s * 3) & 255, (s * 7 + y) & 255)
                    }
                    4 => {
                        let c = ((x / 5) * 40 + (y / 7) * 30) & 255;
                        (c, c, (c + 64 + (l.next() % 9) as i32) & 255)
                    }
                    _ => {
                        let c = (((x / 6) + (y / 10)) % 3) * 100 + ((x >> 1) & 1) * 9;
                        (c, 255 - c, (c * 2) & 255)
                    }
                };
                if kind != 2 && kind != 5 {
                    r += (l.next() % 7) as i32 - 3;
                    g += (l.next() % 7) as i32 - 3;
                    b += (l.next() % 7) as i32 - 3;
                }
                blocks[bi][i as usize] = color_rgba(r, g, b, 255);
            }
            if dup {
                blocks[bi] = blocks[bi - 1];
            }
        }
    }
    blocks
}

// (level, max_ep, max_sel, tex_type, perceptual, disable_hier, validate)
const CFGS: [(u32, u32, u32, u32, bool, bool, bool); 12] = [
    (0, 64, 64, 0, true, false, false),
    (1, 256, 256, 0, true, false, true),
    (2, 256, 256, 0, true, false, false),
    (2, 512, 1024, 0, false, false, false),
    (3, 300, 300, 0, true, false, false),
    (4, 256, 256, 0, true, false, false),
    (5, 128, 600, 0, false, false, false),
    (6, 256, 256, 0, true, false, false),
    (2, 16, 16, 0, true, false, false),
    (2, 256, 256, 3, true, false, false),
    (1, 1024, 256, 0, true, true, false),
    (0, 2000, 2000, 0, false, false, false),
];

fn hash_frontend(f: &BasisuFrontend, mut h: H) -> u64 {
    let nb = f.get_total_output_blocks();
    h.u(nb);
    for i in 0..nb {
        h.bytes(&f.get_output_block(i).bytes);
        h.bytes(&f.get_etc1s_block(i).bytes);
    }
    let ne = f.get_total_endpoint_clusters();
    h.u(ne);
    for i in 0..nb {
        h.u(f.get_subblock_endpoint_cluster_index(i, 0));
        h.u(f.get_subblock_endpoint_cluster_index(i, 1));
    }
    for c in 0..ne {
        h.bytes(f.get_endpoint_cluster_unscaled_color(c, false));
        h.u(f.get_endpoint_cluster_inten_table(c, false));
        h.u(f.get_endpoint_cluster_color_is_used(c, false) as u32);
    }
    let ns = f.get_total_selector_clusters();
    h.u(ns);
    for i in 0..nb {
        h.u(f.get_block_selector_cluster_index(i));
    }
    for c in 0..ns {
        h.u(f
            .get_selector_cluster_selector_bits(c)
            .get_raw_selector_bits());
        let v = f.get_selector_cluster_block_indices(c);
        h.u(v.len() as u32);
        for &x in v {
            h.u(x);
        }
    }
    h.0
}

fn golden() -> BTreeMap<String, String> {
    include_str!("resources/golden/basisu_frontend.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn std_sort_tie_order_matches_libstdcxx() {
    let g = golden();
    for t in 0..40u32 {
        let n = if t < 30 { t * 7 } else { 1000 + t * 333 };
        let mut l = Lcg(t.wrapping_mul(17).wrapping_add(1));
        let keys: Vec<u32> = (0..n)
            .map(|i| match t % 4 {
                0 => l.next() % 5,
                1 => n - i,
                2 => {
                    if i < n / 2 {
                        i
                    } else {
                        n - i
                    }
                }
                _ => l.next() % (n + 1),
            })
            .collect();
        let mut idx = vec![0u32; n as usize];
        indirect_sort(&mut idx, &keys);
        let mut h = H(0xcbf29ce484222325);
        for &x in &idx {
            h.u(x);
        }
        assert_eq!(
            g.get(&format!("sort{t}")),
            Some(&format!("{:016x}", h.0)),
            "sort{t}"
        );
    }
}

#[test]
fn frontend_matches_basisu() {
    let g = golden();
    const SIZES: [[u32; 2]; 4] = [[8, 8], [16, 12], [32, 32], [64, 40]];
    let mut fails = Vec::new();
    let mut total = 0;
    for img in 0..12u32 {
        let [bw, bh] = SIZES[(img % 4) as usize];
        let blocks = make_image(img, bw, bh);
        for (c, &(level, max_ep, max_sel, tex_type, perceptual, disable_hier, validate)) in
            CFGS.iter().enumerate()
        {
            if img >= 6 && level >= 5 && !(img + c as u32).is_multiple_of(2) {
                continue;
            }
            let p = BasisuFrontendParams {
                source_blocks: blocks.clone(),
                max_endpoint_clusters: max_ep,
                max_selector_clusters: max_sel,
                compression_level: level,
                perceptual,
                validate,
                tex_type,
                disable_hierarchical_endpoint_codebooks: disable_hier,
                multithreaded: false,
                job_pool_threads: 1,
                ..Default::default()
            };
            let mut f = BasisuFrontend::new();
            let mut h = H(0xcbf29ce484222325);
            let ok = f.init(&p);
            h.bytes(&[ok as u8]);
            let ok = f.compress();
            h.bytes(&[ok as u8]);
            let h1 = hash_frontend(&f, h);

            let nb = f.get_total_output_blocks();
            let new_block_endpoints: Vec<u32> = (0..nb)
                .map(|i| {
                    let e = f.get_subblock_endpoint_cluster_index(i, 0);
                    if e & 1 != 0 && i % 3 != 0 { e - 1 } else { e }
                })
                .collect();
            let mut old_to_new = Vec::new();
            f.reoptimize_remapped_endpoints(
                &new_block_endpoints,
                &mut old_to_new,
                c & 1 == 0,
                None,
            );
            let mut h2 = H(0xcbf29ce484222325);
            h2.u(old_to_new.len() as u32);
            for &x in &old_to_new {
                h2.u(x as u32);
            }
            let hr = hash_frontend(&f, h2);

            let got = format!("{h1:016x} {hr:016x}");
            let key = format!("{img}_{c}");
            total += 1;
            match g.get(&key) {
                Some(w) if *w == got => {}
                w => fails.push(format!("{key}: want {w:?} got {got}")),
            }
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {total} mismatches:\n{}",
            fails.len(),
            fails.join("\n")
        );
    }
}
