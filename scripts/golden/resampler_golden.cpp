// Golden-data generator for the basisu resampler port (dev tooling, not part
// of the Rust library). Must stay in sync with tests/resampler.rs.
//
// For every filter x size pair x boundary op x parameter set it prints
// "<case>\t<result>": status, FNV hashes of the X/Y contributor lists, the
// number of accepted put_line calls, number of output lines and a hash of
// their sample bits. Extra cases cover shared contributor lists, restart()
// and filter lookup. Build/run via gen_resampler_golden.sh.
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>
#include <memory>
#include "basisu_resampler.h"
#include "basisu_resampler_filters.h"

using namespace basisu;

static uint64_t fnv(uint64_t h, const void* p, size_t n) {
    const uint8_t* b = (const uint8_t*)p;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 0x100000001b3ULL; }
    return h;
}
static const uint64_t FNV0 = 0xcbf29ce484222325ULL;

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
    float sample() { return (float)(next() & 0xffff) / 65535.0f * 1.5f - 0.25f; }
};

struct Size { int sx, sy, dx, dy; };
static const Size SIZES[] = {
    {1, 1, 1, 1}, {7, 5, 3, 2}, {5, 7, 13, 11}, {16, 16, 8, 8}, {33, 17, 16, 8},
    {9, 9, 9, 9}, {64, 3, 5, 40}, {2, 2, 1, 1}, {17, 1, 4, 1}, {3, 30, 1, 2},
};
struct Params { float scale, ofs_x, ofs_y, lo, hi; };
static const Params PARAMS[] = {
    {1.0f, 0.0f, 0.0f, 0.0f, 1.0f},
    {0.75f, 0.25f, -0.5f, 0.0f, 0.0f},
    {2.0f, 0.0f, 1.5f, -0.5f, 0.25f},
    {0.1f, 0.0f, 0.0f, 0.0f, 1.0f},
};

static uint64_t hashClist(const Resampler::Contrib_List* l, int n) {
    uint64_t h = FNV0;
    if (!l) return 0;
    for (int i = 0; i < n; i++) {
        h = fnv(h, &l[i].n, 2);
        for (int k = 0; k < l[i].n; k++) {
            h = fnv(h, &l[i].p[k].weight, 4);
            h = fnv(h, &l[i].p[k].pixel, 2);
        }
    }
    return h;
}

// Feeds src lines exactly like ImageT::resample and hashes all output.
static void drive(Resampler& r, int sx, int sy, int dx, uint32_t seed, int& puts, int& gets, uint64_t& h) {
    Lcg lcg{seed};
    std::vector<float> line(sx);
    for (int y = 0; y < sy + 1; y++) {
        for (int x = 0; x < sx; x++) line[x] = lcg.sample();
        if (r.put_line(line.data())) puts++;
        while (const float* o = r.get_line()) {
            h = fnv(h, o, sizeof(float) * dx);
            gets++;
        }
    }
}

int main() {
    const int nf = Resampler::get_filter_num();
    printf("filters\t%d", nf);
    for (int i = -1; i <= nf; i++) printf(" %s", Resampler::get_filter_name(i) ? Resampler::get_filter_name(i) : "(null)");
    printf("\n");
    for (int f = 0; f < nf; f++) {
        const char* name = Resampler::get_filter_name(f);
        for (size_t z = 0; z < sizeof(SIZES) / sizeof(SIZES[0]); z++) {
            const Size& s = SIZES[z];
            for (int b = 0; b < 3; b++) {
                for (size_t p = 0; p < sizeof(PARAMS) / sizeof(PARAMS[0]); p++) {
                    const Params& q = PARAMS[p];
                    Resampler r(s.sx, s.sy, s.dx, s.dy, (Resampler::Boundary_Op)b, q.lo, q.hi, name,
                                nullptr, nullptr, q.scale, q.scale, q.ofs_x, q.ofs_y);
                    printf("f%d z%zu b%d p%zu\tS%d", f, z, b, p, (int)r.status());
                    if (r.status() == Resampler::STATUS_OKAY) {
                        int puts = 0, gets = 0;
                        uint64_t h = FNV0;
                        drive(r, s.sx, s.sy, s.dx, (uint32_t)(f * 1000 + z * 10 + b + p * 3), puts, gets, h);
                        printf(" X%016llx Y%016llx P%d G%d H%016llx",
                               (unsigned long long)hashClist(r.get_clist_x(), s.dx),
                               (unsigned long long)hashClist(r.get_clist_y(), s.dy), puts, gets,
                               (unsigned long long)h);
                    }
                    printf("\n");
                }
            }
        }
    }
    // Shared contributor lists (ImageT::resample with 3 channels) + restart.
    for (size_t z = 0; z < sizeof(SIZES) / sizeof(SIZES[0]); z++) {
        const Size& s = SIZES[z];
        std::unique_ptr<Resampler> rs[3];
        for (int c = 0; c < 3; c++)
            rs[c] = std::make_unique<Resampler>(s.sx, s.sy, s.dx, s.dy, Resampler::BOUNDARY_CLAMP, 0.0f, 1.0f,
                                                "kaiser", c ? rs[0]->get_clist_x() : nullptr,
                                                c ? rs[0]->get_clist_y() : nullptr, 1.0f, 1.0f, 0.0f, 0.0f);
        printf("shared z%zu\t", z);
        for (int round = 0; round < 2; round++) {
            for (int c = 0; c < 3; c++) {
                int puts = 0, gets = 0;
                uint64_t h = FNV0;
                drive(*rs[c], s.sx, s.sy, s.dx, (uint32_t)(z * 7 + c + round * 100), puts, gets, h);
                printf("S%d P%d G%d H%016llx ", (int)rs[c]->status(), puts, gets, (unsigned long long)h);
                rs[c]->restart();
            }
        }
        printf("\n");
    }
    // Filter name lookup / default filter.
    const char* names[] = {"lanczos4", "Lanczos4", "", "box ", "quadratic_mix", "none"};
    for (const char* n : names) {
        Resampler r(4, 4, 2, 2, Resampler::BOUNDARY_CLAMP, 0.0f, 1.0f, n);
        printf("name '%s'\tS%d F%d\n", n, (int)r.status(), find_resample_filter(n));
    }
    {
        Resampler r(6, 5, 3, 9, Resampler::BOUNDARY_REFLECT, 0.0f, 1.0f, nullptr);
        int puts = 0, gets = 0;
        uint64_t h = FNV0;
        drive(r, 6, 5, 3, 77, puts, gets, h);
        printf("default\tS%d P%d G%d H%016llx\n", (int)r.status(), puts, gets, (unsigned long long)h);
    }
    return 0;
}
