// Golden-data generator for libktx info.c (dev tooling, not part of the Rust
// library). For every KTX/KTX2 file given, prints ktxPrintInfoForMemory output
// and, for KTX2, the text/JSON/minified KTX2 printers plus truncated copies
// (KTX2 only: KTX1 truncation hits uninitialized reads in info.c).
//
// Build/run via scripts/golden/gen_info_golden.sh.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ktx.h>

KTX_error_code ktxPrintKTX1InfoTextForStream(ktxStream* stream);
KTX_error_code ktxMemStream_construct_ro(ktxStream* str, const ktx_uint8_t* bytes, const ktx_size_t size);

static void ktx2(const uint8_t* d, size_t n) {
    printf("-- text\n");
    fflush(stdout);
    printf("-- rc %d\n", ktxPrintKTX2InfoTextForMemory(d, n));
    printf("-- json\n");
    printf("-- rc %d\n", ktxPrintKTX2InfoJSONForMemory(d, n, 1, 4, false));
    printf("-- minified\n");
    printf("-- rc %d\n", ktxPrintKTX2InfoJSONForMemory(d, n, 1, 4, true));
}

static void run(const char* name, const uint8_t* d, size_t n) {
    printf("== %s\n", name);
    printf("-- rc %d\n", ktxPrintInfoForMemory(d, n));
    if (n > 12 && d[5] == '2') {
        ktx2(d, n);
        size_t cuts[] = {100, 200, n / 2, n - 1};
        for (int c = 0; c < 4; c++) {
            if (cuts[c] >= n) continue;
            printf("-- truncated %zu\n", cuts[c]);
            printf("-- rc %d\n", ktxPrintInfoForMemory(d, cuts[c]));
            ktx2(d, cuts[c]);
        }
    } else {
        ktxStream s;
        ktxMemStream_construct_ro(&s, d, n);
        printf("-- ktx1\n");
        printf("-- rc %d\n", ktxPrintKTX1InfoTextForStream(&s));
        s.destruct(&s);
    }
}

static uint8_t* load(const char* path, size_t* n) {
    FILE* f = fopen(path, "rb");
    if (!f) exit(1);
    fseek(f, 0, SEEK_END);
    *n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t* d = malloc(*n);
    if (fread(d, 1, *n, f) != *n) exit(1);
    fclose(f);
    return d;
}

static void swap32(uint8_t* p) {
    uint8_t t = p[0]; p[0] = p[3]; p[3] = t;
    t = p[1]; p[1] = p[2]; p[2] = t;
}

// Synthetic variants: argv[1] is a single-level KTX1 file without KVD,
// argv[2] a KTX1 file with KVD, argv[3] a KTX2 file.
static void synthetic(char** argv) {
    size_t n;
    uint8_t* d = load(argv[1], &n);
    for (int o = 12; o <= 64; o += 4) swap32(d + o);  // header + imageSize
    run("synthetic_big_endian.ktx", d, n);
    free(d);
    d = load(argv[2], &n);
    d[64] = 0xFF, d[65] = 0xFF;  // first key/value size
    run("synthetic_bad_kvd.ktx", d, n);
    free(d);
    d = load(argv[3], &n);
    uint32_t dfdOfs;
    memcpy(&dfdOfs, d + 48, 4);
    d[dfdOfs] ^= 4;
    run("synthetic_bad_dfd_size.ktx2", d, n);
    d[0] ^= 1;
    run("synthetic_bad_identifier.ktx2", d, n);
    free(d);
}

int main(int argc, char** argv) {
    synthetic(argv);
    for (int i = 4; i < argc; i++) {
        size_t n;
        uint8_t* d = load(argv[i], &n);
        const char* name = strrchr(argv[i], '/');
        run(name ? name + 1 : argv[i], d, n);
        free(d);
    }
    return 0;
}
