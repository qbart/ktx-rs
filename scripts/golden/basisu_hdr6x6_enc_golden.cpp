// Golden-data generator for the ASTC HDR 6x6 / UASTC HDR 6x6 intermediate encoder port
// (src/basisu/astc_hdr_6x6_enc.rs; dev tooling, not part of the Rust library):
// astc_6x6_hdr::compress_photo on generated HDR images for user levels 0..12, RDO lambdas, color
// gamuts and strip counts. Prints "<case>\t<fnv1a64 of the intermediate and ASTC data>". Must
// stay in sync with tests/basisu_hdr6x6_enc.rs. Build/run via
// scripts/golden/gen_basisu_hdr6x6_enc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include "basisu_enc.h"
#include "basisu_astc_hdr_6x6_enc.h"

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
};

static void make_image(uint32_t seed, uint32_t w, uint32_t h, imagef& img) {
    Lcg l{seed * 977u + 3u};
    img.resize(w, h);
    const uint32_t kind = seed % 5;
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            float r, g, b;
            switch (kind) {
            case 0: r = x * .5f; g = y * .25f + .01f; b = (x + y) * .1f; break;
            case 1: { float s = (((x / 4) + (y / 3)) & 1) ? 40.0f : .05f; r = s; g = s * .5f; b = s * .25f; break; }
            case 2: r = (float)(l.next() % 1000) * .02f; g = (float)(l.next() % 1000) * .002f; b = (float)(l.next() % 1000) * .05f; break;
            case 3: { float v = (float)((x * 3 + y * 5) % 17) * .3f + (float)(l.next() % 10) * .001f; r = g = b = v; break; }
            default: r = (x < w / 2) ? 1.5f : 300.0f; g = (y < h / 3) ? .002f : 2.0f; b = (float)(l.next() % 50) * .1f; break;
            }
            img(x, y).set(r, g, b, 1.0f);
        }
}

struct Img { uint32_t w, h; };
static const Img IMGS[] = { {6, 6}, {13, 7}, {24, 18}, {31, 20}, {12, 30}, {40, 26} };
struct Cfg { int level; float lambda; bool rec2020; uint32_t threads; };
static const Cfg CFGS[] = {
    {0, 0.0f, false, 1}, {1, 0.0f, false, 1}, {2, 0.0f, false, 1}, {3, 0.0f, true, 1}, {4, 0.0f, false, 2},
    {5, 0.0f, false, 1}, {6, 0.0f, false, 1}, {7, 0.0f, false, 3}, {8, 0.0f, true, 1}, {9, 0.0f, false, 1},
    {10, 0.0f, false, 1}, {11, 0.0f, false, 1}, {12, 0.0f, false, 1},
    {2, 0.5f, false, 1}, {2, 2.0f, false, 4}, {0, 4.0f, false, 1}, {5, 1.0f, true, 1}, {12, 8.0f, false, 1},
};

int main() {
    basisu_encoder_init(false, false);
    const uint32_t ni = sizeof(IMGS) / sizeof(IMGS[0]), nc = sizeof(CFGS) / sizeof(CFGS[0]);
    for (uint32_t i = 0; i < ni; i++)
        for (uint32_t c = 0; c < nc; c++) {
            if (((i + c) % 2) && c != 2 && c != 13) continue;
            imagef img;
            make_image(i * 31 + c, IMGS[i].w, IMGS[i].h, img);
            astc_6x6_hdr::astc_hdr_6x6_global_config cfg;
            cfg.set_user_level(CFGS[c].level);
            cfg.m_lambda = CFGS[c].lambda;
            cfg.m_rec2020_bt2100_color_gamut = CFGS[c].rec2020;
            job_pool pool(CFGS[c].threads);
            uint8_vec inter, astc;
            astc_6x6_hdr::result_metrics m;
            bool ok = astc_6x6_hdr::compress_photo(img, cfg, &pool, inter, astc, m);
            H hh;
            hh.u(ok);
            if (ok) {
                hh.u((uint32_t)inter.size()); hh.bytes(inter.data(), inter.size());
                hh.u((uint32_t)astc.size()); hh.bytes(astc.data(), astc.size());
            }
            printf("%u_%u\t%016llx\n", i, c, (unsigned long long)hh.h);
        }
    return 0;
}
