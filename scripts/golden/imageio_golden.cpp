// Golden-data generator for the imageio reader port (dev tooling, not part
// of the Rust library). Must stay in sync with tests/imageio.rs.
//
// For every input file and target format (VK_FORMAT_UNDEFINED = native) it
// runs ImageInput::open + readImage in a forked AddressSanitizer child and
// prints "<file>\t<target>\t<result>": the spec / format descriptor of the
// opened image, warnings, and the readImage error or a hash of the buffer.
// Only the PNG and EXR readers are built (DISABLE_JPEG, DISABLE_NPBM). Build/run via gen_imageio_golden.sh.
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <sstream>
#include <string>
#include <vector>
#include <sys/wait.h>
#include <unistd.h>
#include "imageio.h"
#include "dfd.h"
#include "vkformat_enum.h"

extern "C" const char* __asan_default_options() { return "detect_leaks=0:exitcode=99"; }

static uint64_t fnv1a(const uint8_t* p, size_t n, uint64_t h = 0xcbf29ce484222325ULL) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

// tools/ktx/format_descriptor.h createFormatDescriptor
static FormatDescriptor fromDfd(const uint32_t* dfd) {
    const uint32_t* bdfd = dfd + 1;
    FormatDescriptor::basicDescriptor basic;
    basic.colorModel = KHR_DFDVAL(bdfd, MODEL);
    basic.colorPrimaries = KHR_DFDVAL(bdfd, PRIMARIES);
    basic.transferFunction = KHR_DFDVAL(bdfd, TRANSFER);
    basic.dataFlags = KHR_DFDVAL(bdfd, FLAGS);
    basic.texelBlockDimension0 = KHR_DFDVAL(bdfd, TEXELBLOCKDIMENSION0);
    basic.texelBlockDimension1 = KHR_DFDVAL(bdfd, TEXELBLOCKDIMENSION1);
    basic.texelBlockDimension2 = KHR_DFDVAL(bdfd, TEXELBLOCKDIMENSION2);
    basic.texelBlockDimension3 = KHR_DFDVAL(bdfd, TEXELBLOCKDIMENSION3);
    basic.bytesPlane0 = KHR_DFDVAL(bdfd, BYTESPLANE0);
    basic.bytesPlane1 = KHR_DFDVAL(bdfd, BYTESPLANE1);
    basic.bytesPlane2 = KHR_DFDVAL(bdfd, BYTESPLANE2);
    basic.bytesPlane3 = KHR_DFDVAL(bdfd, BYTESPLANE3);
    basic.bytesPlane4 = KHR_DFDVAL(bdfd, BYTESPLANE4);
    basic.bytesPlane5 = KHR_DFDVAL(bdfd, BYTESPLANE5);
    basic.bytesPlane6 = KHR_DFDVAL(bdfd, BYTESPLANE6);
    basic.bytesPlane7 = KHR_DFDVAL(bdfd, BYTESPLANE7);
    std::vector<FormatDescriptor::sample> samples;
    for (uint32_t i = 0; i < KHR_DFDSAMPLECOUNT(bdfd); ++i) {
        auto& sample = samples.emplace_back();
        sample.bitOffset = KHR_DFDSVAL(bdfd, i, BITOFFSET);
        sample.bitLength = KHR_DFDSVAL(bdfd, i, BITLENGTH);
        sample.channelId = KHR_DFDSVAL(bdfd, i, CHANNELID);
        const auto dataType = KHR_DFDSVAL(bdfd, i, QUALIFIERS);
        sample.qualifierFloat = (dataType & KHR_DF_SAMPLE_DATATYPE_FLOAT) != 0;
        sample.qualifierSigned = (dataType & KHR_DF_SAMPLE_DATATYPE_SIGNED) != 0;
        sample.qualifierExponent = (dataType & KHR_DF_SAMPLE_DATATYPE_EXPONENT) != 0;
        sample.qualifierLinear = (dataType & KHR_DF_SAMPLE_DATATYPE_LINEAR) != 0;
        sample.samplePosition0 = KHR_DFDSVAL(bdfd, i, SAMPLEPOSITION0);
        sample.samplePosition1 = KHR_DFDSVAL(bdfd, i, SAMPLEPOSITION1);
        sample.samplePosition2 = KHR_DFDSVAL(bdfd, i, SAMPLEPOSITION2);
        sample.samplePosition3 = KHR_DFDSVAL(bdfd, i, SAMPLEPOSITION3);
        sample.lower = KHR_DFDSVAL(bdfd, i, SAMPLELOWER);
        sample.upper = KHR_DFDSVAL(bdfd, i, SAMPLEUPPER);
    }
    return FormatDescriptor{basic, std::move(samples)};
}

static std::string esc(const std::string& s) {
    std::string o;
    for (unsigned char c : s) {
        if (c == '\n') o += "\\n";
        else if (c == '\\') o += "\\\\";
        else if (c == '\t') o += "\\t";
        else if (c >= 0x20 && c < 0x7f) o += (char)c;
        else { char b[8]; snprintf(b, sizeof b, "\\x%02x", c); o += b; }
    }
    return o;
}

static std::string fdSummary(const FormatDescriptor& f) {
    std::ostringstream o;
    const auto& b = f.basic;
    o << "fd:" << b.colorModel << ',' << b.colorPrimaries << ',' << b.transferFunction << ',' << b.dataFlags << " tb:"
      << b.texelBlockDimension0 << ',' << b.texelBlockDimension1 << ',' << b.texelBlockDimension2 << ','
      << b.texelBlockDimension3 << " bp:" << b.bytesPlane0 << ',' << b.bytesPlane1 << ',' << b.bytesPlane2 << ','
      << b.bytesPlane3 << ',' << b.bytesPlane4 << ',' << b.bytesPlane5 << ',' << b.bytesPlane6 << ','
      << b.bytesPlane7;
    uint32_t g;
    memcpy(&g, &f.extended.oeGamma, 4);
    char gb[16];
    snprintf(gb, sizeof gb, "%08x", g);
    o << " ext:" << f.extended.channelCount << ',' << f.extended.sameUnitAllChannels << ',' << gb << ','
      << esc(f.extended.iccProfile.name) << ',' << f.extended.iccProfile.profile.size() << ',' << std::hex
      << fnv1a(f.extended.iccProfile.profile.data(), f.extended.iccProfile.profile.size()) << std::dec;
    o << " s:" << f.samples.size();
    for (const auto& s : f.samples)
        o << " [" << s.bitOffset << ',' << s.bitLength << ',' << s.channelId << ',' << s.qualifierLinear
          << s.qualifierExponent << s.qualifierSigned << s.qualifierFloat << ',' << s.samplePosition0 << ','
          << s.samplePosition1 << ',' << s.samplePosition2 << ',' << s.samplePosition3 << ',' << s.lower << ','
          << s.upper << ']';
    return o.str();
}

static std::string runCase(const char* path, VkFormat vkFormat) {
    std::ostringstream o;
    std::string warnings;
    auto wcb = [&](const std::string& w) { warnings += "{" + esc(w) + "}"; };
    std::unique_ptr<ImageInput> in;
    try {
        in = ImageInput::open(path, nullptr, wcb);
    } catch (const std::exception& e) {
        return "OPEN " + esc(e.what()) + " W" + warnings;
    }
    const ImageSpec& spec = in->spec();
    o << in->formatName() << ' ' << toString(in->formatType()) << ' ' << in->subimageCount() << ' ' << spec.width()
      << 'x' << spec.height() << 'x' << spec.depth() << ' ' << (int)spec.origin().x << ',' << (int)spec.origin().y
      << ',' << (int)spec.origin().z << ' ' << fdSummary(spec.format());
    FormatDescriptor target;
    if (vkFormat != VK_FORMAT_UNDEFINED) {
        uint32_t* dfd = vk2dfd(vkFormat);
        target = fromDfd(dfd);
        free(dfd);
    }
    size_t size = target.isUnknown() ? spec.imageByteCount()
                                     : (size_t)spec.width() * spec.height() * target.pixelByteCount();
    std::vector<uint8_t> buf(size, 0);
    try {
        in->readImage(buf.data(), size, 0, 0, target);
        char h[32];
        snprintf(h, sizeof h, "%016llx", (unsigned long long)fnv1a(buf.data(), size));
        o << "\tREAD " << size << ' ' << h;
    } catch (const std::exception& e) {
        o << "\tERR " << esc(e.what());
    }
    o << " W" << warnings;
    return o.str();
}

static void emit(const char* path, int vk) {
    fflush(stdout);
    int fds[2];
    if (pipe(fds) != 0) { perror("pipe"); exit(1); }
    pid_t pid = fork();
    if (pid == 0) {
        close(fds[0]);
        std::string s = runCase(path, (VkFormat)vk);
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
    printf("%s\t%d\t%s\n", path, vk, ok ? s.c_str() : "CRASH");
}

static const int TARGETS[] = {
    VK_FORMAT_UNDEFINED, VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_R16G16B16A16_UNORM, VK_FORMAT_R32G32B32A32_UINT,
    VK_FORMAT_R32G32B32A32_SFLOAT, VK_FORMAT_R8_UNORM, VK_FORMAT_R8G8_UNORM, VK_FORMAT_R8G8B8_UNORM,
    VK_FORMAT_R16_UNORM, VK_FORMAT_R16G16_UNORM, VK_FORMAT_R16G16B16_UNORM, VK_FORMAT_R16G16B16A16_SFLOAT,
    VK_FORMAT_R32_SFLOAT, VK_FORMAT_R16_SFLOAT, VK_FORMAT_R32_UINT, VK_FORMAT_R8G8B8A8_SRGB,
    VK_FORMAT_R16G16B16A16_SNORM, VK_FORMAT_R5G6B5_UNORM_PACK16, VK_FORMAT_E5B9G9R9_UFLOAT_PACK32};

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++)
        for (int vk : TARGETS) emit(argv[i], vk);
    return 0;
}
