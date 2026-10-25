// Golden-data generator for the 6x6 parts of the ASTC HDR common encoder port
// (src/basisu/astc_hdr_enc.rs; dev tooling, not part of the Rust library): CEM 7 / 11 block
// encoders on 6x6 blocks and subsets with every opt mode, dual plane CEM 11, downsampled CEM 11
// weight grids, refine_endpoints and ISE weight downsampling. Prints "<case>\t<fnv1a64>". Must
// stay in sync with tests/basisu_astc_hdr_enc.rs. Build/run via
// scripts/golden/gen_basisu_astc_hdr_enc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_astc_hdr_common.h"
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

static uint16_t rand_half(Lcg& l, int lo, int hi) {
    int e = lo + (int)(l.next() % (uint32_t)(hi - lo + 1)) + 15;
    if (e < 0) e = 0;
    if (e > 30) e = 30;
    return (uint16_t)((e << 10) | (l.next() & 1023));
}

static void make_block(uint32_t b, basist::half_float h[36][3], vec4F q16[36]) {
    Lcg l{b * 7919u + 23u};
    const uint32_t kind = b % 7;
    uint16_t base[3] = { rand_half(l, -6, 6), rand_half(l, -6, 6), rand_half(l, -6, 6) };
    uint16_t base2[3] = { rand_half(l, -6, 6), rand_half(l, -6, 6), rand_half(l, -6, 6) };
    for (uint32_t i = 0; i < 36; i++) {
        const uint32_t x = i % 6, y = i / 6;
        for (uint32_t c = 0; c < 3; c++) {
            uint32_t v;
            switch (kind) {
            case 0: v = rand_half(l, -10, 15); break;
            case 1: v = base[0] + (l.next() % 64); break;
            case 2: v = base[c] + (x * 30 + y * 20) * (c + 1); break;
            case 3: v = (((x / 3) + (y / 3)) & 1) ? base[c] : base2[c]; break;
            case 4: v = (l.next() & 7) == 0 ? 0 : rand_half(l, -14, -8); break;
            case 5: v = rand_half(l, 8, 15); break;
            default: v = base[c] + (l.next() % 300); break;
            }
            if (v > 0x7BFF) v = 0x7BFF;
            h[i][c] = (uint16_t)v;
        }
        if (kind == 1) h[i][1] = h[i][2] = h[i][0];
        for (uint32_t c = 0; c < 3; c++) q16[i][c] = (float)half_to_qlog16(h[i][c]);
        q16[i][3] = 0.0f;
    }
}

static const opt_mode_t OPT[] = { cNoOpt, cOrdinaryLeastSquares, cWeightedLeastSquares, cWeightedLeastSquaresHeavy, cWeightedAverage };
static const uint32_t GRIDS[][2] = { {2, 2}, {3, 3}, {4, 4}, {5, 5}, {6, 6}, {3, 5}, {6, 2}, {4, 6}, {5, 3} };

int main() {
    basisu_encoder_init(false, false);
    for (uint32_t b = 0; b < 500; b++) {
        basist::half_float h[36][3];
        vec4F q16[36];
        make_block(b, h, q16);
        Lcg l{b * 13u + 5u};
        astc_hdr_codec_base_options o;
        if (b & 1) { o.m_r_err_scale = 1.0f; o.m_g_err_scale = 1.0f; }
        if ((b % 5) == 3) o.m_ultra_quant = true;
        if ((b % 7) == 2) o.m_disable_weight_plane_optimization = false;
        o.m_q_log_bias = (b & 2) ? Q_LOG_BIAS_6x6 : Q_LOG_BIAS_4x4;
        H hh;
        // subset: all 36 pixels or a random part
        uint32_t n = 36;
        basist::half_float sh[36][3];
        vec4F sq[36];
        uint8_t ofs[36];
        if (b % 3 == 1) {
            n = 0;
            for (uint32_t i = 0; i < 36; i++)
                if ((l.next() & 1) || (i == 0)) { memcpy(sh[n], h[i], 6); sq[n] = q16[i]; ofs[n] = (uint8_t)i; n++; }
        } else {
            memcpy(sh, h, sizeof(h));
            for (uint32_t i = 0; i < 36; i++) { sq[i] = q16[i]; ofs[i] = (uint8_t)i; }
        }
        // mode 11 / mode 7
        for (uint32_t t = 0; t < 4; t++) {
            const uint32_t wr = 1 + (l.next() % 8);
            const uint32_t er = (t & 1) ? astc_helpers::BISE_256_LEVELS : 12 + (l.next() % 9);
            const opt_mode_t om = OPT[l.next() % 5];
            const bool uber = (l.next() & 1) != 0, constrain = (l.next() & 3) == 0, direct = (l.next() & 7) == 0;
            uint8_t e[6] = {}, w[36] = {};
            uint32_t sm = 0;
            double err = encode_astc_hdr_block_mode_11(n, sh, sq, wr, sm, BIG_FLOAT_VAL, e, w, o, direct, er, uber, constrain, -1, 7, (l.next() & 3) == 0, om);
            hh.d(err); hh.u(sm); hh.bytes(e, 6); hh.bytes(w, n);
            uint8_t e7[4] = {}, w7[36] = {};
            uint32_t sm7 = 0;
            double err7 = encode_astc_hdr_block_mode_7(n, sh, sq, wr, sm7, BIG_FLOAT_VAL, e7, w7, o, er);
            hh.d(err7); hh.u(sm7); hh.bytes(e7, 4); hh.bytes(w7, n);
            // refine on full block results with a downsampled grid
            if (n == 36) {
                const uint32_t* g = GRIDS[l.next() % 9];
                uint8_t gw[36];
                downsample_ise_weights(wr, wr, 6, 6, g[0], g[1], w, gw);
                hh.bytes(gw, g[0] * g[1]);
                bool r11 = refine_endpoints(11, er, e, 6, 6, g[0], g[1], gw, wr, n, sh, sq, nullptr, o, om);
                hh.u(r11); hh.bytes(e, 6);
                downsample_ise_weights(wr, wr, 6, 6, g[0], g[1], w7, gw);
                bool r7 = refine_endpoints(7, er, e7, 6, 6, g[0], g[1], gw, wr, n, sh, sq, nullptr, o, om);
                hh.u(r7); hh.bytes(e7, 4);
            } else {
                bool r11 = refine_endpoints(11, er, e, 6, 6, 6, 6, w, wr, n, sh, sq, ofs, o, om);
                hh.u(r11); hh.bytes(e, 6);
            }
        }
        if (n == 36) {
            // dual plane
            for (uint32_t c = 0; c < 3; c++) {
                const uint32_t wr = 1 + (l.next() % 5);
                const uint32_t er = 14 + (l.next() % 7);
                uint8_t e[6] = {}, w0[36] = {}, w1[36] = {};
                uint32_t sm = 0;
                double err = encode_astc_hdr_block_mode_11_dual_plane(n, sh, sq, c, wr, sm, BIG_FLOAT_VAL, e, w0, w1, o, false, er, false, (l.next() & 3) == 0, -1, 7, false);
                hh.d(err); hh.u(sm); hh.bytes(e, 6); hh.bytes(w0, n); hh.bytes(w1, n);
                uint8_t dp[72];
                const uint32_t* g = GRIDS[l.next() % 9];
                downsample_ise_weights_dual_plane(wr, 1, 6, 6, g[0], g[1], w0, w1, dp);
                hh.bytes(dp, g[0] * g[1] * 2);
            }
            // downsampled mode 11
            for (uint32_t t = 0; t < 3; t++) {
                const uint32_t* g = GRIDS[l.next() % 9];
                const uint32_t wr = 1 + (l.next() % 6);
                const uint32_t er = 12 + (l.next() % 9);
                const opt_mode_t om = OPT[l.next() % 5];
                uint8_t e[6] = {}, w[36] = {};
                uint32_t sm = 0;
                double err = encode_astc_hdr_block_downsampled_mode_11(6, 6, g[0], g[1], wr, er, n, sh, sq, BIG_FLOAT_VAL, -1, 7, false, om, e, w, sm, o);
                hh.d(err); hh.u(sm); hh.bytes(e, 6); hh.bytes(w, g[0] * g[1]);
            }
        }
        int s11, m11, s7, m7;
        uint8_t rnd[6];
        for (uint32_t i = 0; i < 6; i++) rnd[i] = (uint8_t)l.next();
        decode_cem_11_config(rnd, s11, m11);
        decode_cem_7_config(rnd, s7, m7);
        hh.u(s11); hh.u(m11); hh.u(s7); hh.u(m7);
        printf("%u\t%016llx\n", b, (unsigned long long)hh.h);
    }
    return 0;
}
