//! basisu ETC1 encoder (`basisu_etc.cpp`) vs. golden output of the C++ original
//! (`scripts/golden/gen_basisu_etc_golden.sh`, SSE 4.1 build like libktx).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::enc::{ColorRgba, color_rgba};
use ktx::basisu::etc_enc::*;

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
    fn q(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn c(&mut self, c: &ColorRgba) {
        self.bytes(c);
    }
}

fn make_block(b: u32) -> [ColorRgba; 16] {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(13));
    let kind = b % 7;
    let (br, bg, bb) = (l.next() & 255, l.next() & 255, l.next() & 255);
    let (r2, g2, b2) = (l.next() & 255, l.next() & 255, l.next() & 255);
    let mut px = [[0u8; 4]; 16];
    for i in 0..16 {
        let (n0, n1, n2, n3) = (l.next(), l.next(), l.next(), l.next());
        let (r, g, bl): (i32, i32, i32) = match kind {
            0 => ((n0 & 255) as i32, (n1 & 255) as i32, (n2 & 255) as i32),
            1 => {
                let (x, y) = ((i & 3) as i32, (i >> 2) as i32);
                (
                    br as i32 + x * 20 - 30,
                    bg as i32 + y * 15 - 20,
                    bb as i32 + (x + y) * 9 - 25,
                )
            }
            2 => (br as i32, bg as i32, bb as i32),
            3 => {
                if n0 & 1 != 0 {
                    (br as i32, bg as i32, bb as i32)
                } else {
                    (r2 as i32, g2 as i32, b2 as i32)
                }
            }
            4 => {
                let v = br as i32 + (n0 % 9) as i32 - 4;
                (v, v + (n1 % 3) as i32 - 1, v)
            }
            5 => (
                br as i32 + (n0 % 3) as i32,
                bg as i32 + (n1 % 3) as i32,
                bb as i32 + (n2 % 3) as i32,
            ),
            _ => (
                if b & 8 != 0 {
                    255 - (n0 % 6) as i32
                } else {
                    (n0 % 6) as i32
                },
                if b & 16 != 0 {
                    255 - (n1 % 6) as i32
                } else {
                    (n1 % 6) as i32
                },
                (n2 & 255) as i32,
            ),
        };
        let a = match b % 3 {
            0 => 255,
            1 => (n3 & 255) as i32,
            _ => ((i as u32 * 17 + br) & 255) as i32,
        };
        px[i] = color_rgba(r, g, bl, a);
    }
    px
}

fn block_hash(b: u32) -> u64 {
    static S_DELTAS: [i32; 5] = [-2, -1, 0, 1, 2];
    let px = make_block(b);
    let mut h = H(0xcbf29ce484222325);

    let mut solid = EtcBlock::default();
    let serr = pack_etc1_block_solid_color(&mut solid, &px[0]);
    h.q(serr);
    h.bytes(&solid.bytes);

    let qualities = [
        BasisEtcQuality::Fast,
        BasisEtcQuality::Medium,
        BasisEtcQuality::Slow,
        BasisEtcQuality::Uber,
    ];
    let mut k = 0u32;
    for qi in 0..4 {
        for perc in 0..2 {
            for cf in 0..2 {
                for c4 in 0..2 {
                    for sub in 0..2 {
                        let n = if sub != 0 { 8 } else { 16 };
                        let mut p = Etc1OptimizerParams::default();
                        p.pack.quality = qualities[qi];
                        p.pack.perceptual = perc != 0;
                        p.pack.cluster_fit = cf != 0;
                        p.use_color4 = c4 != 0;
                        p.num_src_pixels = n;
                        p.src_pixels = &px;
                        if cf == 0 && (k + b).is_multiple_of(3) {
                            p.scan_deltas = &S_DELTAS;
                        }
                        p.refinement = (k + b) % 7 != 3;
                        if c4 == 0 && (k + b) % 5 == 1 {
                            p.constrain_against_base_color5 = true;
                            p.base_color5 = [px[0][0] >> 3, px[0][1] >> 3, px[0][2] >> 3, 255];
                        }
                        let mut force = [0u8; 16];
                        if qi >= 2 && (k + b) % 4 == 2 {
                            for i in 0..16u32 {
                                force[i as usize] = ((i * 3 + b + k) & 3) as u8;
                            }
                            p.force_selectors = Some(&force);
                        }
                        let mut r = Etc1OptimizerResults {
                            selectors: vec![0; 16],
                            n,
                            ..Default::default()
                        };
                        let mut opt = Etc1Optimizer::new();
                        opt.init(&p);
                        let ok = opt.compute(&p, &mut r);
                        h.u(ok as u32);
                        h.q(r.error);
                        k += 1;
                        if !ok {
                            continue;
                        }
                        h.c(&r.block_color_unscaled);
                        h.u(r.block_inten_table);
                        h.u(r.block_color4 as u32);
                        h.u(r.n);
                        h.bytes(&r.selectors[..n as usize]);
                        if sub != 0 {
                            continue;
                        }
                        let kk = k - 1;
                        let mut blk = EtcBlock::default();
                        if r.block_color4 {
                            blk.set_block_color4(&r.block_color_unscaled, &r.block_color_unscaled);
                        } else {
                            blk.set_block_color5_etc1s(&r.block_color_unscaled);
                        }
                        blk.set_inten_tables_etc1s(r.block_inten_table);
                        blk.set_flip_bit(kk & 2 != 0);
                        for y in 0..4 {
                            for x in 0..4 {
                                blk.set_selector(x, y, r.selectors[(x + y * 4) as usize] as u32);
                            }
                        }
                        h.bytes(&blk.bytes);
                        h.u(blk.is_etc1s() as u32);
                        h.q(blk.evaluate_etc1_error(&px, p.pack.perceptual, -1));
                        h.q(blk.evaluate_etc1_error(&px, p.pack.perceptual, (kk & 1) as i32));
                        let de = blk.determine_selectors(&px, p.pack.perceptual, 0, 2);
                        h.q(de);
                        h.bytes(&blk.bytes);
                    }
                }
            }
        }
    }

    let mut l = Lcg(b.wrapping_mul(31).wrapping_add(7));
    for t in 0..4u32 {
        let mut rb = EtcBlock::default();
        for i in 0..8 {
            rb.bytes[i] = l.next() as u8;
        }
        let mut up = [[1u8, 2, 3, 4]; 16];
        let ok = unpack_etc1(&rb, &mut up, t & 1 != 0);
        h.u(ok as u32);
        for c in &up {
            h.c(c);
        }
        h.u(rb.is_etc1s() as u32);
        for s in 0..2 {
            let mut bc = [[0u8; 4]; 4];
            h.u(rb.get_block_colors(&mut bc, s) as u32);
            for c in &bc {
                h.c(c);
            }
            let mut lh = [[0u8; 4]; 2];
            h.u(rb.get_block_low_high_colors(&mut lh, s) as u32);
            h.c(&lh[0]);
            h.c(&lh[1]);
            h.c(&rb.get_block_color(s, t & 2 != 0));
            h.c(&rb.get_block_color_sel(s, (t + s) & 3));
        }
        let (lo, hi) = rb.get_selector_range();
        h.u(lo);
        h.u(hi);
        h.u(rb.get_general_bits(13, 7));
        h.c(&rb.get_selector_color(1, 2, t & 3));
        let mut c5 = [[0u8; 4]; 4];
        EtcBlock::get_block_colors5(
            &mut c5,
            &[rb.bytes[0] & 31, rb.bytes[1] & 31, rb.bytes[2] & 31, 255],
            t * 2,
            false,
        );
        for c in &c5 {
            h.c(c);
        }
        EtcBlock::get_block_colors4(
            &mut c5,
            &[rb.bytes[0] & 15, rb.bytes[1] & 15, rb.bytes[2] & 15, 255],
            t * 2 + 1,
            false,
        );
        for c in &c5 {
            h.c(c);
        }
        if ok {
            h.q(rb.evaluate_etc1_error(&px, t >= 2, -1));
            h.q(rb.evaluate_etc1_error(&px, t >= 2, (t & 1) as i32));
            let mut sp = [[0u8; 4]; 8];
            rb.get_subblock_pixels(&mut sp, (t & 1) as i32);
            for c in &sp {
                h.c(c);
            }
            h.q(rb.determine_selectors(&px, t < 2, 0, 2));
            h.bytes(&rb.bytes);
        }
        let mut cb = EtcBlock::default();
        let c0 = [rb.bytes[3] & 31, rb.bytes[4] & 31, rb.bytes[5] & 31, 255];
        let c1 = [rb.bytes[6] & 31, rb.bytes[7] & 31, rb.bytes[2] & 31, 255];
        h.u(cb.set_block_color5_check(&c0, &c1) as u32);
        h.bytes(&cb.bytes);
        cb.set_block_color5_clamp(&c0, &c1);
        h.bytes(&cb.bytes);
        h.u(EtcBlock::try_pack_color5_delta3(&[c0, c1]) as u32);
        h.u(EtcBlock::pack_color5(&px[t as usize], true, 127) as u32);
        h.u(EtcBlock::pack_color4(&px[t as usize], true, 100) as u32);
    }

    let mut alpha = [0u8; 16];
    for i in 0..16 {
        alpha[i] = px[i][3];
    }
    let mut er = PackEacA8Results::default();
    let mask = match b % 5 {
        0 => 0xFFFF,
        1 => 0x0F0F,
        _ => u32::MAX,
    };
    let eerr = pack_eac_a8_results(&mut er, &alpha, 16, b % 4, (b / 4) % 3, mask);
    h.q(eerr);
    h.u(er.base);
    h.u(er.table);
    h.u(er.multiplier);
    h.bytes(&er.selectors[..16]);
    let mut eb = EacA8Block::default();
    pack_eac_a8(&mut eb, &alpha, 1, 1, u32::MAX);
    h.u(eb.base as u32);
    h.u(eb.table as u32);
    h.u(eb.multiplier as u32);
    h.bytes(&eb.selectors);
    h.0
}

#[test]
fn etc_encoder_matches_basisu() {
    let golden: BTreeMap<u32, String> = include_str!("resources/golden/basisu_etc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(k, v)| Some((k.parse().ok()?, v.to_string())))
        .collect();
    assert_eq!(golden.len(), 3000);
    let mut fails = Vec::new();
    for (&b, want) in &golden {
        let got = format!("{:016x}", block_hash(b));
        if &got != want {
            fails.push(format!("block {b} (kind {}): want {want} got {got}", b % 7));
        }
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
