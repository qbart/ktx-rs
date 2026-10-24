// Golden-data generator for the basisu vector quantizer port (src/basisu/vq.rs; dev tooling,
// not part of the Rust library): tree_vector_quant<vec6F / vec16F> generate + retrieve and
// generate_hierarchical_codebook_threaded on generated training sets. Prints
// "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_vq.rs. Build/run via
// scripts/golden/gen_basisu_vq_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <vector>
#include "basisu_enc.h"

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
    void u(uint32_t v) { bytes(&v, 4); }
    void cb(const basisu::vector<uint_vec>& c) {
        u((uint32_t)c.size());
        for (const auto& v : c) { u((uint32_t)v.size()); for (uint32_t x : v) u(x); }
    }
};

// Component value for training vector i, component c of set kind k.
template <uint32_t N>
static void make_vec(uint32_t k, uint32_t i, Lcg& l, vec<N, float>& v) {
    const uint32_t cluster = l.next() % (3 + k * 5);
    for (uint32_t c = 0; c < N; c++) {
        float x;
        switch (k % 6) {
        case 0: x = (float)((l.next() >> 8) & 255) / 255.0f; break;
        case 1: x = (float)((cluster * 37 + c * 11) & 255) / 255.0f + (float)(l.next() % 9) / 255.0f; break;
        case 2: x = (float)(l.next() % 4) / 3.0f; break;
        case 3: x = (i & 1) ? -0.0f : 0.0f; if (c == 0 && cluster == 0) x = 0.5f; break;
        case 4: x = (float)((cluster * 13 + c) % 7) * 0.25f; break;
        default: x = 0.5f + (float)(l.next() % 3) * 1e-7f * (float)(c + 1); break;
        }
        v[c] = x;
    }
}

template <uint32_t N>
static uint64_t run_case(uint32_t k, uint32_t n, uint32_t max_cb, uint32_t max_parent, uint32_t threads, bool even_odd, uint32_t wmode, job_pool* pool) {
    Lcg l{k * 7919u + n * 31u + max_cb};
    tree_vector_quant<vec<N, float>> q;
    for (uint32_t i = 0; i < n; i++) {
        vec<N, float> v;
        make_vec<N>(k, even_odd ? (i & ~1u) : i, l, v);
        uint64_t w = wmode == 0 ? 1 : (wmode == 1 ? 1 + (l.next() % 1000) : (uint64_t)(l.next() % 3) + 1);
        q.add_training_vec(v, w);
        if (even_odd) { q.add_training_vec(v, w + (i & 3)); i++; }
    }
    H h;
    basisu::vector<uint_vec> cb, pcb;
    bool ok = generate_hierarchical_codebook_threaded(q, max_cb, max_parent, cb, pcb, threads, pool, even_odd);
    h.bytes(&ok, 1);
    h.cb(cb);
    h.cb(pcb);
    // Direct generate / retrieve on the raw training set
    tree_vector_quant<vec<N, float>> q2;
    for (const auto& t : q.get_training_vecs()) q2.add_training_vec(t.first, t.second);
    ok = q2.generate(max_cb);
    h.bytes(&ok, 1);
    basisu::vector<uint_vec> c2, c3;
    q2.retrieve(c2);
    h.cb(c2);
    q2.retrieve(max_parent ? max_parent : 5, c3);
    h.cb(c3);
    basisu::vector<vec<N, float>> org;
    q2.retrieve(org);
    u_int32_t no = (uint32_t)org.size();
    h.u(no);
    for (const auto& o : org) h.bytes(&o, sizeof(float) * N);
    return h.h;
}

int main() {
    g_cpu_supports_sse41 = true;
    job_pool pool(4);
    static const uint32_t NS[] = {1, 2, 3, 17, 100, 600, 3000};
    static const uint32_t CBS[] = {1, 2, 7, 64, 256};
    uint32_t idx = 0;
    for (uint32_t k = 0; k < 12; k++)
        for (uint32_t ni = 0; ni < 7; ni++)
            for (uint32_t ci = 0; ci < 5; ci++) {
                const uint32_t n = NS[ni], cbs = CBS[ci];
                const bool eo = ((k + ci) % 3) == 2 && n >= 2;
                const uint32_t parent = (ci & 1) ? cbs / 2 + 1 : 0;
                const uint32_t wmode = (k + ni) % 3;
                uint64_t h6 = run_case<6>(k, n, cbs, parent, 1, eo, wmode, &pool);
                uint64_t h16 = run_case<16>(k, n, cbs, parent, 1, eo, wmode, &pool);
                printf("%u\t%016llx %016llx\n", idx++, (unsigned long long)h6, (unsigned long long)h16);
            }
    // Multi threaded path (>= 65536 * 4 unique vectors)
    for (uint32_t t = 0; t < 2; t++) {
        uint64_t hh = run_case<6>(0, 270000, 600, t ? 100 : 0, 4 + t * 3, false, 1, &pool);
        printf("mt%u\t%016llx\n", t, (unsigned long long)hh);
    }
    return 0;
}
