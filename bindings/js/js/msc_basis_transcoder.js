// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// msc_basis_transcoder.js: EMSCRIPTEN_BINDINGS of transcoder_wrapper.cpp. Runs inside the
// factory scope after the wasm-bindgen glue and embind.js.

return function MSC_TRANSCODER(moduleArg) {
  return __ktxModule('msc_basis_transcoder', moduleArg, (Module) => {
    const g = wasm_bindgen;
    const enums = g.__enums();
    Module.TranscodeTarget = __ktxEnum('TranscodeTarget', enums.TranscodeTarget);
    Module.TextureFormat = __ktxEnum('TextureFormat', enums.TextureFormat);
    Module.TranscodeFlagBits = __ktxEnum('TranscodeFlagBits', enums.TranscodeFlagBits);

    Module.initTranscoders = () => g.initTranscoders();
    Module.isFormatSupported = (targetFormat, texFormat) =>
      g.isFormatSupported(__ktxVal(targetFormat), __ktxVal(texFormat));

    __ktxClass(g.TranscodedImage);
    __ktxClass(g.BasisTranscoderState);
    Module.BasisTranscoderState = g.BasisTranscoderState;

    class ImageInfo extends __ktxClass(g.ImageInfo) {
      constructor(texFormat, width, height, level) {
        super(__ktxVal(texFormat), width, height, level);
      }
    }
    Module.ImageInfo = ImageInfo;

    const result = (image) => (image === undefined ? {} : { transcodedImage: image });

    class BasisLzEtc1sImageTranscoder extends __ktxClass(g.BasisLzEtc1sImageTranscoder) {
      static getBytesPerBlock(format) {
        return g.BasisLzEtc1sImageTranscoder.getBytesPerBlock(__ktxVal(format));
      }
      transcodeImage(targetFormat, jsInSlices, imageInfo, decodeFlags = 0, isVideo = false) {
        return result(super.transcodeImage(__ktxVal(targetFormat), jsInSlices, imageInfo,
          __ktxVal(decodeFlags), !!isVideo));
      }
    }
    Module.BasisLzEtc1sImageTranscoder = BasisLzEtc1sImageTranscoder;

    class UastcImageTranscoder extends __ktxClass(g.UastcImageTranscoder) {
      static getBytesPerBlock(format) {
        return g.UastcImageTranscoder.getBytesPerBlock(__ktxVal(format));
      }
      transcodeImage(targetFormat, jsInImage, imageInfo, decodeFlags = 0, hasAlpha = false,
        isVideo = false) {
        return result(super.transcodeImage(__ktxVal(targetFormat), jsInImage, imageInfo,
          __ktxVal(decodeFlags), !!hasAlpha, !!isVideo));
      }
    }
    Module.UastcImageTranscoder = UastcImageTranscoder;
  });
};
