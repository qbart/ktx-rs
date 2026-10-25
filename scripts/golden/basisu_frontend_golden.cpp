// Golden-data generator for the basisu ETC1S frontend port (src/basisu/frontend.rs; dev
// tooling, not part of the Rust library): basisu_frontend init + compress on generated images
// for a set of compression levels / cluster counts / flags, then reoptimize_remapped_endpoints.
// Also checks the libstdc++ std::sort tie order used by indirect_sort. Prints
// "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_frontend.rs. Build/run via
// scripts/golden/gen_basisu_frontend_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include <algorithm>
#include "basisu_enc.h"
#include "basisu_etc.h"
#include "basisu_frontend.h"
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

static void make_image(uint32_t img, uint32_t bw, uint32_t bh, basisu::vector<pixel_block>& blocks) {
    Lcg l{img * 7919u + 5u};
    blocks.resize(bw * bh);
    const uint32_t kind = (img >= 10) ? 5 : img % 5;
    for (uint32_t by = 0; by < bh; by++)
        for (uint32_t bx = 0; bx < bw; bx++) {
            pixel_block& pb = blocks[by * bw + bx];
            const bool dup = (kind == 3) && (bx & 1) && bx > 0;
            for (uint32_t i = 0; i < 16; i++) {
                int x = (int)(bx * 4 + (i & 3)), y = (int)(by * 4 + (i >> 2));
                int r, g, b;
                switch (kind) {
                case 0: r = x * 3; g = y * 2; b = 128 + (x - y); break;
                case 1: { int c = (((x >> 3) + (y >> 3)) & 1) ? 220 : 30; r = c; g = (c + x * 4) & 255; b = 255 - c; break; }
                case 2: r = (int)(l.next() & 255); g = (int)((l.next() >> 4) & 255); b = (int)((l.next() >> 8) & 255); break;
                case 3: { int s = ((x * x + y * y) >> 4) & 255; r = s; g = (s * 3) & 255; b = (s * 7 + y) & 255; break; }
                case 4: { int c = ((x / 5) * 40 + (y / 7) * 30) & 255; r = c; g = c; b = (c + 64 + (int)(l.next() % 9)) & 255; break; }
                default: { int c = (((x / 6) + (y / 10)) % 3) * 100 + ((x >> 1) & 1) * 9; r = c; g = 255 - c; b = (c * 2) & 255; break; }
                }
                if (kind != 2 && kind != 5) { r += (int)(l.next() % 7) - 3; g += (int)(l.next() % 7) - 3; b += (int)(l.next() % 7) - 3; }
                pb.m_pixels[i >> 2][i & 3].set(r, g, b, 255);
            }
            if (dup) pb = blocks[by * bw + bx - 1];
        }
}

struct Cfg { uint32_t level, max_ep, max_sel, tex_type; bool perceptual, disable_hier, validate; };

static const Cfg CFGS[] = {
    {0, 64, 64, 0, true, false, false},
    {1, 256, 256, 0, true, false, true},
    {2, 256, 256, 0, true, false, false},
    {2, 512, 1024, 0, false, false, false},
    {3, 300, 300, 0, true, false, false},
    {4, 256, 256, 0, true, false, false},
    {5, 128, 600, 0, false, false, false},
    {6, 256, 256, 0, true, false, false},
    {2, 16, 16, 0, true, false, false},
    {2, 256, 256, 3, true, false, false},
    {1, 1024, 256, 0, true, true, false},
    {0, 2000, 2000, 0, false, false, false},
};

static uint64_t hash_frontend(basisu_frontend& f, H h) {
    const uint32_t nb = f.get_total_output_blocks();
    h.u(nb);
    for (uint32_t i = 0; i < nb; i++) { h.bytes(&f.get_output_block(i), 8); h.bytes(&f.get_etc1s_block(i), 8); }
    const uint32_t ne = f.get_total_endpoint_clusters();
    h.u(ne);
    for (uint32_t i = 0; i < nb; i++) { h.u(f.get_subblock_endpoint_cluster_index(i, 0)); h.u(f.get_subblock_endpoint_cluster_index(i, 1)); }
    for (uint32_t c = 0; c < ne; c++) {
        h.bytes(&f.get_endpoint_cluster_unscaled_color(c, false), 4);
        h.u(f.get_endpoint_cluster_inten_table(c, false));
        h.u(f.get_endpoint_cluster_color_is_used(c, false));
    }
    const uint32_t ns = f.get_total_selector_clusters();
    h.u(ns);
    for (uint32_t i = 0; i < nb; i++) h.u(f.get_block_selector_cluster_index(i));
    for (uint32_t c = 0; c < ns; c++) {
        h.u(f.get_selector_cluster_selector_bits(c).get_raw_selector_bits());
        const uint_vec& v = f.get_selector_cluster_block_indices(c);
        h.u((uint32_t)v.size());
        for (uint32_t x : v) h.u(x);
    }
    return h.h;
}

int main() {
    basist::basisu_transcoder_init();
    pack_etc1_solid_color_init();
    g_cpu_supports_sse41 = true;
    job_pool pool(1);

    // std::sort tie order (indirect_sort)
    for (uint32_t t = 0; t < 40; t++) {
        const uint32_t n = (t < 30) ? t * 7 : 1000 + t * 333;
        Lcg l{t * 17u + 1u};
        std::vector<uint32_t> keys(n);
        for (uint32_t i = 0; i < n; i++) {
            switch (t % 4) {
            case 0: keys[i] = l.next() % 5; break;
            case 1: keys[i] = n - i; break;
            case 2: keys[i] = (i < n / 2) ? i : n - i; break;
            default: keys[i] = l.next() % (n + 1); break;
            }
        }
        uint_vec idx(n);
        if (n) indirect_sort(n, idx.data(), keys.data());
        H h;
        for (uint32_t x : idx) h.u(x);
        printf("sort%u\t%016llx\n", t, (unsigned long long)h.h);
    }

    static const uint32_t SIZES[][2] = {{8, 8}, {16, 12}, {32, 32}, {64, 40}};
    for (uint32_t img = 0; img < 12; img++) {
        const uint32_t bw = SIZES[img % 4][0], bh = SIZES[img % 4][1];
        basisu::vector<pixel_block> blocks;
        make_image(img, bw, bh, blocks);
        for (uint32_t c = 0; c < sizeof(CFGS) / sizeof(CFGS[0]); c++) {
            const Cfg& cfg = CFGS[c];
            if (img >= 6 && cfg.level >= 5 && (img + c) % 2) continue;
            basisu_frontend f;
            basisu_frontend::params p;
            p.m_num_source_blocks = bw * bh;
            p.m_pSource_blocks = blocks.data();
            p.m_max_endpoint_clusters = cfg.max_ep;
            p.m_max_selector_clusters = cfg.max_sel;
            p.m_compression_level = cfg.level;
            p.m_perceptual = cfg.perceptual;
            p.m_validate = cfg.validate;
            p.m_tex_type = (basist::basis_texture_type)cfg.tex_type;
            p.m_disable_hierarchical_endpoint_codebooks = cfg.disable_hier;
            p.m_multithreaded = false;
            p.m_pJob_pool = &pool;
            p.m_pGlobal_codebooks = nullptr;
            H h;
            bool ok = f.init(p);
            h.bytes(&ok, 1);
            ok = f.compress();
            h.bytes(&ok, 1);
            uint64_t h1 = hash_frontend(f, h);
            // Backend style endpoint remap: merge every pair of neighbouring clusters.
            const uint32_t nb = f.get_total_output_blocks();
            const uint32_t ne = f.get_total_endpoint_clusters();
            uint_vec new_block_endpoints(nb);
            for (uint32_t i = 0; i < nb; i++) {
                uint32_t e = f.get_subblock_endpoint_cluster_index(i, 0);
                new_block_endpoints[i] = ((e & 1) && (i % 3 != 0)) ? e - 1 : e;
            }
            int_vec old_to_new;
            f.reoptimize_remapped_endpoints(new_block_endpoints, old_to_new, (c & 1) == 0, nullptr);
            H h2;
            h2.u((uint32_t)old_to_new.size());
            for (int x : old_to_new) h2.u((uint32_t)x);
            uint64_t hr = hash_frontend(f, h2);
            (void)ne;
            printf("%u_%u\t%016llx %016llx\n", img, c, (unsigned long long)h1, (unsigned long long)hr);
        }
    }
    return 0;
}
