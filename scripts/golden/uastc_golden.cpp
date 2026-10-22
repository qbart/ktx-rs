// Golden-data generator for UASTC LDR 4x4 transcoding (dev tooling, not part of
// the Rust library). Mirrors libktx's transcodeUastcLDR4x4: each level is
// zstd-inflated, then every image of every level is transcoded with
// basisu_lowlevel_uastc_ldr_4x4_transcoder::transcode_image. Writes
// <outdir>/<ktx2>_<format>_<flags>_L<level>.bin (all images of the level).
//
// Build/run via scripts/golden/gen_transcode_golden.sh.
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include "transcoder/basisu_transcoder.h"
#include "zstd.h"

using namespace basist;

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

template <typename T> static T rd(const std::vector<uint8_t>& d, size_t o) {
    T v;
    memcpy(&v, d.data() + o, sizeof(T));
    return v;
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <ktx2 dir> <out dir>\n", argv[0]);
        return 1;
    }
    basisu_transcoder_init();
    std::string dir = argv[1], outDir = argv[2];
    const char* files[] = {"color_grid_uastc_zstd_5.ktx2", "ktx_document_uastc_rdo_4_zstd_5.ktx2",
                           "Iron_Bars_001_normal_uastc_zstd_10.ktx2",
                           "cubemap_goldengate_uastc_rdo_4_zstd_5.ktx2"};
    const uint32_t formats[] = {0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 18, 19, 20, 21, 13, 14, 15, 16};
    const uint32_t flagsets[] = {0, cDecodeFlagsTranscodeAlphaDataToOpaqueFormats, cDecodeFlagsHighQuality};
    for (const char* name : files) {
        std::vector<uint8_t> d;
        if (!read_file(dir + "/" + name, d)) {
            fprintf(stderr, "cannot read %s\n", name);
            return 1;
        }
        uint32_t w = rd<uint32_t>(d, 20), h = rd<uint32_t>(d, 24), depth0 = rd<uint32_t>(d, 28);
        uint32_t layers = rd<uint32_t>(d, 32), faces = rd<uint32_t>(d, 36), levels = rd<uint32_t>(d, 40);
        uint32_t scheme = rd<uint32_t>(d, 44), dfdOff = rd<uint32_t>(d, 48);
        if (!layers) layers = 1;
        if (!levels) levels = 1;
        if (!depth0) depth0 = 1;
        uint32_t chan = d[dfdOff + 4 + 24 + 3] & 0xF;
        bool hasAlpha = chan == 3 || chan == 5;
        printf("%s %ux%u levels=%u faces=%u alpha=%d\n", name, w, h, levels, faces, hasAlpha);
        for (uint32_t level = 0; level < levels; level++) {
            uint64_t off = rd<uint64_t>(d, 80 + level * 24), len = rd<uint64_t>(d, 88 + level * 24);
            uint64_t ulen = rd<uint64_t>(d, 96 + level * 24);
            std::vector<uint8_t> lvl(ulen);
            if (scheme == 2) {
                size_t r = ZSTD_decompress(lvl.data(), ulen, d.data() + off, len);
                if (ZSTD_isError(r) || r != ulen) { fprintf(stderr, "zstd failed\n"); return 1; }
            } else {
                memcpy(lvl.data(), d.data() + off, ulen);
            }
            uint32_t lw = w >> level ? w >> level : 1, lh = h >> level ? h >> level : 1;
            uint32_t ld = depth0 >> level ? depth0 >> level : 1;
            uint32_t bx = (lw + 3) / 4, by = (lh + 3) / 4;
            uint32_t images = layers * faces * ld;
            uint32_t inSize = bx * by * 16;
            for (uint32_t fmt : formats) {
                transcoder_texture_format tf = (transcoder_texture_format)fmt;
                uint32_t bpb = basis_get_bytes_per_block_or_pixel(tf);
                bool unc = basis_transcoder_format_is_uncompressed(tf);
                // libktx sizes PVRTC1 images with a 2x2 block minimum.
                bool pvrtc1 = fmt == 8 || fmt == 9;
                uint32_t kbx = pvrtc1 && bx < 2 ? 2 : bx, kby = pvrtc1 && by < 2 ? 2 : by;
                uint32_t outSize = unc ? lw * lh * bpb : kbx * kby * bpb;
                for (uint32_t flags : flagsets) {
                    std::vector<uint8_t> out((size_t)outSize * images);
                    basisu_lowlevel_uastc_ldr_4x4_transcoder uit;
                    basisu_transcoder_state st;
                    bool ok = true;
                    for (uint32_t img = 0; img < images && ok; img++) {
                        ok = uit.transcode_image(tf, out.data() + (size_t)img * outSize,
                                                 (uint32_t)((out.size() - (size_t)img * outSize) / bpb),
                                                 lvl.data(), (uint32_t)lvl.size(), bx, by, lw, lh, level,
                                                 img * inSize, inSize, flags, hasAlpha, false, 0, &st, 0, -1, -1);
                    }
                    std::string op = outDir + "/" + name + "_" + std::to_string(fmt) + "_" +
                                     std::to_string(flags) + "_L" + std::to_string(level) + ".bin";
                    if (!ok) {
                        printf("  fmt %u flags %u level %u: transcode failed\n", fmt, flags, level);
                        continue;
                    }
                    FILE* f = fopen(op.c_str(), "wb");
                    fwrite(out.data(), 1, out.size(), f);
                    fclose(f);
                }
            }
        }
    }
    return 0;
}
