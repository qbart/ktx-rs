// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// msc_basis_transcoder.js: the embind API surface and every image of the test files transcoded
// to every target, compared with libktx's ktxTexture2_TranscodeBasis (golden/transcode.tsv).
'use strict';

const test = require('node:test');
const assert = require('node:assert');
const path = require('path');
const { readKtx2, readBasisLzSgd, fnv1a64, readGolden } = require('./ktx2.js');

const MSC_TRANSCODER = require(path.join(__dirname, '..', 'dist', 'msc_basis_transcoder.js'));
const KHR_DF_MODEL_UASTC = 166;

let M;
test.before(async () => {
  M = await MSC_TRANSCODER();
  M.initTranscoders();
});

test('module surface', () => {
  for (const name of ['TranscodeTarget', 'TextureFormat', 'TranscodeFlagBits', 'initTranscoders',
    'isFormatSupported', 'ImageInfo', 'BasisLzEtc1sImageTranscoder', 'UastcImageTranscoder',
    'BasisTranscoderState']) {
    assert.ok(name in M, name);
  }
  const { TranscodeTarget: T, TextureFormat: F, TranscodeFlagBits: B } = M;
  assert.strictEqual(T.BC7_RGBA.value, 6);
  assert.strictEqual(T.BC7_M5_RGBA.value, 6);
  assert.strictEqual(T.ASTC_4x4_RGBA.value, 10);
  assert.strictEqual(T.BC6HU_RGB.value, 22);
  assert.strictEqual(T.EAC_RG11.value, 21);
  assert.strictEqual(F.UASTC_HDR_6x6_INTERMEDIATE.value, 4);
  assert.strictEqual(B.TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS.value, 4);
  assert.strictEqual(B.HIGH_QUALITY.value, 32);
  assert.strictEqual(T.values[13], T.RGBA32);
  assert.ok(Object.isFrozen(T.RGBA32));

  assert.strictEqual(M.isFormatSupported(T.BC7_RGBA, F.ETC1S), true);
  assert.strictEqual(M.isFormatSupported(T.PVRTC2_4_RGB, F.UASTC_LDR_4x4), false);
  assert.strictEqual(M.isFormatSupported(T.BC6HU_RGB, F.ETC1S), false);
  assert.strictEqual(M.isFormatSupported(T.BC6HU_RGB, F.UASTC_HDR_4x4), true);
  assert.strictEqual(M.isFormatSupported(6, 0), true);

  assert.strictEqual(M.BasisLzEtc1sImageTranscoder.getBytesPerBlock(T.RGBA32), 4);
  assert.strictEqual(M.UastcImageTranscoder.getBytesPerBlock(T.BC7_RGBA), 16);

  const info = new M.ImageInfo(F.ETC1S, 33, 17, 2);
  assert.deepStrictEqual(
    [info.width, info.height, info.numBlocksX, info.numBlocksY, info.level, info.flags,
      info.rgbByteOffset, info.rgbByteLength, info.alphaByteOffset, info.alphaByteLength],
    [33, 17, 9, 5, 2, 0, 0, 0, 0, 0]);
  info.alphaByteLength = 7;
  assert.strictEqual(info.alphaByteLength, 7);
  assert.strictEqual(info.isDeleted(), false);
  info.delete();
  assert.strictEqual(info.isDeleted(), true);
  new M.BasisTranscoderState().delete();
});

test('failed transcode returns no transcodedImage', () => {
  const uit = new M.UastcImageTranscoder();
  const info = new M.ImageInfo(M.TextureFormat.UASTC_LDR_4x4, 4, 4, 0);
  info.rgbByteLength = 16;
  // Too little data for the declared slice.
  const r = uit.transcodeImage(M.TranscodeTarget.RGBA32, new Uint8Array(8), info, 0, false);
  assert.deepStrictEqual(Object.keys(r), []);
  assert.strictEqual(r.transcodedImage, undefined);
  info.delete();
  uit.delete();
});

// One image per KTX2 image (level, layer, face) like the upstream usage example.
function* images(ktx) {
  const h = ktx.header;
  const layers = Math.max(1, h.layerCount);
  for (let level = 0; level < h.levels.length; level++) {
    const width = Math.max(1, h.pixelWidth >> level);
    const height = Math.max(1, h.pixelHeight >> level);
    for (let layer = 0; layer < layers; layer++) {
      for (let face = 0; face < h.faceCount; face++) {
        yield { level, layer, face, width, height };
      }
    }
  }
}

function transcodeEtc1s(ktx, target) {
  const { data, header: h } = ktx;
  const sgd = readBasisLzSgd(ktx);
  const bit = new M.BasisLzEtc1sImageTranscoder();
  const sub = (o, n) => data.subarray(o, o + n);
  assert.ok(bit.decodePalettes(sgd.endpointCount, sub(sgd.endpointsStart, sgd.endpointsByteLength),
    sgd.selectorCount, sub(sgd.selectorsStart, sgd.selectorsByteLength)));
  assert.ok(bit.decodeTables(sub(sgd.tablesStart, sgd.tablesByteLength)));
  const out = [];
  let index = 0;
  for (const img of images(ktx)) {
    const desc = sgd.imageDescs[index++];
    const info = new M.ImageInfo(M.TextureFormat.ETC1S, img.width, img.height, img.level);
    info.flags = desc.imageFlags;
    info.rgbByteOffset = 0;
    info.rgbByteLength = desc.rgbSliceByteLength;
    info.alphaByteOffset = desc.alphaSliceByteLength > 0
      ? desc.alphaSliceByteOffset - desc.rgbSliceByteOffset : 0;
    info.alphaByteLength = desc.alphaSliceByteLength;
    const levelOffset = h.levels[img.level].byteOffset;
    const end = Math.max(desc.rgbSliceByteOffset + desc.rgbSliceByteLength,
      desc.alphaSliceByteOffset + desc.alphaSliceByteLength);
    const slices = data.subarray(levelOffset + desc.rgbSliceByteOffset, levelOffset + end);
    const { transcodedImage } = bit.transcodeImage(target, slices, info, 0, false);
    info.delete();
    if (transcodedImage === undefined) {
      bit.delete();
      return 'fail';
    }
    out.push({ img, bytes: transcodedImage.get_typed_memory_view() });
    transcodedImage.delete();
  }
  bit.delete();
  return out;
}

function transcodeUastc(ktx, target) {
  const { data, header: h } = ktx;
  const uit = new M.UastcImageTranscoder();
  // libktx: alpha unless the DFD channel is KHR_DF_CHANNEL_UASTC_RGB.
  const hasAlpha = h.channelId0 !== 0;
  const out = [];
  let levelImage = -1;
  let prevLevel = -1;
  for (const img of images(ktx)) {
    levelImage = img.level === prevLevel ? levelImage + 1 : 0;
    prevLevel = img.level;
    const info = new M.ImageInfo(M.TextureFormat.UASTC_LDR_4x4, img.width, img.height, img.level);
    const imageByteLength = info.numBlocksX * info.numBlocksY * 16;
    info.rgbByteOffset = 0;
    info.rgbByteLength = imageByteLength;
    const start = h.levels[img.level].byteOffset + levelImage * imageByteLength;
    const inImage = data.subarray(start, start + imageByteLength);
    const { transcodedImage } = uit.transcodeImage(target, inImage, info, 0, hasAlpha, false);
    info.delete();
    if (transcodedImage === undefined) {
      uit.delete();
      return 'fail';
    }
    out.push({ img, bytes: transcodedImage.get_typed_memory_view() });
    transcodedImage.delete();
  }
  uit.delete();
  return out;
}

const golden = readGolden();
for (const [file, byTarget] of golden) {
  test(`transcode ${path.basename(file)}`, () => {
    const ktx = readKtx2(file);
    const uastc = ktx.header.colorModel === KHR_DF_MODEL_UASTC;
    for (const [target, want] of byTarget) {
      const t = M.TranscodeTarget[target];
      const texFormat = uastc ? M.TextureFormat.UASTC_LDR_4x4 : M.TextureFormat.ETC1S;
      const got = !M.isFormatSupported(t, texFormat) ? 'fail'
        : uastc ? transcodeUastc(ktx, t) : transcodeEtc1s(ktx, t);
      if (want === 'fail') {
        // libktx refuses these (unsupported target or PVRTC1 on non power of two sizes).
        continue;
      }
      assert.notStrictEqual(got, 'fail', `${target}: transcode failed`);
      assert.strictEqual(got.length, want.length, `${target}: image count`);
      got.forEach(({ img, bytes }, i) => {
        const w = want[i];
        assert.strictEqual(`${img.level}.${img.layer}.${img.face}`, w.image);
        // msc pads PVRTC1 to 8x8 blocks for GL; libktx does not.
        assert.ok(bytes.length >= w.len, `${target} ${w.image}: ${bytes.length} < ${w.len}`);
        assert.strictEqual(fnv1a64(bytes.subarray(0, w.len)), w.hash, `${target} ${w.image}`);
      });
    }
  });
}
