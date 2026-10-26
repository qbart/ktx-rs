// Golden-data generator for tests/transcodetests.rs (dev tooling, not part of
// the Rust library). Transcodes the reference .basis files with the original
// Basis Universal transcoder exactly like tests/transcodetests/transcodetests.cc
// and writes each result to <outdir>/<basis>_<format>.bin.
//
// Build/run via scripts/golden/gen_transcode_golden.sh.
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>
#include "basisu_c_binding.h"
#include "transcoder/basisu_transcoder.h"

static bool read_file(const std::string& path, std::vector<uint8_t>& out) {
    FILE* f = fopen(path.c_str(), "rb");
    if (!f) return false;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    out.resize(n);
    size_t r = fread(out.data(), 1, n, f);
    fclose(f);
    return r == (size_t)n;
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <basis dir> <out dir>\n", argv[0]);
        return 1;
    }
    basist::basisu_transcoder_init();
    std::string basisDir = argv[1], outDir = argv[2];
    const char* files[] = {"color_grid.basis", "kodim17.basis", "alpha_simple.basis"};
    // 11, 12 (ATC) and 17 (FXT1, compiled out like libktx) are only used by tests/transcode_atc.rs.
    const uint32_t formats[] = {0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 18, 19, 21, 13, 14, 15, 16, 11, 12, 17};
    for (const char* name : files) {
        std::vector<uint8_t> data;
        if (!read_file(basisDir + "/" + name, data)) {
            fprintf(stderr, "cannot read %s\n", name);
            return 1;
        }
        basis_file basisu;
        basisu.open(data.data(), (uint32_t)data.size());
        printf("%s %u %u alpha=%u\n", name, basisu.getImageWidth(0, 0), basisu.getImageHeight(0, 0),
               basisu.getHasAlpha());
        for (uint32_t fmt : formats) {
            uint32_t size = basisu.getImageTranscodedSizeInBytes(0, 0, fmt);
            std::vector<uint8_t> out(size);
            basisu.startTranscoding();
            uint32_t ok = basisu.transcodeImage(out.data(), size, 0, 0, fmt, 0, 0);
            std::string op = outDir + "/" + name + "_" + std::to_string(fmt) + ".bin";
            if (!ok) {
                printf("  fmt %u: transcode failed\n", fmt);
                continue;
            }
            FILE* f = fopen(op.c_str(), "wb");
            fwrite(out.data(), 1, size, f);
            fclose(f);
        }
        basisu.close();
    }
    return 0;
}
