// Golden-data generator for the astcenc compressor port (dev tooling, not part of the Rust
// library). Compresses generated images through the public astcenc_compress_image API for
// block sizes x profiles x qualities x flag sets x data types and prints
// "<case>\t<fnv1a64 of the compressed blocks>".
// Must stay in sync with tests/astcenc_compress.rs. Build/run via
// scripts/golden/gen_astcenc_compress_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "astcenc.h"

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
    float unit() { return (float)(next() & 0xffff) / 65535.0f; }
};

static uint64_t fnv(const uint8_t* p, size_t n) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

// Texel value in [0, 1] (HDR content scales it later) for content kind `kind`.
static float texel(Lcg& l, int kind, unsigned x, unsigned y, unsigned z, int c) {
    float n = l.unit();
    switch (kind) {
    case 0: // smooth gradient with light noise
        return (c == 3) ? 1.0f : ((float)(x * (c + 1) + y * 3 + z * 5) / 48.0f + n * 0.02f) - (float)(int)((float)(x * (c + 1) + y * 3 + z * 5) / 48.0f);
    case 1: // noise
        return n;
    case 2: // two regions with alpha edge
        return ((x + 2 * y) % 11 < 5) ? (c == 0 ? 0.9f : 0.1f + n * 0.05f) : (c == 2 ? 0.8f : 0.3f + n * 0.05f);
    case 3: // grayscale with alpha ramp
        return c == 3 ? (float)((x + y) % 9) / 8.0f : 0.25f + 0.5f * (float)((x * 7 + y * 3) % 13) / 12.0f;
    default: // constant
        return c == 1 ? 0.5f : 0.25f;
    }
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 4, 1}, {6, 6, 1}, {8, 8, 1}, {10, 5, 1}, {12, 12, 1}, {3, 3, 3}, {4, 4, 4}};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR, ASTCENC_PRF_LDR_SRGB, ASTCENC_PRF_HDR_RGB_LDR_A, ASTCENC_PRF_HDR};
    const float QUALITIES[] = {ASTCENC_PRE_FASTEST, ASTCENC_PRE_MEDIUM, ASTCENC_PRE_THOROUGH};
    const unsigned FLAGS[] = {0, ASTCENC_FLG_MAP_NORMAL, ASTCENC_FLG_USE_ALPHA_WEIGHT | ASTCENC_FLG_USE_PERCEPTUAL,
                              ASTCENC_FLG_USE_DECODE_UNORM8};
    for (int si = 0; si < 8; si++) {
        const auto* s = SIZES[si];
        for (int pi = 0; pi < 4; pi++) {
            for (int qi = 0; qi < 3; qi++) {
                for (int fi = 0; fi < 4; fi++) {
                    unsigned flags = FLAGS[fi];
                    bool hdr = pi >= 2;
                    if (hdr && fi == 3) continue; // decode unorm8 needs LDR
                    if (pi == 0 && fi == 3) flags = ASTCENC_FLG_MAP_RGBM;
                    int dt = (si + pi + qi + fi) % 3;
                    int kind = (si * 3 + pi + qi * 2 + fi) % 5;
                    astcenc_config c;
                    if (astcenc_config_init(PROFILES[pi], s[0], s[1], s[2], QUALITIES[qi], flags, &c) != ASTCENC_SUCCESS) return 1;
                    astcenc_context* ctx = nullptr;
                    if (astcenc_context_alloc(&c, 1, &ctx, nullptr) != ASTCENC_SUCCESS) return 2;
                    unsigned dx = s[0] * 2 + 1, dy = s[1] * 2 - 1, dz = s[2] == 1 ? 1 : s[2] + 1;
                    Lcg l{(uint32_t)(si * 1000 + pi * 100 + qi * 10 + fi)};
                    std::vector<std::vector<uint8_t>> p8(dz);
                    std::vector<std::vector<uint16_t>> p16(dz);
                    std::vector<std::vector<float>> p32(dz);
                    for (unsigned z = 0; z < dz; z++) {
                        for (unsigned y = 0; y < dy; y++) {
                            for (unsigned x = 0; x < dx; x++) {
                                for (int ch = 0; ch < 4; ch++) {
                                    float v = texel(l, kind, x, y, z, ch);
                                    if (v > 1.0f) v = 1.0f;
                                    float hv = (hdr && ch != 3) ? v * 6.0f : v;
                                    p8[z].push_back((uint8_t)(v * 255.0f + 0.5f));
                                    p16[z].push_back(0);
                                    p32[z].push_back(hv);
                                }
                            }
                        }
                        for (size_t i = 0; i < p32[z].size(); i++) {
                            // float -> half via the library's own F16C rounding path is not
                            // exposed; use a simple RNE conversion shared with the Rust test.
                            float f = p32[z][i];
                            uint32_t u; memcpy(&u, &f, 4);
                            uint32_t e = (u >> 23) & 0xff, m = u & 0x7fffff;
                            uint16_t h;
                            if (f == 0.0f) h = 0;
                            else if (e < 113) h = 0; // tiny values flush to zero
                            else {
                                uint32_t he = e - 112, hm = m >> 13, rest = m & 0x1fff;
                                if (rest > 0x1000 || (rest == 0x1000 && (hm & 1))) hm++;
                                if (hm == 0x400) { hm = 0; he++; }
                                h = (uint16_t)((he << 10) | hm);
                            }
                            p16[z][i] = h;
                        }
                    }
                    std::vector<void*> ptrs(dz);
                    for (unsigned z = 0; z < dz; z++)
                        ptrs[z] = dt == 0 ? (void*)p8[z].data() : dt == 1 ? (void*)p16[z].data() : (void*)p32[z].data();
                    astcenc_image img{dx, dy, dz, dt == 0 ? ASTCENC_TYPE_U8 : dt == 1 ? ASTCENC_TYPE_F16 : ASTCENC_TYPE_F32,
                                      ptrs.data()};
                    astcenc_swizzle swz{ASTCENC_SWZ_R, ASTCENC_SWZ_G, ASTCENC_SWZ_B, ASTCENC_SWZ_A};
                    if (fi == 1) swz = astcenc_swizzle{ASTCENC_SWZ_R, ASTCENC_SWZ_R, ASTCENC_SWZ_R, ASTCENC_SWZ_G};
                    size_t blocks = ((dx + s[0] - 1) / s[0]) * ((dy + s[1] - 1) / s[1]) * ((dz + s[2] - 1) / s[2]);
                    std::vector<uint8_t> out(blocks * 16);
                    if (astcenc_compress_image(ctx, &img, &swz, out.data(), out.size(), 0) != ASTCENC_SUCCESS) return 3;
                    printf("s%d p%d q%d f%d\t%016llx\n", si, pi, qi, fi, (unsigned long long)fnv(out.data(), out.size()));
                    astcenc_context_free(ctx);
                }
            }
        }
    }
    return 0;
}
