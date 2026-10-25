// Golden-data generator for the UASTC HDR 4x4 encoder port (src/basisu/uastc_hdr_4x4_enc.rs,
// astc_hdr_enc.rs; dev tooling, not part of the Rust library): astc_hdr_4x4_enc_block on
// generated HDR blocks for every quality level and option variant, and unpack_bc6h on random
// blocks. Prints "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_uastc_hdr_4x4_enc.rs.
// Build/run via scripts/golden/gen_basisu_uastc_hdr_4x4_enc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_gpu_texture.h"
#include "basisu_uastc_hdr_4x4_enc.h"
#include "../transcoder/basisu_transcoder.h"

using namespace basisu;

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
};

struct H {
    uint64_t h = 0xcbf29ce484222325ULL;
    void bytes(const void* p, size_t n) {
        const uint8_t* b = (const uint8_t*)p;
        for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 0x100000001b3ULL; }
    }
    void u(uint32_t v) { bytes(&v, 4); }
    void d(double v) { bytes(&v, 8); }
};

// A random positive finite half in roughly [2^lo, 2^hi).
static uint16_t rand_half(Lcg& l, int lo, int hi) {
    int e = lo + (int)(l.next() % (uint32_t)(hi - lo + 1)) + 15;
    if (e < 0) e = 0;
    if (e > 30) e = 30;
    return (uint16_t)((e << 10) | (l.next() & 1023));
}

static void make_block(uint32_t b, uint16_t h[48]) {
    Lcg l{b * 7919u + 17u};
    const uint32_t kind = b % 9;
    uint16_t base[3] = { rand_half(l, -6, 6), rand_half(l, -6, 6), rand_half(l, -6, 6) };
    uint16_t base2[3] = { rand_half(l, -6, 6), rand_half(l, -6, 6), rand_half(l, -6, 6) };
    for (uint32_t i = 0; i < 16; i++) {
        const uint32_t x = i & 3, y = i >> 2;
        for (uint32_t c = 0; c < 3; c++) {
            uint32_t v;
            switch (kind) {
            case 0: v = rand_half(l, -10, 15); break;                              // wild
            case 1: v = base[0] + (l.next() % 64); break;                           // greyscale-ish (fixed below)
            case 2: v = base[c]; break;                                             // solid
            case 3: v = base[c] + ((i == 5) ? 1 : 0); break;                        // near solid
            case 4: v = base[c] + (x * 40 + y * 25) * (c + 1); break;               // gradient
            case 5: v = (((x >> 1) + (y >> 1)) & 1) ? base[c] : base2[c]; break;     // two colors
            case 6: v = (l.next() & 7) == 0 ? 0 : rand_half(l, -14, -8); break;     // tiny / zero
            case 7: v = rand_half(l, 8, 15); break;                                 // bright
            default: v = base[c] + (l.next() % 200); break;                         // noisy
            }
            if (v > 0x7BFF) v = 0x7BFF;
            h[i * 3 + c] = (uint16_t)v;
        }
        if (kind == 1) h[i * 3 + 1] = h[i * 3 + 2] = h[i * 3 + 0];
    }
}

struct Cfg { int level; bool perceptual; bool uber; bool ultra; bool favor_astc; };
static const Cfg CFGS[] = {
    {0, true, false, false, false},
    {1, true, false, false, false},
    {2, true, false, false, false},
    {3, true, false, false, false},
    {4, true, false, false, false},
    {0, false, false, false, false},
    {1, false, false, false, true},
    {2, false, true, false, false},
    {3, true, true, true, false},
    {4, false, true, true, true},
    {1, true, false, true, false},
};

int main() {
    basisu_encoder_init(false, false);
    const uint32_t NB = 600;
    const uint32_t nc = sizeof(CFGS) / sizeof(CFGS[0]);
    for (uint32_t b = 0; b < NB; b++) {
        uint16_t hb[48];
        make_block(b, hb);
        float fb[48];
        for (uint32_t i = 0; i < 48; i++) fb[i] = basist::half_to_float(hb[i]);
        H hh;
        for (uint32_t c = 0; c < nc; c++) {
            // Each block runs a subset of the configs to keep the run short.
            if (((b + c) % 3) != 0 && c != 1) continue;
            const Cfg& cfg = CFGS[c];
            uastc_hdr_4x4_codec_options o;
            o.set_quality_level(cfg.level);
            o.m_allow_uber_mode = cfg.uber;
            o.m_ultra_quant = cfg.ultra;
            if (!cfg.perceptual) { o.m_r_err_scale = 1.0f; o.m_g_err_scale = 1.0f; }
            o.m_bc6h_err_weight = cfg.favor_astc ? .1f : .65f;
            basisu::vector<astc_hdr_4x4_pack_results> res;
            bool ok = astc_hdr_4x4_enc_block(fb, hb, o, res);
            hh.u(c);
            hh.u(ok);
            hh.u((uint32_t)res.size());
            double best_err = 1e+30f;
            int best = -1;
            for (uint32_t i = 0; i < res.size(); i++) {
                astc_hdr_4x4_pack_results& r = res[i];
                hh.d(r.m_best_block_error);
                hh.u(r.m_is_solid);
                hh.u(r.m_best_submodes[0]);
                hh.u(r.m_best_submodes[1]);
                hh.u(r.m_best_pat_index);
                hh.u(r.m_constrained_weights);
                hh.u(r.m_improved_via_refinement_flag);
                basist::astc_blk blk;
                bool pk = astc_hdr_4x4_pack_results_to_block(blk, r);
                hh.u(pk);
                hh.bytes(&blk, 16);
                hh.bytes(&r.m_bc6h_block, 16);
                // The compressor's choice
                basist::half_float un[16 * 3];
                unpack_bc6h(&r.m_bc6h_block, un, false);
                double e6 = compute_block_error(16, hb, un, o);
                double overall = e6 * o.m_bc6h_err_weight + r.m_best_block_error * (1.0f - o.m_bc6h_err_weight);
                if (!i || overall < best_err) { best_err = overall; best = (int)i; }
                hh.d(e6);
            }
            hh.u((uint32_t)best);
        }
        printf("b%u\t%016llx\n", b, (unsigned long long)hh.h);
    }
    // unpack_bc6h on random blocks
    for (uint32_t b = 0; b < 400; b++) {
        Lcg l{b * 31u + 3u};
        uint8_t blk[16];
        for (uint32_t i = 0; i < 16; i++) blk[i] = (uint8_t)l.next();
        H hh;
        for (uint32_t s = 0; s < 2; s++) {
            basist::half_float out[48];
            memset(out, 0x55, sizeof(out));
            bool ok = unpack_bc6h(blk, out, s != 0);
            hh.u(ok);
            hh.bytes(out, sizeof(out));
        }
        printf("u%u\t%016llx\n", b, (unsigned long long)hh.h);
    }
    return 0;
}
