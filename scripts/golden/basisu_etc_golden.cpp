// Golden-data generator for the basisu ETC1 encoder port (src/basisu/etc_enc.rs; dev tooling,
// not part of the Rust library). For generated 4x4 blocks runs pack_etc1_block_solid_color,
// etc1_optimizer over quality/perceptual/cluster-fit/color4/subblock/constraint/force-selector
// variants, etc_block packing/decoding helpers on the results and on random blocks, and
// pack_eac_a8. Prints "<block>\t<fnv1a64>". Must stay in sync with tests/basisu_etc.rs.
// Build/run via scripts/golden/gen_basisu_etc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"
#include "basisu_etc.h"

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
    void q(uint64_t v) { bytes(&v, 8); }
    void c(const color_rgba& c) { bytes(c.m_comps, 4); }
};

static void make_block(uint32_t b, color_rgba* px) {
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
        px[i].set(r, g, bl, a);
    }
}

int main() {
    pack_etc1_solid_color_init();
    g_cpu_supports_sse41 = true;
    static const int s_deltas[] = {-2, -1, 0, 1, 2};
    const uint32_t NUM_BLOCKS = 3000;
    for (uint32_t b = 0; b < NUM_BLOCKS; b++) {
        color_rgba px[16];
        make_block(b, px);
        H h;

        etc_block solid;
        solid.clear();
        uint64_t serr = pack_etc1_block_solid_color(solid, px[0].m_comps);
        h.q(serr);
        h.bytes(solid.m_bytes, 8);

        uint32_t k = 0;
        for (int qi = 0; qi < 4; qi++)
        for (int perc = 0; perc < 2; perc++)
        for (int cf = 0; cf < 2; cf++)
        for (int c4 = 0; c4 < 2; c4++)
        for (int sub = 0; sub < 2; sub++, k++) {
            const uint32_t n = sub ? 8 : 16;
            etc1_optimizer::params p;
            p.m_quality = (basis_etc_quality)qi;
            p.m_perceptual = perc != 0;
            p.m_cluster_fit = cf != 0;
            p.m_use_color4 = c4 != 0;
            p.m_num_src_pixels = n;
            p.m_pSrc_pixels = px;
            if (!cf && ((k + b) % 3 == 0)) { p.m_pScan_deltas = s_deltas; p.m_scan_delta_size = 5; }
            p.m_refinement = ((k + b) % 7) != 3;
            if (!c4 && ((k + b) % 5 == 1)) {
                p.m_constrain_against_base_color5 = true;
                p.m_base_color5.set(px[0].r >> 3, px[0].g >> 3, px[0].b >> 3, 255);
            }
            uint8_t force[16];
            if (qi >= cETCQualitySlow && ((k + b) % 4 == 2)) {
                for (uint32_t i = 0; i < 16; i++) force[i] = (uint8_t)((i * 3 + b + k) & 3);
                p.m_pForce_selectors = force;
            }
            uint8_t sels[16];
            memset(sels, 0, sizeof(sels));
            etc1_optimizer::results r;
            r.m_pSelectors = sels;
            r.m_n = n;
            etc1_optimizer opt;
            opt.init(p, r);
            bool ok = opt.compute();
            h.u(ok);
            h.q(r.m_error);
            if (!ok) continue;
            h.c(r.m_block_color_unscaled);
            h.u(r.m_block_inten_table);
            h.u(r.m_block_color4);
            h.u(r.m_n);
            h.bytes(sels, n);

            if (sub) continue;
            etc_block blk;
            blk.clear();
            if (r.m_block_color4)
                blk.set_block_color4(r.m_block_color_unscaled, r.m_block_color_unscaled);
            else
                blk.set_block_color5_etc1s(r.m_block_color_unscaled);
            blk.set_inten_tables_etc1s(r.m_block_inten_table);
            blk.set_flip_bit((k & 2) != 0);
            for (uint32_t y = 0; y < 4; y++)
                for (uint32_t x = 0; x < 4; x++)
                    blk.set_selector(x, y, sels[x + y * 4]);
            h.bytes(blk.m_bytes, 8);
            h.u(blk.is_etc1s());
            h.q(blk.evaluate_etc1_error(px, p.m_perceptual));
            h.q(blk.evaluate_etc1_error(px, p.m_perceptual, (int)(k & 1)));
            uint64_t de = blk.determine_selectors(px, p.m_perceptual);
            h.q(de);
            h.bytes(blk.m_bytes, 8);
        }

        // Random raw blocks: decoding helpers.
        Lcg l{b * 31u + 7u};
        for (int t = 0; t < 4; t++) {
            etc_block rb;
            for (int i = 0; i < 8; i++) rb.m_bytes[i] = (uint8_t)l.next();
            color_rgba up[16];
            for (int i = 0; i < 16; i++) up[i].set(1, 2, 3, 4);
            bool ok = unpack_etc1(rb, up, (t & 1) != 0);
            h.u(ok);
            for (int i = 0; i < 16; i++) h.c(up[i]);
            h.u(rb.is_etc1s());
            for (uint32_t s = 0; s < 2; s++) {
                color_rgba bc[4];
                h.u(rb.get_block_colors(bc, s));
                for (int i = 0; i < 4; i++) h.c(bc[i]);
                color_rgba lh[2];
                h.u(rb.get_block_low_high_colors(lh, s));
                h.c(lh[0]); h.c(lh[1]);
                h.c(rb.get_block_color(s, (t & 2) != 0));
                color_rgba one;
                rb.get_block_color(one, s, (t + s) & 3);
                h.c(one);
            }
            uint32_t lo, hi;
            rb.get_selector_range(lo, hi);
            h.u(lo); h.u(hi);
            h.u(rb.get_general_bits(13, 7));
            h.c(rb.get_selector_color(1, 2, t & 3));
            color_rgba c5[4];
            etc_block::get_block_colors5(c5, color_rgba(rb.m_bytes[0] & 31, rb.m_bytes[1] & 31, rb.m_bytes[2] & 31, 255), t * 2, false);
            for (int i = 0; i < 4; i++) h.c(c5[i]);
            etc_block::get_block_colors4(c5, color_rgba(rb.m_bytes[0] & 15, rb.m_bytes[1] & 15, rb.m_bytes[2] & 15, 255), t * 2 + 1, false);
            for (int i = 0; i < 4; i++) h.c(c5[i]);
            if (ok) {
                h.q(rb.evaluate_etc1_error(px, t >= 2));
                h.q(rb.evaluate_etc1_error(px, t >= 2, t & 1));
                color_rgba sp[8];
                rb.get_subblock_pixels(sp, t & 1);
                for (int i = 0; i < 8; i++) h.c(sp[i]);
                h.q(rb.determine_selectors(px, t < 2));
                h.bytes(rb.m_bytes, 8);
            }
            etc_block cb;
            cb.clear();
            color_rgba c0(rb.m_bytes[3] & 31, rb.m_bytes[4] & 31, rb.m_bytes[5] & 31, 255);
            color_rgba c1(rb.m_bytes[6] & 31, rb.m_bytes[7] & 31, rb.m_bytes[2] & 31, 255);
            h.u(cb.set_block_color5_check(c0, c1));
            h.bytes(cb.m_bytes, 8);
            cb.set_block_color5_clamp(c0, c1);
            h.bytes(cb.m_bytes, 8);
            color_rgba pair[2] = {c0, c1};
            h.u(etc_block::try_pack_color5_delta3(pair));
            h.u(etc_block::pack_color5(px[t], true));
            h.u(etc_block::pack_color4(px[t], true, 100));
        }

        // EAC A8.
        uint8_t alpha[16];
        for (int i = 0; i < 16; i++) alpha[i] = px[i].a;
        pack_eac_a8_results er;
        uint32_t mask = (b % 5 == 0) ? 0xFFFFu : ((b % 5 == 1) ? 0x0F0Fu : UINT32_MAX);
        uint64_t eerr = pack_eac_a8(er, alpha, 16, b % 4, (b / 4) % 3, mask);
        h.q(eerr);
        h.u(er.m_base); h.u(er.m_table); h.u(er.m_multiplier);
        h.bytes(er.m_selectors.data(), 16);
        eac_a8_block eb;
        memset(&eb, 0, sizeof(eb));
        pack_eac_a8(&eb, alpha, 1, 1, UINT32_MAX);
        h.u(eb.m_base); h.u(eb.m_table); h.u(eb.m_multiplier);
        h.bytes(eb.m_selectors, 6);

        printf("%u\t%016llx\n", b, (unsigned long long)h.h);
    }
    return 0;
}
