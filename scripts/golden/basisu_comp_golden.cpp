// Golden-data generator for the basis_compressor port (src/basisu/comp.rs, basis_file.rs; dev
// tooling, not part of the Rust library): basis_compressor::process on generated LDR inputs
// (single images, image arrays of differing sizes, video frames, alpha) for ETC1S and UASTC LDR
// 4x4 configurations. Prints "<case>\t<fnv1a64 of error code + .basis file>". Must stay in sync
// with tests/basisu_comp.rs. Build/run via scripts/golden/gen_basisu_comp_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_comp.h"
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
};

// alpha: 0 = opaque, 1 = gradient alpha, 2 = all 255 except one pixel.
static image make_image(uint32_t seed, uint32_t w, uint32_t h, uint32_t kind, uint32_t alpha) {
    Lcg l{seed * 7919u + 11u};
    image img(w, h);
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            int r, g, b;
            switch (kind) {
            case 0: r = (int)(x * 255 / (w > 1 ? w - 1 : 1)); g = (int)(y * 255 / (h > 1 ? h - 1 : 1)); b = 128 + (int)x - (int)y; break;
            case 1: { int c = (((x >> 2) + (y >> 2)) & 1) ? 220 : 30; r = c; g = (c + (int)x * 4) & 255; b = 255 - c; break; }
            case 2: r = (int)(l.next() & 255); g = (int)((l.next() >> 4) & 255); b = (int)((l.next() >> 8) & 255); break;
            default: { int c = (int)(((x / 6) + (y / 5)) % 3) * 100 + (int)((x >> 1) & 1) * 9; r = c; g = 255 - c; b = (c * 2) & 255; break; }
            }
            if (kind != 2) { r += (int)(l.next() % 7) - 3; g += (int)(l.next() % 7) - 3; b += (int)(l.next() % 7) - 3; }
            int a = 255;
            if (alpha == 1) a = (int)((x * 37 + y * 11 + (l.next() & 15)) & 255);
            else if (alpha == 2 && x == w / 2 && y == h / 2) a = 254;
            img(x, y).set(r, g, b, a);
        }
    return img;
}

struct Input { uint32_t tex_type; uint32_t n; uint32_t w, h; uint32_t kind; uint32_t alpha_mask; bool halve; };

static const Input INPUTS[] = {
    {0, 1, 16, 16, 0, 0, false},
    {0, 1, 13, 7, 2, 1, false},
    {0, 1, 37, 29, 1, 0, false},
    {0, 3, 40, 24, 0, 0, true},
    {0, 3, 22, 18, 3, 2, true},
    {3, 3, 24, 16, 1, 0, false},
    {3, 3, 20, 12, 0, 7, false},
    {0, 1, 64, 64, 3, 0, false},
    {0, 1, 1, 1, 2, 0, false},
    {0, 1, 5, 3, 0, 4, false},
    {1, 2, 12, 12, 2, 0, false},
    {0, 1, 128, 96, 2, 0, false},
    {0, 1, 96, 80, 0, 1, false},
};

static void make_input(uint32_t i, basis_compressor_params& p) {
    const Input& in = INPUTS[i];
    p.m_tex_type = (basist::basis_texture_type)in.tex_type;
    uint32_t w = in.w, h = in.h;
    for (uint32_t k = 0; k < in.n; k++) {
        uint32_t alpha = (in.alpha_mask >> k) & 1;
        if (in.alpha_mask == 4) alpha = 2;
        p.m_source_images.push_back(make_image(i * 17 + k, w, h, in.kind, alpha));
        if (in.halve) { w = w > 1 ? w / 2 : 1; h = h > 1 ? h / 2 : 1; }
    }
}

struct Cfg {
    bool uastc;
    // ETC1S
    int quality; uint32_t max_ep, max_sel; int level; bool perceptual, no_ep_rdo, no_sel_rdo;
    float ep_thresh, sel_thresh; // < 0 = unset
    uint32_t swizzle; bool force_alpha, check_alpha;
    // UASTC
    uint32_t flags; bool rdo; float lambda; int dict; float smooth_scale, std_dev; bool favor_simpler, mt; uint32_t threads;
};

static const uint32_t SWIZZLES[][4] = {{0, 1, 2, 3}, {0, 0, 0, 1}, {2, 1, 0, 3}};

#define E(q, ep, sel, lvl, perc, nep, nsel, et, st, sw, fa, ca) {false, q, ep, sel, lvl, perc, nep, nsel, et, st, sw, fa, ca, 0, false, 0, 0, 0, 0, false, false, 1}
#define U(fl, rdo, lam, dict, ss, sd, fs, mt, th, sw, fa, ca) {true, -1, 0, 0, 2, true, false, false, -1, -1, sw, fa, ca, fl, rdo, lam, dict, ss, sd, fs, mt, th}

static const Cfg CFGS[] = {
    E(128, 0, 0, 2, true, false, false, -1, -1, 0, false, true),
    E(1, 0, 0, 1, true, false, false, -1, -1, 0, false, true),
    E(64, 0, 0, 0, false, false, false, -1, -1, 0, false, true),
    E(140, 0, 0, 3, true, false, false, -1, -1, 0, false, true),
    E(170, 0, 0, 2, true, false, false, 2.0f, -1, 0, false, true),
    E(200, 0, 0, 4, true, true, false, -1, -1, 0, false, true),
    E(230, 0, 0, 5, false, false, true, -1, 0.5f, 0, false, true),
    E(255, 0, 0, 6, true, false, false, -1, -1, 0, false, true),
    E(-1, 300, 200, 2, true, false, false, -1, -1, 0, false, true),
    E(-1, 16, 16, 1, false, true, true, -1, -1, 0, false, true),
    E(128, 0, 0, 2, true, false, false, -1, -1, 1, false, true),
    E(128, 0, 0, 2, true, false, false, -1, -1, 2, true, true),
    E(128, 0, 0, 2, true, false, false, -1, -1, 0, false, false),
    E(-1, 0, 0, 2, true, false, false, -1, -1, 0, false, true),
    U(2, false, 0, 0, 0, 0, true, true, 1, 0, false, true),
    U(0, false, 0, 0, 0, 0, true, true, 1, 0, false, true),
    U(1 | 64, false, 0, 0, 0, 0, true, true, 1, 1, false, true),
    U(3 | 16, false, 0, 0, 0, 0, true, true, 1, 0, true, true),
    U(4 | 8 | 128, false, 0, 0, 0, 0, true, true, 1, 2, false, false),
    U(2, true, 1.0f, 4096, 10.0f, 18.0f, true, true, 1, 0, false, true),
    U(2, true, 3.0f, 1024, 20.0f, 9.0f, false, true, 1, 0, false, true),
    U(1, true, 0.5f, 8192, 10.0f, 18.0f, true, true, 4, 0, false, true),
    U(2, true, 5.0f, 64, 1.0f, 1.0f, true, false, 4, 0, false, true),
    E(223, 0, 0, 2, true, false, false, -1, -1, 0, false, true),
    E(129, 0, 0, 2, true, false, false, -1, -1, 0, false, true),
    E(100, 0, 0, 1, true, false, false, -1, -1, 0, false, true),
    E(30, 0, 0, 2, false, false, false, -1, -1, 0, false, true),
    E(223, 0, 0, 6, false, false, false, -1, -1, 0, false, true),
};

int main() {
    basisu_encoder_init(false, false);
    const uint32_t ni = sizeof(INPUTS) / sizeof(INPUTS[0]), nc = sizeof(CFGS) / sizeof(CFGS[0]);
    for (uint32_t i = 0; i < ni; i++)
        for (uint32_t c = 0; c < nc; c++) {
            const Cfg& cfg = CFGS[c];
            job_pool pool(cfg.threads);
            basis_compressor_params p;
            p.m_status_output = false;
            p.m_pJob_pool = &pool;
            make_input(i, p);
            p.m_uastc = cfg.uastc;
            for (uint32_t k = 0; k < 4; k++) p.m_swizzle[k] = (char)SWIZZLES[cfg.swizzle][k];
            if (cfg.force_alpha) p.m_force_alpha = true;
            if (!cfg.check_alpha) p.m_check_for_alpha = false;
            if (!cfg.uastc) {
                p.m_quality_level = cfg.quality;
                p.m_etc1s_max_endpoint_clusters = cfg.max_ep;
                p.m_etc1s_max_selector_clusters = cfg.max_sel;
                p.m_etc1s_compression_level = cfg.level;
                p.set_srgb_options(cfg.perceptual);
                p.m_no_endpoint_rdo = cfg.no_ep_rdo;
                p.m_no_selector_rdo = cfg.no_sel_rdo;
                if (cfg.ep_thresh >= 0) p.m_endpoint_rdo_thresh = cfg.ep_thresh;
                if (cfg.sel_thresh >= 0) p.m_selector_rdo_thresh = cfg.sel_thresh;
            } else {
                p.m_pack_uastc_ldr_4x4_flags = cfg.flags;
                if (cfg.rdo) {
                    p.m_rdo_uastc_ldr_4x4 = true;
                    p.m_rdo_uastc_ldr_4x4_quality_scalar = cfg.lambda;
                    p.m_rdo_uastc_ldr_4x4_dict_size = cfg.dict;
                    p.m_rdo_uastc_ldr_4x4_max_smooth_block_error_scale = cfg.smooth_scale;
                    p.m_rdo_uastc_ldr_4x4_smooth_block_max_std_dev = cfg.std_dev;
                    p.m_rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode = cfg.favor_simpler;
                    p.m_rdo_uastc_ldr_4x4_multithreading = cfg.mt;
                }
            }
            basis_compressor comp;
            H hh;
            if (!comp.init(p)) {
                hh.u(0xFFFFFFFFu);
            } else {
                basis_compressor::error_code ec = comp.process();
                hh.u((uint32_t)ec);
                if (ec == basis_compressor::cECSuccess) {
                    const uint8_vec& f = comp.get_output_basis_file();
                    hh.u((uint32_t)f.size());
                    hh.bytes(f.data(), f.size());
                }
            }
            printf("%u_%u\t%016llx\n", i, c, (unsigned long long)hh.h);
        }
    return 0;
}
