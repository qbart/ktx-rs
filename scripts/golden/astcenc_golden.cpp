// Golden-data generator for the astcenc decompressor port (dev tooling, not
// part of the Rust library). Decodes pseudo random ASTC data (every 2D and 3D
// block size, every profile, U8 / F16 / F32 output, a couple of swizzles)
// with astcenc_decompress_image and prints "<case> <fnv1a64>".
// Must stay in sync with tests/astcenc.rs. Build/run via
// scripts/golden/gen_astcenc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <string>
#include <vector>
#include "astcenc.h"

static uint64_t fnv1a(const uint8_t* p, size_t n, uint64_t h = 0xcbf29ce484222325ULL) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

static uint32_t lcg_state;
static uint32_t lcg() { lcg_state = lcg_state * 1664525u + 1013904223u; return lcg_state >> 8; }

// Block i of the test data: mostly random, every 8th a void-extent block.
static void make_block(uint8_t* b, unsigned i) {
    for (int k = 0; k < 16; k++) b[k] = (uint8_t)lcg();
    if (i % 8 == 7) {
        uint32_t ve = (lcg() & 1) ? 0x1FC : 0x3FC; // U16 or F16 constant
        b[0] = (uint8_t)(ve & 0xFF);
        b[1] = (uint8_t)((b[1] & 0xF8) | (ve >> 8) | 0x0C) ;
        if (i % 16 == 7) { for (int k = 1; k < 8; k++) b[k] |= (k == 1 ? 0xFE : 0xFF); b[1] = (uint8_t)((ve >> 8) | 0xFC); }
    }
}

int main() {
    const unsigned SIZES[][3] = {{4, 4, 1}, {5, 4, 1}, {5, 5, 1}, {6, 5, 1}, {6, 6, 1}, {8, 5, 1}, {8, 6, 1},
                                 {8, 8, 1}, {10, 5, 1}, {10, 6, 1}, {10, 8, 1}, {10, 10, 1}, {12, 10, 1}, {12, 12, 1},
                                 {3, 3, 3}, {4, 3, 3}, {4, 4, 3}, {4, 4, 4}, {5, 4, 4}, {5, 5, 4}, {5, 5, 5},
                                 {6, 5, 5}, {6, 6, 5}, {6, 6, 6}};
    const astcenc_profile PROFILES[] = {ASTCENC_PRF_LDR_SRGB, ASTCENC_PRF_LDR, ASTCENC_PRF_HDR_RGB_LDR_A, ASTCENC_PRF_HDR};
    const astcenc_type TYPES[] = {ASTCENC_TYPE_U8, ASTCENC_TYPE_F16, ASTCENC_TYPE_F32};
    const astcenc_swizzle SWIZZLES[] = {{ASTCENC_SWZ_R, ASTCENC_SWZ_G, ASTCENC_SWZ_B, ASTCENC_SWZ_A},
                                        {ASTCENC_SWZ_A, ASTCENC_SWZ_Z, ASTCENC_SWZ_1, ASTCENC_SWZ_0}};
    for (auto& s : SIZES) {
        const unsigned bx = s[0], by = s[1], bz = s[2];
        const unsigned blocks_x = 7, blocks_y = 5, blocks_z = bz > 1 ? 2 : 1;
        const unsigned dim_x = blocks_x * bx - 1, dim_y = blocks_y * by - 2, dim_z = bz > 1 ? blocks_z * bz - 1 : 1;
        const unsigned nblocks = blocks_x * blocks_y * blocks_z;
        std::vector<uint8_t> data(nblocks * 16);
        lcg_state = bx * 10000 + by * 100 + bz;
        for (unsigned i = 0; i < nblocks; i++) make_block(&data[i * 16], i);
        for (int p = 0; p < 4; p++)
            for (int t = 0; t < 3; t++)
                for (int sw = 0; sw < 2; sw++) {
                    astcenc_config config{};
                    astcenc_error ec = astcenc_config_init(PROFILES[p], bx, by, bz, ASTCENC_PRE_MEDIUM,
                                                           ASTCENC_FLG_DECOMPRESS_ONLY, &config);
                    if (ec != ASTCENC_SUCCESS) { printf("config error %d\n", ec); return 1; }
                    astcenc_context* ctx = nullptr;
                    ec = astcenc_context_alloc(&config, 1, &ctx, nullptr);
                    if (ec != ASTCENC_SUCCESS) { printf("alloc error %d\n", ec); return 1; }
                    size_t comp = TYPES[t] == ASTCENC_TYPE_U8 ? 1 : TYPES[t] == ASTCENC_TYPE_F16 ? 2 : 4;
                    std::vector<std::vector<uint8_t>> planes(dim_z, std::vector<uint8_t>(dim_x * dim_y * 4 * comp));
                    std::vector<void*> ptrs(dim_z);
                    for (unsigned z = 0; z < dim_z; z++) ptrs[z] = planes[z].data();
                    astcenc_image image{};
                    image.dim_x = dim_x;
                    image.dim_y = dim_y;
                    image.dim_z = dim_z;
                    image.data_type = TYPES[t];
                    image.data = ptrs.data();
                    ec = astcenc_decompress_image(ctx, data.data(), data.size(), &image, &SWIZZLES[sw], 0);
                    astcenc_context_free(ctx);
                    uint64_t h = 0xcbf29ce484222325ULL;
                    for (auto& pl : planes) h = fnv1a(pl.data(), pl.size(), h);
                    printf("%ux%ux%u_p%d_t%d_s%d %d %016llx\n", bx, by, bz, p, t, sw, (int)ec, (unsigned long long)h);
                }
    }
    return 0;
}
