// Golden-data generator for UASTC HDR transcoding (dev tooling, not part of
// the Rust library). Mirrors libktx's transcodeUastcHDR4x4 (each level is
// zstd-inflated, then every image is transcoded with
// basisu_lowlevel_uastc_hdr_4x4_transcoder::transcode_image) and
// transcodeUastcHDR6x6_intermediate (every image of a level is located via the
// SGD image descriptions and transcoded with
// basisu_lowlevel_uastc_hdr_6x6_intermediate_transcoder::transcode_image).
// Writes <outdir>/<ktx2>_<format>_<flags>_L<level>.bin (all images of the level).
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

static void write_out(const std::string& outDir, const char* name, uint32_t fmt, uint32_t flags,
                      uint32_t level, bool ok, const std::vector<uint8_t>& out) {
    std::string op = outDir + "/" + name + "_" + std::to_string(fmt) + "_" + std::to_string(flags) +
                     "_L" + std::to_string(level) + ".bin";
    if (!ok) {
        printf("  fmt %u flags %u level %u: transcode failed\n", fmt, flags, level);
        return;
    }
    FILE* f = fopen(op.c_str(), "wb");
    fwrite(out.data(), 1, out.size(), f);
    fclose(f);
}

static int run_6x6i(const std::string& dir, const std::string& outDir) {
    const char* files[] = {"Desk_uastc_hdr6x6i.ktx2"};
    const uint32_t formats[] = {22, 27, 24, 25, 26};
    const uint32_t flagsets[] = {0, cDecodeFlagsHighQuality};
    for (const char* name : files) {
        std::vector<uint8_t> d;
        if (!read_file(dir + "/" + name, d)) {
            fprintf(stderr, "cannot read %s\n", name);
            return 1;
        }
        uint32_t w = rd<uint32_t>(d, 20), h = rd<uint32_t>(d, 24), depth0 = rd<uint32_t>(d, 28);
        uint32_t layers = rd<uint32_t>(d, 32), faces = rd<uint32_t>(d, 36), levels = rd<uint32_t>(d, 40);
        uint64_t sgdOff = rd<uint64_t>(d, 64);
        if (!layers) layers = 1;
        if (!levels) levels = 1;
        if (!depth0) depth0 = 1;
        printf("%s %ux%u levels=%u faces=%u\n", name, w, h, levels, faces);
        uint32_t firstImage = 0;
        for (uint32_t level = 0; level < levels; level++) {
            uint64_t off = rd<uint64_t>(d, 80 + level * 24), len = rd<uint64_t>(d, 88 + level * 24);
            uint32_t lw = w >> level ? w >> level : 1, lh = h >> level ? h >> level : 1;
            uint32_t ld = depth0 >> level ? depth0 >> level : 1;
            uint32_t bx = (lw + 5) / 6, by = (lh + 5) / 6;
            uint32_t images = layers * faces * ld;
            for (uint32_t fmt : formats) {
                transcoder_texture_format tf = (transcoder_texture_format)fmt;
                uint32_t bpb = basis_get_bytes_per_block_or_pixel(tf);
                uint32_t bw = basis_get_block_width(tf), bh = basis_get_block_height(tf);
                bool unc = basis_transcoder_format_is_uncompressed(tf);
                uint32_t outSize = unc ? lw * lh * bpb : ((lw + bw - 1) / bw) * ((lh + bh - 1) / bh) * bpb;
                for (uint32_t flags : flagsets) {
                    std::vector<uint8_t> out((size_t)outSize * images);
                    basisu_lowlevel_uastc_hdr_6x6_intermediate_transcoder uit;
                    basisu_transcoder_state st;
                    bool ok = true;
                    for (uint32_t img = 0; img < images && ok; img++) {
                        size_t desc = sgdOff + (size_t)(firstImage + img) * 12;
                        uint32_t sliceOff = rd<uint32_t>(d, desc), sliceLen = rd<uint32_t>(d, desc + 4);
                        ok = uit.transcode_image(tf, out.data() + (size_t)img * outSize,
                                                 (uint32_t)((out.size() - (size_t)img * outSize) / bpb),
                                                 d.data() + off, (uint32_t)len, bx, by, lw, lh, level,
                                                 sliceOff, sliceLen, flags, false, false, 0, &st, 0, -1, -1);
                    }
                    write_out(outDir, name, fmt, flags, level, ok, out);
                }
            }
            firstImage += images;
        }
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <ktx2 dir> <out dir>\n", argv[0]);
        return 1;
    }
    basisu_transcoder_init();
    std::string dir = argv[1], outDir = argv[2];
    const char* files[] = {"Desk_uastc_hdr4x4_zstd_15.ktx2"};
    const uint32_t formats[] = {22, 23, 24, 25, 26};
    const uint32_t flagsets[] = {0};
    for (const char* name : files) {
        std::vector<uint8_t> d;
        if (!read_file(dir + "/" + name, d)) {
            fprintf(stderr, "cannot read %s\n", name);
            return 1;
        }
        uint32_t w = rd<uint32_t>(d, 20), h = rd<uint32_t>(d, 24), depth0 = rd<uint32_t>(d, 28);
        uint32_t layers = rd<uint32_t>(d, 32), faces = rd<uint32_t>(d, 36), levels = rd<uint32_t>(d, 40);
        uint32_t scheme = rd<uint32_t>(d, 44);
        if (!layers) layers = 1;
        if (!levels) levels = 1;
        if (!depth0) depth0 = 1;
        printf("%s %ux%u levels=%u faces=%u\n", name, w, h, levels, faces);
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
                uint32_t outSize = unc ? lw * lh * bpb : bx * by * bpb;
                for (uint32_t flags : flagsets) {
                    std::vector<uint8_t> out((size_t)outSize * images);
                    basisu_lowlevel_uastc_hdr_4x4_transcoder uit;
                    basisu_transcoder_state st;
                    bool ok = true;
                    for (uint32_t img = 0; img < images && ok; img++) {
                        ok = uit.transcode_image(tf, out.data() + (size_t)img * outSize,
                                                 (uint32_t)((out.size() - (size_t)img * outSize) / bpb),
                                                 lvl.data(), (uint32_t)lvl.size(), bx, by, lw, lh, level,
                                                 img * inSize, inSize, flags, false, false, 0, &st, 0, -1, -1);
                    }
                    write_out(outDir, name, fmt, flags, level, ok, out);
                }
            }
        }
    }
    return run_6x6i(dir, outDir);
}
