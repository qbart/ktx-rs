// Golden-data generator for the tinyexr writer port (dev tooling, not part of
// the Rust library). Saves synthetic scanline images (every input / requested
// pixel type pair, special float values, optional chromaticities attribute)
// with SaveEXRImageToMemory and prints "<case> <size> <fnv1a64>".
// Must stay in sync with tests/tinyexr.rs. Build/run via
// scripts/golden/gen_tinyexr_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include "encoder/basisu_miniz.h" // the miniz tinyexr links against (unused for NONE)
#include "tinyexr.h"

static uint64_t fnv1a(const uint8_t* p, size_t n) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

static uint32_t lcg_state;
static uint32_t lcg() { lcg_state = lcg_state * 1664525u + 1013904223u; return lcg_state >> 8; }

static const uint32_t SPECIAL[] = {0x00000000u, 0x80000000u, 0x3f800000u, 0xbf800000u, 0x7f800000u, 0xff800000u,
                                   0x7fc00000u, 0x00000001u, 0x387fe000u, 0x33800000u, 0x477fe000u, 0x477ff000u,
                                   0x47800000u, 0x38800000u, 0x38000000u, 0x3f801000u, 0x3f802000u, 0xc7123456u};

// Value of channel c at pixel i for an input pixel type.
static uint32_t value(int pixel_type, int c, size_t i) {
    uint32_t r = lcg();
    if (pixel_type == TINYEXR_PIXELTYPE_UINT) return r * 2654435761u;
    if (pixel_type == TINYEXR_PIXELTYPE_HALF) return (r ^ (uint32_t)c) & 0xffff;
    if (i % 3 == 0) return SPECIAL[(r >> 4) % (sizeof(SPECIAL) / sizeof(SPECIAL[0]))];
    float f = (float)((int)(r % 200001) - 100000) / (float)(1u << (r % 24));
    uint32_t u;
    memcpy(&u, &f, 4);
    return u;
}

static void run(const std::string& name, int w, int h, const std::vector<int>& pixel_types,
                const std::vector<int>& requested, bool chroma) {
    lcg_state = (uint32_t)(w * 131 + h * 7 + pixel_types.size());
    const char* NAMES[] = {"A", "B", "G", "R"};
    int n = (int)pixel_types.size();
    std::vector<std::vector<uint32_t>> images(n, std::vector<uint32_t>((size_t)w * h));
    std::vector<unsigned char*> ptrs(n);
    for (size_t i = 0; i < (size_t)w * h; i++)
        for (int c = 0; c < n; c++) images[c][i] = value(pixel_types[c], c, i);
    // HALF input images are arrays of unsigned short
    std::vector<std::vector<uint16_t>> halves(n);
    for (int c = 0; c < n; c++) {
        if (pixel_types[c] == TINYEXR_PIXELTYPE_HALF) {
            halves[c].resize(images[c].size());
            for (size_t i = 0; i < images[c].size(); i++) halves[c][i] = (uint16_t)images[c][i];
            ptrs[c] = reinterpret_cast<unsigned char*>(halves[c].data());
        } else {
            ptrs[c] = reinterpret_cast<unsigned char*>(images[c].data());
        }
    }
    EXRHeader header;
    InitEXRHeader(&header);
    EXRImage image;
    InitEXRImage(&image);
    image.images = ptrs.data();
    image.width = w;
    image.height = h;
    image.num_channels = n;
    header.num_channels = n;
    header.compression_type = TINYEXR_COMPRESSIONTYPE_NONE;
    std::vector<EXRChannelInfo> infos(n);
    std::vector<int> pt(pixel_types), rq(requested);
    for (int c = 0; c < n; c++) strncpy(infos[c].name, NAMES[c % 4], 255);
    header.channels = infos.data();
    header.pixel_types = pt.data();
    header.requested_pixel_types = rq.data();
    EXRAttribute attr{};
    float prim[8] = {0.64f, 0.33f, 0.30f, 0.60f, 0.15f, 0.06f, 0.3127f, 0.3290f};
    if (chroma) {
        strncpy(attr.name, "chromaticities", sizeof(attr.name));
        strncpy(attr.type, "chromaticities", sizeof(attr.type));
        attr.size = sizeof(prim);
        attr.value = reinterpret_cast<unsigned char*>(prim);
        header.custom_attributes = &attr;
        header.num_custom_attributes = 1;
    }
    unsigned char* data = nullptr;
    const char* err = nullptr;
    size_t size = SaveEXRImageToMemory(&image, &header, &data, &err);
    printf("%s %zu %016llx\n", name.c_str(), size, (unsigned long long)(size ? fnv1a(data, size) : 0));
    free(data);
    if (err) FreeEXRErrorMessage(err);
}

int main() {
    const int U = TINYEXR_PIXELTYPE_UINT, H = TINYEXR_PIXELTYPE_HALF, F = TINYEXR_PIXELTYPE_FLOAT;
    const int PAIRS[][2] = {{F, H}, {F, F}, {U, U}, {H, H}, {H, F}};
    const int SIZES[][2] = {{1, 1}, {7, 3}, {32, 17}};
    for (auto& s : SIZES)
        for (int n = 1; n <= 4; n++)
            for (auto& p : PAIRS)
                for (int chroma = 0; chroma < 2; chroma++) {
                    std::vector<int> pt(n, p[0]), rq(n, p[1]);
                    std::string name = std::to_string(s[0]) + "x" + std::to_string(s[1]) + "_n" + std::to_string(n) +
                                       "_" + std::to_string(p[0]) + std::to_string(p[1]) + (chroma ? "_chroma" : "");
                    run(name, s[0], s[1], pt, rq, chroma);
                }
    // mixed channel types (depth float + stencil half, uint + float)
    run("mixed_ds", 9, 5, {F, F}, {F, H}, false);
    run("mixed_uf", 9, 5, {U, F, F}, {U, F, H}, true);
    return 0;
}
