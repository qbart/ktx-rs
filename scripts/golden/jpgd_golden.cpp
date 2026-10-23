// Golden-data generator for the jpgd port (dev tooling, not part of the Rust
// library). Must stay in sync with tests/jpgd.rs.
//
// For every input file, its byte flips / truncations and both flag values it
// runs jpeg_decoder over a memory stream in a forked AddressSanitizer child
// and prints "<case>\t<flags>\t<result>": constructor status, size,
// components, begin_decoding status, then every decode() status with a hash
// of all returned scan lines, the error code and total bytes read.
// Build/run via gen_jpgd_golden.sh.
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <sstream>
#include <string>
#include <vector>
#include <sys/wait.h>
#include <unistd.h>
#include "jpgd.h"

extern "C" const char* __asan_default_options() { return "detect_leaks=0:exitcode=99"; }

static uint64_t fnv1a(const uint8_t* p, size_t n, uint64_t h = 0xcbf29ce484222325ULL) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
};

static std::string runCase(const std::vector<uint8_t>& file, uint32_t flags) {
    std::ostringstream o;
    std::vector<uint8_t> buf(file);  // exact size so ASan sees over-reads
    jpgd::jpeg_decoder_mem_stream stream(buf.empty() ? nullptr : buf.data(), (jpgd::uint)buf.size());
    jpgd::jpeg_decoder d(&stream, flags);
    o << "C" << d.get_error_code() << ' ' << d.get_width() << 'x' << d.get_height() << 'x'
      << d.get_num_components();
    if (d.get_error_code() != jpgd::JPGD_SUCCESS) return o.str();
    int b = d.begin_decoding();
    o << " B" << b << ',' << d.get_error_code();
    if (b != jpgd::JPGD_SUCCESS) return o.str();
    o << " bpp" << d.get_bytes_per_pixel();
    uint64_t h = 0xcbf29ce484222325ULL;
    int lines = 0, status = 0;
    for (int i = 0; i < d.get_height() + 2; i++) {
        const void* p = nullptr;
        jpgd::uint len = 0;
        status = d.decode(&p, &len);
        if (status != jpgd::JPGD_SUCCESS) break;
        h = fnv1a((const uint8_t*)p, len, h);
        lines++;
    }
    char hb[32];
    snprintf(hb, sizeof hb, "%016llx", (unsigned long long)h);
    o << " L" << lines << " S" << status << " E" << d.get_error_code() << " T" << d.get_total_bytes_read() << ' '
      << hb;
    return o.str();
}

static void emit(const std::string& name, const std::vector<uint8_t>& file, uint32_t flags) {
    fflush(stdout);
    int fds[2];
    if (pipe(fds) != 0) { perror("pipe"); exit(1); }
    pid_t pid = fork();
    if (pid == 0) {
        close(fds[0]);
        alarm(20);
        std::string s = runCase(file, flags);
        size_t off = 0;
        while (off < s.size()) {
            ssize_t w = write(fds[1], s.data() + off, s.size() - off);
            if (w <= 0) _exit(2);
            off += (size_t)w;
        }
        _exit(0);
    }
    close(fds[1]);
    std::string s;
    char b[4096];
    ssize_t n;
    while ((n = read(fds[0], b, sizeof b)) > 0) s.append(b, (size_t)n);
    close(fds[0]);
    int status = 0;
    waitpid(pid, &status, 0);
    bool ok = WIFEXITED(status) && WEXITSTATUS(status) == 0;
    printf("%s\t%u\t%s\n", name.c_str(), flags, ok ? s.c_str() : "CRASH");
}

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++) {
        FILE* f = fopen(argv[i], "rb");
        if (!f) { perror(argv[i]); return 1; }
        std::vector<uint8_t> d;
        uint8_t b[65536];
        size_t n;
        while ((n = fread(b, 1, sizeof b, f)) > 0) d.insert(d.end(), b, b + n);
        fclose(f);
        std::string name = argv[i];
        for (uint32_t flags = 0; flags < 2; flags++) emit(name, d, flags);
        if (d.empty() || d.size() > 4096) continue;
        // corruptions
        Lcg rng{(uint32_t)fnv1a((const uint8_t*)name.data(), name.size())};
        for (int k = 0; k < 40; k++) {
            std::vector<uint8_t> c = d;
            size_t pos = rng.next() % c.size();
            c[pos] = (uint8_t)rng.next();
            emit(name + "#f" + std::to_string(k), c, k & 1);
        }
        for (int k = 0; k < 6; k++) {
            size_t len = rng.next() % d.size();
            emit(name + "#t" + std::to_string(k), std::vector<uint8_t>(d.begin(), d.begin() + len), k & 1);
        }
    }
    return 0;
}
