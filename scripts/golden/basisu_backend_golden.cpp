// Golden-data generator for the basisu ETC1S backend port (src/basisu/backend.rs; dev tooling,
// not part of the Rust library): basisu_frontend + basisu_backend encode on generated single
// slice, multi slice and video frame inputs for a set of compression levels / RDO thresholds.
// Prints "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_backend.rs. Build/run via
// scripts/golden/gen_basisu_backend_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_etc.h"
#include "basisu_frontend.h"
#include "basisu_backend.h"
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
    void v(const uint8_vec& x) { u((uint32_t)x.size()); bytes(x.data(), x.size()); }
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

// Slice layouts: 0 = one slice, 1 = two slices (second half size), 2 = three video frames.
static void make_input(uint32_t img, uint32_t layout, basisu::vector<pixel_block>& blocks, basisu_backend_slice_desc_vec& slices) {
    static const uint32_t SIZES[][2] = {{8, 8}, {7, 5}, {16, 12}, {33, 20}};
    const uint32_t bw = SIZES[img % 4][0], bh = SIZES[img % 4][1];
    const uint32_t nslices = layout == 0 ? 1 : (layout == 1 ? 2 : 3);
    blocks.clear();
    slices.clear();
    for (uint32_t s = 0; s < nslices; s++) {
        uint32_t w = bw, h = bh;
        if (layout == 1 && s == 1) { w = (bw + 1) / 2; h = (bh + 1) / 2; }
        basisu::vector<pixel_block> b;
        make_image(layout == 1 ? img + s * 3 : img, w, h, b);
        if (layout == 2 && s > 0) {
            Lcg l{img * 31u + s};
            for (uint32_t i = 0; i < w * h; i++)
                if (((i % w) + (i / w) + s) % 5 == 0)
                    for (uint32_t p = 0; p < 16; p++) {
                        color_rgba& c = b[i].m_pixels[p >> 2][p & 3];
                        c.r = (uint8_t)(c.r + 20 + l.next() % 30);
                        c.b = (uint8_t)(c.b ^ (l.next() & 63));
                    }
        }
        basisu_backend_slice_desc d;
        d.m_first_block_index = (uint32_t)blocks.size();
        d.m_orig_width = w * 4 - (s & 1);
        d.m_orig_height = h * 4 - 1;
        d.m_width = w * 4;
        d.m_height = h * 4;
        d.m_num_blocks_x = w;
        d.m_num_blocks_y = h;
        d.m_num_macroblocks_x = (w + 1) / 2;
        d.m_num_macroblocks_y = (h + 1) / 2;
        d.m_source_file_index = layout == 2 ? s : 0;
        d.m_mip_index = layout == 1 ? s : 0;
        d.m_alpha = false;
        d.m_iframe = layout == 2 && s == 0;
        slices.push_back(d);
        for (auto& x : b) blocks.push_back(x);
    }
}

struct Cfg { uint32_t level, max_ep, max_sel; bool perceptual; float ep_rdo, sel_rdo; };

static const Cfg CFGS[] = {
    {0, 64, 64, true, 0.0f, 0.0f},
    {0, 256, 256, true, 1.5f, 1.25f},
    {1, 256, 256, true, 1.5f, 1.25f},
    {1, 128, 512, false, 3.0f, 2.0f},
    {2, 256, 256, true, 1.5f, 1.25f},
    {2, 512, 1024, false, 1.5f, 1.25f},
    {2, 16, 16, true, 1.5f, 1.25f},
    {2, 256, 256, true, 0.0f, 0.0f},
    {3, 300, 300, true, 2.5f, 1.0f},
    {4, 256, 256, true, 1.5f, 1.25f},
    {5, 128, 600, false, 1.0f, 3.0f},
    {6, 256, 256, true, 1.5f, 1.25f},
    {2, 2000, 2000, true, 10.0f, 10.0f},
};

int main() {
    basist::basisu_transcoder_init();
    pack_etc1_solid_color_init();
    g_cpu_supports_sse41 = true;
    job_pool pool(1);

    for (uint32_t img = 0; img < 12; img++) {
        for (uint32_t layout = 0; layout < 3; layout++) {
            basisu::vector<pixel_block> blocks;
            basisu_backend_slice_desc_vec slices;
            make_input(img, layout, blocks, slices);
            for (uint32_t c = 0; c < sizeof(CFGS) / sizeof(CFGS[0]); c++) {
                const Cfg& cfg = CFGS[c];
                if ((img + layout + c) % 3 == 1) continue;
                basisu_frontend f;
                basisu_frontend::params p;
                p.m_num_source_blocks = (uint32_t)blocks.size();
                p.m_pSource_blocks = blocks.data();
                p.m_max_endpoint_clusters = cfg.max_ep;
                p.m_max_selector_clusters = cfg.max_sel;
                p.m_compression_level = cfg.level;
                p.m_perceptual = cfg.perceptual;
                p.m_validate = false;
                p.m_tex_type = layout == 2 ? basist::cBASISTexTypeVideoFrames : basist::cBASISTexType2D;
                p.m_multithreaded = false;
                p.m_pJob_pool = &pool;
                p.m_pGlobal_codebooks = nullptr;
                H h;
                bool ok = f.init(p) && f.compress();
                h.bytes(&ok, 1);
                basisu_backend be;
                basisu_backend_params bp;
                bp.m_etc1s = true;
                bp.m_compression_level = cfg.level;
                bp.m_endpoint_rdo_quality_thresh = cfg.ep_rdo;
                bp.m_selector_rdo_quality_thresh = cfg.sel_rdo;
                bp.m_used_global_codebooks = false;
                be.init(&f, bp, slices);
                uint32_t total = be.encode();
                h.u(total);
                const basisu_backend_output& o = be.get_output();
                h.u(o.m_num_endpoints);
                h.u(o.m_num_selectors);
                h.u(o.m_srgb);
                h.v(o.m_endpoint_palette);
                h.v(o.m_selector_palette);
                h.v(o.m_slice_image_tables);
                h.u((uint32_t)o.m_slice_image_data.size());
                for (const auto& d : o.m_slice_image_data) h.v(d);
                for (uint16_t crc : o.m_slice_image_crcs) h.bytes(&crc, 2);
                printf("%u_%u_%u\t%016llx\n", img, layout, c, (unsigned long long)h.h);
            }
        }
    }
    return 0;
}
