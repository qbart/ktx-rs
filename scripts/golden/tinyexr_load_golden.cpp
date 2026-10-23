// Golden-data generator for the tinyexr reader port (dev tooling, not part of
// the Rust library). Must stay in sync with tests/tinyexr_load.rs.
//
//   tinyexr_load_golden gen <corpus>            writes synthetic EXR files made
//                                                with SaveEXRImageToMemory
//   tinyexr_load_golden run <corpus> <files...> prints one line per case
//
// Every case runs ParseEXRVersionFromMemory, ParseEXRHeaderFromMemory and
// LoadEXRImageFromMemory in a forked child built with AddressSanitizer; a
// child that crashes, throws or touches memory outside its buffers prints
// "CRASH" (undefined behavior in C, only checked for "no panic" in Rust).
// tinyexr's malloc is mapped to calloc so untouched pixels are zero.
// Build/run via scripts/golden/gen_tinyexr_load_golden.sh.
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <limits>
#include <set>
#include <sstream>
#include <string>
#include <vector>
#include <sys/wait.h>
#include <unistd.h>
#include "encoder/basisu_miniz.h"
#define malloc(n) calloc(1, (n))
#define TINYEXR_IMPLEMENTATION
#include "tinyexr.h"
#undef malloc

extern "C" const char* __asan_default_options() { return "detect_leaks=0:exitcode=99"; }

static uint64_t fnv1a(const uint8_t* p, size_t n, uint64_t h = 0xcbf29ce484222325ULL) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
};

// ---------------------------------------------------------------- corpus --

struct Entry {
    std::string name;
    std::vector<uint8_t> data;
};

static void put_u32(std::vector<uint8_t>& v, uint32_t x) {
    for (int i = 0; i < 4; i++) v.push_back((uint8_t)(x >> (8 * i)));
}

static uint32_t get_u32(const std::vector<uint8_t>& v, size_t p) {
    return (uint32_t)v[p] | ((uint32_t)v[p + 1] << 8) | ((uint32_t)v[p + 2] << 16) | ((uint32_t)v[p + 3] << 24);
}

static std::vector<Entry> read_corpus(const char* path) {
    std::vector<Entry> out;
    FILE* f = fopen(path, "rb");
    if (!f) { perror(path); exit(1); }
    std::vector<uint8_t> all;
    uint8_t buf[65536];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) all.insert(all.end(), buf, buf + n);
    fclose(f);
    size_t p = 0;
    while (p + 4 <= all.size()) {
        uint32_t nl = get_u32(all, p); p += 4;
        std::string name(all.begin() + p, all.begin() + p + nl); p += nl;
        uint32_t dl = get_u32(all, p); p += 4;
        out.push_back({name, std::vector<uint8_t>(all.begin() + p, all.begin() + p + dl)});
        p += dl;
    }
    return out;
}

// Pixel value of channel c at pixel i for `pattern` (0 random, 1 smooth, 2 zero,
// 3 low entropy noise: tinyexr's PIZ writer overflows on random data).
static uint32_t value(Lcg& rng, int pixel_type, int pattern, int c, int x, int y) {
    uint32_t r = rng.next();
    if (pattern == 2) return 0;
    if (pattern == 3) {
        if (pixel_type == TINYEXR_PIXELTYPE_HALF) return 0x3800u | (r & 0x10f);
        if (pixel_type == TINYEXR_PIXELTYPE_FLOAT) return 0x3f800000u | ((r & 0x1f) << 18);
        return r & 0x10f;
    }
    if (pattern == 1) {
        uint32_t v = (uint32_t)(x * 3 + y * 5 + c * 7);
        if (pixel_type == TINYEXR_PIXELTYPE_HALF) return 0x3c00u + (v & 0x3ff);
        if (pixel_type == TINYEXR_PIXELTYPE_FLOAT) return 0x3f800000u + (v << 12);
        return v;
    }
    if (pixel_type == TINYEXR_PIXELTYPE_HALF) return r & 0xffff;
    if (pixel_type == TINYEXR_PIXELTYPE_FLOAT) return (r * 2654435761u) & 0xbfffffffu;
    return r * 2654435761u;
}

struct Image {
    std::vector<std::vector<uint8_t>> chans;  // bytes per channel
};

static Image make_image(Lcg& rng, const std::vector<int>& types, int pattern, int w, int h, int stride) {
    Image im;
    im.chans.resize(types.size());
    for (size_t c = 0; c < types.size(); c++) {
        int es = types[c] == TINYEXR_PIXELTYPE_HALF ? 2 : 4;
        im.chans[c].assign((size_t)stride * h * es, 0);
    }
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++)
            for (size_t c = 0; c < types.size(); c++) {
                uint32_t v = value(rng, types[c], pattern, (int)c, x, y);
                int es = types[c] == TINYEXR_PIXELTYPE_HALF ? 2 : 4;
                memcpy(&im.chans[c][((size_t)y * stride + x) * es], &v, es);
            }
    return im;
}

static const char* NAMES4[] = {"A", "B", "G", "R"};

static bool save(EXRImage* image, EXRHeader* header, std::vector<uint8_t>& out) {
    unsigned char* mem = nullptr;
    const char* err = nullptr;
    size_t n = SaveEXRImageToMemory(image, header, &mem, &err);
    if (n == 0 || n > (size_t)1 << 30) {
        fprintf(stderr, "save failed: %s (%dx%d tiles %d)\n", err ? err : "?", image->width, image->height, image->num_tiles);
        return false;
    }
    out.assign(mem, mem + n);
    free(mem);
    return true;
}

static void setup_header(EXRHeader& header, const std::vector<int>& types, std::vector<EXRChannelInfo>& chans,
                         std::vector<int>& pt, std::vector<int>& rt, int compression) {
    InitEXRHeader(&header);
    chans.assign(types.size(), EXRChannelInfo());
    pt = types;
    rt = types;
    for (size_t c = 0; c < types.size(); c++) {
        const char* nm = types.size() == 1 ? "Y" : NAMES4[c % 4];
        strncpy(chans[c].name, nm, 255);
        if (types.size() > 4) snprintf(chans[c].name, 255, "layer%zu.%s", c / 4, NAMES4[c % 4]);
    }
    header.num_channels = (int)types.size();
    header.channels = chans.data();
    header.pixel_types = pt.data();
    header.requested_pixel_types = rt.data();
    header.compression_type = compression;
}

static bool gen_scanline(const std::vector<int>& types, int compression, int w, int h, int pattern, uint32_t seed,
                         std::vector<uint8_t>& out) {
    Lcg rng{seed};
    Image im = make_image(rng, types, pattern, w, h, w);
    std::vector<unsigned char*> ptrs;
    for (auto& c : im.chans) ptrs.push_back(c.data());
    EXRImage image;
    InitEXRImage(&image);
    image.images = ptrs.data();
    image.width = w;
    image.height = h;
    image.num_channels = (int)types.size();
    EXRHeader header;
    std::vector<EXRChannelInfo> chans;
    std::vector<int> pt, rt;
    setup_header(header, types, chans, pt, rt, compression);
    return save(&image, &header, out);
}

static int level_size(int top, int level, int round_up) {
    int b = 1 << level;
    int s = top / b;
    if (round_up && s * b < top) s++;
    return std::max(s, 1);
}

static int round_log2(int x, int round_up) {
    int y = 0, r = 0;
    unsigned u = (unsigned)x;
    while (u > 1) { if (u & 1) r = 1; y++; u >>= 1; }
    return round_up ? y + r : y;
}

static bool gen_tiled(const std::vector<int>& types, int compression, int w, int h, int tsx, int tsy, int mode,
                      int round_up, int pattern, uint32_t seed, std::vector<uint8_t>& out) {
    Lcg rng{seed};
    int nxl = 1, nyl = 1;
    if (mode == 1) nxl = nyl = round_log2(std::max(w, h), round_up) + 1;
    if (mode == 2) { nxl = round_log2(w, round_up) + 1; nyl = round_log2(h, round_up) + 1; }
    std::vector<std::pair<int, int>> levels;
    if (mode != 2)
        for (int l = 0; l < nxl; l++) levels.push_back({l, l});
    else
        for (int ly = 0; ly < nyl; ly++)
            for (int lx = 0; lx < nxl; lx++) levels.push_back({lx, ly});

    std::vector<EXRImage> imgs(levels.size());
    std::vector<std::vector<EXRTile>> tiles(levels.size());
    std::vector<Image> store;
    store.reserve(4096);
    std::vector<std::vector<unsigned char*>> ptrs;
    ptrs.reserve(4096);
    for (size_t li = 0; li < levels.size(); li++) {
        int lw = level_size(w, levels[li].first, round_up);
        int lh = level_size(h, levels[li].second, round_up);
        int ntx = (lw + tsx - 1) / tsx, nty = (lh + tsy - 1) / tsy;
        for (int ty = 0; ty < nty; ty++)
            for (int tx = 0; tx < ntx; tx++) {
                EXRTile t;
                memset(&t, 0, sizeof t);
                t.offset_x = tx;
                t.offset_y = ty;
                t.level_x = levels[li].first;
                t.level_y = levels[li].second;
                t.width = std::min(tsx, lw - tx * tsx);
                t.height = std::min(tsy, lh - ty * tsy);
                store.push_back(make_image(rng, types, pattern, t.width, t.height, tsx));
                ptrs.emplace_back();
                for (auto& c : store.back().chans) ptrs.back().push_back(c.data());
                t.images = ptrs.back().data();
                tiles[li].push_back(t);
            }
        InitEXRImage(&imgs[li]);
        imgs[li].tiles = tiles[li].data();
        imgs[li].num_tiles = (int)tiles[li].size();
        imgs[li].width = lw;
        imgs[li].height = lh;
        imgs[li].level_x = levels[li].first;
        imgs[li].level_y = levels[li].second;
        imgs[li].num_channels = (int)types.size();
        if (li > 0) imgs[li - 1].next_level = &imgs[li];
    }
    EXRHeader header;
    std::vector<EXRChannelInfo> chans;
    std::vector<int> pt, rt;
    setup_header(header, types, chans, pt, rt, compression);
    header.tiled = 1;
    header.tile_size_x = tsx;
    header.tile_size_y = tsy;
    header.tile_level_mode = mode;
    header.tile_rounding_mode = round_up;
    header.data_window.max_x = header.display_window.max_x = w - 1;
    header.data_window.max_y = header.display_window.max_y = h - 1;
    return save(&imgs[0], &header, out);
}

// Offset of the first byte after the header (the offset table).
static size_t header_end(const std::vector<uint8_t>& d) {
    size_t p = 8;
    while (p < d.size() && d[p] != 0) {
        p += strlen((const char*)&d[p]) + 1;
        p += strlen((const char*)&d[p]) + 1;
        p += 4 + get_u32(d, p);
    }
    return p + 1;
}

static void set_line_order(std::vector<uint8_t>& d, uint8_t v) {
    const char key[] = "lineOrder\0lineOrder";
    auto it = std::search(d.begin(), d.end(), key, key + sizeof key);
    if (it != d.end()) *(it + sizeof key + 4) = v;
}

static int gen(const char* path) {
    std::vector<Entry> out;
    const std::vector<std::vector<int>> type_sets = {
        {TINYEXR_PIXELTYPE_HALF}, {TINYEXR_PIXELTYPE_FLOAT}, {TINYEXR_PIXELTYPE_UINT},
        {TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF},
        {TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_UINT, TINYEXR_PIXELTYPE_HALF,
         TINYEXR_PIXELTYPE_FLOAT}};
    const int sizes[][2] = {{1, 1}, {5, 3}, {23, 19}, {3, 40}, {40, 2}};
    int idx = 0;
    for (size_t t = 0; t < type_sets.size(); t++)
        for (int comp = 0; comp <= 4; comp++)
            for (auto& s : sizes) {
                int pattern = idx % 3;
                if (comp == TINYEXR_COMPRESSIONTYPE_PIZ && pattern == 0) pattern = 3;
                std::vector<uint8_t> d;
                if (!gen_scanline(type_sets[t], comp, s[0], s[1], pattern, 1000u + idx, d)) return 1;
                char name[128];
                snprintf(name, sizeof name, "s-t%zu-c%d-%dx%d-p%d", t, comp, s[0], s[1], pattern);
                out.push_back({name, d});
                std::vector<uint8_t> lo = d;
                set_line_order(lo, 1);
                out.push_back({std::string(name) + "-lo1", lo});
                if (idx % 4 == 0) {
                    // zero the first offset: the reader reconstructs the table
                    std::vector<uint8_t> z = d;
                    size_t he = header_end(z);
                    if (he + 8 <= z.size()) memset(&z[he], 0, 8);
                    out.push_back({std::string(name) + "-z", z});
                }
                idx++;
            }
    const std::vector<std::vector<int>> tile_types = {
        {TINYEXR_PIXELTYPE_HALF},
        {TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_UINT},
        {TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF, TINYEXR_PIXELTYPE_HALF}};
    const int tsz[][4] = {{20, 13, 8, 8}, {33, 17, 16, 4}, {5, 7, 5, 7}};
    const int modes[][2] = {{0, 0}, {1, 0}, {1, 1}, {2, 0}, {2, 1}};
    for (size_t t = 0; t < tile_types.size(); t++)
        for (int comp = 0; comp <= 4; comp++)
            for (auto& m : modes)
                for (auto& s : tsz) {
                    int pattern = idx % 3;
                    if (comp == TINYEXR_COMPRESSIONTYPE_PIZ && pattern == 0) pattern = 3;
                    std::vector<uint8_t> d;
                    if (!gen_tiled(tile_types[t], comp, s[0], s[1], s[2], s[3], m[0], m[1], pattern, 1000u + idx, d))
                        return 1;
                    char name[128];
                    snprintf(name, sizeof name, "t-t%zu-c%d-m%d%d-%dx%d-%dx%d-p%d", t, comp, m[0], m[1], s[0], s[1],
                             s[2], s[3], pattern);
                    out.push_back({name, d});
                    if (idx % 4 == 0) {
                        std::vector<uint8_t> lo = d;
                        set_line_order(lo, 1);
                        out.push_back({std::string(name) + "-lo1", lo});
                        std::vector<uint8_t> z = d;
                        size_t he = header_end(z);
                        if (he + 8 <= z.size()) memset(&z[he], 0, 8);
                        out.push_back({std::string(name) + "-z", z});
                    }
                    idx++;
                }
    std::vector<uint8_t> all;
    for (auto& e : out) {
        put_u32(all, (uint32_t)e.name.size());
        all.insert(all.end(), e.name.begin(), e.name.end());
        put_u32(all, (uint32_t)e.data.size());
        all.insert(all.end(), e.data.begin(), e.data.end());
    }
    FILE* f = fopen(path, "wb");
    if (!f) { perror(path); return 1; }
    fwrite(all.data(), 1, all.size(), f);
    fclose(f);
    fprintf(stderr, "%zu files, %zu bytes\n", out.size(), all.size());
    return 0;
}

// ------------------------------------------------------------------- run --

static std::string q(const char* s, size_t n) {
    std::string o;
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c == '\n') o += "\\n";
        else if (c == '\\') o += "\\\\";
        else if (c >= 0x20 && c < 0x7f) o += (char)c;
        else { char b[8]; snprintf(b, sizeof b, "\\x%02x", c); o += b; }
    }
    return o;
}

static std::string qs(const char* s) { return s ? q(s, strlen(s)) : std::string("-"); }

static uint32_t fbits(float f) { uint32_t u; memcpy(&u, &f, 4); return u; }

static std::string header_summary(const EXRHeader& h) {
    std::ostringstream o;
    char b[64];
    o << "dw:" << h.data_window.min_x << ',' << h.data_window.min_y << ',' << h.data_window.max_x << ','
      << h.data_window.max_y;
    o << " dp:" << h.display_window.min_x << ',' << h.display_window.min_y << ',' << h.display_window.max_x << ','
      << h.display_window.max_y;
    o << " lo:" << h.line_order << " c:" << h.compression_type << " t:" << h.tiled << ',' << h.tile_size_x << ','
      << h.tile_size_y << ',' << h.tile_level_mode << ',' << h.tile_rounding_mode;
    o << " cc:" << h.chunk_count << " hl:" << h.header_len;
    snprintf(b, sizeof b, " f:%08x,%08x,%08x,%08x", fbits(h.pixel_aspect_ratio), fbits(h.screen_window_center[0]),
             fbits(h.screen_window_center[1]), fbits(h.screen_window_width));
    o << b;
    o << " v:" << h.long_name << ',' << h.non_image << ',' << h.multipart << " n:" << qs(h.name);
    o << " ch:" << h.num_channels;
    for (int c = 0; c < h.num_channels; c++)
        o << ' ' << qs(h.channels[c].name) << ':' << h.channels[c].pixel_type << ':' << h.channels[c].x_sampling << ':'
          << h.channels[c].y_sampling << ':' << (int)h.channels[c].p_linear << ':' << h.pixel_types[c] << ':'
          << h.requested_pixel_types[c];
    o << " at:" << h.num_custom_attributes;
    for (int i = 0; i < h.num_custom_attributes; i++) {
        const EXRAttribute& a = h.custom_attributes[i];
        o << ' ' << qs(a.name) << ':' << qs(a.type) << ':' << a.size << ':' << std::hex
          << fnv1a(a.value, (size_t)a.size) << std::dec;
    }
    return o.str();
}

static int elem_size(const EXRHeader& h, int c) {
    if (h.channels[c].pixel_type == TINYEXR_PIXELTYPE_HALF && h.requested_pixel_types[c] == TINYEXR_PIXELTYPE_HALF)
        return 2;
    return 4;
}

static void le32(std::vector<uint8_t>& v, int32_t x) { put_u32(v, (uint32_t)x); }

static std::string image_summary(const EXRImage& img, const EXRHeader& h) {
    std::ostringstream o;
    o << " pt:";
    for (int c = 0; c < h.num_channels; c++) o << h.pixel_types[c] << (c + 1 < h.num_channels ? "," : "");
    o << " nl:" << EXRNumLevels(&img);
    for (const EXRImage* l = &img; l; l = l->next_level) {
        o << " [" << l->width << 'x' << l->height << ' ' << l->level_x << ',' << l->level_y << ' ' << l->num_channels
          << ' ' << l->num_tiles;
        uint64_t hash = 0xcbf29ce484222325ULL;
        if (l->images) {
            o << " S";
            for (int c = 0; c < l->num_channels; c++) {
                size_t n = (size_t)l->width * l->height * elem_size(h, c);
                hash = fnv1a(l->images[c], n, hash);
            }
        }
        if (l->tiles) {
            o << " T";
            for (int t = 0; t < l->num_tiles; t++) {
                const EXRTile& tl = l->tiles[t];
                std::vector<uint8_t> v;
                le32(v, tl.offset_x); le32(v, tl.offset_y); le32(v, tl.level_x); le32(v, tl.level_y);
                le32(v, tl.width); le32(v, tl.height);
                hash = fnv1a(v.data(), v.size(), hash);
                for (int c = 0; c < l->num_channels; c++) {
                    size_t n = (size_t)h.tile_size_x * h.tile_size_y * elem_size(h, c);
                    hash = fnv1a(tl.images[c], n, hash);
                }
            }
        }
        char b[32];
        snprintf(b, sizeof b, " %016llx]", (unsigned long long)hash);
        o << b;
    }
    return o.str();
}

// Inputs whose decoding would allocate too much for a test (same rule in Rust).
static bool too_big(const EXRHeader& h) {
    const int64_t T = 1024 * 8192, LIMIT = 1 << 26;
    int64_t w = (int64_t)h.data_window.max_x - h.data_window.min_x + 1;
    int64_t ht = (int64_t)h.data_window.max_y - h.data_window.min_y + 1;
    int64_t nc = h.num_channels;
    if (w > 0 && ht > 0 && w <= T && ht <= T && w * ht * nc * 4 > LIMIT) return true;
    if (h.tiled) {
        int64_t tx = h.tile_size_x, ty = h.tile_size_y;
        if (tx >= 0 && ty >= 0 && tx <= T && ty <= T && tx * ty * nc * 4 > LIMIT) return true;
        if (tx > 0 && ty > 0 && w > 0 && ht > 0 && w <= T && ht <= T && ((w + tx - 1) / tx) * ((ht + ty - 1) / ty) > (1 << 16))
            return true;
    } else if (h.chunk_count > (1 << 23)) {
        return true;
    }
    return false;
}

static std::string run_case(const std::vector<uint8_t>& file, int variant) {
    std::ostringstream o;
    size_t size = file.size();
    unsigned char* buf = (unsigned char*)calloc(1, size ? size : 1);
    if (size) memcpy(buf, file.data(), size);

    EXRVersion version;
    memset(&version, 0, sizeof version);
    int vr = ParseEXRVersionFromMemory(&version, buf, size);
    o << "V" << vr;
    if (vr != 0) { free(buf); return o.str(); }
    o << ' ' << version.version << ',' << version.tiled << ',' << version.long_name << ',' << version.non_image << ','
      << version.multipart;

    EXRHeader header;
    InitEXRHeader(&header);
    const char* err = nullptr;
    int hr = ParseEXRHeaderFromMemory(&header, &version, buf, size, &err);
    o << "\tH" << hr << ' ' << qs(err) << ' ' << header_summary(header);
    FreeEXRErrorMessage(err);
    err = nullptr;
    if (hr != 0) { FreeEXRHeader(&header); free(buf); return o.str(); }
    if (too_big(header)) { FreeEXRHeader(&header); free(buf); return o.str() + "\tBIG"; }

    for (int c = 0; c < header.num_channels; c++) {
        if (variant >= 1 && variant <= 2 && header.pixel_types[c] == TINYEXR_PIXELTYPE_HALF)
            header.requested_pixel_types[c] = TINYEXR_PIXELTYPE_FLOAT;
        if (variant == 3) header.requested_pixel_types[c] = TINYEXR_PIXELTYPE_UINT;
    }
    if (variant == 2) header.line_order = 0;

    EXRImage image;
    InitEXRImage(&image);
    int lr = LoadEXRImageFromMemory(&image, &header, buf, size, &err);
    o << "\tL" << lr << ' ' << qs(err);
    FreeEXRErrorMessage(err);
    if (lr == 0) {
        o << image_summary(image, header);
        FreeEXRImage(&image);
    }
    FreeEXRHeader(&header);
    free(buf);
    return o.str();
}

static void emit(const std::string& name, int variant, const std::vector<uint8_t>& file) {
    fflush(stdout);
    int fds[2];
    if (pipe(fds) != 0) { perror("pipe"); exit(1); }
    pid_t pid = fork();
    if (pid == 0) {
        close(fds[0]);
        std::string s = run_case(file, variant);
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
    printf("%s\t%d\t%s\n", name.c_str(), variant, ok ? s.c_str() : "CRASH");
}

static int run(const char* corpus, int nfiles, char** files) {
    std::vector<Entry> entries = read_corpus(corpus);
    size_t synthetic = entries.size();
    for (int i = 0; i < nfiles; i++) {
        FILE* f = fopen(files[i], "rb");
        if (!f) { perror(files[i]); return 1; }
        std::vector<uint8_t> d;
        uint8_t b[65536];
        size_t n;
        while ((n = fread(b, 1, sizeof b, f)) > 0) d.insert(d.end(), b, b + n);
        fclose(f);
        entries.push_back({files[i], d});
    }
    for (size_t i = 0; i < entries.size(); i++) {
        const Entry& e = entries[i];
        int nvar = i < synthetic ? 4 : 3;
        for (int v = 0; v < nvar; v++) emit(e.name, v, e.data);
        if (e.data.empty() || e.data.size() > 65536) continue;
        // corruptions
        Lcg rng{(uint32_t)fnv1a((const uint8_t*)e.name.data(), e.name.size())};
        size_t size = e.data.size();
        int nflip = i < synthetic ? 6 : 12;
        for (int k = 0; k < nflip; k++) {
            uint32_t r = rng.next();
            size_t pos = (k % 2 == 0) ? r % std::min<size_t>(size, 400) : r % size;
            uint8_t val = (uint8_t)rng.next();
            std::vector<uint8_t> d = e.data;
            d[pos] = val;
            emit(e.name + "#f" + std::to_string(k), 0, d);
        }
        for (int k = 0; k < 2; k++) {
            size_t len = rng.next() % size;
            std::vector<uint8_t> d(e.data.begin(), e.data.begin() + len);
            emit(e.name + "#t" + std::to_string(k), 0, d);
        }
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc >= 3 && strcmp(argv[1], "gen") == 0) return gen(argv[2]);
    if (argc >= 3 && strcmp(argv[1], "run") == 0) return run(argv[2], argc - 3, argv + 3);
    fprintf(stderr, "usage: %s gen <corpus> | run <corpus> <files...>\n", argv[0]);
    return 1;
}
