// Golden-data generator for the basisu miniz deflate port (dev tooling, not
// part of the Rust library). Compresses synthetic inputs and the given files
// with tdefl at levels 0..10 and every strategy, and with mz_compress2 at
// levels 1..9, printing "<case> <size> <fnv1a64>" per output.
// Build/run via scripts/golden/gen_miniz_golden.sh (x86-64).
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <string>
#include <vector>
#include "encoder/basisu_miniz.h"

using namespace buminiz;

static uint64_t fnv1a(const uint8_t* p, size_t n) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

static uint32_t lcg_state;
static uint32_t lcg() { lcg_state = lcg_state * 1664525u + 1013904223u; return lcg_state >> 8; }

// Must match `synthetic_inputs` in tests/miniz.rs.
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

static void run(const std::string& name, const std::vector<uint8_t>& in) {
    std::vector<uint8_t> out(mz_compressBound(in.size()) + TDEFL_OUT_BUF_SIZE);
    for (int level = 0; level <= 10; level++) {
        for (int strategy = 0; strategy <= 4; strategy++) {
            mz_uint flags = tdefl_create_comp_flags_from_zip_params(level, level % 2 ? 15 : -15, strategy);
            tdefl_compressor* c = (tdefl_compressor*)malloc(sizeof(tdefl_compressor));
            tdefl_init(c, NULL, NULL, (int)flags);
            size_t in_size = in.size(), out_size = out.size();
            tdefl_status st = tdefl_compress(c, in.data(), &in_size, out.data(), &out_size, TDEFL_FINISH);
            free(c);
            printf("%s_tdefl_L%d_S%d %d %zu %016llx\n", name.c_str(), level, strategy, (int)st, out_size,
                   (unsigned long long)fnv1a(out.data(), out_size));
        }
    }
    for (int level = 1; level <= 9; level++) {
        mz_ulong n = mz_compressBound(in.size());
        int st = mz_compress2(out.data(), &n, in.data(), in.size(), level);
        printf("%s_compress2_L%d %d %lu %016llx\n", name.c_str(), level, st, (unsigned long)n,
               (unsigned long long)fnv1a(out.data(), st == MZ_OK ? n : 0));
    }
}

int main(int argc, char** argv) {
    for (auto& s : synthetic()) run(s.first, s.second);
    for (int i = 1; i < argc; i++) {
        FILE* f = fopen(argv[i], "rb");
        if (!f) return 1;
        std::vector<uint8_t> d;
        int ch;
        while ((ch = fgetc(f)) != EOF) d.push_back((uint8_t)ch);
        fclose(f);
        std::string p = argv[i];
        run(p.substr(p.find_last_of('/') + 1), d);
    }
    return 0;
}
