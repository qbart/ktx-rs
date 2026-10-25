// Golden-data generator for the building blocks of the ASTC HDR 6x6 encoder port
// (src/basisu/astc_hdr_6x6_enc.rs; dev tooling, not part of the Rust library). Includes
// basisu_astc_hdr_6x6_enc.cpp to reach its static helpers: PQ / ITP, partition tables and vp
// trees, partition estimates, 2 / 3 subset block encoders, value / block coding, block filters,
// stats, ITP metrics, smooth maps, tone mapping, Gaussian resampling, deblocking penalty and
// strip sizes. Prints "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_hdr6x6_parts.rs.
// Build/run via scripts/golden/gen_basisu_hdr6x6_parts_golden.sh.
#include "../../ref/ktx/external/basis_universal/encoder/basisu_astc_hdr_6x6_enc.cpp"
#include <cstdio>

using namespace astc_6x6_hdr;

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
    void f(float v) { bytes(&v, 4); }
    void d(double v) { bytes(&v, 8); }
};

static uint16_t rand_half(Lcg& l, int lo, int hi) {
    int e = lo + (int)(l.next() % (uint32_t)(hi - lo + 1)) + 15;
    if (e < 0) e = 0;
    if (e > 30) e = 30;
    return (uint16_t)((e << 10) | (l.next() & 1023));
}

static void make_block(uint32_t b, half_vec3 h[36], vec4F q16[36], vec4F f[36]) {
    Lcg l{b * 7919u + 29u};
    const uint32_t kind = b % 6;
    uint16_t base[3] = { rand_half(l, -4, 6), rand_half(l, -4, 6), rand_half(l, -4, 6) };
    uint16_t base2[3] = { rand_half(l, -4, 6), rand_half(l, -4, 6), rand_half(l, -4, 6) };
    uint16_t base3[3] = { rand_half(l, -4, 6), rand_half(l, -4, 6), rand_half(l, -4, 6) };
    for (uint32_t i = 0; i < 36; i++) {
        const uint32_t x = i % 6, y = i / 6;
        for (uint32_t c = 0; c < 3; c++) {
            uint32_t v;
            switch (kind) {
            case 0: v = rand_half(l, -8, 12); break;
            case 1: v = base[c] + (x * 30 + y * 20) * (c + 1); break;
            case 2: v = (x + y < 5) ? base[c] : base2[c]; break;
            case 3: v = (x < 2) ? base[c] : ((y < 3) ? base2[c] : base3[c]); break;
            case 4: v = base[0] + (l.next() % 64); break;
            default: v = base[c] + (l.next() % 300); break;
            }
            if (v > 0x7BFF) v = 0x7BFF;
            h[i][c] = (uint16_t)v;
            f[i][c] = basist::half_to_float((uint16_t)v);
            q16[i][c] = (float)half_to_qlog16((uint16_t)v);
        }
        q16[i][3] = 0.0f;
        f[i][3] = 1.0f;
    }
}

static void make_image(uint32_t seed, uint32_t w, uint32_t h, imagef& img) {
    Lcg l{seed * 131u + 7u};
    img.resize(w, h);
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            float base = (float)((x * 7 + y * 3) % 23) * .25f;
            for (uint32_t c = 0; c < 3; c++) {
                float v = (seed & 1) ? base * (c + 1) + (float)(l.next() % 100) * .01f : basist::half_to_float(rand_half(l, -6, 8));
                img(x, y)[c] = v;
            }
            img(x, y)[3] = 1.0f;
        }
}

int main() {
    basisu_encoder_init(false, false);
    {
        H hh;
        for (uint32_t i = 0; i < NUM_UNIQUE_PARTITIONS2; i++) hh.bytes(g_partitions2[i].m_parts, 36);
        hh.bytes(g_part2_seed_to_unique_index, sizeof(g_part2_seed_to_unique_index));
        for (uint32_t i = 0; i < NUM_UNIQUE_PARTITIONS3; i++) hh.bytes(g_partitions3[i].m_parts, 36);
        hh.bytes(g_part3_seed_to_unique_index, sizeof(g_part3_seed_to_unique_index));
        for (uint32_t w = 1; w <= 6; w++)
            for (uint32_t i = 0; i < 6; i++) {
                hh.u(g_contrib_lists[w][i].n);
                for (uint32_t j = 0; j < g_contrib_lists[w][i].n; j++) { hh.u(g_contrib_lists[w][i].p[j].pixel); hh.f(g_contrib_lists[w][i].p[j].weight); }
            }
        printf("tables\t%016llx\n", (unsigned long long)hh.h);
    }
    for (uint32_t i = 0; i < 400; i++) {
        Lcg l{i * 17u + 1u};
        H hh;
        for (uint32_t k = 0; k < 16; k++) {
            float v;
            switch (l.next() % 4) {
            case 0: v = (float)(l.next() % 100000) * .0001f; break;
            case 1: v = basist::half_to_float(rand_half(l, -15, 15)); break;
            case 2: v = (float)(l.next() % 1000) * 1e-7f; break;
            default: v = (float)(l.next() % 60000); break;
            }
            hh.f(forwardPQTab(v));
            const float g = basist::half_to_float(rand_half(l, -10, 10));
            const float bb = (float)(l.next() % 500) * .1f;
            vec3F rgb(v, g, bb), out;
            linear_rgb_to_ictcp(rgb, out, (k & 1) != 0, (k & 2) != 0);
            hh.f(out[0]); hh.f(out[1]); hh.f(out[2]);
        }
        printf("pq%u\t%016llx\n", i, (unsigned long long)hh.h);
    }
    for (uint32_t b = 0; b < 240; b++) {
        half_vec3 h[36]; vec4F q16[36], f[36];
        make_block(b, h, q16, f);
        Lcg l{b * 31u + 11u};
        H hh;
        static const uint32_t NB[] = { 1, 6, 24, 48 };
        int parts[64];
        for (uint32_t k = 0; k < 4; k++) {
            memset(parts, 0, sizeof(parts));
            bool s2 = estimate_partition2_6x6((basist::half_float(*)[3])h, parts, NB[k]);
            hh.u(s2); hh.bytes(parts, sizeof(int) * NB[k]);
            memset(parts, 0, sizeof(parts));
            bool s3 = estimate_partition3_6x6((basist::half_float(*)[3])h, parts, NB[k]);
            hh.u(s3); hh.bytes(parts, sizeof(int) * NB[k]);
        }
        vec3F hf[36];
        for (uint32_t i = 0; i < 36; i++) hf[i].set((float)h[i][0], (float)h[i][1], (float)h[i][2]);
        int cand[64];
        estimate_partition2_6x6((basist::half_float(*)[3])h, cand, 48);
        astc_hdr_codec_base_options coptions;
        coptions.m_q_log_bias = Q_LOG_BIAS_6x6;
        if (b & 1) { coptions.m_r_err_scale = 1.0f; coptions.m_g_err_scale = 1.0f; }
        int m11[64] = {}, m7[64] = {};
        estimate_partitions_mode7_and_11(2, NUM_UNIQUE_PARTITIONS2, g_partitions2, 48, (uint32_t*)cand, hf, coptions, 8, m11, m7);
        hh.bytes(m11, sizeof(int) * 8); hh.bytes(m7, sizeof(int) * 8);
        {
            static int all2[NUM_UNIQUE_PARTITIONS2];
            for (uint32_t i = 0; i < NUM_UNIQUE_PARTITIONS2; i++) all2[i] = i;
            estimate_partitions_mode7_and_11(2, NUM_UNIQUE_PARTITIONS2, g_partitions2, NUM_UNIQUE_PARTITIONS2, (uint32_t*)all2, hf, coptions, 20, m11, m7);
            hh.bytes(m11, sizeof(int) * 20); hh.bytes(m7, sizeof(int) * 20);
        }
        int c3[64] = {}, p3[64] = {};
        bool s3 = estimate_partition3_6x6((basist::half_float(*)[3])h, c3, 48);
        if (s3) estimate_partitions_mode7(3, NUM_UNIQUE_PARTITIONS3, g_partitions3, 48, (uint32_t*)c3, hf, coptions, 8, (uint32_t*)p3);
        hh.bytes(p3, sizeof(int) * 8);
        // block encoders over the 2 / 3 subset block modes
        for (uint32_t t = 0; t < 3; t++) {
            uint32_t bm;
            do { bm = l.next() % TOTAL_BLOCK_MODE_DECS; } while (g_block_mode_descs[bm].m_num_partitions < 2);
            const block_mode_desc& md = g_block_mode_descs[bm];
            const opt_mode_t om = (l.next() & 1) ? cWeightedLeastSquares : cOrdinaryLeastSquares;
            const bool uber = (l.next() & 1) != 0;
            const uint32_t comp_level = l.next() % 5;
            if (md.m_num_partitions == 2) {
                trial_result res[2];
                bool st = encode_block_2_subsets(res, md.m_grid_x, md.m_grid_y, md.m_cem, md.m_weight_ise_range, md.m_endpoint_ise_range, h, q16, coptions, uber,
                    (md.m_cem == 11) ? m11[t] : m7[t], comp_level, om, (l.next() & 1) != 0);
                hh.u(st);
                for (uint32_t r = 0; r < 2; r++) {
                    hh.u(st && res[r].m_valid);
                    if (st && res[r].m_valid) {
                        basist::astc_blk blk;
                        astc_helpers::pack_astc_block((astc_helpers::astc_block&)blk, res[r].m_log_blk);
                        hh.bytes(&blk, 16);
                        bitwise_coder c;
                        code_block(c, res[r].m_log_blk, (block_mode)bm, endpoint_mode::cRaw, nullptr);
                        c.flush();
                        hh.u(c.get_total_bits_u32()); hh.bytes(c.get_bytes().data(), c.get_bytes().size());
                    }
                }
            } else if (s3) {
                trial_result res;
                bool st = encode_block_3_subsets(res, md.m_cem, md.m_grid_x, md.m_grid_y, md.m_weight_ise_range, md.m_endpoint_ise_range, h, q16, coptions, uber,
                    p3, 1 + (l.next() % 4), comp_level, om);
                hh.u(st);
                if (st) {
                    hh.d(res.m_err);
                    basist::astc_blk blk;
                    astc_helpers::pack_astc_block((astc_helpers::astc_block&)blk, res.m_log_blk);
                    hh.bytes(&blk, 16);
                    bitwise_coder c;
                    code_block(c, res.m_log_blk, (block_mode)bm, endpoint_mode::cRaw, nullptr);
                    c.flush();
                    hh.u(c.get_total_bits_u32()); hh.bytes(c.get_bytes().data(), c.get_bytes().size());
                }
            }
        }
        // filters, stats, metrics
        vec4F xf[36], yf[36];
        filter_block(3, 6, f, xf);
        filter_block(6, 3, f, yf);
        hh.f(diff_blocks(f, xf)); hh.f(diff_blocks(f, yf));
        for (uint32_t i = 0; i < 36; i++) { hh.f(xf[i][0]); hh.f(xf[i][1]); hh.f(xf[i][2]); hh.f(yf[i][2]); }
        vec3F lp[36];
        filter_block(3, 3, hf, lp);
        hh.f(sub_and_compute_std_dev(lp, hf));
        const uint32_t g = 1 + (l.next() % 6), g2 = 1 + (l.next() % 6);
        filter_block(g, g2, hf, lp);
        for (uint32_t i = 0; i < 36; i++) hh.f(lp[i][1]);
        basisu::stats<float> st[3];
        for (uint32_t c = 0; c < 3; c++) {
            st[c].calc(36, &hf[0][c], 3);
            hh.f(st[c].m_std_dev); hh.f(st[c].m_range); hh.f(st[c].m_avg);
        }
        basisu::comparative_stats<float> cs;
        cs.calc_pearson(36, &hf[0][0], &hf[0][1], 3, 3, &st[0], &st[1]);
        hh.f(cs.m_pearson);
        vec3F itp_a[36], itp_b[36];
        astc_hdr_6x6_global_config cfg;
        cfg.m_rec2020_bt2100_color_gamut = (b & 2) != 0;
        for (uint32_t i = 0; i < 36; i++) {
            linear_rgb_to_itp(vec3F(f[i]), itp_a[i], cfg);
            linear_rgb_to_itp(vec3F(xf[i]), itp_b[i], cfg);
        }
        hh.f(compute_block_ssim_itp(6, 6, itp_a, itp_b));
        hh.f(compute_block_mse_itp(6, 6, itp_a, itp_b, true));
        hh.f(compute_block_mse_itp(6, 6, itp_a, itp_b, false));
        hh.f(compute_pixel_delta_itp(itp_a[3], itp_b[3], itp_a[3], (b & 4) != 0));
        // encode_values
        for (uint32_t r = 0; r <= 20; r++) {
            uint8_t vals[64];
            const uint32_t levels = astc_helpers::get_ise_levels(r);
            const uint32_t n = 1 + (l.next() % 64);
            for (uint32_t i = 0; i < n; i++) vals[i] = (uint8_t)(l.next() % levels);
            bitwise_coder c;
            hh.u(encode_values(c, n, vals, r));
            c.flush();
            hh.bytes(c.get_bytes().data(), c.get_bytes().size());
        }
        printf("b%u\t%016llx\n", b, (unsigned long long)hh.h);
    }
    for (uint32_t i = 0; i < 24; i++) {
        H hh;
        imagef img;
        const uint32_t w = 5 + i * 3 % 31, h = 4 + i * 5 % 23;
        make_image(i, w, h, img);
        image tm;
        bool tst = tonemap_image_compressive2(tm, img);
        hh.u(tst); hh.bytes(tm.get_ptr(), tm.get_total_pixels() * 4);
        smooth_map_params rp;
        rp.m_debug_images = false;
        vector2D<float> sm;
        create_smooth_maps2(sm, tm, rp);
        hh.bytes(sm.get_ptr(), sm.size_in_bytes());
        imagef f1(w, h), f2(w, h);
        image_resample(img, f1, "gaussian", 1.45f);
        image_resample(img, f2, "gaussian", 1.83f);
        for (uint32_t y = 0; y < h; y++) for (uint32_t x = 0; x < w; x++) { hh.bytes(&f1(x, y)[0], 16); hh.bytes(&f2(x, y)[0], 16); }
        astc_hdr_6x6_global_config cfg;
        cfg.m_rec2020_bt2100_color_gamut = (i & 1) != 0;
        imagef itp;
        convet_rgb_image_to_itp(img, itp, cfg);
        for (uint32_t y = 0; y < h; y++) for (uint32_t x = 0; x < w; x++) hh.bytes(&itp(x, y)[0], 12);
        candidate_encoding cand;
        for (uint32_t by = 0; by < (h + 5) / 6; by++)
            for (uint32_t bx = 0; bx < (w + 5) / 6; bx++) {
                for (uint32_t y = 0; y < 6; y++) for (uint32_t x = 0; x < 6; x++) cand.m_comp_pixels_itp[y][x] = vec3F(itp.get_clamped(bx * 6 + x + 1, by * 6 + y));
                hh.f(calc_deblocking_penalty_itp(bx, by, w, h, itp, cand));
            }
        printf("img%u\t%016llx\n", i, (unsigned long long)hh.h);
    }
    {
        H hh;
        astc_hdr_6x6_global_config cfg;
        for (uint32_t nby = 1; nby <= 70; nby += 3)
            for (uint32_t th = 1; th <= 9; th += 2)
                for (uint32_t k = 0; k < 4; k++) {
                    uint32_t ts = 0, rps = 0;
                    bool ok = calc_strip_size((k & 1) ? 0.0f : 1.5f, nby, th, (k & 2) != 0, ts, rps, cfg);
                    hh.u(ok); hh.u(ts); hh.u(rps);
                }
        printf("strips\t%016llx\n", (unsigned long long)hh.h);
    }
    return 0;
}
