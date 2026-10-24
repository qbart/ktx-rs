//! astcenc compressor setup (config presets, block size descriptor, partition tables) vs.
//! golden output of the C++ original (`scripts/golden/gen_astcenc_bsd_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::astcenc::internal::*;
use ktx::astcenc::vecmath::round_up_to_simd_multiple_vla as rup;
use ktx::astcenc::*;

struct H(u64);
impl H {
    fn new() -> H {
        H(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    fn i8(&mut self, v: i8) {
        self.bytes(&v.to_le_bytes());
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn cfg_hash(c: &AstcencConfig) -> u64 {
    let mut h = H::new();
    h.u32(c.profile as u32);
    for v in [c.flags, c.block_x, c.block_y, c.block_z] {
        h.u32(v);
    }
    for v in [c.cw_r_weight, c.cw_g_weight, c.cw_b_weight, c.cw_a_weight] {
        h.f32(v);
    }
    h.u32(c.a_scale_radius);
    h.f32(c.rgbm_m_scale);
    for v in [
        c.tune_partition_count_limit,
        c.tune_2partition_index_limit,
        c.tune_3partition_index_limit,
        c.tune_4partition_index_limit,
        c.tune_block_mode_limit,
        c.tune_refinement_limit,
        c.tune_candidate_limit,
        c.tune_2partitioning_candidate_limit,
        c.tune_3partitioning_candidate_limit,
        c.tune_4partitioning_candidate_limit,
    ] {
        h.u32(v);
    }
    for v in [
        c.tune_db_limit,
        c.tune_mse_overshoot,
        c.tune_2partition_early_out_limit_factor,
        c.tune_3partition_early_out_limit_factor,
        c.tune_2plane_early_out_limit_correlation,
        c.tune_search_mode0_enable,
    ] {
        h.f32(v);
    }
    h.0
}

fn bsd_hash(b: &BlockSizeDescriptor) -> u64 {
    let mut h = H::new();
    for v in [b.dim_x, b.dim_y, b.dim_z, b.texel_count] {
        h.u8(v);
    }
    for v in [
        b.decimation_mode_count_always,
        b.decimation_mode_count_selected,
        b.decimation_mode_count_all,
        b.block_mode_count_1plane_always,
        b.block_mode_count_1plane_selected,
        b.block_mode_count_1plane_2plane_selected,
        b.block_mode_count_all,
    ] {
        h.u32(v);
    }
    for i in 0..4 {
        h.u32(b.partitioning_count_selected[i]);
        h.u32(b.partitioning_count_all[i]);
    }
    for d in &b.decimation_modes {
        h.i8(d.maxprec_1plane);
        h.i8(d.maxprec_2planes);
        h.u16(d.refprec_1plane);
        h.u16(d.refprec_2planes);
    }
    for d in b
        .decimation_tables
        .iter()
        .take(b.decimation_mode_count_all as usize)
    {
        for v in [
            d.texel_count,
            d.max_texel_weight_count,
            d.weight_count,
            d.weight_x,
            d.weight_y,
            d.weight_z,
        ] {
            h.u8(v);
        }
        let tn = rup(d.texel_count as usize);
        let wn = rup(d.weight_count as usize);
        h.bytes(&d.texel_weight_count[..tn]);
        for k in 0..4 {
            h.bytes(&d.texel_weights_tr[k][..tn]);
            h.bytes(&d.texel_weight_contribs_int_tr[k][..tn]);
            for v in &d.texel_weight_contribs_float_tr[k][..tn] {
                h.f32(*v);
            }
        }
        h.bytes(&d.weight_texel_count[..wn]);
        let maxtcw = d.weight_texel_count[..d.weight_count as usize]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        for j in 0..maxtcw as usize {
            for i in 0..wn {
                h.u8(d.weight_texels_tr[j][i]);
                h.f32(d.weights_texel_contribs_tr[j][i]);
            }
        }
        for i in 0..d.weight_count as usize {
            for j in 0..d.weight_texel_count[i] as usize {
                h.f32(d.texel_contrib_for_weight[j][i]);
            }
        }
    }
    for v in &b.block_mode_packed_index {
        h.u16(*v);
    }
    for m in b.block_modes.iter().take(b.block_mode_count_all as usize) {
        h.u16(m.mode_index);
        h.u8(m.decimation_mode);
        h.u8(m.quant_mode);
        h.u8(m.weight_bits);
        h.u8(m.is_dual_plane as u8);
    }
    for pc in 1..=4u32 {
        for i in 0..b.partitioning_count_all[pc as usize - 1] {
            let p = b.get_raw_partition_info(pc, i);
            h.u16(p.partition_count);
            h.u16(p.partition_index);
            h.bytes(&p.partition_texel_count);
            h.bytes(&p.partition_of_texel[..b.texel_count as usize]);
            for q in 0..pc as usize {
                h.bytes(&p.texels_of_partition[q][..rup(p.partition_texel_count[q] as usize)]);
            }
            let i = i as usize;
            match pc {
                2 => b.coverage_bitmaps_2[i].iter().for_each(|v| h.u64(*v)),
                3 => b.coverage_bitmaps_3[i].iter().for_each(|v| h.u64(*v)),
                4 => b.coverage_bitmaps_4[i].iter().for_each(|v| h.u64(*v)),
                _ => {}
            }
        }
    }
    for row in &b.partitioning_packed_index {
        for v in row {
            h.u16(*v);
        }
    }
    h.bytes(&b.kmeans_texels[..(b.texel_count as usize).min(BLOCK_MAX_KMEANS_TEXELS)]);
    h.0
}

fn fmt_g(q: f32) -> String {
    if q.fract() == 0.0 {
        format!("{}", q as i32)
    } else {
        format!("{q}")
    }
}

#[test]
fn compressor_setup_matches_astcenc() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/astcenc_bsd.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sizes: [(u32, u32, u32); 24] = [
        (4, 4, 1),
        (5, 4, 1),
        (5, 5, 1),
        (6, 5, 1),
        (6, 6, 1),
        (8, 5, 1),
        (8, 6, 1),
        (8, 8, 1),
        (10, 5, 1),
        (10, 6, 1),
        (10, 8, 1),
        (10, 10, 1),
        (12, 10, 1),
        (12, 12, 1),
        (3, 3, 3),
        (4, 3, 3),
        (4, 4, 3),
        (4, 4, 4),
        (5, 4, 4),
        (5, 5, 4),
        (5, 5, 5),
        (6, 5, 5),
        (6, 6, 5),
        (6, 6, 6),
    ];
    let qualities = [
        0.0f32, 10.0, 60.0, 98.0, 99.0, 100.0, 35.0, 80.0, 99.5, -1.0, 101.0,
    ];
    let flags = [
        0,
        ASTCENC_FLG_SELF_DECOMPRESS_ONLY,
        ASTCENC_FLG_MAP_NORMAL,
        ASTCENC_FLG_USE_PERCEPTUAL,
        ASTCENC_FLG_MAP_RGBM,
        ASTCENC_FLG_USE_ALPHA_WEIGHT,
        ASTCENC_FLG_MAP_NORMAL | ASTCENC_FLG_SELF_DECOMPRESS_ONLY,
        ASTCENC_FLG_MAP_NORMAL | ASTCENC_FLG_MAP_RGBM,
        ASTCENC_FLG_USE_DECODE_UNORM8,
        1 << 9,
    ];
    let profiles = [AstcencProfile::Ldr, AstcencProfile::Hdr];
    let mut fails = Vec::new();
    let mut n = 0;
    for &(x, y, z) in &sizes {
        for &q in &qualities {
            for &f in &flags {
                for &p in &profiles {
                    let full = (f == 0
                        || f == ASTCENC_FLG_SELF_DECOMPRESS_ONLY
                        || f == ASTCENC_FLG_MAP_NORMAL)
                        && p == AstcencProfile::Ldr;
                    let key = format!("{x}x{y}x{z} q{} f{f} p{}", fmt_g(q), p as i32);
                    let got = match astcenc_config_init(p, x, y, z, q, f) {
                        Err(e) => format!("E{}", e as i32),
                        Ok(c) => {
                            let mut s = format!("{:016x}", cfg_hash(&c));
                            if full {
                                match astcenc_context_alloc(&c, 1) {
                                    Err(e) => s += &format!(" E{}", e as i32),
                                    Ok(ctx) => {
                                        let bh = ctx.compress_bsd().map_or(0, bsd_hash);
                                        s += &format!(" {:016x} {bh:016x}", cfg_hash(&ctx.config));
                                    }
                                }
                            }
                            s
                        }
                    };
                    n += 1;
                    match golden.get(&key) {
                        Some(want) if *want == got => {}
                        Some(want) => fails.push(format!("{key}: want {want} got {got}")),
                        None => fails.push(format!("{key}: missing")),
                    }
                }
            }
        }
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!(
            "{} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
