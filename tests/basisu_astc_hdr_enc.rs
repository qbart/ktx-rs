//! basisu ASTC HDR common encoder 6x6 paths (`basisu_astc_hdr_common.cpp`) vs. golden output of
//! the C++ original (`scripts/golden/gen_basisu_astc_hdr_enc_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::astc_hdr_enc::*;

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
    fn d(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn rand_half(l: &mut Lcg, lo: i32, hi: i32) -> u32 {
    let e = (lo + (l.next() % (hi - lo + 1) as u32) as i32 + 15).clamp(0, 30);
    ((e as u32) << 10) | (l.next() & 1023)
}

fn make_block(b: u32) -> ([[u16; 3]; 36], [[f32; 4]; 36]) {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(23));
    let kind = b % 7;
    let base = [
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
    ];
    let base2 = [
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
    ];
    let mut h = [[0u16; 3]; 36];
    let mut q16 = [[0f32; 4]; 36];
    for i in 0..36u32 {
        let (x, y) = (i % 6, i / 6);
        for c in 0..3u32 {
            let cu = c as usize;
            let mut v = match kind {
                0 => rand_half(&mut l, -10, 15),
                1 => base[0] + (l.next() % 64),
                2 => base[cu] + (x * 30 + y * 20) * (c + 1),
                3 => {
                    if ((x / 3) + (y / 3)) & 1 != 0 {
                        base[cu]
                    } else {
                        base2[cu]
                    }
                }
                4 => {
                    if l.next() & 7 == 0 {
                        0
                    } else {
                        rand_half(&mut l, -14, -8)
                    }
                }
                5 => rand_half(&mut l, 8, 15),
                _ => base[cu] + (l.next() % 300),
            };
            if v > 0x7BFF {
                v = 0x7BFF;
            }
            h[i as usize][cu] = v as u16;
        }
        if kind == 1 {
            let v = h[i as usize][0];
            h[i as usize][1] = v;
            h[i as usize][2] = v;
        }
        for c in 0..3 {
            q16[i as usize][c] = half_to_qlog16(h[i as usize][c] as i32) as f32;
        }
    }
    (h, q16)
}

const OPT: [OptMode; 5] = [
    OptMode::NoOpt,
    OptMode::OrdinaryLeastSquares,
    OptMode::WeightedLeastSquares,
    OptMode::WeightedLeastSquaresHeavy,
    OptMode::WeightedAverage,
];
const GRIDS: [[u32; 2]; 9] = [
    [2, 2],
    [3, 3],
    [4, 4],
    [5, 5],
    [6, 6],
    [3, 5],
    [6, 2],
    [4, 6],
    [5, 3],
];

fn block_hash(b: u32) -> u64 {
    let (h, q16) = make_block(b);
    let mut l = Lcg(b.wrapping_mul(13).wrapping_add(5));
    let mut o = AstcHdrCodecBaseOptions::default();
    if b & 1 != 0 {
        o.r_err_scale = 1.0;
        o.g_err_scale = 1.0;
    }
    if b % 5 == 3 {
        o.ultra_quant = true;
    }
    if b % 7 == 2 {
        o.disable_weight_plane_optimization = false;
    }
    o.q_log_bias = if b & 2 != 0 {
        Q_LOG_BIAS_6X6
    } else {
        Q_LOG_BIAS_4X4
    };
    let mut hh = H(0xcbf29ce484222325);
    let mut n = 36usize;
    let mut sh = [[0u16; 3]; 36];
    let mut sq = [[0f32; 4]; 36];
    let mut ofs = [0u8; 36];
    if b % 3 == 1 {
        n = 0;
        for i in 0..36 {
            if (l.next() & 1) != 0 || i == 0 {
                sh[n] = h[i];
                sq[n] = q16[i];
                ofs[n] = i as u8;
                n += 1;
            }
        }
    } else {
        sh = h;
        sq = q16;
        for (i, x) in ofs.iter_mut().enumerate() {
            *x = i as u8;
        }
    }
    for t in 0..4 {
        let wr = 1 + (l.next() % 8);
        let er = if t & 1 != 0 { 20 } else { 12 + (l.next() % 9) };
        let om = OPT[(l.next() % 5) as usize];
        let uber = (l.next() & 1) != 0;
        let constrain = (l.next() & 3) == 0;
        let direct = (l.next() & 7) == 0;
        let mut e = [0u8; 6];
        let mut w = [0u8; 36];
        let mut sm = 0u32;
        let ignore = (l.next() & 3) == 0;
        let err = encode_astc_hdr_block_mode_11(
            n,
            &sh,
            &sq,
            wr,
            &mut sm,
            BIG_FLOAT_VAL as f64,
            &mut e,
            &mut w,
            &o,
            direct,
            er,
            uber,
            constrain,
            -1,
            7,
            ignore,
            om,
            None,
        );
        hh.d(err);
        hh.u(sm);
        hh.bytes(&e);
        hh.bytes(&w[..n]);
        let mut e7 = [0u8; 4];
        let mut w7 = [0u8; 36];
        let mut sm7 = 0u32;
        let err7 = encode_astc_hdr_block_mode_7(
            n,
            &sh,
            &sq,
            wr,
            &mut sm7,
            BIG_FLOAT_VAL as f64,
            &mut e7,
            &mut w7,
            &o,
            er,
            0,
            5,
            None,
        );
        hh.d(err7);
        hh.u(sm7);
        hh.bytes(&e7);
        hh.bytes(&w7[..n]);
        if n == 36 {
            let g = GRIDS[(l.next() % 9) as usize];
            let mut gw = [0u8; 36];
            downsample_ise_weights(wr, wr, 6, 6, g[0], g[1], &w, &mut gw);
            hh.bytes(&gw[..(g[0] * g[1]) as usize]);
            let r11 = refine_endpoints(
                11, er, &mut e, 6, 6, g[0], g[1], &gw, wr, n, &sh, &sq, None, &o, om,
            );
            hh.u(r11 as u32);
            hh.bytes(&e);
            downsample_ise_weights(wr, wr, 6, 6, g[0], g[1], &w7, &mut gw);
            let r7 = refine_endpoints(
                7, er, &mut e7, 6, 6, g[0], g[1], &gw, wr, n, &sh, &sq, None, &o, om,
            );
            hh.u(r7 as u32);
            hh.bytes(&e7);
        } else {
            let r11 = refine_endpoints(
                11,
                er,
                &mut e,
                6,
                6,
                6,
                6,
                &w,
                wr,
                n,
                &sh,
                &sq,
                Some(&ofs),
                &o,
                om,
            );
            hh.u(r11 as u32);
            hh.bytes(&e);
        }
    }
    if n == 36 {
        for c in 0..3 {
            let wr = 1 + (l.next() % 5);
            let er = 14 + (l.next() % 7);
            let mut e = [0u8; 6];
            let mut w0 = [0u8; 36];
            let mut w1 = [0u8; 36];
            let mut sm = 0u32;
            let constrain = (l.next() & 3) == 0;
            let err = encode_astc_hdr_block_mode_11_dual_plane(
                n,
                &sh,
                &sq,
                c,
                wr,
                &mut sm,
                BIG_FLOAT_VAL as f64,
                &mut e,
                &mut w0,
                &mut w1,
                &o,
                false,
                er,
                false,
                constrain,
                -1,
                7,
                false,
            );
            hh.d(err);
            hh.u(sm);
            hh.bytes(&e);
            hh.bytes(&w0[..n]);
            hh.bytes(&w1[..n]);
            let mut dp = [0u8; 72];
            let g = GRIDS[(l.next() % 9) as usize];
            downsample_ise_weights_dual_plane(wr, 1, 6, 6, g[0], g[1], &w0, &w1, &mut dp);
            hh.bytes(&dp[..(g[0] * g[1] * 2) as usize]);
        }
        for _ in 0..3 {
            let g = GRIDS[(l.next() % 9) as usize];
            let wr = 1 + (l.next() % 6);
            let er = 12 + (l.next() % 9);
            let om = OPT[(l.next() % 5) as usize];
            let mut e = [0u8; 6];
            let mut w = [0u8; 36];
            let mut sm = 0u32;
            let err = encode_astc_hdr_block_downsampled_mode_11(
                6,
                6,
                g[0],
                g[1],
                wr,
                er,
                n,
                &sh,
                &sq,
                BIG_FLOAT_VAL as f64,
                -1,
                7,
                false,
                om,
                &mut e,
                &mut w,
                &mut sm,
                &o,
                None,
            );
            hh.d(err);
            hh.u(sm);
            hh.bytes(&e);
            hh.bytes(&w[..(g[0] * g[1]) as usize]);
        }
    }
    let mut rnd = [0u8; 6];
    for x in rnd.iter_mut() {
        *x = l.next() as u8;
    }
    let (s11, m11) = decode_cem_11_config(&rnd);
    let (s7, m7) = decode_cem_7_config(&rnd);
    for v in [s11, m11, s7, m7] {
        hh.u(v as u32);
    }
    hh.0
}

#[test]
fn astc_hdr_enc_6x6_paths_match_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_astc_hdr_enc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    for (key, want) in &golden {
        let Ok(b) = key.parse::<u32>() else {
            panic!("bad key {key}");
        };
        let got = format!("{:016x}", block_hash(b));
        if *want != got {
            fails.push(format!("{key}: want {want} got {got}"));
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {} mismatches:\n{}",
            fails.len(),
            golden.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
