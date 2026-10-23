// Golden-data generator for the lodepng encoder port (dev tooling, not part
// of the Rust library). Encodes synthetic images in every raw color type /
// bit depth with option variants, and zlib-compresses the synthetic inputs of
// the miniz golden, printing "<case> <error> <size> <fnv1a64>" per output.
// Must stay in sync with tests/lodepng.rs. Build/run via
// scripts/golden/gen_lodepng_golden.sh.
#include <cstdio>
#include <cstdint>
#include <string>
#include <vector>
#include "lodepng.h"

static uint64_t fnv1a(const uint8_t* p, size_t n) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

static uint32_t lcg_state;
static uint32_t lcg() { lcg_state = lcg_state * 1664525u + 1013904223u; return lcg_state >> 8; }

static const char* KINDS[] = {"noise", "gradient", "fewcolors", "grey", "key", "flat", "grey4", "alphagrad"};
static const uint16_t FEW[5][4] = {{10, 200, 30, 255}, {250, 250, 250, 255}, {0, 0, 0, 255}, {90, 10, 160, 255}, {33, 66, 99, 255}};

// 16-bit RGBA of pixel (x, y) for a kind.
static void pixel(int kind, unsigned x, unsigned y, uint16_t out[4]) {
    switch (kind) {
    case 0: for (int c = 0; c < 4; c++) out[c] = (uint16_t)(lcg() & 0xffff); break;
    case 1: for (int c = 0; c < 4; c++) out[c] = (uint16_t)(((x * 7 + y * 13 + c * 50) & 255) * 257); break;
    case 2: case 4: {
        unsigned k = lcg() % 5;
        for (int c = 0; c < 4; c++) out[c] = (uint16_t)(FEW[k][c] * 257);
        if (kind == 4 && k == 0) out[3] = 0;
        break;
    }
    case 3: { uint16_t v = (uint16_t)((lcg() & 255) * 257); out[0] = out[1] = out[2] = v; out[3] = 65535; break; }
    case 5: out[0] = out[1] = out[2] = out[3] = 0; break;
    case 6: { uint16_t v = (uint16_t)((lcg() % 16) * 17 * 257); out[0] = out[1] = out[2] = v; out[3] = 65535; break; }
    default: for (int c = 0; c < 3; c++) out[c] = (uint16_t)(((x * 3 + y * 5 + c * 40) & 255) * 257);
             out[3] = (uint16_t)(((x * 5) & 255) * 257); break;
    }
}

static unsigned channels(LodePNGColorType ct) {
    return ct == LCT_GREY ? 1 : ct == LCT_GREY_ALPHA ? 2 : ct == LCT_RGB ? 3 : 4;
}

// Raw image buffer in the given mode (MSB-first bit packing below 8 bits).
static std::vector<unsigned char> make_image(int kind, LodePNGColorType ct, unsigned bd, unsigned w, unsigned h) {
    lcg_state = (uint32_t)(kind * 1000 + ct * 100 + bd);
    unsigned nc = channels(ct);
    std::vector<unsigned char> out((size_t(w) * h * nc * bd + 7) / 8, 0);
    size_t bit = 0;
    for (unsigned y = 0; y < h; y++)
        for (unsigned x = 0; x < w; x++) {
            uint16_t p[4];
            pixel(kind, x, y, p);
            uint16_t v[4];
            if (ct == LCT_GREY) { v[0] = p[0]; }
            else if (ct == LCT_GREY_ALPHA) { v[0] = p[0]; v[1] = p[3]; }
            else if (ct == LCT_RGB) { v[0] = p[0]; v[1] = p[1]; v[2] = p[2]; }
            else { v[0] = p[0]; v[1] = p[1]; v[2] = p[2]; v[3] = p[3]; }
            for (unsigned c = 0; c < nc; c++) {
                unsigned val = v[c] >> (16 - bd);
                for (int b = (int)bd - 1; b >= 0; b--, bit++)
                    if ((val >> b) & 1) out[bit >> 3] |= (unsigned char)(0x80 >> (bit & 7));
            }
        }
    return out;
}

static void emit(const std::string& name, unsigned error, const std::vector<unsigned char>& png) {
    printf("%s %u %zu %016llx\n", name.c_str(), error, png.size(), (unsigned long long)fnv1a(png.data(), png.size()));
}

static void encode_case(const std::string& name, int kind, LodePNGColorType ct, unsigned bd, unsigned w, unsigned h,
                        const std::string& variant) {
    std::vector<unsigned char> image = make_image(kind, ct, bd, w, h);
    lodepng::State state;
    state.info_raw.colortype = ct;
    state.info_raw.bitdepth = bd;
    state.info_png.color.colortype = ct;
    state.info_png.color.bitdepth = bd;
    if (variant == "sbit") {
        state.info_png.sbit_defined = 1;
        state.info_png.sbit_r = state.info_png.sbit_g = state.info_png.sbit_b = bd < 5 ? bd : 5;
        state.info_png.sbit_a = bd;
    } else if (variant == "sbitfull") {
        state.info_png.sbit_defined = 1;
        state.info_png.sbit_r = state.info_png.sbit_g = state.info_png.sbit_b = state.info_png.sbit_a = bd;
    } else if (variant == "srgb") {
        state.info_png.srgb_defined = 1;
        state.info_png.srgb_intent = 0;
        state.info_png.chrm_defined = 1;
        state.info_png.chrm_white_x = 31270; state.info_png.chrm_white_y = 32900;
        state.info_png.chrm_red_x = 64000; state.info_png.chrm_red_y = 33000;
        state.info_png.chrm_green_x = 30000; state.info_png.chrm_green_y = 60000;
        state.info_png.chrm_blue_x = 15000; state.info_png.chrm_blue_y = 6000;
    } else if (variant == "gama") {
        state.info_png.gama_defined = 1;
        state.info_png.gama_gamma = 100000;
    } else if (variant.size() == 2 && variant[0] == 'f') {
        state.encoder.filter_strategy = (LodePNGFilterStrategy)(variant[1] - '0');
    } else if (variant == "entropy") {
        state.encoder.filter_strategy = LFS_ENTROPY;
    } else if (variant == "brute") {
        state.encoder.filter_strategy = LFS_BRUTE_FORCE;
    } else if (variant == "interlace") {
        state.info_png.interlace_method = 1;
    } else if (variant == "btype0") {
        state.encoder.zlibsettings.btype = 0;
    } else if (variant == "btype1") {
        state.encoder.zlibsettings.btype = 1;
    } else if (variant == "noauto") {
        state.encoder.auto_convert = 0;
    } else if (variant == "win32k") {
        state.encoder.zlibsettings.windowsize = 32768;
        state.encoder.zlibsettings.nicematch = 258;
    } else if (variant == "nolz77") {
        state.encoder.zlibsettings.use_lz77 = 0;
    }
    std::vector<unsigned char> png;
    unsigned error = lodepng::encode(png, image, w, h, state);
    emit(name, error, png);
}

// Must match `synthetic_inputs` in tests/common/synthetic.rs.
static std::vector<std::pair<std::string, std::vector<uint8_t>>> synthetic() {
    std::vector<std::pair<std::string, std::vector<uint8_t>>> v;
    v.push_back({"empty", {}});
    v.push_back({"abc", {'a', 'b', 'c'}});
    std::vector<uint8_t> d;
    lcg_state = 1; d.clear(); for (int i = 0; i < 100000; i++) d.push_back((uint8_t)lcg());
    v.push_back({"random", d});
    lcg_state = 2; d.clear(); for (int i = 0; i < 300000; i++) d.push_back((uint8_t)(lcg() & 3));
    v.push_back({"lowentropy", d});
    d.clear(); for (uint32_t i = 0; i < 200000; i++) d.push_back((uint8_t)((i * i) >> 7));
    v.push_back({"quadratic", d});
    d.assign(1000000, 0);
    v.push_back({"zeros", d});
    static const char* words[] = {"the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ", "ktx ", "texture\n"};
    lcg_state = 3; d.clear();
    while (d.size() < 150000) { const char* w = words[lcg() % 10]; while (*w) d.push_back((uint8_t)*w++); }
    v.push_back({"text", d});
    lcg_state = 4; d.clear();
    for (int i = 0; i < 70000; i++) { uint32_t r = lcg(); if (r % 7 == 0 && d.size() > 300) { size_t back = 1 + r % 300, len = 3 + (r >> 9) % 40; for (size_t k = 0; k < len; k++) d.push_back(d[d.size() - back]); } else d.push_back((uint8_t)(r >> 3)); }
    v.push_back({"backrefs", d});
    return v;
}

int main() {
    const LodePNGColorType CTS[] = {LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY_ALPHA, LCT_GREY_ALPHA,
                                    LCT_RGB, LCT_RGB, LCT_RGBA, LCT_RGBA};
    const unsigned BDS[] = {1, 2, 4, 8, 16, 8, 16, 8, 16, 8, 16};
    const unsigned SIZES[][2] = {{1, 1}, {3, 2}, {17, 9}, {64, 48}, {150, 100}};
    const char* VARIANTS[] = {"sbit", "sbitfull", "srgb", "gama", "f0", "f1", "f2", "f3", "f4", "entropy", "brute",
                              "interlace", "btype0", "btype1", "noauto", "win32k", "nolz77"};
    for (int kind = 0; kind < 8; kind++)
        for (int m = 0; m < 11; m++)
            for (auto& s : SIZES) {
                std::string base = std::string(KINDS[kind]) + "_" + std::to_string(CTS[m]) + "_" + std::to_string(BDS[m]) +
                                   "_" + std::to_string(s[0]) + "x" + std::to_string(s[1]);
                encode_case(base + "_default", kind, CTS[m], BDS[m], s[0], s[1], "default");
                if (s[0] == 17 || s[0] == 64)
                    for (const char* v : VARIANTS) encode_case(base + "_" + v, kind, CTS[m], BDS[m], s[0], s[1], v);
            }
    for (auto& s : synthetic()) {
        for (int variant = 0; variant < 4; variant++) {
            LodePNGCompressSettings settings;
            lodepng_compress_settings_init(&settings);
            if (variant == 1) {
                if (s.second.empty()) continue; // C divides by zero (blocksize 0)
                settings.btype = 1;
            }
            if (variant == 2) { settings.windowsize = 32768; settings.nicematch = 258; }
            if (variant == 3) settings.lazymatching = 0;
            unsigned char* out = 0;
            size_t outsize = 0;
            unsigned error = lodepng_zlib_compress(&out, &outsize, s.second.data(), s.second.size(), &settings);
            std::vector<unsigned char> v(out, out + outsize);
            free(out);
            emit(s.first + "_zlib" + std::to_string(variant), error, v);
        }
    }
    return 0;
}
