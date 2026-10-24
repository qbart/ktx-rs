// Golden-data generator for the astcenc compressor setup port (dev tooling, not
// part of the Rust library): astcenc_config_init for every block size, quality
// level / interpolation, flag set and profile, then astcenc_context_alloc with a
// hash of the complete compressor block size descriptor. Prints
// "<case>\t<config fields> <context config> <bsd hash>".
// Must stay in sync with tests/astcenc_bsd.rs. Build/run via
// scripts/golden/gen_astcenc_bsd_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <algorithm>
#include "astcenc.h"
#include "astcenc_internal_entry.h"

struct H {
    uint64_t h = 0xcbf29ce484222325ULL;
    void bytes(const void* p, size_t n) {
        const uint8_t* b = (const uint8_t*)p;
        for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 0x100000001b3ULL; }
    }
    template <typename T> void v(T x) { bytes(&x, sizeof(x)); }
};

static void cfg_hash(H& h, const astcenc_config& c) {
    h.v((uint32_t)c.profile); h.v(c.flags); h.v(c.block_x); h.v(c.block_y); h.v(c.block_z);
    h.v(c.cw_r_weight); h.v(c.cw_g_weight); h.v(c.cw_b_weight); h.v(c.cw_a_weight);
    h.v(c.a_scale_radius); h.v(c.rgbm_m_scale);
    h.v(c.tune_partition_count_limit); h.v(c.tune_2partition_index_limit); h.v(c.tune_3partition_index_limit);
    h.v(c.tune_4partition_index_limit); h.v(c.tune_block_mode_limit); h.v(c.tune_refinement_limit);
    h.v(c.tune_candidate_limit); h.v(c.tune_2partitioning_candidate_limit);
    h.v(c.tune_3partitioning_candidate_limit); h.v(c.tune_4partitioning_candidate_limit);
    h.v(c.tune_db_limit); h.v(c.tune_mse_overshoot); h.v(c.tune_2partition_early_out_limit_factor);
    h.v(c.tune_3partition_early_out_limit_factor); h.v(c.tune_2plane_early_out_limit_correlation);
    h.v(c.tune_search_mode0_enable);
}

static size_t rup(size_t n) { return (n + ASTCENC_SIMD_WIDTH - 1) / ASTCENC_SIMD_WIDTH * ASTCENC_SIMD_WIDTH; }

static uint64_t bsd_hash(const block_size_descriptor& b) {
    H h;
    h.v(b.dim_x); h.v(b.dim_y); h.v(b.dim_z); h.v(b.texel_count);
    h.v(b.decimation_mode_count_always); h.v(b.decimation_mode_count_selected); h.v(b.decimation_mode_count_all);
    h.v(b.block_mode_count_1plane_always); h.v(b.block_mode_count_1plane_selected);
    h.v(b.block_mode_count_1plane_2plane_selected); h.v(b.block_mode_count_all);
    for (int i = 0; i < 4; i++) { h.v(b.partitioning_count_selected[i]); h.v(b.partitioning_count_all[i]); }
    for (unsigned i = 0; i < WEIGHTS_MAX_DECIMATION_MODES; i++) {
        const auto& d = b.decimation_modes[i];
        h.v(d.maxprec_1plane); h.v(d.maxprec_2planes); h.v(d.refprec_1plane); h.v(d.refprec_2planes);
    }
    for (unsigned m = 0; m < b.decimation_mode_count_all; m++) {
        const auto& d = b.decimation_tables[m];
        h.v(d.texel_count); h.v(d.max_texel_weight_count); h.v(d.weight_count);
        h.v(d.weight_x); h.v(d.weight_y); h.v(d.weight_z);
        size_t tn = rup(d.texel_count), wn = rup(d.weight_count);
        h.bytes(d.texel_weight_count, tn);
        for (int k = 0; k < 4; k++) {
            h.bytes(d.texel_weights_tr[k], tn);
            h.bytes(d.texel_weight_contribs_int_tr[k], tn);
            h.bytes(d.texel_weight_contribs_float_tr[k], tn * 4);
        }
        h.bytes(d.weight_texel_count, wn);
        unsigned maxtcw = 0;
        for (unsigned i = 0; i < d.weight_count; i++) maxtcw = std::max<unsigned>(maxtcw, d.weight_texel_count[i]);
        for (unsigned j = 0; j < maxtcw; j++)
            for (size_t i = 0; i < wn; i++) { h.v(d.weight_texels_tr[j][i]); h.v(d.weights_texel_contribs_tr[j][i]); }
        for (unsigned i = 0; i < d.weight_count; i++)
            for (unsigned j = 0; j < d.weight_texel_count[i]; j++) h.v(d.texel_contrib_for_weight[j][i]);
    }
    h.bytes(b.block_mode_packed_index, sizeof(b.block_mode_packed_index));
    for (unsigned i = 0; i < b.block_mode_count_all; i++) {
        const auto& m = b.block_modes[i];
        h.v(m.mode_index); h.v(m.decimation_mode); h.v(m.quant_mode); h.v(m.weight_bits); h.v((uint8_t)m.is_dual_plane);
    }
    for (unsigned pc = 1; pc <= 4; pc++) {
        for (unsigned i = 0; i < b.partitioning_count_all[pc - 1]; i++) {
            const auto& p = b.get_raw_partition_info(pc, i);
            h.v(p.partition_count); h.v(p.partition_index); h.bytes(p.partition_texel_count, 4);
            h.bytes(p.partition_of_texel, b.texel_count);
            for (unsigned q = 0; q < pc; q++) h.bytes(p.texels_of_partition[q], rup(p.partition_texel_count[q]));
            if (pc == 2) h.bytes(b.coverage_bitmaps_2[i], 16);
            if (pc == 3) h.bytes(b.coverage_bitmaps_3[i], 24);
            if (pc == 4) h.bytes(b.coverage_bitmaps_4[i], 32);
        }
    }
    h.bytes(b.partitioning_packed_index, sizeof(b.partitioning_packed_index));
    h.bytes(b.kmeans_texels, std::min<unsigned>(b.texel_count, BLOCK_MAX_KMEANS_TEXELS));
    return h.h;
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 4, 1}, {5, 5, 1}, {6, 5, 1}, {6, 6, 1}, {8, 5, 1}, {8, 6, 1},
                                 {8, 8, 1}, {10, 5, 1}, {10, 6, 1}, {10, 8, 1}, {10, 10, 1}, {12, 10, 1}, {12, 12, 1},
                                 {3, 3, 3}, {4, 3, 3}, {4, 4, 3}, {4, 4, 4}, {5, 4, 4}, {5, 5, 4}, {5, 5, 5},
                                 {6, 5, 5}, {6, 6, 5}, {6, 6, 6}};
    const float QUALITIES[] = {0.0f, 10.0f, 60.0f, 98.0f, 99.0f, 100.0f, 35.0f, 80.0f, 99.5f, -1.0f, 101.0f};
    const unsigned FLAGS[] = {0, ASTCENC_FLG_SELF_DECOMPRESS_ONLY, ASTCENC_FLG_MAP_NORMAL, ASTCENC_FLG_USE_PERCEPTUAL,
                              ASTCENC_FLG_MAP_RGBM, ASTCENC_FLG_USE_ALPHA_WEIGHT,
                              ASTCENC_FLG_MAP_NORMAL | ASTCENC_FLG_SELF_DECOMPRESS_ONLY,
                              ASTCENC_FLG_MAP_NORMAL | ASTCENC_FLG_MAP_RGBM, ASTCENC_FLG_USE_DECODE_UNORM8, 1u << 9};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR, ASTCENC_PRF_HDR};
    for (const auto& s : SIZES)
        for (float q : QUALITIES)
            for (unsigned f : FLAGS)
                for (auto prof : PROFILES) {
                    // The full descriptor only for a subset (cost); config for everything.
                    bool full = (f == 0 || f == ASTCENC_FLG_SELF_DECOMPRESS_ONLY || f == ASTCENC_FLG_MAP_NORMAL) &&
                                prof == ASTCENC_PRF_LDR;
                    printf("%ux%ux%u q%g f%u p%d\t", s[0], s[1], s[2], q, f, (int)prof);
                    astcenc_config c;
                    astcenc_error e = astcenc_config_init(prof, s[0], s[1], s[2], q, f, &c);
                    if (e != ASTCENC_SUCCESS) { printf("E%d\n", (int)e); continue; }
                    H h1; cfg_hash(h1, c);
                    printf("%016llx", (unsigned long long)h1.h);
                    if (!full) { printf("\n"); continue; }
                    astcenc_context* ctx = nullptr;
                    e = astcenc_context_alloc(&c, 1, &ctx, nullptr);
                    if (e != ASTCENC_SUCCESS) { printf(" E%d\n", (int)e); continue; }
                    H h2; cfg_hash(h2, ctx->context.config);
                    printf(" %016llx %016llx\n", (unsigned long long)h2.h,
                           (unsigned long long)bsd_hash(*ctx->context.bsd));
                    astcenc_context_free(ctx);
                }
    return 0;
}
