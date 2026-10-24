// Golden-data generator for the astcenc compressor kernels port (dev tooling, not part of
// the Rust library). Calls the internal kernels directly on generated image blocks:
// load_image_block / load_image_block_fast_ldr, compute_avgs_and_dirs_{2,3,3_rgb,4},
// compute_error_squared_{rgb,rgba}. Prints "<case>\t<hash>" (NaNs canonicalized).
// Must stay in sync with tests/astcenc_kernels.rs. Build/run via
// scripts/golden/gen_astcenc_kernels_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "astcenc.h"
#include "astcenc_internal_entry.h"

struct H {
    uint64_t h = 0xcbf29ce484222325ULL;
    void bytes(const void* p, size_t n) {
        const uint8_t* b = (const uint8_t*)p;
        for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 0x100000001b3ULL; }
    }
    void f(float x) { uint32_t u; memcpy(&u, &x, 4); if (x != x) u = 0x7fc00000u; bytes(&u, 4); }
    void v4(vfloat4 a) { f(a.lane<0>()); f(a.lane<1>()); f(a.lane<2>()); f(a.lane<3>()); }
    void u(uint32_t x) { bytes(&x, 4); }
};

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
    float unit() { return (float)(next() & 0xffff) / 65535.0f; }
};

static void hash_block(H& h, const image_block& b) {
    for (unsigned i = 0; i < b.texel_count; i++) { h.f(b.data_r[i]); h.f(b.data_g[i]); h.f(b.data_b[i]); h.f(b.data_a[i]); }
    h.v4(b.origin_texel); h.v4(b.data_min); h.v4(b.data_mean); h.v4(b.data_max);
    h.u(b.grayscale); h.u(b.rgb_lns[0]); h.u(b.alpha_lns[0]);
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 5, 1}, {6, 6, 1}, {8, 8, 1}, {10, 5, 1}, {12, 12, 1},
                                 {3, 3, 3}, {4, 4, 4}, {6, 6, 6}};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR, ASTCENC_PRF_LDR_SRGB, ASTCENC_PRF_HDR_RGB_LDR_A, ASTCENC_PRF_HDR};
    const astcenc_swizzle SWZ[] = {{ASTCENC_SWZ_R, ASTCENC_SWZ_G, ASTCENC_SWZ_B, ASTCENC_SWZ_A},
                                   {ASTCENC_SWZ_B, ASTCENC_SWZ_R, ASTCENC_SWZ_1, ASTCENC_SWZ_G}};
    for (int si = 0; si < 9; si++) {
        const auto* s = SIZES[si];
        astcenc_config c;
        if (astcenc_config_init(ASTCENC_PRF_LDR, s[0], s[1], s[2], 60.0f, 0, &c) != ASTCENC_SUCCESS) return 1;
        astcenc_context* ctx = nullptr;
        if (astcenc_context_alloc(&c, 1, &ctx, nullptr) != ASTCENC_SUCCESS) return 1;
        const block_size_descriptor& bsd = *ctx->context.bsd;
        // 2 x size image so edge clamping is exercised (last block partially outside)
        unsigned dx = s[0] * 2 - 1, dy = s[1] * 2 - 1, dz = s[2] == 1 ? 1 : s[2] * 2 - 1;
        for (int dt = 0; dt < 3; dt++) {
            for (int kind = 0; kind < 4; kind++) {
                Lcg l{(uint32_t)(si * 100 + dt * 10 + kind)};
                std::vector<std::vector<uint8_t>> p8(dz);
                std::vector<std::vector<uint16_t>> p16(dz);
                std::vector<std::vector<float>> p32(dz);
                for (unsigned z = 0; z < dz; z++) {
                    for (unsigned i = 0; i < dx * dy * 4; i++) {
                        float v = l.unit();
                        if (kind == 1) v = (i % 4 == 3) ? 1.0f : 0.5f;             // constant
                        if (kind == 2 && (i % 4) != 3) v = ((i / 4) % 7) / 6.0f;   // grayscale
                        if (kind == 3) v = v * 4.0f - 1.0f;                        // HDR-ish, negatives
                        p8[z].push_back((uint8_t)(l.next() & 0xff));
                        if (kind == 1) p8[z].back() = (i % 4 == 3) ? 255 : 128;
                        if (kind == 2 && (i % 4) != 3) p8[z].back() = (uint8_t)((i / 4) * 37);
                        uint32_t hb = l.next();
                        uint16_t half = (uint16_t)(kind == 3 ? (hb & 0xffff) : (hb % 0x3c01));
                        if (kind == 1) half = (i % 4 == 3) ? 0x3c00 : 0x3800;
                        p16[z].push_back(half);
                        p32[z].push_back(v);
                    }
                }
                std::vector<void*> ptrs(dz);
                for (unsigned z = 0; z < dz; z++)
                    ptrs[z] = dt == 0 ? (void*)p8[z].data() : dt == 1 ? (void*)p16[z].data() : (void*)p32[z].data();
                astcenc_image img{dx, dy, dz, dt == 0 ? ASTCENC_TYPE_U8 : dt == 1 ? ASTCENC_TYPE_F16 : ASTCENC_TYPE_F32,
                                  ptrs.data()};
                for (int pi_ = 0; pi_ < 4; pi_++) {
                    for (int sw = 0; sw < 2; sw++) {
                        for (unsigned bz = 0; bz < (s[2] == 1 ? 1u : 2u); bz++)
                        for (unsigned by = 0; by < 2; by++)
                        for (unsigned bx = 0; bx < 2; bx++) {
                            image_block blk;
                            blk.texel_count = (uint8_t)(s[0] * s[1] * s[2]);
                            blk.channel_weight = vfloat4(1.0f, 0.5f + kind * 0.25f, 2.0f, 0.75f);
                            load_image_block(PROFILES[pi_], img, blk, bsd, bx * s[0], by * s[1], bz * s[2], SWZ[sw]);
                            H h;
                            hash_block(h, blk);
                            char key[96];
                            snprintf(key, sizeof key, "s%d t%d k%d p%d w%d b%u%u%u", si, dt, kind, pi_, sw, bx, by, bz);
                            if (dt == 0 && pi_ <= 1 && sw == 0 && s[2] == 1) {
                                image_block fb;
                                fb.texel_count = blk.texel_count;
                                load_image_block_fast_ldr(PROFILES[pi_], img, fb, bsd, bx * s[0], by * s[1], 0, SWZ[0]);
                                hash_block(h, fb);
                            }
                            // Kernels over a few partitionings of each count
                            for (unsigned pc = 1; pc <= 4; pc++) {
                                unsigned n = bsd.partitioning_count_selected[pc - 1];
                                for (unsigned k = 0; k < 3 && k < n; k++) {
                                    const partition_info& pi = bsd.get_raw_partition_info(pc, (k * 37 + bx + by) % n);
                                    partition_metrics pm[4];
                                    compute_avgs_and_dirs_4_comp(pi, blk, pm);
                                    for (unsigned q = 0; q < pc; q++) { h.v4(pm[q].avg); h.v4(pm[q].dir); }
                                    for (unsigned om = 0; om < 4; om++) {
                                        compute_avgs_and_dirs_3_comp(pi, blk, om, pm);
                                        for (unsigned q = 0; q < pc; q++) { h.v4(pm[q].avg); h.v4(pm[q].dir); }
                                    }
                                    compute_avgs_and_dirs_3_comp_rgb(pi, blk, pm);
                                    for (unsigned q = 0; q < pc; q++) { h.v4(pm[q].avg); h.v4(pm[q].dir); }
                                    const unsigned C2[3][2] = {{0, 1}, {0, 2}, {1, 2}};
                                    for (auto& cc : C2) {
                                        compute_avgs_and_dirs_2_comp(pi, blk, cc[0], cc[1], pm);
                                        for (unsigned q = 0; q < pc; q++) { h.v4(pm[q].avg); h.v4(pm[q].dir); }
                                    }
                                    // Lines from the 3 comp rgb metrics (as used by find_best_partitioning)
                                    compute_avgs_and_dirs_3_comp_rgb(pi, blk, pm);
                                    partition_lines3 pl[4];
                                    processed_line4 up4[4], sp4[4];
                                    for (unsigned q = 0; q < pc; q++) {
                                        vfloat4 dir = normalize_safe(pm[q].dir, vfloat4(0.57735f, 0.57735f, 0.57735f, 0.0f));
                                        vfloat4 avg = pm[q].avg;
                                        vfloat4 sdir = normalize_safe(avg, vfloat4(0.57735f, 0.57735f, 0.57735f, 0.0f));
                                        pl[q].uncor_pline.amod = avg - dir * dot(avg, dir);
                                        pl[q].uncor_pline.bs = dir;
                                        pl[q].samec_pline.amod = vfloat4::zero();
                                        pl[q].samec_pline.bs = sdir;
                                        up4[q].amod = vfloat4(avg.lane<0>(), avg.lane<1>(), avg.lane<2>(), 0.25f);
                                        up4[q].bs = vfloat4(dir.lane<0>(), dir.lane<1>(), dir.lane<2>(), 0.5f);
                                        sp4[q].amod = vfloat4::zero();
                                        sp4[q].bs = vfloat4(sdir.lane<0>(), sdir.lane<1>(), sdir.lane<2>(), 0.5f);
                                    }
                                    float ue = 0, se = 0;
                                    compute_error_squared_rgb(pi, blk, pl, ue, se);
                                    h.f(ue); h.f(se);
                                    for (unsigned q = 0; q < pc; q++) h.f(pl[q].line_length);
                                    float ll[4];
                                    compute_error_squared_rgba(pi, blk, up4, sp4, ll, ue, se);
                                    h.f(ue); h.f(se);
                                    for (unsigned q = 0; q < pc; q++) h.f(ll[q]);
                                }
                            }
                            printf("%s\t%016llx\n", key, (unsigned long long)h.h);
                        }
                    }
                }
            }
        }
        astcenc_context_free(ctx);
    }
    return 0;
}
