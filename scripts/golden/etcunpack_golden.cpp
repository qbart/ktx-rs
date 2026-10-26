// Golden-data generator for tests/etcunpack.rs (dev tooling, not part of the Rust library):
// _ktxUnpackETC (lib/src/etcunpack.cxx + external/etcdec) on random ETC1 / ETC2 / EAC images
// of every source format, several sizes and context capabilities.
// Prints "<case>\t<result> <format> <internalFormat> <type> <fnv1a64 of image>".
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <vector>
#include "GL/glcorearb.h"
#include "ktx.h"
#include "ktxint.h"

extern "C" KTX_error_code
_ktxUnpackETC(const GLubyte* srcETC, const GLenum srcFormat,
              ktx_uint32_t activeWidth, ktx_uint32_t activeHeight,
              GLubyte** dstImage, GLenum* format, GLenum* internalFormat, GLenum* type,
              GLint R16Formats, GLboolean supportsSRGB);

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
};

int main() {
    const struct { GLenum fmt; int blockBytes; } formats[] = {
        {0x8D64, 8},  // GL_ETC1_RGB8_OES
        {GL_COMPRESSED_RGB8_ETC2, 8},
        {GL_COMPRESSED_SRGB8_ETC2, 8},
        {GL_COMPRESSED_RGBA8_ETC2_EAC, 16},
        {GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8},
        {GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8},
        {GL_COMPRESSED_R11_EAC, 8},
        {GL_COMPRESSED_SIGNED_R11_EAC, 8},
        {GL_COMPRESSED_RG11_EAC, 16},
        {GL_COMPRESSED_SIGNED_RG11_EAC, 16},
    };
    const uint32_t sizes[][2] = {{4, 4}, {8, 8}, {5, 7}, {1, 1}, {13, 6}, {16, 12}};
    const int r16s[] = {3, 1, 2, 0};
    uint32_t c = 0;
    for (auto& f : formats) {
        for (auto& sz : sizes) {
            for (int r16 : r16s) {
                for (int srgb = 0; srgb < 2; srgb++, c++) {
                    Lcg l{c * 7919u + 13u};
                    uint32_t nb = ((sz[0] + 3) / 4) * ((sz[1] + 3) / 4);
                    std::vector<GLubyte> src(nb * f.blockBytes);
                    for (auto& b : src) b = (GLubyte)l.next();
                    GLubyte* dst = nullptr;
                    GLenum format = 0, internalFormat = 0, type = 0;
                    KTX_error_code r = _ktxUnpackETC(src.data(), f.fmt, sz[0], sz[1], &dst, &format,
                                                     &internalFormat, &type, r16, srgb);
                    uint64_t h = 0xcbf29ce484222325ULL;
                    if (r == KTX_SUCCESS) {
                        size_t px = (f.fmt == GL_COMPRESSED_RG11_EAC || f.fmt == GL_COMPRESSED_SIGNED_RG11_EAC) ? 4
                                  : (f.fmt == GL_COMPRESSED_R11_EAC || f.fmt == GL_COMPRESSED_SIGNED_R11_EAC) ? 2
                                  : (f.blockBytes == 16 || f.fmt == GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2
                                     || f.fmt == GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2) ? 4 : 3;
                        size_t n = px * sz[0] * sz[1];
                        for (size_t i = 0; i < n; i++) { h ^= dst[i]; h *= 0x100000001b3ULL; }
                        free(dst);
                    }
                    printf("c%u\t%d %#x %#x %#x %016llx\n", c, (int)r, format, internalFormat, type,
                           (unsigned long long)h);
                }
            }
        }
    }
    // Hand-built blocks: H mode with equal base colors (ordering bit), signed EAC base -128.
    const struct { GLenum fmt; const char* hex; } extra[] = {
        {GL_COMPRESSED_RGB8_ETC2, "2b07ab3a5c8cc1ab"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "2b07ab3a5c8cc1ab"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "2b07ab385c8cc1ab"},
        {GL_COMPRESSED_RGB8_ETC2, "49fa49e21b591d75"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "49fa49e21b591d75"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "49fa49e01b591d75"},
        {GL_COMPRESSED_RGB8_ETC2, "8714878e85a0bcc1"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "8714878e85a0bcc1"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "8714878c85a0bcc1"},
        {GL_COMPRESSED_RGB8_ETC2, "660e66625c8cc1ab"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "660e66625c8cc1ab"},
        {GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, "660e66605c8cc1ab"},
        {GL_COMPRESSED_SIGNED_R11_EAC, "803a5c8cc1ab2b07"},
        {GL_COMPRESSED_SIGNED_R11_EAC, "80055c8cc1ab2b07"},
        {GL_COMPRESSED_SIGNED_R11_EAC, "81f15c8cc1ab2b07"},
        {GL_COMPRESSED_SIGNED_RG11_EAC, "8023ffffffffffff7f2a000000000000"},
    };
    for (uint32_t i = 0; i < sizeof(extra) / sizeof(extra[0]); i++) {
        std::vector<GLubyte> src;
        for (const char* p = extra[i].hex; p[0] && p[1]; p += 2) {
            unsigned v; sscanf(p, "%2x", &v); src.push_back((GLubyte)v);
        }
        GLubyte* dst = nullptr;
        GLenum format = 0, internalFormat = 0, type = 0;
        KTX_error_code r = _ktxUnpackETC(src.data(), extra[i].fmt, 4, 4, &dst, &format, &internalFormat, &type, 3, 1);
        uint64_t h = 0xcbf29ce484222325ULL;
        size_t n = extra[i].fmt == GL_COMPRESSED_RGB8_ETC2 ? 48 : extra[i].fmt == GL_COMPRESSED_SIGNED_R11_EAC ? 32 : 64;
        if (r == KTX_SUCCESS) {
            for (size_t k = 0; k < n; k++) { h ^= dst[k]; h *= 0x100000001b3ULL; }
            free(dst);
        }
        printf("x%u\t%d %#x %#x %#x %016llx\n", i, (int)r, format, internalFormat, type, (unsigned long long)h);
    }
    return 0;
}
