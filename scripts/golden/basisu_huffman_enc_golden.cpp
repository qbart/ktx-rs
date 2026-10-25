// Golden-data generator for the basisu encoder Huffman port (src/basisu/huffman_enc.rs; dev
// tooling, not part of the Rust library): huffman_encoding_table::init + emit_huffman_table,
// bitwise_coder ops and palette_index_reorderer on generated inputs. Prints
// "<case>\t<fnv1a64>". Must stay in sync with tests/basisu_huffman_enc.rs. Build/run via
// scripts/golden/gen_basisu_huffman_enc_golden.sh.
#include <cstdio>
#include <cstdint>
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
    void q(uint64_t v) { bytes(&v, 8); }
};

static const uint32_t SIZES[] = {1, 2, 3, 5, 21, 64, 257, 1000, 4096, 16383, 16385};

static void make_hist(uint32_t c, histogram& h) {
    Lcg l{c * 7919u + 17u};
    const uint32_t n = SIZES[c % 11];
    h.init(n);
    const uint32_t kind = (c / 11) % 8;
    for (uint32_t i = 0; i < n; i++) {
        uint32_t v;
        switch (kind) {
        case 0: v = l.next() % 50; break;
        case 1: v = 1 + (100000 / (i + 1)) % 60000; break;
        case 2: { uint32_t a = 1, b = 1; for (uint32_t k = 0; k < (i % 25); k++) { uint32_t t = a + b; a = b; b = t; } v = a; break; }
        case 3: v = ((l.next() % 7) == 0) ? 1 + l.next() % 300 : 0; break;
        case 4: v = (l.next() % 3 == 0) ? 70000 + l.next() % 5000000 : l.next() % 4; break;
        case 5: v = (i == (c % n)) ? 5 : 0; break;
        case 6: v = 0; break;
        default: v = (i == 0) ? 65535 : (((i / 7) & 1) ? 0 : 1 + ((i / 3) % 4) * 1000); break;
        }
        h[i] = v;
    }
}

static uint64_t huff_case(uint32_t c) {
    histogram hist;
    make_hist(c, hist);
    H h;
    for (uint32_t mcs = 7; mcs <= 17; mcs += ((c & 1) ? 3 : 1)) {
        huffman_encoding_table t;
        bool ok = t.init(hist, mcs);
        h.u(ok);
        if (!ok) continue;
        h.u(t.get_total_used_codes());
        h.bytes(t.get_code_sizes().data(), t.get_code_sizes().size());
        h.bytes(t.get_codes().data(), t.get_codes().size() * 2);
        bitwise_coder coder;
        coder.init(16);
        coder.put_bits((c * 5) & ((1u << (c % 8)) - 1), (c % 8));
        uint32_t bits = coder.emit_huffman_table(t);
        h.u(bits);
        for (uint32_t i = 0; i < hist.size() && i < 3000; i++)
            if (t.get_code_sizes()[i]) h.u(coder.put_code(i, t));
        h.u(coder.flush());
        h.q(coder.get_total_bits());
        h.bytes(coder.get_bytes().data(), coder.get_bytes().size());
    }
    return h.h;
}

static uint64_t coder_case(uint32_t c) {
    Lcg l{c * 31u + 5u};
    H h;
    bitwise_coder a, b;
    a.init(4);
    b.init(4);
    histogram hist(40);
    for (uint32_t i = 0; i < 40; i++) hist[i] = 1 + l.next() % 90;
    huffman_encoding_table t;
    t.init(hist, 12);
    const uint32_t nops = 20 + c * 7;
    for (uint32_t i = 0; i < nops; i++) {
        bitwise_coder& d = (l.next() & 3) ? a : b;
        uint32_t op = l.next() % 7, r;
        switch (op) {
        case 0: { uint32_t nb = l.next() % 33; uint32_t v = nb == 32 ? (l.next() << 8) ^ l.next() : (l.next() & ((1u << nb) - 1)); r = d.put_bits(v, nb); break; }
        case 1: { uint32_t m = 1 + l.next() % 8; uint32_t v = l.next() % (65u << m); r = d.put_rice(v, m); break; }
        case 2: { uint32_t cb = 1 + l.next() % 8; uint32_t v = l.next() >> (l.next() % 24); r = d.put_vlc(v, cb); break; }
        case 3: { uint32_t n = 2 + l.next() % 1000; r = d.put_truncated_binary(l.next() % n, n); break; }
        case 4: r = d.put_code(l.next() % 40, t); break;
        case 5: r = d.flush(); break;
        default: a.append(b); r = (uint32_t)a.get_total_bits(); break;
        }
        h.u(r);
    }
    h.q(a.get_total_bits());
    h.q(b.get_total_bits());
    a.flush();
    b.flush();
    h.bytes(a.get_bytes().data(), a.get_bytes().size());
    h.bytes(b.get_bytes().data(), b.get_bytes().size());
    return h.h;
}

static float dist_func(uint32_t i, uint32_t j, void*) {
    return (float)((i * 7 + j * 13) % 101) / 100.0f;
}

static uint64_t reorder_case(uint32_t c) {
    Lcg l{c * 131u + 3u};
    const uint32_t num_syms = 1 + l.next() % ((c % 3 == 0) ? 8 : 300);
    const uint32_t num_indices = (c % 17 == 0) ? 1 : 1 + l.next() % 3000;
    const uint32_t kind = c % 4;
    uint_vec idx(num_indices);
    uint32_t cur = l.next() % num_syms;
    for (uint32_t i = 0; i < num_indices; i++) {
        switch (kind) {
        case 0: cur = l.next() % num_syms; break;
        case 1: if (l.next() % 5 == 0) cur = l.next() % num_syms; break;
        case 2: cur = (cur + num_syms + (l.next() % 5) - 2) % num_syms; break;
        default: cur = (l.next() % 4 == 0) ? l.next() % num_syms : (i * 3) % num_syms; break;
        }
        idx[i] = cur;
    }
    H h;
    for (uint32_t m = 0; m < 3; m++) {
        palette_index_reorderer r;
        float w = m == 0 ? 0.0f : (m == 1 ? 0.5f : 1.0f);
        r.init(num_indices, idx.data(), num_syms, m ? dist_func : nullptr, nullptr, w);
        const uint_vec& t = r.get_remap_table();
        h.u((uint32_t)t.size());
        for (uint32_t x : t) h.u(x);
    }
    return h.h;
}

int main() {
    for (uint32_t c = 0; c < 88 * 3; c++) printf("h%u\t%016llx\n", c, (unsigned long long)huff_case(c));
    for (uint32_t c = 0; c < 200; c++) printf("c%u\t%016llx\n", c, (unsigned long long)coder_case(c));
    for (uint32_t c = 0; c < 120; c++) printf("r%u\t%016llx\n", c, (unsigned long long)reorder_case(c));
    return 0;
}
