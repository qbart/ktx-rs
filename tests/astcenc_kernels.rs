//! astcenc compressor kernels vs. golden output of the C++ original
//! (`scripts/golden/gen_astcenc_kernels_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::astcenc::averages_and_directions::*;
use ktx::astcenc::image::{load_image_block, load_image_block_fast_ldr};
use ktx::astcenc::internal::*;
use ktx::astcenc::vecmath::*;
use ktx::astcenc::*;

struct H(u64);
impl H {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn f(&mut self, x: f32) {
        let u = if x.is_nan() {
            0x7fc00000u32
        } else {
            x.to_bits()
        };
        self.bytes(&u.to_le_bytes());
    }
    fn v4(&mut self, a: Vfloat4) {
        for l in a.m {
            self.f(l);
        }
    }
    fn u(&mut self, x: u32) {
        self.bytes(&x.to_le_bytes());
    }
}

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
    fn unit(&mut self) -> f32 {
        (self.next() & 0xffff) as f32 / 65535.0
    }
}

fn hash_block(h: &mut H, b: &ImageBlock) {
    for i in 0..b.texel_count as usize {
        h.f(b.data_r[i]);
        h.f(b.data_g[i]);
        h.f(b.data_b[i]);
        h.f(b.data_a[i]);
    }
    for v in [b.origin_texel, b.data_min, b.data_mean, b.data_max] {
        h.v4(v);
    }
    h.u(b.grayscale as u32);
    h.u(b.rgb_lns[0] as u32);
    h.u(b.alpha_lns[0] as u32);
}

#[test]
fn kernels_match_astcenc() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/astcenc_kernels.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sizes: [(u32, u32, u32); 9] = [
        (4, 4, 1),
        (5, 5, 1),
        (6, 6, 1),
        (8, 8, 1),
        (10, 5, 1),
        (12, 12, 1),
        (3, 3, 3),
        (4, 4, 4),
        (6, 6, 6),
    ];
    let profiles = [
        AstcencProfile::Ldr,
        AstcencProfile::LdrSrgb,
        AstcencProfile::HdrRgbLdrA,
        AstcencProfile::Hdr,
    ];
    let swz = [
        AstcencSwizzle::RGBA,
        AstcencSwizzle {
            r: AstcencSwz::B,
            g: AstcencSwz::R,
            b: AstcencSwz::One,
            a: AstcencSwz::G,
        },
    ];
    let unit3 = Vfloat4::new(0.57735, 0.57735, 0.57735, 0.0);
    let mut fails = Vec::new();
    let mut n = 0;
    for (si, &(sx, sy, sz)) in sizes.iter().enumerate() {
        let c = match astcenc_config_init(AstcencProfile::Ldr, sx, sy, sz, 60.0, 0) {
            Ok(c) => c,
            Err(e) => panic!("{e:?}"),
        };
        let ctx = match astcenc_context_alloc_isa(&c, 1, AstcencIsa::Baseline) {
            Ok(c) => c,
            Err(e) => panic!("{e:?}"),
        };
        let Some(bsd) = ctx.compress_bsd() else {
            panic!("no bsd")
        };
        let (dx, dy) = (sx * 2 - 1, sy * 2 - 1);
        let dz = if sz == 1 { 1 } else { sz * 2 - 1 };
        for dt in 0..3 {
            for kind in 0..4u32 {
                let mut l = Lcg((si * 100 + dt * 10 + kind as usize) as u32);
                let mut p8 = vec![Vec::new(); dz as usize];
                let mut p16 = vec![Vec::new(); dz as usize];
                let mut p32 = vec![Vec::new(); dz as usize];
                for z in 0..dz as usize {
                    for i in 0..(dx * dy * 4) {
                        let mut v = l.unit();
                        if kind == 1 {
                            v = if i % 4 == 3 { 1.0 } else { 0.5 };
                        }
                        if kind == 2 && i % 4 != 3 {
                            v = ((i / 4) % 7) as f32 / 6.0;
                        }
                        if kind == 3 {
                            v = v * 4.0 - 1.0;
                        }
                        let mut b8 = (l.next() & 0xff) as u8;
                        if kind == 1 {
                            b8 = if i % 4 == 3 { 255 } else { 128 };
                        }
                        if kind == 2 && i % 4 != 3 {
                            b8 = ((i / 4) * 37) as u8;
                        }
                        p8[z].push(b8);
                        let hb = l.next();
                        let mut half = if kind == 3 {
                            (hb & 0xffff) as u16
                        } else {
                            (hb % 0x3c01) as u16
                        };
                        if kind == 1 {
                            half = if i % 4 == 3 { 0x3c00 } else { 0x3800 };
                        }
                        p16[z].push(half);
                        p32[z].push(v);
                    }
                }
                let img = AstcencImage {
                    dim_x: dx,
                    dim_y: dy,
                    dim_z: dz,
                    data: match dt {
                        0 => AstcencImageData::U8(p8),
                        1 => AstcencImageData::F16(p16),
                        _ => AstcencImageData::F32(p32),
                    },
                };
                for (pi_, &prof) in profiles.iter().enumerate() {
                    for (sw, sws) in swz.iter().enumerate() {
                        for bz in 0..(if sz == 1 { 1 } else { 2 }) {
                            for by in 0..2usize {
                                for bx in 0..2usize {
                                    let mut blk = ImageBlock {
                                        texel_count: (sx * sy * sz) as u8,
                                        channel_weight: Vfloat4::new(
                                            1.0,
                                            0.5 + kind as f32 * 0.25,
                                            2.0,
                                            0.75,
                                        ),
                                        ..ImageBlock::default()
                                    };
                                    load_image_block(
                                        prof,
                                        &img,
                                        &mut blk,
                                        bsd,
                                        bx * sx as usize,
                                        by * sy as usize,
                                        bz * sz as usize,
                                        sws,
                                    );
                                    let mut h = H(0xcbf29ce484222325);
                                    hash_block(&mut h, &blk);
                                    let key =
                                        format!("s{si} t{dt} k{kind} p{pi_} w{sw} b{bx}{by}{bz}");
                                    if dt == 0 && pi_ <= 1 && sw == 0 && sz == 1 {
                                        let mut fb = ImageBlock {
                                            texel_count: blk.texel_count,
                                            ..ImageBlock::default()
                                        };
                                        load_image_block_fast_ldr(
                                            &img,
                                            &mut fb,
                                            bsd,
                                            bx * sx as usize,
                                            by * sy as usize,
                                        );
                                        hash_block(&mut h, &fb);
                                    }
                                    for pc in 1..=4u32 {
                                        let np = bsd.partitioning_count_selected[pc as usize - 1];
                                        for k in 0..3u32.min(np) {
                                            let pi = bsd.get_raw_partition_info(
                                                pc,
                                                (k * 37 + bx as u32 + by as u32) % np,
                                            );
                                            let mut pm = [PartitionMetrics::default(); 4];
                                            let dump = |h: &mut H, pm: &[PartitionMetrics; 4]| {
                                                for q in 0..pc as usize {
                                                    h.v4(pm[q].avg);
                                                    h.v4(pm[q].dir);
                                                }
                                            };
                                            compute_avgs_and_dirs_4_comp(pi, &blk, &mut pm);
                                            dump(&mut h, &pm);
                                            for om in 0..4 {
                                                compute_avgs_and_dirs_3_comp(pi, &blk, om, &mut pm);
                                                dump(&mut h, &pm);
                                            }
                                            compute_avgs_and_dirs_3_comp_rgb(pi, &blk, &mut pm);
                                            dump(&mut h, &pm);
                                            for (c1, c2) in [(0, 1), (0, 2), (1, 2)] {
                                                compute_avgs_and_dirs_2_comp(
                                                    pi, &blk, c1, c2, &mut pm,
                                                );
                                                dump(&mut h, &pm);
                                            }
                                            compute_avgs_and_dirs_3_comp_rgb(pi, &blk, &mut pm);
                                            let mut pl = [PartitionLines3::default(); 4];
                                            let mut up4 = [ProcessedLine::default(); 4];
                                            let mut sp4 = [ProcessedLine::default(); 4];
                                            for q in 0..pc as usize {
                                                let dir = normalize_safe(pm[q].dir, unit3);
                                                let avg = pm[q].avg;
                                                let sdir = normalize_safe(avg, unit3);
                                                pl[q].uncor_pline.amod = avg - dir * avg.dot(dir);
                                                pl[q].uncor_pline.bs = dir;
                                                pl[q].samec_pline.amod = Vfloat4::zero();
                                                pl[q].samec_pline.bs = sdir;
                                                up4[q].amod = Vfloat4::new(
                                                    avg.m[0], avg.m[1], avg.m[2], 0.25,
                                                );
                                                up4[q].bs =
                                                    Vfloat4::new(dir.m[0], dir.m[1], dir.m[2], 0.5);
                                                sp4[q].amod = Vfloat4::zero();
                                                sp4[q].bs = Vfloat4::new(
                                                    sdir.m[0], sdir.m[1], sdir.m[2], 0.5,
                                                );
                                            }
                                            let (mut ue, mut se) = (0.0, 0.0);
                                            compute_error_squared_rgb(
                                                pi, &blk, &mut pl, &mut ue, &mut se,
                                            );
                                            h.f(ue);
                                            h.f(se);
                                            for q in 0..pc as usize {
                                                h.f(pl[q].line_length);
                                            }
                                            let mut ll = [0.0f32; 4];
                                            compute_error_squared_rgba(
                                                pi, &blk, &up4, &sp4, &mut ll, &mut ue, &mut se,
                                            );
                                            h.f(ue);
                                            h.f(se);
                                            for q in 0..pc as usize {
                                                h.f(ll[q]);
                                            }
                                        }
                                    }
                                    n += 1;
                                    let got = format!("{:016x}", h.0);
                                    match golden.get(&key) {
                                        Some(w) if *w == got => {}
                                        Some(w) => fails.push(format!("{key}: want {w} got {got}")),
                                        None => fails.push(format!("{key}: missing")),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
