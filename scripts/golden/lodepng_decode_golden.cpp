// Golden-data generator for the lodepng decoder port (dev tooling, not part
// of the Rust library). Encodes the synthetic images of lodepng_golden.cpp,
// adds hand-built ancillary chunks and corrupted variants, and decodes them
// with lodepng_decode / lodepng_decode_chunks + lodepng_finish_decode under
// several raw modes and decoder settings. Prints
// "<case> <error> <w> <h> <size> <fnv1a64 of output> <fnv1a64 of info text>".
// Must stay in sync with tests/lodepng.rs. Build/run via
// scripts/golden/gen_lodepng_golden.sh.
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
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

// Encoder variants (subset of lodepng_golden.cpp).
static std::vector<unsigned char> encode(int kind, LodePNGColorType ct, unsigned bd, unsigned w, unsigned h,
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
    } else if (variant == "interlace") {
        state.info_png.interlace_method = 1;
    } else if (variant == "btype0") {
        state.encoder.zlibsettings.btype = 0;
    } else if (variant == "btype1") {
        state.encoder.zlibsettings.btype = 1;
    } else if (variant == "noauto") {
        state.encoder.auto_convert = 0;
    } else if (variant == "noautointerlace") {
        state.encoder.auto_convert = 0;
        state.info_png.interlace_method = 1;
    }
    std::vector<unsigned char> png;
    lodepng::encode(png, image, w, h, state);
    return png;
}

static void put32(std::string& s, unsigned v) {
    char b[16]; snprintf(b, sizeof b, "%u,", v); s += b;
}

static void put_bytes(std::string& s, const unsigned char* p, size_t n) {
    char b[24]; snprintf(b, sizeof b, "%zu:%016llx,", n, (unsigned long long)fnv1a(p, n)); s += b;
}

static void put_color(std::string& s, const LodePNGColorMode& c) {
    put32(s, c.colortype); put32(s, c.bitdepth); put32(s, (unsigned)c.palettesize);
    if (c.palettesize) put_bytes(s, c.palette, c.palettesize * 4);
    put32(s, c.key_defined); put32(s, c.key_r); put32(s, c.key_g); put32(s, c.key_b);
}

// Text summary of the decoder state; must match `info_text` in tests/lodepng.rs.
static std::string info_text(const LodePNGState& st) {
    const LodePNGInfo& i = st.info_png;
    std::string s;
    put_color(s, i.color); s += "|"; put_color(s, st.info_raw); s += "|";
    put32(s, i.compression_method); put32(s, i.filter_method); put32(s, i.interlace_method);
    put32(s, i.background_defined);
    if (i.background_defined) { put32(s, i.background_r); put32(s, i.background_g); put32(s, i.background_b); }
    put32(s, (unsigned)i.text_num);
    for (size_t k = 0; k < i.text_num; k++) {
        put_bytes(s, (const unsigned char*)i.text_keys[k], strlen(i.text_keys[k]));
        put_bytes(s, (const unsigned char*)i.text_strings[k], strlen(i.text_strings[k]));
    }
    put32(s, (unsigned)i.itext_num);
    for (size_t k = 0; k < i.itext_num; k++) {
        put_bytes(s, (const unsigned char*)i.itext_keys[k], strlen(i.itext_keys[k]));
        put_bytes(s, (const unsigned char*)i.itext_langtags[k], strlen(i.itext_langtags[k]));
        put_bytes(s, (const unsigned char*)i.itext_transkeys[k], strlen(i.itext_transkeys[k]));
        put_bytes(s, (const unsigned char*)i.itext_strings[k], strlen(i.itext_strings[k]));
    }
    put32(s, i.time_defined);
    if (i.time_defined) { put32(s, i.time.year); put32(s, i.time.month); put32(s, i.time.day);
                          put32(s, i.time.hour); put32(s, i.time.minute); put32(s, i.time.second); }
    put32(s, i.phys_defined);
    if (i.phys_defined) { put32(s, i.phys_x); put32(s, i.phys_y); put32(s, i.phys_unit); }
    put32(s, i.gama_defined);
    if (i.gama_defined) put32(s, i.gama_gamma);
    put32(s, i.chrm_defined);
    if (i.chrm_defined) {
        put32(s, i.chrm_white_x); put32(s, i.chrm_white_y); put32(s, i.chrm_red_x); put32(s, i.chrm_red_y);
        put32(s, i.chrm_green_x); put32(s, i.chrm_green_y); put32(s, i.chrm_blue_x); put32(s, i.chrm_blue_y);
    }
    put32(s, i.srgb_defined);
    if (i.srgb_defined) put32(s, i.srgb_intent);
    put32(s, i.iccp_defined);
    if (i.iccp_defined) {
        put_bytes(s, (const unsigned char*)(i.iccp_name ? i.iccp_name : ""), i.iccp_name ? strlen(i.iccp_name) : 0);
        put_bytes(s, i.iccp_profile, i.iccp_profile_size);
    }
    put32(s, i.cicp_defined);
    if (i.cicp_defined) {
        put32(s, i.cicp_color_primaries); put32(s, i.cicp_transfer_function);
        put32(s, i.cicp_matrix_coefficients); put32(s, i.cicp_video_full_range_flag);
    }
    put32(s, i.mdcv_defined);
    if (i.mdcv_defined) {
        put32(s, i.mdcv_red_x); put32(s, i.mdcv_red_y); put32(s, i.mdcv_green_x); put32(s, i.mdcv_green_y);
        put32(s, i.mdcv_blue_x); put32(s, i.mdcv_blue_y); put32(s, i.mdcv_white_x); put32(s, i.mdcv_white_y);
        put32(s, i.mdcv_max_luminance); put32(s, i.mdcv_min_luminance);
    }
    put32(s, i.clli_defined);
    if (i.clli_defined) { put32(s, i.clli_max_cll); put32(s, i.clli_max_fall); }
    put32(s, i.exif_defined);
    if (i.exif_defined) put_bytes(s, i.exif, i.exif_size);
    put32(s, i.sbit_defined);
    if (i.sbit_defined) { put32(s, i.sbit_r); put32(s, i.sbit_g); put32(s, i.sbit_b); put32(s, i.sbit_a); }
    for (int k = 0; k < 3; k++) put_bytes(s, i.unknown_chunks_data[k], i.unknown_chunks_size[k]);
    return s;
}

static int g_dump = 0; // set LODEPNG_DUMP=1 to print the info text

static void emit(const std::string& name, unsigned error, unsigned w, unsigned h, const unsigned char* out,
                 size_t size, const LodePNGState& st) {
    std::string info = info_text(st);
    printf("%s %u %u %u %zu %016llx %016llx\n", name.c_str(), error, w, h, size,
           (unsigned long long)fnv1a(out, size), (unsigned long long)fnv1a((const uint8_t*)info.data(), info.size()));
    if (g_dump) printf("  %s\n", info.c_str());
}

struct Settings { int ignore_crc, ignore_adler32, ignore_end, ignore_critical, remember_unknown, read_text, color_convert; };

static void apply(LodePNGState& st, const Settings& s);
static int lodepng_color_mode_equal(const LodePNGColorMode* a, const LodePNGColorMode* b) {
    if (a->colortype != b->colortype || a->bitdepth != b->bitdepth || a->key_defined != b->key_defined) return 0;
    if (a->key_defined && (a->key_r != b->key_r || a->key_g != b->key_g || a->key_b != b->key_b)) return 0;
    if (a->palettesize != b->palettesize) return 0;
    for (size_t i = 0; i != a->palettesize * 4; ++i) if (a->palette[i] != b->palette[i]) return 0;
    return 1;
}

static void apply(LodePNGState& st, const Settings& s) {
    st.decoder.ignore_crc = s.ignore_crc;
    st.decoder.zlibsettings.ignore_adler32 = s.ignore_adler32;
    st.decoder.ignore_end = s.ignore_end;
    st.decoder.ignore_critical = s.ignore_critical;
    st.decoder.remember_unknown_chunks = s.remember_unknown;
    st.decoder.read_text_chunks = s.read_text;
    st.decoder.color_convert = s.color_convert;
}

// mode: 0 = same as PNG (color_convert off), 1 = RGBA8, 2 = RGB8, 3 = RGBA16, 4 = GREY8,
// 5 = split (decode_chunks + finish_decode with the pnginput.cc raw mode).
static void decode_case(const std::string& name, const std::vector<unsigned char>& png, int mode, const Settings& s) {
    LodePNGState st;
    lodepng_state_init(&st);
    apply(st, s);
    if (mode == 0) st.decoder.color_convert = 0;
    if (mode == 2) st.info_raw.colortype = LCT_RGB;
    if (mode == 3) st.info_raw.bitdepth = 16;
    if (mode == 4) st.info_raw.colortype = LCT_GREY;
    unsigned w = 0, h = 0;
    if (mode != 5) {
        unsigned char* out = 0;
        unsigned error = lodepng_decode(&out, &w, &h, &st, png.data(), png.size());
        size_t size = error ? 0 : lodepng_get_raw_size(w, h, &st.info_raw);
        emit(name, error, w, h, out, size, st);
        free(out);
    } else {
        void* idat = 0;
        size_t idatsize = 0;
        unsigned error = lodepng_decode_chunks(&idat, &idatsize, &w, &h, &st, png.data(), png.size());
        if (error) { emit(name, error, w, h, 0, 0, st); lodepng_state_cleanup(&st); return; }
        lodepng_color_mode_copy(&st.info_raw, &st.info_png.color);
        if (st.info_png.color.colortype == LCT_GREY && st.info_png.color.key_defined) st.info_raw.colortype = LCT_GREY_ALPHA;
        if (st.info_png.color.colortype == LCT_RGB && st.info_png.color.key_defined) st.info_raw.colortype = LCT_RGBA;
        if (st.info_png.color.colortype == LCT_PALETTE) {
            const unsigned char* trns = lodepng_chunk_find_const(png.data() + 33, png.data() + png.size() - 1, "tRNS");
            st.info_raw.colortype = trns ? LCT_RGBA : LCT_RGB;
            st.info_raw.bitdepth = 8;
        }
        size_t size = lodepng_get_raw_size(w, h, &st.info_raw);
        if (size > (1u << 26)) { // corrupted header: no caller buffer that big
            free(idat);
            emit(name, 9999, w, h, 0, 0, st);
            lodepng_state_cleanup(&st);
            return;
        }
        std::vector<unsigned char> buf(size + 1, 0);
        if (!lodepng_color_mode_equal(&st.info_raw, &st.info_png.color)) {
            // lodepng_finish_decode converts and frees an unset buffer when inflating fails
            // (undefined behavior); the port reports the inflate error after the 56 check.
            LodePNGState probe;
            lodepng_state_init(&probe);
            apply(probe, s);
            probe.decoder.color_convert = 0;
            unsigned char* tmp = 0;
            unsigned pw, ph;
            unsigned perr = lodepng_decode(&tmp, &pw, &ph, &probe, png.data(), png.size());
            free(tmp);
            lodepng_state_cleanup(&probe);
            if (perr) {
                if (!(st.info_raw.colortype == LCT_RGB || st.info_raw.colortype == LCT_RGBA) && st.info_raw.bitdepth != 8)
                    perr = 56;
                free(idat);
                emit(name, perr, w, h, 0, 0, st);
                lodepng_state_cleanup(&st);
                return;
            }
        }
        error = lodepng_finish_decode(buf.data(), buf.size(), w, h, &st, idat, idatsize);
        emit(name, error, w, h, buf.data(), error ? 0 : size, st);
    }
    lodepng_state_cleanup(&st);
}

static void put_be32(std::vector<unsigned char>& v, unsigned x) {
    v.push_back((unsigned char)(x >> 24)); v.push_back((unsigned char)(x >> 16));
    v.push_back((unsigned char)(x >> 8)); v.push_back((unsigned char)x);
}

static std::vector<unsigned char> chunk(const char* type, const std::vector<unsigned char>& data) {
    std::vector<unsigned char> c;
    put_be32(c, (unsigned)data.size());
    c.insert(c.end(), type, type + 4);
    c.insert(c.end(), data.begin(), data.end());
    put_be32(c, lodepng_crc32(c.data() + 4, data.size() + 4));
    return c;
}

static std::vector<unsigned char> bytes(const char* s) { return std::vector<unsigned char>(s, s + strlen(s)); }

static std::vector<unsigned char> zlib(const std::vector<unsigned char>& d) {
    LodePNGCompressSettings settings;
    lodepng_compress_settings_init(&settings);
    unsigned char* out = 0; size_t outsize = 0;
    lodepng_zlib_compress(&out, &outsize, d.data(), d.size(), &settings);
    std::vector<unsigned char> v(out, out + outsize);
    free(out);
    return v;
}

static std::vector<unsigned char> cat(std::initializer_list<std::vector<unsigned char>> parts) {
    std::vector<unsigned char> v;
    for (auto& p : parts) v.insert(v.end(), p.begin(), p.end());
    return v;
}

// Ancillary chunk sets inserted after IHDR (where = 0) or before IEND (where = 1).
// Must match `extra_chunks` in tests/lodepng.rs.
static std::vector<std::pair<std::string, std::vector<unsigned char>>> extra_chunks() {
    std::vector<std::pair<std::string, std::vector<unsigned char>>> v;
    std::vector<unsigned char> z = {0};
    v.push_back({"text", chunk("tEXt", cat({bytes("Title"), z, bytes("Hello world")}))});
    v.push_back({"text_nonull", chunk("tEXt", bytes("Comment"))});
    v.push_back({"text_emptykey", chunk("tEXt", cat({z, bytes("x")}))});
    v.push_back({"text_longkey", chunk("tEXt", cat({std::vector<unsigned char>(80, 'k'), z, bytes("x")}))});
    v.push_back({"text_embedded_nul", chunk("tEXt", cat({bytes("K"), z, bytes("ab"), z, bytes("cd")}))});
    v.push_back({"ztxt", chunk("zTXt", cat({bytes("Desc"), z, z, zlib(bytes("compressed text payload"))}))});
    v.push_back({"ztxt_method", chunk("zTXt", cat({bytes("Desc"), z, {1}, zlib(bytes("x"))}))});
    v.push_back({"ztxt_short", chunk("zTXt", cat({bytes("Desc"), z}))});
    v.push_back({"ztxt_badzlib", chunk("zTXt", cat({bytes("Desc"), z, z, bytes("garbage!")}))});
    v.push_back({"itxt", chunk("iTXt", cat({bytes("Key"), z, z, z, bytes("en"), z, bytes("Schl"), z, bytes("plain")}))});
    v.push_back({"itxt_z", chunk("iTXt", cat({bytes("Key"), z, {1}, z, bytes("pl"), z, z, zlib(bytes("zipped itext"))}))});
    v.push_back({"itxt_short", chunk("iTXt", bytes("Ke"))});
    v.push_back({"itxt_method", chunk("iTXt", cat({bytes("Key"), z, z, {1}, z, z, bytes("x")}))});
    v.push_back({"itxt_notrans", chunk("iTXt", cat({bytes("Key"), z, z, z, bytes("en")}))});
    v.push_back({"time", chunk("tIME", {7, 230, 10, 23, 12, 30, 59})});
    v.push_back({"time_bad", chunk("tIME", {7, 230, 10})});
    v.push_back({"phys", chunk("pHYs", {0, 0, 11, 19, 0, 0, 11, 19, 1})});
    v.push_back({"phys_bad", chunk("pHYs", {0, 0, 11})});
    v.push_back({"gama", chunk("gAMA", {0, 0, 177, 143})});
    v.push_back({"gama_bad", chunk("gAMA", {0, 0})});
    v.push_back({"chrm", chunk("cHRM", std::vector<unsigned char>(32, 7))});
    v.push_back({"chrm_bad", chunk("cHRM", std::vector<unsigned char>(31, 7))});
    v.push_back({"srgb", chunk("sRGB", {2})});
    v.push_back({"srgb_bad", chunk("sRGB", {2, 3})});
    v.push_back({"iccp", chunk("iCCP", cat({bytes("icc"), z, z, zlib(std::vector<unsigned char>(300, 42))}))});
    v.push_back({"iccp_empty", chunk("iCCP", cat({bytes("icc"), z, z, zlib({})}))});
    v.push_back({"iccp_method", chunk("iCCP", cat({bytes("icc"), z, {3}, zlib({1})}))});
    v.push_back({"iccp_twice", cat({chunk("iCCP", cat({bytes("a"), z, z, zlib({1, 2})})),
                                     chunk("iCCP", cat({bytes("bb"), z, z, zlib({3})}))})});
    v.push_back({"cicp", chunk("cICP", {1, 13, 0, 1})});
    v.push_back({"cicp_bad", chunk("cICP", {1, 13, 0})});
    v.push_back({"mdcv", chunk("mDCV", {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24})});
    v.push_back({"mdcv_bad", chunk("mDCV", {1, 2})});
    v.push_back({"clli", chunk("cLLI", {0, 0, 3, 232, 0, 0, 1, 144})});
    v.push_back({"clli_bad", chunk("cLLI", {0})});
    v.push_back({"exif", chunk("eXIf", {'M', 'M', 0, 42, 1, 2, 3})});
    v.push_back({"exif_empty", chunk("eXIf", {})});
    v.push_back({"unknown_anc", chunk("abCd", {1, 2, 3})});
    v.push_back({"unknown_two", cat({chunk("prVt", {9}), chunk("xyZw", {})})});
    v.push_back({"unknown_critical", chunk("ABCD", {1})});
    v.push_back({"bad_name", chunk("ab1d", {1})});
    v.push_back({"reserved", chunk("abcd", {1})});
    v.push_back({"badcrc", [] { auto c = chunk("tEXt", cat({bytes("A"), {0}, bytes("b")})); c.back() ^= 1; return c; }()});
    v.push_back({"badcrc_unknown", [] { auto c = chunk("abCd", {5}); c.back() ^= 1; return c; }()});
    v.push_back({"sbit1", chunk("sBIT", {1})});
    v.push_back({"sbit2", chunk("sBIT", {1, 1})});
    v.push_back({"sbit3", chunk("sBIT", {1, 2, 3})});
    v.push_back({"sbit4", chunk("sBIT", {1, 2, 3, 4})});
    v.push_back({"sbit_zero", chunk("sBIT", {0, 0, 0})});
    v.push_back({"sbit_big", chunk("sBIT", {9, 9, 9, 9})});
    v.push_back({"bkgd1", chunk("bKGD", {0})});
    v.push_back({"bkgd2", chunk("bKGD", {0, 9})});
    v.push_back({"bkgd6", chunk("bKGD", {0, 1, 0, 2, 0, 3})});
    v.push_back({"bkgd_badindex", chunk("bKGD", {200})});
    v.push_back({"trns_grey", chunk("tRNS", {0, 5})});
    v.push_back({"trns_rgb", chunk("tRNS", {0, 1, 0, 2, 0, 3})});
    v.push_back({"trns_pal", chunk("tRNS", {1, 2, 3})});
    v.push_back({"plte", chunk("PLTE", {1, 2, 3, 4, 5, 6})});
    v.push_back({"plte_bad", chunk("PLTE", {1, 2})});
    v.push_back({"empty_idat", chunk("IDAT", {})});
    v.push_back({"iend_early", chunk("IEND", {})});
    v.push_back({"signature", std::vector<unsigned char>{137, 80, 78, 71, 13, 10, 26, 10}});
    return v;
}

// Inserts `extra` at the given place of `png` (0 = after IHDR, 1 = before IEND).
static std::vector<unsigned char> insert(const std::vector<unsigned char>& png, const std::vector<unsigned char>& extra, int where) {
    size_t pos = where == 0 ? 33 : png.size() - 12;
    std::vector<unsigned char> v(png.begin(), png.begin() + pos);
    v.insert(v.end(), extra.begin(), extra.end());
    v.insert(v.end(), png.begin() + pos, png.end());
    return v;
}

int main() {
    g_dump = getenv("LODEPNG_DUMP") != 0;
    const LodePNGColorType CTS[] = {LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY, LCT_GREY_ALPHA, LCT_GREY_ALPHA,
                                    LCT_RGB, LCT_RGB, LCT_RGBA, LCT_RGBA};
    const unsigned BDS[] = {1, 2, 4, 8, 16, 8, 16, 8, 16, 8, 16};
    const unsigned SIZES[][2] = {{1, 1}, {3, 2}, {17, 9}, {64, 48}};
    const char* VARIANTS[] = {"sbit", "srgb", "gama", "f1", "f2", "f3", "f4", "interlace", "btype0", "btype1",
                              "noauto", "noautointerlace"};
    const Settings DEF = {0, 0, 0, 0, 0, 1, 1};
    for (int kind = 0; kind < 8; kind++)
        for (int m = 0; m < 11; m++)
            for (auto& s : SIZES) {
                std::string base = std::string(KINDS[kind]) + "_" + std::to_string(CTS[m]) + "_" + std::to_string(BDS[m]) +
                                   "_" + std::to_string(s[0]) + "x" + std::to_string(s[1]);
                std::vector<unsigned char> png = encode(kind, CTS[m], BDS[m], s[0], s[1], "default");
                for (int mode = 0; mode < 6; mode++)
                    decode_case(base + "_default_m" + std::to_string(mode), png, mode, DEF);
                if (s[0] == 3 || s[0] == 17)
                    for (const char* v : VARIANTS) {
                        std::vector<unsigned char> p = encode(kind, CTS[m], BDS[m], s[0], s[1], v);
                        for (int mode : {0, 1, 5})
                            decode_case(base + "_" + v + "_m" + std::to_string(mode), p, mode, DEF);
                    }
            }

    // Ancillary chunks on a palette, a grey, an RGB 16 and an RGBA image.
    struct Base { const char* name; int kind; LodePNGColorType ct; unsigned bd; const char* variant; };
    const Base BASES[] = {{"pal", 2, LCT_RGB, 8, "default"}, {"grey", 3, LCT_GREY, 8, "noauto"},
                          {"rgb16", 1, LCT_RGB, 16, "noauto"}, {"rgba", 7, LCT_RGBA, 8, "noauto"}};
    const Settings EXTRA_SETTINGS[] = {DEF, {0, 0, 0, 1, 1, 1, 1}, {0, 0, 0, 0, 0, 0, 1}, {1, 0, 0, 0, 0, 1, 1}};
    for (auto& b : BASES) {
        std::vector<unsigned char> png = encode(b.kind, b.ct, b.bd, 5, 4, b.variant);
        for (auto& e : extra_chunks())
            for (int where = 0; where < 2; where++)
                for (int si = 0; si < 4; si++)
                    for (int mode : {0, 5})
                        decode_case(std::string("extra_") + b.name + "_" + e.first + "_w" + std::to_string(where) + "_s" +
                                        std::to_string(si) + "_m" + std::to_string(mode),
                                    insert(png, e.second, where), mode, EXTRA_SETTINGS[si]);
    }

    // Corruptions: byte flips and truncations under several settings.
    const Settings CORRUPT_SETTINGS[] = {DEF, {1, 0, 0, 0, 0, 1, 1}, {1, 1, 0, 0, 0, 1, 1}, {1, 1, 1, 1, 0, 1, 1}};
    for (int kind = 0; kind < 8; kind++)
        for (int m = 0; m < 11; m++)
            for (const char* v : {"default", "interlace", "btype1"}) {
                std::vector<unsigned char> png = encode(kind, CTS[m], BDS[m], 17, 9, v);
                lcg_state = (uint32_t)(kind * 100 + m * 7 + strlen(v));
                for (int c = 0; c < 8; c++) {
                    std::vector<unsigned char> p = png;
                    uint32_t r = lcg();
                    if (c < 5) {
                        size_t pos = r % p.size();
                        p[pos] ^= (unsigned char)(1 + (r >> 12) % 255);
                    } else {
                        p.resize(r % p.size());
                    }
                    for (int si = 0; si < 4; si++)
                        decode_case(std::string("corrupt_") + KINDS[kind] + "_" + std::to_string(m) + "_" + v + "_c" +
                                        std::to_string(c) + "_s" + std::to_string(si),
                                    p, si == 3 ? 5 : 1, CORRUPT_SETTINGS[si]);
                }
            }
    return 0;
}
