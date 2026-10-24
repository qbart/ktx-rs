// Golden-data generator for the astcenc compressor search kernels port (dev tooling, not
// part of the Rust library). Calls the internal kernels directly on generated image blocks:
// find_best_partition_candidates, compute_angular_endpoints_{1plane,2planes},
// compute_ideal_endpoint_formats (fed like compress_symbolic.cpp does) and
// pack_color_endpoints. Prints "<case>\t<hash>" (NaNs canonicalized).
// Must stay in sync with tests/astcenc_formats.rs. Build/run via
// scripts/golden/gen_astcenc_formats_golden.sh.
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

static const int8_t FREE_BITS[4] {115 - 4, 111 - 4 - PARTITION_INDEX_BITS, 108 - 4 - PARTITION_INDEX_BITS,
                                  105 - 4 - PARTITION_INDEX_BITS};

// compute_ideal_endpoint_formats over the given block mode range; hashes the candidates.
static void formats(H& h, const partition_info& pi, const image_block& blk, const endpoints& ep,
                    compression_working_buffers& tmp, unsigned start, unsigned end) {
    for (unsigned limit : {1u, 4u, 8u}) {
        uint8_t pfs[TUNE_MAX_TRIAL_CANDIDATES][BLOCK_MAX_PARTITIONS] = {};
        int bm[TUNE_MAX_TRIAL_CANDIDATES] = {};
        quant_method ql[TUNE_MAX_TRIAL_CANDIDATES] = {}, qlm[TUNE_MAX_TRIAL_CANDIDATES] = {};
        unsigned n = compute_ideal_endpoint_formats(pi, blk, ep, tmp.qwt_bitcounts, tmp.qwt_errors, limit,
                                                    start, end, pfs, bm, ql, qlm, tmp);
        h.u(n);
        for (unsigned i = 0; i < n; i++) {
            h.u(bm[i]); h.u(ql[i]); h.u(qlm[i]);
            for (unsigned p = 0; p < pi.partition_count; p++) h.bytes(&pfs[i][p], 1);
        }
    }
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 5, 1}, {6, 6, 1}, {8, 8, 1}, {10, 5, 1}, {12, 12, 1},
                                 {3, 3, 3}, {4, 4, 4}, {6, 6, 6}};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR, ASTCENC_PRF_HDR};
    auto* tmpp = new compression_working_buffers;
    compression_working_buffers& tmp = *tmpp;
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
                        memset((void*)&tmp, 0, sizeof(tmp));
                        H h;
                        char key[96];
                        snprintf(key, sizeof key, "s%d t%d k%d p%d b%u", si, dt, kind, pi_, bx);
                        // Partitioning search
                        for (unsigned pc = 2; pc <= 4; pc++) {
                            unsigned n = bsd.partitioning_count_selected[pc - 1];
                            if (n == 0) continue;
                            for (unsigned v = 0; v < 2; v++) {
                                unsigned bp[TUNE_MAX_PARTITIONING_CANDIDATES] = {};
                                unsigned limit = v ? (n < 7 ? n : 7) : n;
                                unsigned req = v ? 2 : 8;
                                unsigned got = find_best_partition_candidates(bsd, blk, pc, limit, bp, req);
                                h.u(got);
                                for (unsigned i = 0; i < got; i++) h.u(bp[i]);
                            }
                        }
                        // 1 plane: angular endpoints, weight errors and endpoint formats
                        for (unsigned pc = 1; pc <= 4; pc++) {
                            unsigned n = bsd.partitioning_count_selected[pc - 1];
                            if (n == 0) continue;
                            const partition_info& pi = bsd.get_raw_partition_info(pc, (bx * 17 + kind) % n);
                            endpoints_and_weights ei {};
                            compute_ideal_colors_and_weights_1plane(blk, pi, ei);
                            for (int oa = 0; oa < 2; oa++) {
                                bool only_always = oa == 1;
                                if (only_always && bsd.decimation_mode_count_always == 0) continue;
                                unsigned mwq = only_always ? QUANT_8 : QUANT_32;
                                unsigned ndm = only_always ? bsd.decimation_mode_count_always
                                                           : bsd.decimation_mode_count_selected;
                                for (unsigned d = 0; d < ndm; d++) {
                                    if (!bsd.get_decimation_mode(d).is_ref_1plane((quant_method)mwq)) continue;
                                    compute_ideal_weights_for_decimation(ei, bsd.get_decimation_info(d),
                                                                         tmp.dec_weights_ideal + d * BLOCK_MAX_WEIGHTS);
                                }
                                compute_angular_endpoints_1plane(only_always, bsd, tmp.dec_weights_ideal, mwq, tmp);
                                unsigned nbm = only_always ? bsd.block_mode_count_1plane_always
                                                           : bsd.block_mode_count_1plane_selected;
                                for (unsigned i = 0; i < nbm; i++) {
                                    const block_mode& bm = bsd.block_modes[i];
                                    h.f(tmp.weight_low_value1[i]); h.f(tmp.weight_high_value1[i]);
                                    int bitcount = FREE_BITS[pc - 1] - bm.weight_bits;
                                    if (bm.quant_mode > mwq || bitcount <= 0) { tmp.qwt_errors[i] = 1e38f; continue; }
                                    tmp.qwt_bitcounts[i] = (int8_t)bitcount;
                                    const auto& di = bsd.get_decimation_info(bm.decimation_mode);
                                    float uqf[BLOCK_MAX_WEIGHTS];
                                    compute_quantized_weights_for_decimation(
                                        di, tmp.weight_low_value1[i], tmp.weight_high_value1[i],
                                        tmp.dec_weights_ideal + BLOCK_MAX_WEIGHTS * bm.decimation_mode, uqf,
                                        tmp.dec_weights_uquant + BLOCK_MAX_WEIGHTS * i, bm.get_weight_quant_mode());
                                    tmp.qwt_errors[i] = compute_error_of_weight_set_1plane(ei, di, uqf);
                                }
                                formats(h, pi, blk, ei.ep, tmp, 0, nbm);
                            }
                        }
                        // 2 planes
                        for (unsigned p2 = 0; p2 < 4; p2 += 3 - kind % 3) {
                            if (p2 == 3 && blk.is_constant_channel(3)) continue;
                            endpoints_and_weights ei1 {}, ei2 {};
                            compute_ideal_colors_and_weights_2planes(bsd, blk, p2, ei1, ei2);
                            unsigned mwq = QUANT_32;
                            for (unsigned d = 0; d < bsd.decimation_mode_count_selected; d++) {
                                if (!bsd.get_decimation_mode(d).is_ref_2plane((quant_method)mwq)) continue;
                                const auto& di = bsd.get_decimation_info(d);
                                compute_ideal_weights_for_decimation(ei1, di, tmp.dec_weights_ideal + d * BLOCK_MAX_WEIGHTS);
                                compute_ideal_weights_for_decimation(
                                    ei2, di, tmp.dec_weights_ideal + d * BLOCK_MAX_WEIGHTS + WEIGHTS_PLANE2_OFFSET);
                            }
                            compute_angular_endpoints_2planes(bsd, tmp.dec_weights_ideal, mwq, tmp);
                            unsigned start = bsd.block_mode_count_1plane_selected;
                            unsigned end = bsd.block_mode_count_1plane_2plane_selected;
                            for (unsigned i = start; i < end; i++) {
                                const block_mode& bm = bsd.block_modes[i];
                                h.f(tmp.weight_low_value1[i]); h.f(tmp.weight_high_value1[i]);
                                h.f(tmp.weight_low_value2[i]); h.f(tmp.weight_high_value2[i]);
                                if (bm.quant_mode > mwq) { tmp.qwt_errors[i] = 1e38f; continue; }
                                tmp.qwt_bitcounts[i] = (int8_t)(109 - bm.weight_bits);
                                const auto& di = bsd.get_decimation_info(bm.decimation_mode);
                                float u1[BLOCK_MAX_WEIGHTS], u2[BLOCK_MAX_WEIGHTS];
                                uint8_t* q = tmp.dec_weights_uquant + BLOCK_MAX_WEIGHTS * i;
                                const float* id = tmp.dec_weights_ideal + BLOCK_MAX_WEIGHTS * bm.decimation_mode;
                                compute_quantized_weights_for_decimation(di, tmp.weight_low_value1[i], tmp.weight_high_value1[i],
                                                                         id, u1, q, bm.get_weight_quant_mode());
                                compute_quantized_weights_for_decimation(di, tmp.weight_low_value2[i], tmp.weight_high_value2[i],
                                                                         id + WEIGHTS_PLANE2_OFFSET, u2,
                                                                         q + WEIGHTS_PLANE2_OFFSET, bm.get_weight_quant_mode());
                                tmp.qwt_errors[i] = compute_error_of_weight_set_2planes(ei1, ei2, di, u1, u2);
                            }
                            endpoints epm;
                            epm.partition_count = 1;
                            vmask4 p2m = vint4::lane_id() == vint4(p2);
                            epm.endpt0[0] = select(ei1.ep.endpt0[0], ei2.ep.endpt0[0], p2m);
                            epm.endpt1[0] = select(ei1.ep.endpt1[0], ei2.ep.endpt1[0], p2m);
                            formats(h, bsd.get_partition_info(1, 0), blk, epm, tmp, start, end);
                        }
                        printf("%s\t%016llx\n", key, (unsigned long long)h.h);
                    }
                }
            }
        }
        astcenc_context_free(ctx);
    }
    // Endpoint packing: every format and color quant level
    for (unsigned t = 0; t < 96; t++) {
        Lcg l{0x5eed0000u + t};
        auto comp = [&](float lo, float hi) { return lo + l.unit() * (hi - lo); };
        float lo = t % 4 == 3 ? -2000.0f : 0.0f, hi = t % 4 == 3 ? 70000.0f : 65535.0f;
        // Draws lanes in order (argument evaluation order is unspecified)
        auto V = [&](float l0, float h0, float l1, float h1, float l2, float h2, float l3, float h3) {
            float x = comp(l0, h0); float y = comp(l1, h1); float z = comp(l2, h2); float w = comp(l3, h3);
            return vfloat4(x, y, z, w);
        };
        vfloat4 c0 = V(lo, hi, lo, hi, lo, hi, lo, hi);
        vfloat4 c1 = V(lo, hi, lo, hi, lo, hi, lo, hi);
        if (t % 8 == 1) { vfloat4 o = V(0, 900, 0, 0, 0, 0, 0, 0); c1 = c0 + vfloat4(o.lane<0>()); }
        if (t % 8 == 2) c1 = c0;
        if (t % 8 == 5) {
            c0 = c0 * 0.05f;
            vfloat4 o = V(0, 3000, 0, 3000, 0, 3000, 0, 0);
            c1 = c0 + vfloat4(o.lane<0>(), o.lane<1>(), o.lane<2>(), 0.0f);
        }
        vfloat4 rgbs = V(lo, hi, lo, hi, lo, hi, 0.0f, 1.1f);
        vfloat4 rgbo = V(lo, hi, lo, hi, lo, hi, 0.0f, 20000.0f);
        H h;
        for (int fmt = 0; fmt < 16; fmt++) {
            for (int q = QUANT_6; q <= QUANT_256; q++) {
                uint8_t out[8] = {};
                uint8_t r = pack_color_endpoints(c0, c1, rgbs, rgbo, fmt, out, (quant_method)q);
                h.bytes(&r, 1);
                h.bytes(out, 8);
            }
        }
        printf("pack t%u\t%016llx\n", t, (unsigned long long)h.h);
    }
    delete tmpp;
    return 0;
}
