// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu ETC1S backend (`basisu_backend.cpp`) vs. golden output of the C++ original
//! (`scripts/golden/gen_basisu_backend_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::backend::*;
use ktx::basisu::enc::color_rgba;
use ktx::basisu::frontend::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

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
    fn v(&mut self, x: &[u8]) {
        self.u(x.len() as u32);
        self.bytes(x);
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

// Slice layouts: 0 = one slice, 1 = two slices (second half size), 2 = three video frames.
fn make_input(img: u32, layout: u32) -> (Vec<PixelBlock>, Vec<BasisuBackendSliceDesc>) {
    const SIZES: [[u32; 2]; 4] = [[8, 8], [7, 5], [16, 12], [33, 20]];
    let [bw, bh] = SIZES[(img % 4) as usize];
    let nslices = [1, 2, 3][layout as usize];
    let mut blocks = Vec::new();
    let mut slices = Vec::new();
    for s in 0..nslices {
        let (mut w, mut h) = (bw, bh);
        if layout == 1 && s == 1 {
            w = bw.div_ceil(2);
            h = bh.div_ceil(2);
        }
        let mut b = make_image(if layout == 1 { img + s * 3 } else { img }, w, h);
        if layout == 2 && s > 0 {
            let mut l = Lcg(img.wrapping_mul(31).wrapping_add(s));
            for i in 0..w * h {
                if ((i % w) + (i / w) + s) % 5 == 0 {
                    for p in 0..16 {
                        let c = &mut b[i as usize][p];
                        c[0] = (c[0] as u32 + 20 + l.next() % 30) as u8;
                        c[2] ^= (l.next() & 63) as u8;
                    }
                }
            }
        }
        slices.push(BasisuBackendSliceDesc {
            first_block_index: blocks.len() as u32,
            orig_width: w * 4 - (s & 1),
            orig_height: h * 4 - 1,
            width: w * 4,
            height: h * 4,
            num_blocks_x: w,
            num_blocks_y: h,
            num_macroblocks_x: w.div_ceil(2),
            num_macroblocks_y: h.div_ceil(2),
            source_file_index: if layout == 2 { s } else { 0 },
            mip_index: if layout == 1 { s } else { 0 },
            alpha: false,
            iframe: layout == 2 && s == 0,
        });
        blocks.extend(b);
    }
    (blocks, slices)
}

// (level, max_ep, max_sel, perceptual, ep_rdo, sel_rdo)
const CFGS: [(u32, u32, u32, bool, f32, f32); 13] = [
    (0, 64, 64, true, 0.0, 0.0),
    (0, 256, 256, true, 1.5, 1.25),
    (1, 256, 256, true, 1.5, 1.25),
    (1, 128, 512, false, 3.0, 2.0),
    (2, 256, 256, true, 1.5, 1.25),
    (2, 512, 1024, false, 1.5, 1.25),
    (2, 16, 16, true, 1.5, 1.25),
    (2, 256, 256, true, 0.0, 0.0),
    (3, 300, 300, true, 2.5, 1.0),
    (4, 256, 256, true, 1.5, 1.25),
    (5, 128, 600, false, 1.0, 3.0),
    (6, 256, 256, true, 1.5, 1.25),
    (2, 2000, 2000, true, 10.0, 10.0),
];

#[test]
fn backend_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_backend.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    let mut total = 0;
    for img in 0..12u32 {
        for layout in 0..3u32 {
            let (blocks, slices) = make_input(img, layout);
            for (c, &(level, max_ep, max_sel, perceptual, ep_rdo, sel_rdo)) in
                CFGS.iter().enumerate()
            {
                if (img + layout + c as u32) % 3 == 1 {
                    continue;
                }
                let p = BasisuFrontendParams {
                    source_blocks: blocks.clone(),
                    max_endpoint_clusters: max_ep,
                    max_selector_clusters: max_sel,
                    compression_level: level,
                    perceptual,
                    validate: false,
                    tex_type: if layout == 2 {
                        C_BASIS_TEX_TYPE_VIDEO_FRAMES
                    } else {
                        C_BASIS_TEX_TYPE_2D
                    },
                    multithreaded: false,
                    job_pool_threads: 1,
                    ..Default::default()
                };
                let mut f = BasisuFrontend::new();
                let mut h = H(0xcbf29ce484222325);
                let ok = f.init(&p) && f.compress();
                h.bytes(&[ok as u8]);
                let mut be = BasisuBackend::new();
                let bp = BasisuBackendParams {
                    etc1s: true,
                    compression_level: level,
                    endpoint_rdo_quality_thresh: ep_rdo,
                    selector_rdo_quality_thresh: sel_rdo,
                    used_global_codebooks: false,
                    ..Default::default()
                };
                be.init(&bp, &slices);
                let total_bytes = be.encode(&mut f);
                h.u(total_bytes);
                let o = be.get_output();
                h.u(o.num_endpoints);
                h.u(o.num_selectors);
                h.u(o.srgb as u32);
                h.v(&o.endpoint_palette);
                h.v(&o.selector_palette);
                h.v(&o.slice_image_tables);
                h.u(o.slice_image_data.len() as u32);
                for d in &o.slice_image_data {
                    h.v(d);
                }
                for &crc in &o.slice_image_crcs {
                    h.bytes(&crc.to_le_bytes());
                }
                let got = format!("{:016x}", h.0);
                let key = format!("{img}_{layout}_{c}");
                total += 1;
                match golden.get(&key) {
                    Some(w) if *w == got => {}
                    Some(w) => fails.push(format!("{key}: want {w} got {got}")),
                    None => fails.push(format!("{key}: missing")),
                }
            }
        }
    }
    assert_eq!(total, golden.len(), "case count");
    if !fails.is_empty() {
        panic!(
            "{} of {total} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
