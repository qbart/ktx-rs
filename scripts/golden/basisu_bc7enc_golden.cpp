// Golden-data generator for the basisu bc7enc port (src/basisu/bc7enc.rs; dev tooling, not
// part of the Rust library). For generated 4x4 blocks runs color_cell_compression over the
// ASTC endpoint ranges / BC7 modes the UASTC encoder uses (perceptual, uber levels, least
// squares passes, pixel counts, forced selectors) and color_cell_compression_est_astc.
// Prints "<block>\t<fnv1a64>". Must stay in sync with tests/basisu_bc7enc.rs.
// Build/run via scripts/golden/gen_basisu_bc7enc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include "basisu_enc.h"
#include "basisu_bc7enc.h"
#include "../transcoder/basisu_transcoder.h"

using namespace basisu;
using namespace basist;

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
    void q(uint64_t v) { bytes(&v, 8); }
};

static void make_block(uint32_t b, color_quad_u8* px) {
    Lcg l{b * 7919u + 13u};
    const uint32_t kind = b % 7;
    uint32_t br = l.next() & 255, bg = l.next() & 255, bb = l.next() & 255;
    uint32_t r2 = l.next() & 255, g2 = l.next() & 255, b2 = l.next() & 255;
    for (int i = 0; i < 16; i++) {
        int r, g, bl, a;
        uint32_t n0 = l.next(), n1 = l.next(), n2 = l.next(), n3 = l.next();
        switch (kind) {
        case 0: r = n0 & 255; g = n1 & 255; bl = n2 & 255; break;
        case 1: { int x = i & 3, y = i >> 2; r = (int)br + x * 20 - 30; g = (int)bg + y * 15 - 20; bl = (int)bb + (x + y) * 9 - 25; break; }
        case 2: r = br; g = bg; bl = bb; break;
        case 3: if (n0 & 1) { r = br; g = bg; bl = bb; } else { r = r2; g = g2; bl = b2; } break;
        case 4: { int v = (int)br + (int)(n0 % 9) - 4; r = v; g = v + (int)(n1 % 3) - 1; bl = v; break; }
        case 5: r = (int)br + (int)(n0 % 3); g = (int)bg + (int)(n1 % 3); bl = (int)bb + (int)(n2 % 3); break;
        default: r = (b & 8) ? 255 - (int)(n0 % 6) : (int)(n0 % 6); g = (b & 16) ? 255 - (int)(n1 % 6) : (int)(n1 % 6); bl = (int)(n2 & 255); break;
        }
        a = (b % 3 == 0) ? 255 : ((b % 3 == 1) ? (int)(n3 & 255) : (int)((i * 17 + br) & 255));
        px[i].m_c[0] = (uint8_t)clampi(r, 0, 255);
        px[i].m_c[1] = (uint8_t)clampi(g, 0, 255);
        px[i].m_c[2] = (uint8_t)clampi(bl, 0, 255);
        px[i].m_c[3] = (uint8_t)clampi(a, 0, 255);
    }
}

struct Cfg { uint32_t mode, range, comp_bits, pbits, share, table, alpha; };
// table: 0 bc7w2, 1 bc7w3, 2 bc7w4, 3 astcw4, 4 astcw5, 5 astc 3 levels
static const Cfg CFGS[] = {
    {255, 19, 0, 0, 0, 3, 0}, {255, 19, 0, 0, 0, 3, 1}, {255, 11, 0, 0, 0, 4, 0}, {255, 8, 0, 0, 0, 1, 0},
    {255, 7, 0, 0, 0, 0, 0}, {255, 8, 0, 0, 0, 0, 1}, {255, 13, 0, 0, 0, 0, 0}, {255, 20, 0, 0, 0, 0, 0},
    {255, 20, 0, 0, 0, 0, 1}, {255, 5, 0, 0, 0, 5, 1}, {255, 13, 0, 0, 0, 3, 0}, {1, 0, 6, 1, 1, 1, 0},
    {6, 0, 7, 1, 0, 2, 1}, {3, 0, 7, 1, 0, 0, 0}, {255, 0, 5, 0, 0, 0, 0}, {255, 0, 5, 0, 0, 1, 0},
};
static const uint32_t NPS[6] = {16, 16, 8, 5, 2, 1};

static void table(uint32_t t, uint32_t& n, const uint32_t*& w, const bc7enc_vec4F*& wx) {
    switch (t) {
    case 0: n = 4; w = g_bc7_weights2; wx = (const bc7enc_vec4F*)g_bc7_weights2x; break;
    case 1: n = 8; w = g_bc7_weights3; wx = (const bc7enc_vec4F*)g_bc7_weights3x; break;
    case 2: n = 16; w = g_bc7_weights4; wx = (const bc7enc_vec4F*)g_bc7_weights4x; break;
    case 3: n = 16; w = g_astc_weights4; wx = (const bc7enc_vec4F*)g_astc_weights4x; break;
    case 4: n = 32; w = g_astc_weights5; wx = (const bc7enc_vec4F*)g_astc_weights5x; break;
    default: n = 3; w = g_astc_weights_3levels; wx = (const bc7enc_vec4F*)g_astc_weights_3levelsx; break;
    }
}

int main() {
    basisu_transcoder_init();
    bc7enc_compress_block_init();
    {
        H h;
        for (uint32_t r = 0; r < BC7ENC_TOTAL_ASTC_RANGES; r++)
            for (uint32_t i = 0; i < 256; i++) {
                h.bytes(&g_astc_sorted_order_unquant[r][i].m_unquant, 1);
                h.bytes(&g_astc_sorted_order_unquant[r][i].m_index, 1);
            }
        printf("tables\t%016llx\n", (unsigned long long)h.h);
    }
    const uint32_t NUM_BLOCKS = 1400;
    for (uint32_t b = 0; b < NUM_BLOCKS; b++) {
        color_quad_u8 px[16];
        make_block(b, px);
        Lcg l{b * 31u + 7u};
        H h;
        for (uint32_t ci = 0; ci < sizeof(CFGS) / sizeof(CFGS[0]); ci++) {
            const Cfg& cfg = CFGS[ci];
            for (uint32_t v = 0; v < 12; v++) {
                uint32_t n; const uint32_t* w; const bc7enc_vec4F* wx;
                table(cfg.table, n, w, wx);
                const uint32_t np = NPS[(b + v + ci) % 6];
                const uint32_t ofs = (b + v) % (17 - np);
                uint8_t force[16];
                for (int i = 0; i < 16; i++) force[i] = (uint8_t)(l.next() % n);

                color_cell_compressor_params p;
                memset(&p, 0, sizeof(p));
                p.m_num_pixels = np;
                p.m_pPixels = px + ofs;
                p.m_num_selector_weights = n;
                p.m_pSelector_weights = w;
                p.m_pSelector_weightsx = wx;
                p.m_astc_endpoint_range = cfg.range;
                p.m_comp_bits = cfg.comp_bits;
                p.m_has_pbits = cfg.pbits;
                p.m_endpoints_share_pbit = cfg.share;
                p.m_has_alpha = cfg.alpha;
                const bool perc = v & 1;
                p.m_perceptual = perc;
                if (perc) { p.m_weights[0] = 128; p.m_weights[1] = 64; p.m_weights[2] = 16; p.m_weights[3] = 32; }
                else { p.m_weights[0] = 1; p.m_weights[1] = 1; p.m_weights[2] = 1; p.m_weights[3] = 1; }
                if (v >= 10) p.m_pForce_selectors = force;

                bc7enc_compress_block_params cp;
                bc7enc_compress_block_params_init(&cp);
                cp.m_uber_level = (v >> 1) % 5;
                cp.m_least_squares_passes = 1 + ((v >> 2) & 1);

                color_cell_compressor_results r;
                uint8_t sel[16], sel_temp[16];
                memset(&r, 0, sizeof(r));
                memset(sel, 0, 16);
                memset(sel_temp, 0, 16);
                r.m_pSelectors = sel;
                r.m_pSelectors_temp = sel_temp;
                uint64_t err = color_cell_compression(cfg.mode, &p, &r, &cp);
                h.q(err);
                h.q(r.m_best_overall_err);
                h.bytes(r.m_low_endpoint.m_c, 4);
                h.bytes(r.m_high_endpoint.m_c, 4);
                h.u(r.m_pbits[0]);
                h.u(r.m_pbits[1]);
                h.bytes(r.m_astc_low_endpoint.m_c, 4);
                h.bytes(r.m_astc_high_endpoint.m_c, 4);
                h.bytes(sel, 16);
            }
        }
        for (uint32_t t = 0; t < 6; t++)
            for (uint32_t nc = 3; nc <= 4; nc++)
                for (uint32_t wi = 0; wi < 2; wi++)
                    for (uint32_t bi = 0; bi < 2; bi++) {
                        uint32_t n; const uint32_t* w; const bc7enc_vec4F* wx;
                        table(t, n, w, wx);
                        const uint32_t ws1[4] = {1, 1, 1, 1}, ws2[4] = {128, 64, 16, 32};
                        const uint32_t np = NPS[(b + t + nc + wi) % 6];
                        h.q(color_cell_compression_est_astc(n, nc, w, np, px + (16 - np), bi ? 1500 : UINT64_MAX, wi ? ws2 : ws1));
                    }
        printf("%u\t%016llx\n", b, (unsigned long long)h.h);
    }
    return 0;
}
