// Golden-data generator for the astcenc compressor kernels port (dev tooling, not part of
// the Rust library). Calls the internal kernels directly on generated image blocks:
// compute_ideal_colors_and_weights_{1,2}plane(s), compute_ideal_weights_for_decimation,
// compute_quantized_weights_for_decimation, compute_error_of_weight_set_*, recompute_ideal_colors_*. Prints "<case>\t<hash>" (NaNs canonicalized).
// Must stay in sync with tests/astcenc_weights.rs. Build/run via
// scripts/golden/gen_astcenc_weights_golden.sh.
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


static void hash_ei(H& h, const endpoints_and_weights& ei, unsigned texel_count) {
    h.u(ei.is_constant_weight_error_scale); h.u(ei.ep.partition_count);
    for (unsigned q = 0; q < ei.ep.partition_count; q++) { h.v4(ei.ep.endpt0[q]); h.v4(ei.ep.endpt1[q]); }
    for (unsigned i = 0; i < texel_count; i++) { h.f(ei.weights[i]); h.f(ei.weight_error_scale[i]); }
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 5, 1}, {6, 6, 1}, {8, 8, 1}, {10, 5, 1}, {12, 12, 1},
                                 {3, 3, 3}, {4, 4, 4}, {6, 6, 6}};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR, ASTCENC_PRF_HDR};
    for (int si = 0; si < 9; si++) {
        const auto* s = SIZES[si];
        astcenc_config c;
        if (astcenc_config_init(ASTCENC_PRF_LDR, s[0], s[1], s[2], 60.0f, 0, &c) != ASTCENC_SUCCESS) return 1;
        astcenc_context* ctx = nullptr;
        if (astcenc_context_alloc(&c, 1, &ctx, nullptr) != ASTCENC_SUCCESS) return 1;
        const block_size_descriptor& bsd = *ctx->context.bsd;
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
                        if (kind == 1) v = (i % 4 == 3) ? 1.0f : 0.5f;
                        if (kind == 2 && (i % 4) != 3) v = ((i / 4) % 7) / 6.0f;
                        if (kind == 3) v = v * 4.0f - 1.0f;
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
                astcenc_swizzle swz{ASTCENC_SWZ_R, ASTCENC_SWZ_G, ASTCENC_SWZ_B, ASTCENC_SWZ_A};
                for (int pi_ = 0; pi_ < 2; pi_++) {
                    for (unsigned bx = 0; bx < 2; bx++) {
                        image_block blk;
                        blk.texel_count = (uint8_t)(s[0] * s[1] * s[2]);
                        blk.channel_weight = vfloat4(1.0f, 0.5f + kind * 0.25f, 2.0f, 0.75f);
                        load_image_block(PROFILES[pi_], img, blk, bsd, bx * s[0], 0, 0, swz);
                        H h;
                        char key[96];
                        snprintf(key, sizeof key, "s%d t%d k%d p%d b%u", si, dt, kind, pi_, bx);
                        unsigned tc = blk.texel_count;
                        unsigned ndm = bsd.decimation_mode_count_selected;
                        // 1 plane
                        for (unsigned pc = 1; pc <= 4; pc++) {
                            unsigned n = bsd.partitioning_count_selected[pc - 1];
                            for (unsigned k = 0; k < 2 && k < n; k++) {
                                const partition_info& pi = bsd.get_raw_partition_info(pc, (k * 53 + bx) % n);
                                endpoints_and_weights ei {};
                                compute_ideal_colors_and_weights_1plane(blk, pi, ei);
                                hash_ei(h, ei, tc);
                                for (unsigned d = 0; d < ndm; d += (d < 5 ? 1 : 7)) {
                                    const decimation_info& di = bsd.get_decimation_info(d);
                                    float ideal[72] = {0};
                                    compute_ideal_weights_for_decimation(ei, di, ideal);
                                    for (unsigned w = 0; w < di.weight_count; w++) h.f(ideal[w]);
                                    for (unsigned q = 0; q < 12; q += (pc == 1 ? 1 : 5)) {
                                        float uval[72] = {0};
                                        uint8_t uq[72] = {0};
                                        float lo = (q % 2) ? 0.1f : 0.0f, hi = (q % 2) ? 0.85f : 1.0f;
                                        compute_quantized_weights_for_decimation(di, lo, hi, ideal, uval, uq, (quant_method)q);
                                        for (unsigned w = 0; w < di.weight_count; w++) { h.f(uval[w]); h.bytes(&uq[w], 1); }
                                        h.f(compute_error_of_weight_set_1plane(ei, di, uval));
                                        endpoints ep = ei.ep;
                                        vfloat4 rgbs[4], rgbo[4];
                                        for (int z = 0; z < 4; z++) { rgbs[z] = vfloat4::zero(); rgbo[z] = vfloat4::zero(); }
                                        recompute_ideal_colors_1plane(blk, pi, di, uq, ep, rgbs, rgbo);
                                        for (unsigned z = 0; z < pc; z++) {
                                            h.v4(ep.endpt0[z]); h.v4(ep.endpt1[z]); h.v4(rgbs[z]); h.v4(rgbo[z]);
                                        }
                                    }
                                }
                            }
                        }
                        // 2 planes
                        for (unsigned p2 = 0; p2 < 4; p2++) {
                            if (p2 == 3 && blk.is_constant_channel(3)) continue;
                            endpoints_and_weights ei1 {}, ei2 {};
                            compute_ideal_colors_and_weights_2planes(bsd, blk, p2, ei1, ei2);
                            hash_ei(h, ei1, tc);
                            hash_ei(h, ei2, tc);
                            for (unsigned d = 0; d < ndm; d++) {
                                const decimation_info& di = bsd.get_decimation_info(d);
                                if (di.weight_count > 32 || (d % 3) != 0) continue;
                                float id1[72] = {0}, id2[72] = {0};
                                compute_ideal_weights_for_decimation(ei1, di, id1);
                                compute_ideal_weights_for_decimation(ei2, di, id2);
                                for (unsigned q = 0; q < 12; q += 4) {
                                    float u1[72] = {0}, u2[72] = {0};
                                    uint8_t q1[72] = {0}, q2[72] = {0};
                                    compute_quantized_weights_for_decimation(di, 0.0f, 1.0f, id1, u1, q1, (quant_method)q);
                                    compute_quantized_weights_for_decimation(di, 0.0f, 1.0f, id2, u2, q2, (quant_method)q);
                                    h.f(compute_error_of_weight_set_2planes(ei1, ei2, di, u1, u2));
                                    endpoints ep = ei1.ep;
                                    vfloat4 rgbs = vfloat4::zero(), rgbo = vfloat4::zero();
                                    recompute_ideal_colors_2planes(blk, bsd, di, q1, q2, ep, rgbs, rgbo, (int)p2);
                                    h.v4(ep.endpt0[0]); h.v4(ep.endpt1[0]); h.v4(rgbs); h.v4(rgbo);
                                }
                            }
                        }
                        printf("%s\t%016llx\n", key, (unsigned long long)h.h);
                    }
                }
            }
        }
        astcenc_context_free(ctx);
    }
    return 0;
}
