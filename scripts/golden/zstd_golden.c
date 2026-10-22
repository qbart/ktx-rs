// Golden-data generator for zstd deflate (dev tooling, not part of the Rust
// library). Uses the libzstd 1.5.7 amalgamation bundled with basisu, which is
// what libktx links. Prints "<case> <size> <fnv1a64>" per output:
// - raw frames: ZSTD_compressCCtx of synthetic inputs and whole files,
// - ktx2 files: ktxTexture2_DeflateZstd-like compression of every level
//   (last level first, one CCtx), hashed as the concatenated level data.
// Build/run via scripts/golden/gen_zstd_golden.sh.
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "zstd.h"

static uint64_t fnv1a(const uint8_t* p, size_t n, uint64_t h) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}
#define FNV0 0xcbf29ce484222325ULL

static uint32_t lcg_state;
static uint32_t lcg(void) { lcg_state = lcg_state * 1664525u + 1013904223u; return lcg_state >> 8; }

static const int LEVELS[] = {-5, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22};
#define NLEVELS (int)(sizeof(LEVELS) / sizeof(LEVELS[0]))

static ZSTD_CCtx* cctx;

static void run_raw(const char* name, const uint8_t* in, size_t n) {
    size_t cap = ZSTD_compressBound(n);
    uint8_t* out = malloc(cap ? cap : 1);
    for (int i = 0; i < NLEVELS; i++) {
        size_t r = ZSTD_compressCCtx(cctx, out, cap, in, n, LEVELS[i]);
        if (ZSTD_isError(r)) { fprintf(stderr, "%s: %s\n", name, ZSTD_getErrorName(r)); exit(1); }
        printf("%s_L%d %zu %016llx\n", name, LEVELS[i], r, (unsigned long long)fnv1a(out, r, FNV0));
    }
    free(out);
}

static uint32_t rd32(const uint8_t* p) { return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }
static uint64_t rd64(const uint8_t* p) { return rd32(p) | (uint64_t)rd32(p + 4) << 32; }

static void run_ktx2(const char* name, const uint8_t* in, size_t n) {
    uint32_t levels = rd32(in + 40);
    if (levels == 0) levels = 1;
    if (rd32(in + 44) != 0) { fprintf(stderr, "%s: supercompressed\n", name); exit(1); }
    size_t cap = 0;
    for (uint32_t l = 0; l < levels; l++) cap += ZSTD_compressBound(rd64(in + 80 + 24 * l + 8));
    uint8_t* out = malloc(cap);
    for (int i = 0; i < NLEVELS; i++) {
        if (LEVELS[i] < 1) continue;
        size_t off = 0, remaining = cap;
        for (int l = (int)levels - 1; l >= 0; l--) {
            const uint8_t* e = in + 80 + 24 * l;
            size_t r = ZSTD_compressCCtx(cctx, out + off, remaining, in + rd64(e), rd64(e + 8), LEVELS[i]);
            if (ZSTD_isError(r)) { fprintf(stderr, "%s: %s\n", name, ZSTD_getErrorName(r)); exit(1); }
            off += r; remaining -= r;
        }
        printf("%s_deflate_L%d %zu %016llx\n", name, LEVELS[i], off, (unsigned long long)fnv1a(out, off, FNV0));
    }
    free(out);
    (void)n;
}

// Must match `synthetic_inputs` in tests/common/synthetic.rs.
static void synthetic(void) {
    uint8_t* d = malloc(1100000);
    size_t n;
    run_raw("empty", d, 0);
    run_raw("abc", (const uint8_t*)"abc", 3);
    lcg_state = 1; for (n = 0; n < 100000; n++) d[n] = (uint8_t)lcg();
    run_raw("random", d, n);
    lcg_state = 2; for (n = 0; n < 300000; n++) d[n] = (uint8_t)(lcg() & 3);
    run_raw("lowentropy", d, n);
    for (n = 0; n < 200000; n++) d[n] = (uint8_t)(((uint32_t)n * (uint32_t)n) >> 7);
    run_raw("quadratic", d, n);
    memset(d, 0, 1000000);
    run_raw("zeros", d, 1000000);
    static const char* words[] = {"the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ", "ktx ", "texture\n"};
    lcg_state = 3; n = 0;
    while (n < 150000) { const char* w = words[lcg() % 10]; while (*w) d[n++] = (uint8_t)*w++; }
    run_raw("text", d, n);
    lcg_state = 4; n = 0;
    for (int i = 0; i < 70000; i++) {
        uint32_t r = lcg();
        if (r % 7 == 0 && n > 300) { size_t back = 1 + r % 300, len = 3 + (r >> 9) % 40; for (size_t k = 0; k < len; k++, n++) d[n] = d[n - back]; }
        else d[n++] = (uint8_t)(r >> 3);
    }
    run_raw("backrefs", d, n);
    free(d);
}

int main(int argc, char** argv) {
    cctx = ZSTD_createCCtx();
    synthetic();
    for (int i = 1; i < argc; i++) {
        FILE* f = fopen(argv[i], "rb");
        if (!f) return 1;
        fseek(f, 0, SEEK_END); long len = ftell(f); fseek(f, 0, SEEK_SET);
        uint8_t* d = malloc(len);
        if (fread(d, 1, len, f) != (size_t)len) return 1;
        fclose(f);
        const char* name = strrchr(argv[i], '/'); name = name ? name + 1 : argv[i];
        run_raw(name, d, len);
        if (len > 12 && !memcmp(d + 1, "KTX 20", 6)) run_ktx2(name, d, len);
        free(d);
    }
    ZSTD_freeCCtx(cctx);
    return 0;
}
