// Golden-data generator for the basisu UASTC RDO post-pass port (uastc_rdo in
// src/basisu/uastc_enc.rs; dev tooling, not part of the Rust library). Encodes generated
// images with encode_uastc, then runs uastc_rdo with a set of parameter / flag / job
// configurations. Prints "<image>_<config>\t<fnv1a64 of the blocks>". Must stay in sync with
// tests/basisu_uastc_rdo.rs. Build/run via scripts/golden/gen_basisu_uastc_rdo_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_etc.h"
#include "basisu_bc7enc.h"
#include "basisu_uastc_enc.h"
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
};

static const uint32_t BW = 24, BH = 16;

// Pixels in block order (16 per block, row major inside the block).
static void make_image(uint32_t img, std::vector<color_rgba>& px) {
    Lcg l{img * 7919u + 17u};
    px.resize(BW * BH * 16);
    const uint32_t kind = img % 4;
    if (img == 10) {
        // Blocks tiled from 4 tiles (tile 0 very noisy, so skipped) to repeat selector patterns.
        int tiles[4][16][3];
        for (int t = 0; t < 4; t++)
            for (int i = 0; i < 16; i++)
                for (int c = 0; c < 3; c++)
                    tiles[t][i][c] = t ? (int)(l.next() % 40) + 60 * t : (int)(l.next() & 255);
        for (uint32_t b = 0; b < BW * BH; b++) {
            uint32_t t = l.next() % 4, o = l.next() % 3;
            for (uint32_t i = 0; i < 16; i++)
                px[b * 16 + i].set(tiles[t][i][0] + o, tiles[t][i][1], tiles[t][i][2] + o, 255);
        }
        return;
    }
    const uint32_t noise = (img >= 8) ? 24 + (img - 8) * 16 : 1 + (img % 3) * 6;
    const bool alpha = (img % 5) == 3 || img == 9;
    for (uint32_t by = 0; by < BH; by++)
        for (uint32_t bx = 0; bx < BW; bx++)
            for (uint32_t i = 0; i < 16; i++) {
                int x = (int)(bx * 4 + (i & 3)), y = (int)(by * 4 + (i >> 2));
                int r, g, b;
                switch (kind) {
                case 0: r = x * 2; g = y * 3; b = (x + y); break;
                case 1: { int c = ((x / 8 + y / 8) & 1) ? 200 : 40; r = c; g = c / 2 + x; b = 255 - c; break; }
                case 2: { int s = (x * x + y * y) / 37; r = (s * 13) & 255; g = (s * 7) & 255; b = (s * 3) & 255; break; }
                default: { int c = ((x ^ y) & 15) * 16; r = c; g = c; b = c; break; }
                }
                int n0 = (int)(l.next() % (2 * noise + 1)) - (int)noise;
                int n1 = (int)(l.next() % (2 * noise + 1)) - (int)noise;
                int n2 = (int)(l.next() % (2 * noise + 1)) - (int)noise;
                int a = alpha ? ((x * 5 + y * 3) & 255) : 255;
                px[(by * BW + bx) * 16 + i].set(r + n0, g + n1, b + n2, a);
            }
}

struct Cfg {
    uint32_t enc_flags, rdo_flags, jobs, dict, literal;
    float lambda, ratio, skip, smooth_dev, smooth_scale;
    bool refine;
};

static const Cfg CFGS[] = {
    {2, 2, 1, 4096, 100, 0.5f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2, 1, 4096, 100, 1.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2, 1, 4096, 100, 3.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2, 1, 4096, 100, 10.0f, 10.0f, 30.0f, 18.0f, 10.0f, true},
    {2, 2, 1, 4096, 100, 3.0f, 10.0f, 8.0f, 18.0f, 10.0f, false},
    {2, 2, 1, 256, 100, 2.0f, 1.25f, 8.0f, 18.0f, 10.0f, true},
    {2, 2, 1, 32768, 50, 2.0f, 10.0f, 40.0f, 40.0f, 1.0f, true},
    {2, 2, 4, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2, 7, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {0, 0, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {1, 1, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {3, 3, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 4, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2 | cPackUASTCFavorUASTCError, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2, 2 | cPackUASTCETC1FasterHints, 1, 4096, 100, 2.0f, 10.0f, 8.0f, 18.0f, 10.0f, true},
    {2 | cPackUASTCFavorSimplerModes, 2 | cPackUASTCETC1DisableFlipAndIndividual, 1, 4096, 200, 1.5f, 3.0f, 12.0f, 18.0f, 10.0f, true},
};

int main() {
    basist::basisu_transcoder_init();
    bc7enc_compress_block_init();
    pack_etc1_solid_color_init();
    g_cpu_supports_sse41 = true;
    job_pool pool(4);
    const uint32_t NUM_IMAGES = 11, N = BW * BH;
    std::vector<color_rgba> px;
    for (uint32_t img = 0; img < NUM_IMAGES; img++) {
        make_image(img, px);
        uint32_t last_enc = UINT32_MAX;
        std::vector<basist::uastc_block> enc(N);
        for (uint32_t c = 0; c < sizeof(CFGS) / sizeof(CFGS[0]); c++) {
            const Cfg& cfg = CFGS[c];
            if (cfg.enc_flags != last_enc) {
                for (uint32_t b = 0; b < N; b++)
                    encode_uastc((const uint8_t*)&px[b * 16], enc[b], cfg.enc_flags);
                last_enc = cfg.enc_flags;
            }
            std::vector<basist::uastc_block> blocks(enc);
            uastc_rdo_params p;
            p.m_lz_dict_size = cfg.dict;
            p.m_lz_literal_cost = cfg.literal;
            p.m_lambda = cfg.lambda;
            p.m_max_allowed_rms_increase_ratio = cfg.ratio;
            p.m_skip_block_rms_thresh = cfg.skip;
            p.m_max_smooth_block_std_dev = cfg.smooth_dev;
            p.m_smooth_block_max_error_scale = cfg.smooth_scale;
            p.m_endpoint_refinement = cfg.refine;
            bool ok = uastc_rdo(N, blocks.data(), px.data(), p, cfg.rdo_flags, cfg.jobs > 1 ? &pool : nullptr, cfg.jobs);
            uint32_t changed = 0;
            for (uint32_t b = 0; b < N; b++) changed += memcmp(&blocks[b], &enc[b], 16) != 0;
            H h;
            h.bytes(&ok, 1);
            h.bytes(blocks.data(), N * 16);
            printf("%u_%u\t%016llx %u\n", img, c, (unsigned long long)h.h, changed);
        }
    }
    return 0;
}
