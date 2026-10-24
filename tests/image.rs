//! imageio `image.hpp` port vs. golden output of the C++ original
//! (`scripts/golden/gen_image_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::resampler::BoundaryOp;
use ktx::cli::imageio::image::*;
use ktx::dfd::khr_df::{KHR_DF_PRIMARIES_DISPLAYP3, KHR_DF_TRANSFER_LINEAR, KHR_DF_TRANSFER_SRGB};

const FNV0: u64 = 0xcbf29ce484222325;

fn fnv(mut h: u64, b: &[u8]) -> u64 {
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn hv(v: &[u8]) -> u64 {
    fnv(FNV0, v)
}

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

const SPECIALS: [f32; 16] = [
    0.0,
    1.0,
    -0.0,
    0.5,
    1e-8,
    65504.0,
    70000.0,
    1e30,
    -1e30,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NAN,
    16777216.0,
    -0.25,
    0.999,
    3e9,
];

/// LCG pixel generator of the harness (`gen<T>`).
trait Gen: Component {
    fn gen_value(l: &mut Lcg) -> Self;
}

macro_rules! gen_int {
    ($t:ty) => {
        impl Gen for $t {
            fn gen_value(l: &mut Lcg) -> Self {
                let r = l.next();
                let sel = l.next() % 8;
                match sel {
                    0 => <$t>::MAX,
                    1 => <$t>::MIN,
                    2 => 0,
                    _ => ((r << 8) ^ l.next()) as $t,
                }
            }
        }
    };
}
gen_int!(u8);
gen_int!(u16);
gen_int!(u32);
gen_int!(i8);
gen_int!(i16);
gen_int!(i32);

impl Gen for f32 {
    fn gen_value(l: &mut Lcg) -> Self {
        let r = l.next();
        let sel = l.next() % 8;
        if sel == 0 {
            return SPECIALS[(l.next() % 16) as usize];
        }
        (r & 0xffff) as f32 / 65535.0 * 2.5 - 0.5
    }
}

fn fill<T: Gen, const N: usize>(img: &mut ImageT<T, N>, seed: u32) {
    let mut l = Lcg(seed);
    for p in img.pixels.iter_mut() {
        for c in p.iter_mut() {
            *c = T::gen_value(&mut l);
        }
    }
}

fn himg(img: &dyn Image) -> u64 {
    let h = fnv(FNV0, &img.as_bytes()[..img.get_byte_count()]);
    let mut meta = Vec::new();
    for v in [
        img.get_width(),
        img.get_height(),
        img.get_transfer_function(),
        img.get_primaries(),
    ] {
        meta.extend_from_slice(&v.to_le_bytes());
    }
    fnv(h, &meta)
}

fn prims() -> [ColorPrimaries; 11] {
    [
        ColorPrimaries::bt709(),
        ColorPrimaries::bt601_625_ebu(),
        ColorPrimaries::bt601_525_smpte(),
        ColorPrimaries::bt2020(),
        ColorPrimaries::ciexyz(),
        ColorPrimaries::aces(),
        ColorPrimaries::acescc(),
        ColorPrimaries::ntsc1953(),
        ColorPrimaries::pal525(),
        ColorPrimaries::display_p3(),
        ColorPrimaries::adobe_rgb(),
    ]
}

fn tfs() -> Vec<Box<dyn TransferFunction>> {
    vec![
        Box::new(TransferFunctionLinear),
        Box::new(TransferFunctionSRGB),
        Box::new(TransferFunctionITU::default()),
        Box::new(TransferFunctionGamma::new(2.2)),
        Box::new(TransferFunctionGamma::new(0.45)),
        Box::new(TransferFunctionBT2100PqEotf),
    ]
}

const FILTERS: [&str; 5] = ["box", "tent", "lanczos4", "kaiser", "mitchell"];
const RSIZES: [(u32, u32); 5] = [(3, 2), (9, 11), (7, 5), (1, 1), (16, 3)];
const OPS: [BoundaryOp; 3] = [BoundaryOp::Wrap, BoundaryOp::Reflect, BoundaryOp::Clamp];

struct Out(Vec<(String, String)>);
impl Out {
    fn put(&mut self, t: usize, op: &str, h: u64) {
        self.0.push((format!("t{t} {op}"), format!("{h:016x}")));
    }
    fn err(&mut self, t: usize, op: &str, e: &str) {
        self.0.push((format!("t{t} {op}"), format!("E:{e}")));
    }
}

fn run_type<T: Gen, const N: usize>(t: usize, o: &mut Out) {
    const W: u32 = 5;
    const H: u32 = 3;
    let fresh = || {
        let mut img = ImageT::<T, N>::new(W, H);
        fill(&mut img, 1000 + t as u32);
        img
    };
    let base = fresh();
    o.put(t, "base", himg(&base));
    o.put(
        t,
        "info",
        ((base.get_byte_count() as u64) << 32)
            | ((base.get_pixel_size() as u64) << 16)
            | ((base.get_component_count() as u64) << 8)
            | base.get_component_size() as u64,
    );
    if T::SIZE != 4 {
        for nc in 1..=4 {
            for bits in [8u32, 16, 32] {
                for sb in [0, bits - 2] {
                    o.put(
                        t,
                        &format!("unorm c{nc} b{bits} s{sb}"),
                        hv(&base.get_unorm(nc, bits, sb)),
                    );
                }
            }
        }
        let pk: [[u32; 4]; 13] = [
            [5, 6, 5, 0],
            [4, 4, 4, 4],
            [5, 5, 5, 1],
            [1, 5, 5, 5],
            [8, 8, 8, 8],
            [10, 10, 10, 2],
            [2, 10, 10, 10],
            [16, 16, 16, 16],
            [8, 8, 0, 0],
            [3, 3, 2, 0],
            [32, 32, 0, 0],
            [4, 4, 0, 0],
            [16, 0, 0, 0],
        ];
        for p in pk {
            o.put(
                t,
                &format!("unormpacked {},{},{},{}", p[0], p[1], p[2], p[3]),
                hv(&base.get_unorm_packed(p[0], p[1], p[2], p[3])),
            );
        }
    }
    for nc in 1..=4 {
        for bits in [16, 32] {
            o.put(
                t,
                &format!("sfloat c{nc} b{bits}"),
                hv(&base.get_sfloat(nc, bits)),
            );
        }
        for bits in [8, 16, 32, 64] {
            o.put(
                t,
                &format!("uint c{nc} b{bits}"),
                hv(&base.get_uint(nc, bits)),
            );
            o.put(
                t,
                &format!("sint c{nc} b{bits}"),
                hv(&base.get_sint(nc, bits)),
            );
        }
    }
    o.put(t, "b10g11r11", hv(&base.get_b10g11r11()));
    o.put(t, "e5b9g9r9", hv(&base.get_e5b9g9r9()));
    for p in [
        [8, 8, 8, 8],
        [10, 10, 10, 2],
        [2, 10, 10, 10],
        [11, 11, 5, 5],
    ] {
        let k = format!("{},{},{},{}", p[0], p[1], p[2], p[3]);
        o.put(
            t,
            &format!("uintpacked {k}"),
            hv(&base.get_uint_packed(p[0], p[1], p[2], p[3])),
        );
        o.put(
            t,
            &format!("sintpacked {k}"),
            hv(&base.get_sint_packed(p[0], p[1], p[2], p[3])),
        );
    }
    {
        let mut img = fresh();
        img.yflip();
        o.put(t, "yflip", himg(&img));
        let mut img2 = ImageT::<T, N>::new(3, 4);
        fill(&mut img2, 7 + t as u32);
        img2.yflip();
        o.put(t, "yflip4", himg(&img2));
    }
    let tfs = tfs();
    let prims = prims();
    let tcs: [(usize, usize, i32); 11] = [
        (0, 1, -1),
        (1, 0, -1),
        (1, 1, -1),
        (2, 0, -1),
        (0, 2, -1),
        (3, 0, -1),
        (0, 5, -1),
        (5, 0, -1),
        (1, 0, 9),
        (0, 0, 3),
        (4, 1, 10),
    ];
    for (d, e, p) in tcs {
        let mut img = fresh();
        let m = (p >= 0).then(|| prims[p as usize].transform_to(&prims[0]));
        img.transform_color_space(tfs[d].as_ref(), tfs[e].as_ref(), m.as_ref());
        o.put(t, &format!("tcs {d},{e},{p}"), himg(&img));
    }
    {
        let mut img = fresh();
        img.normalize();
        o.put(t, "normalize", himg(&img));
    }
    for s in ["rgba", "bgr1", "a0g1", "rrrr", "gbar", "01ba"] {
        let mut img = fresh();
        img.swizzle(s);
        o.put(t, &format!("swizzle {s}"), himg(&img));
    }
    for s in ["rgba", "abgr"] {
        let mut src = fresh();
        src.set_transfer_function(KHR_DF_TRANSFER_SRGB);
        src.set_primaries(KHR_DF_PRIMARIES_DISPLAYP3);
        let mut d1 = ImageT::<T, 1>::new(W, H);
        let mut d2 = ImageT::<T, 2>::new(W, H);
        let mut d3 = ImageT::<T, 3>::new(W, H);
        let mut d4 = ImageT::<T, 4>::new(W, H);
        let ok = src.copy_to_r(&mut d1, s).is_ok()
            && src.copy_to_rg(&mut d2, s).is_ok()
            && src.copy_to_rgb(&mut d3, s).is_ok()
            && src.copy_to_rgba(&mut d4, s).is_ok();
        assert!(ok);
        let mut hs = Vec::new();
        for h in [himg(&d1), himg(&d2), himg(&d3), himg(&d4)] {
            hs.extend_from_slice(&h.to_le_bytes());
        }
        o.put(t, &format!("copy {s}"), hv(&hs));
        // Mismatched destination component type is rejected instead of UB.
        let mut wrong_f = ImageT::<f32, 1>::new(W, H);
        let mut wrong_u = ImageT::<u8, 1>::new(W, H);
        let wrong: &mut dyn Image = if T::IS_FLOAT {
            &mut wrong_u
        } else {
            &mut wrong_f
        };
        assert!(src.copy_to_r(wrong, s).is_err());
    }
    for tf in [0, 1] {
        let mut img = fresh();
        img.set_transfer_function(if tf == 1 {
            KHR_DF_TRANSFER_SRGB
        } else {
            KHR_DF_TRANSFER_LINEAR
        });
        img.premultiply_alpha();
        o.put(t, &format!("premultiply {tf}"), himg(&img));
    }
    for (i, (s, off)) in [(2.0f32, -0.5f32), (1.0, 0.0), (0.5, 0.25)]
        .into_iter()
        .enumerate()
    {
        let mut img = fresh();
        img.map_range(s, off);
        o.put(t, &format!("maprange {i}"), himg(&img));
    }
    let mut src = ImageT::<T, N>::new(7, 5);
    fill(&mut src, 500 + t as u32);
    for (f, filter) in FILTERS.iter().enumerate() {
        for (z, &(w, h)) in RSIZES.iter().enumerate() {
            for tf in 0..2 {
                for sc in 0..2 {
                    src.set_transfer_function(if tf == 1 {
                        KHR_DF_TRANSFER_SRGB
                    } else {
                        KHR_DF_TRANSFER_LINEAR
                    });
                    let k = format!("resample f{f} z{z} tf{tf} s{sc}");
                    let scale = if sc == 1 { 0.5 } else { 1.0 };
                    match src.resample(w, h, filter, scale, OPS[(f + z) % 3]) {
                        Ok(r) => o.put(t, &k, himg(r.as_ref())),
                        Err(e) => o.err(t, &k, e.what()),
                    }
                }
            }
        }
    }
    for (name, w, h) in [("nope", 3, 3), ("box", 16385, 1)] {
        let k = format!("resample err {name}");
        match src.resample(w, h, name, 1.0, BoundaryOp::Clamp) {
            Ok(r) => o.put(t, &k, himg(r.as_ref())),
            Err(e) => o.err(t, &k, e.what()),
        }
    }
}

#[test]
fn image_matches_imageio() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/image.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut o = Out(Vec::new());

    let mut ins: Vec<f32> = (-20..=120).map(|i| i as f32 / 100.0).collect();
    ins.extend_from_slice(&SPECIALS);
    for (i, tf) in tfs().iter().enumerate() {
        let (mut he, mut hd) = (FNV0, FNV0);
        for &x in &ins {
            he = fnv(he, &tf.encode(x).to_le_bytes());
            hd = fnv(hd, &tf.decode(x).to_le_bytes());
        }
        o.0.push((format!("tf{i}"), format!("{he:016x} {hd:016x}")));
    }
    let prims = prims();
    for (a, pa) in prims.iter().enumerate() {
        let mut h = FNV0;
        for pb in &prims {
            let m = pa.transform_to(pb);
            for v in m.matrix.as_flattened() {
                h = fnv(h, &v.to_le_bytes());
            }
        }
        o.0.push((format!("prim{a}"), format!("{h:016x}")));
    }

    run_type::<u8, 1>(0, &mut o);
    run_type::<u8, 2>(1, &mut o);
    run_type::<u8, 3>(2, &mut o);
    run_type::<u8, 4>(3, &mut o);
    run_type::<u16, 1>(4, &mut o);
    run_type::<u16, 2>(5, &mut o);
    run_type::<u16, 3>(6, &mut o);
    run_type::<u16, 4>(7, &mut o);
    run_type::<u32, 1>(8, &mut o);
    run_type::<u32, 2>(9, &mut o);
    run_type::<u32, 3>(10, &mut o);
    run_type::<u32, 4>(11, &mut o);
    run_type::<i8, 1>(12, &mut o);
    run_type::<i8, 2>(13, &mut o);
    run_type::<i8, 3>(14, &mut o);
    run_type::<i8, 4>(15, &mut o);
    run_type::<i16, 1>(16, &mut o);
    run_type::<i16, 2>(17, &mut o);
    run_type::<i16, 3>(18, &mut o);
    run_type::<i16, 4>(19, &mut o);
    run_type::<i32, 1>(20, &mut o);
    run_type::<i32, 2>(21, &mut o);
    run_type::<i32, 3>(22, &mut o);
    run_type::<i32, 4>(23, &mut o);
    run_type::<f32, 1>(24, &mut o);
    run_type::<f32, 2>(25, &mut o);
    run_type::<f32, 3>(26, &mut o);
    run_type::<f32, 4>(27, &mut o);

    assert_eq!(o.0.len(), golden.len());
    let mut fails = Vec::new();
    for (k, v) in &o.0 {
        match golden.get(k) {
            Some(want) if want == v => {}
            Some(want) => fails.push(format!("{k}: want {want} got {v}")),
            None => fails.push(format!("{k}: missing golden")),
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
