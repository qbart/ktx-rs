// Golden-data generator for the image.hpp port (dev tooling, not part of the
// Rust library). Must stay in sync with tests/image.rs.
//
// For 28 ImageT<T, N> types filled with LCG data (with edge values) it prints
// "t<type> <op>\t<hash>" for every conversion getter, color space transform,
// normalize, swizzle, copyTo, premultiplyAlpha, mapRange, yflip and resample.
// Transfer functions and primary transforms are also hashed directly.
// Build/run via gen_image_golden.sh.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>
#include <memory>
#include <string>
#include <vector>
#include "image.hpp"

static uint64_t fnv(uint64_t h, const void* p, size_t n) {
    const uint8_t* b = (const uint8_t*)p;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 0x100000001b3ULL; }
    return h;
}
static const uint64_t FNV0 = 0xcbf29ce484222325ULL;
static uint64_t hv(const std::vector<uint8_t>& v) { return fnv(FNV0, v.data(), v.size()); }

struct Lcg {
    uint32_t s;
    uint32_t next() { s = s * 1664525u + 1013904223u; return s >> 8; }
};

static const float SPECIALS[] = {0.0f, 1.0f, -0.0f, 0.5f, 1e-8f, 65504.0f, 70000.0f, 1e30f, -1e30f,
                                 INFINITY, -INFINITY, NAN, 16777216.0f, -0.25f, 0.999f, 3e9f};

template <typename T> T gen(Lcg& l) {
    uint32_t r = l.next();
    uint32_t sel = l.next() % 8;
    if constexpr (std::is_floating_point_v<T>) {
        if (sel == 0) return SPECIALS[l.next() % 16];
        return (float)(r & 0xffff) / 65535.0f * 2.5f - 0.5f;
    } else {
        if (sel == 0) return std::numeric_limits<T>::max();
        if (sel == 1) return std::numeric_limits<T>::min();
        if (sel == 2) return T{0};
        uint32_t v = (r << 8) ^ l.next();
        return (T)v;
    }
}

template <typename T, uint32_t N> void fill(ImageT<T, N>& img, uint32_t seed) {
    Lcg l{seed};
    uint8_t* bytes = (uint8_t*)img;
    for (uint32_t i = 0; i < img.getPixelCount() * N; i++) {
        T v = gen<T>(l);
        memcpy(bytes + i * sizeof(T), &v, sizeof(T));
    }
}

static uint64_t himg(Image& img) {
    uint64_t h = fnv(FNV0, (uint8_t*)img, img.getByteCount());
    uint32_t meta[4] = {img.getWidth(), img.getHeight(), (uint32_t)img.getTransferFunction(), (uint32_t)img.getPrimaries()};
    return fnv(h, meta, sizeof(meta));
}

static void out(int t, const std::string& op, uint64_t h) {
    printf("t%d %s\t%016llx\n", t, op.c_str(), (unsigned long long)h);
}

static const ColorPrimariesBT709 pBT709;
static const ColorPrimariesBT601_625_EBU pEBU;
static const ColorPrimariesBT601_525_SMPTE pSMPTE;
static const ColorPrimariesBT2020 pBT2020;
static const ColorPrimariesCIEXYZ pXYZ;
static const ColorPrimariesACES pACES;
static const ColorPrimariesACEScc pACEScc;
static const ColorPrimariesNTSC1953 pNTSC;
static const ColorPrimariesPAL525 pPAL;
static const ColorPrimariesDisplayP3 pP3;
static const ColorPrimariesAdobeRGB pAdobe;
static const ColorPrimaries* PRIMS[] = {&pBT709, &pEBU, &pSMPTE, &pBT2020, &pXYZ, &pACES,
                                        &pACEScc, &pNTSC, &pPAL, &pP3, &pAdobe};

static const TransferFunctionLinear tfLinear;
static const TransferFunctionSRGB tfSRGB;
static const TransferFunctionITU tfITU;
static const TransferFunctionGamma tfGamma(2.2f);
static const TransferFunctionGamma tfGamma2(0.45f);
static const TransferFunctionBT2100_PQ_EOTF tfPQ;
static const TransferFunction* TFS[] = {&tfLinear, &tfSRGB, &tfITU, &tfGamma, &tfGamma2, &tfPQ};

static const char* FILTERS[] = {"box", "tent", "lanczos4", "kaiser", "mitchell"};
static const uint32_t RSIZES[][2] = {{3, 2}, {9, 11}, {7, 5}, {1, 1}, {16, 3}};

template <typename T, uint32_t N> void runType(int t) {
    const uint32_t W = 5, H = 3;
    auto fresh = [&]() {
        auto img = std::make_unique<ImageT<T, N>>(W, H);
        fill(*img, 1000 + t);
        return img;
    };
    auto base = fresh();
    out(t, "base", himg(*base));
    out(t, "info", (uint64_t)base->getByteCount() << 32 | base->getPixelSize() << 16 |
                       base->getComponentCount() << 8 | base->getComponentSize());
    const bool size4 = sizeof(T) == 4;
    if (!size4) {
        for (uint32_t nc = 1; nc <= 4; nc++)
            for (uint32_t bits : {8u, 16u, 32u})
                for (uint32_t sb : {0u, bits - 2})
                    out(t, "unorm c" + std::to_string(nc) + " b" + std::to_string(bits) + " s" + std::to_string(sb),
                        hv(base->getUNORM(nc, bits, sb)));
        const uint32_t PK[][4] = {{5, 6, 5, 0}, {4, 4, 4, 4}, {5, 5, 5, 1}, {1, 5, 5, 5}, {8, 8, 8, 8},
                                  {10, 10, 10, 2}, {2, 10, 10, 10}, {16, 16, 16, 16}, {8, 8, 0, 0},
                                  {3, 3, 2, 0}, {32, 32, 0, 0}, {4, 4, 0, 0}, {16, 0, 0, 0}};
        for (auto& p : PK)
            out(t, "unormpacked " + std::to_string(p[0]) + "," + std::to_string(p[1]) + "," + std::to_string(p[2]) + "," +
                       std::to_string(p[3]),
                hv(base->getUNORMPacked(p[0], p[1], p[2], p[3])));
    }
    for (uint32_t nc = 1; nc <= 4; nc++) {
        for (uint32_t bits : {16u, 32u})
            out(t, "sfloat c" + std::to_string(nc) + " b" + std::to_string(bits), hv(base->getSFloat(nc, bits)));
        for (uint32_t bits : {8u, 16u, 32u, 64u}) {
            out(t, "uint c" + std::to_string(nc) + " b" + std::to_string(bits), hv(base->getUINT(nc, bits)));
            out(t, "sint c" + std::to_string(nc) + " b" + std::to_string(bits), hv(base->getSINT(nc, bits)));
        }
    }
    out(t, "b10g11r11", hv(base->getB10G11R11()));
    out(t, "e5b9g9r9", hv(base->getE5B9G9R9()));
    const uint32_t IPK[][4] = {{8, 8, 8, 8}, {10, 10, 10, 2}, {2, 10, 10, 10}, {11, 11, 5, 5}};
    for (auto& p : IPK) {
        std::string k = std::to_string(p[0]) + "," + std::to_string(p[1]) + "," + std::to_string(p[2]) + "," +
                        std::to_string(p[3]);
        out(t, "uintpacked " + k, hv(base->getUINTPacked(p[0], p[1], p[2], p[3])));
        out(t, "sintpacked " + k, hv(base->getSINTPacked(p[0], p[1], p[2], p[3])));
    }
    {
        auto img = fresh();
        img->yflip();
        out(t, "yflip", himg(*img));
        auto img2 = std::make_unique<ImageT<T, N>>(3, 4);
        fill(*img2, 7 + t);
        img2->yflip();
        out(t, "yflip4", himg(*img2));
    }
    const int TCS[][3] = {{0, 1, -1}, {1, 0, -1}, {1, 1, -1}, {2, 0, -1}, {0, 2, -1}, {3, 0, -1},
                          {0, 5, -1}, {5, 0, -1}, {1, 0, 9}, {0, 0, 3}, {4, 1, 10}};
    for (auto& c : TCS) {
        auto img = fresh();
        ColorPrimaryTransform m;
        if (c[2] >= 0) m = PRIMS[c[2]]->transformTo(pBT709);
        img->transformColorSpace(*TFS[c[0]], *TFS[c[1]], c[2] >= 0 ? &m : nullptr);
        out(t, "tcs " + std::to_string(c[0]) + "," + std::to_string(c[1]) + "," + std::to_string(c[2]), himg(*img));
    }
    {
        auto img = fresh();
        img->normalize();
        out(t, "normalize", himg(*img));
    }
    for (const char* s : {"rgba", "bgr1", "a0g1", "rrrr", "gbar", "01ba"}) {
        auto img = fresh();
        img->swizzle(s);
        out(t, std::string("swizzle ") + s, himg(*img));
    }
    for (const char* s : {"rgba", "abgr"}) {
        auto src = fresh();
        src->setTransferFunction(KHR_DF_TRANSFER_SRGB);
        src->setPrimaries(KHR_DF_PRIMARIES_DISPLAYP3);
        ImageT<T, 1> d1(W, H); ImageT<T, 2> d2(W, H); ImageT<T, 3> d3(W, H); ImageT<T, 4> d4(W, H);
        src->copyToR(d1, s); src->copyToRG(d2, s); src->copyToRGB(d3, s); src->copyToRGBA(d4, s);
        uint64_t hs[4] = {himg(d1), himg(d2), himg(d3), himg(d4)};
        out(t, std::string("copy ") + s, fnv(FNV0, hs, sizeof(hs)));
    }
    for (int tf : {0, 1}) {
        auto img = fresh();
        img->setTransferFunction(tf ? KHR_DF_TRANSFER_SRGB : KHR_DF_TRANSFER_LINEAR);
        img->premultiplyAlpha();
        out(t, "premultiply " + std::to_string(tf), himg(*img));
    }
    const float MR[][2] = {{2.0f, -0.5f}, {1.0f, 0.0f}, {0.5f, 0.25f}};
    for (int i = 0; i < 3; i++) {
        auto img = fresh();
        img->mapRange(MR[i][0], MR[i][1]);
        out(t, "maprange " + std::to_string(i), himg(*img));
    }
    {
        auto src = std::make_unique<ImageT<T, N>>(7, 5);
        fill(*src, 500 + t);
        for (int f = 0; f < 5; f++)
            for (int z = 0; z < 5; z++)
                for (int tf = 0; tf < 2; tf++)
                    for (int sc = 0; sc < 2; sc++) {
                        src->setTransferFunction(tf ? KHR_DF_TRANSFER_SRGB : KHR_DF_TRANSFER_LINEAR);
                        const auto wrap = (basisu::Resampler::Boundary_Op)((f + z) % 3);
                        std::string k = "resample f" + std::to_string(f) + " z" + std::to_string(z) + " tf" +
                                        std::to_string(tf) + " s" + std::to_string(sc);
                        try {
                            auto r = src->resample(RSIZES[z][0], RSIZES[z][1], FILTERS[f], sc ? 0.5f : 1.0f, wrap);
                            out(t, k, himg(*r));
                        } catch (const std::exception& ex) {
                            printf("t%d %s\tE:%s\n", t, k.c_str(), ex.what());
                        }
                    }
        for (auto& e : std::vector<std::pair<std::string, std::array<uint32_t, 2>>>{
                 {"nope", {3, 3}}, {"box", {16385, 1}}}) {
            try {
                auto r = src->resample(e.second[0], e.second[1], e.first.c_str(), 1.0f,
                                       basisu::Resampler::BOUNDARY_CLAMP);
                out(t, "resample err " + e.first, himg(*r));
            } catch (const std::exception& ex) {
                printf("t%d resample err %s\tE:%s\n", t, e.first.c_str(), ex.what());
            }
        }
    }
}

int main() {
    // Transfer functions.
    std::vector<float> ins;
    for (int i = -20; i <= 120; i++) ins.push_back(i / 100.0f);
    for (float s : SPECIALS) ins.push_back(s);
    for (int i = 0; i < 6; i++) {
        uint64_t he = FNV0, hd = FNV0;
        for (float x : ins) {
            float e = TFS[i]->encode(x), d = TFS[i]->decode(x);
            he = fnv(he, &e, 4);
            hd = fnv(hd, &d, 4);
        }
        printf("tf%d\t%016llx %016llx\n", i, (unsigned long long)he, (unsigned long long)hd);
    }
    for (int a = 0; a < 11; a++) {
        uint64_t h = FNV0;
        for (int b = 0; b < 11; b++) {
            ColorPrimaryTransform m = PRIMS[a]->transformTo(*PRIMS[b]);
            h = fnv(h, m.matrix, sizeof(m.matrix));
        }
        printf("prim%d\t%016llx\n", a, (unsigned long long)h);
    }
    int t = 0;
    runType<uint8_t, 1>(t++); runType<uint8_t, 2>(t++); runType<uint8_t, 3>(t++); runType<uint8_t, 4>(t++);
    runType<uint16_t, 1>(t++); runType<uint16_t, 2>(t++); runType<uint16_t, 3>(t++); runType<uint16_t, 4>(t++);
    runType<uint32_t, 1>(t++); runType<uint32_t, 2>(t++); runType<uint32_t, 3>(t++); runType<uint32_t, 4>(t++);
    runType<int8_t, 1>(t++); runType<int8_t, 2>(t++); runType<int8_t, 3>(t++); runType<int8_t, 4>(t++);
    runType<int16_t, 1>(t++); runType<int16_t, 2>(t++); runType<int16_t, 3>(t++); runType<int16_t, 4>(t++);
    runType<int32_t, 1>(t++); runType<int32_t, 2>(t++); runType<int32_t, 3>(t++); runType<int32_t, 4>(t++);
    runType<float, 1>(t++); runType<float, 2>(t++); runType<float, 3>(t++); runType<float, 4>(t++);
    return 0;
}
