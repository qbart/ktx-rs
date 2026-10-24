// Golden-data generator for the basisu UASTC LDR 4x4 encoder port (src/basisu/uastc_enc.rs;
// dev tooling, not part of the Rust library). For generated 4x4 blocks runs encode_uastc at
// every pack level and with the favor / ETC1 hint flags, and decodes random BC1 / BC7 blocks
// with unpack_block. Prints "<block>\t<fnv1a64>". Must stay in sync with
// tests/basisu_uastc_enc.rs. Build/run via scripts/golden/gen_basisu_uastc_enc_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include "basisu_enc.h"
#include "basisu_etc.h"
#include "basisu_bc7enc.h"
#include "basisu_uastc_enc.h"
#include "basisu_gpu_texture.h"
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
};

static void make_block(uint32_t b, uint8_t* px) {
    Lcg l{b * 7919u + 13u};
    const uint32_t kind = b % 9;
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
        case 6: r = (b & 8) ? 255 - (int)(n0 % 6) : (int)(n0 % 6); g = (b & 16) ? 255 - (int)(n1 % 6) : (int)(n1 % 6); bl = (int)(n2 & 255); break;
        case 7: { int v = (int)br + (i & 3) * 17 - (i >> 2) * 9; r = g = bl = v; break; }
        default: { int v = (n0 & 1) ? (int)br : (int)r2; r = g = bl = v; break; }
        }
        a = (b % 3 == 0) ? 255 : ((b % 3 == 1) ? (int)(n3 & 255) : (int)((i * 17 + br) & 255));
        if (b % 11 == 5) a = (n3 & 1) ? 255 : 254;
        px[i * 4 + 0] = (uint8_t)clampi(r, 0, 255);
        px[i * 4 + 1] = (uint8_t)clampi(g, 0, 255);
        px[i * 4 + 2] = (uint8_t)clampi(bl, 0, 255);
        px[i * 4 + 3] = (uint8_t)clampi(a, 0, 255);
    }
}

static const uint32_t EXTRA_FLAGS[6] = {
    cPackUASTCFavorUASTCError, cPackUASTCFavorBC7Error, cPackUASTCETC1FasterHints,
    cPackUASTCETC1FastestHints, cPackUASTCETC1DisableFlipAndIndividual, cPackUASTCFavorSimplerModes,
};

int main() {
    basist::basisu_transcoder_init();
    bc7enc_compress_block_init();
    pack_etc1_solid_color_init();
    g_cpu_supports_sse41 = true;
    const uint32_t NUM_BLOCKS = 3000;
    for (uint32_t b = 0; b < NUM_BLOCKS; b++) {
        uint8_t px[64];
        make_block(b, px);
        H h;
        for (uint32_t v = 0; v < 11; v++) {
            uint32_t flags;
            if (v < 5) {
                flags = v;
                if (v == 4 && (b % 5) != 0) continue;
            } else {
                flags = ((v == 10) ? 3 : (b % 3)) | EXTRA_FLAGS[v - 5];
            }
            basist::uastc_block blk;
            memset(&blk, 0, sizeof(blk));
            encode_uastc(px, blk, flags);
            h.bytes(&blk, 16);
        }
        Lcg l{b * 101u + 3u};
        uint8_t raw[16];
        for (int i = 0; i < 16; i++) raw[i] = (uint8_t)l.next();
        color_rgba out[16];
        memset(out, 0, sizeof(out));
        unpack_block(texture_format::cBC1, raw, out, false);
        h.bytes(out, sizeof(out));
        memset(out, 0, sizeof(out));
        bool ok = unpack_block(texture_format::cBC7, raw, out, false);
        h.bytes(&ok, 1);
        h.bytes(out, sizeof(out));
        printf("%u\t%016llx\n", b, (unsigned long long)h.h);
    }
    return 0;
}
