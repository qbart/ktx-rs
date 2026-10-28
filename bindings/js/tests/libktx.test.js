// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// libktx.js and libktx_read.js: the embind API surface, texture properties, transcodeBasis of
// every image vs libktx (golden/transcode.tsv) and the write scenarios vs the native library
// (golden/write.tsv).
'use strict';

const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const path = require('path');
const { ROOT, fnv1a64, readGolden } = require('./ktx2.js');

const dist = path.join(__dirname, '..', 'dist');
const createKtxModule = require(path.join(dist, 'libktx.js'));
const createKtxReadModule = require(path.join(dist, 'libktx_read.js'));

const load = (file) => new Uint8Array(fs.readFileSync(path.join(ROOT, file)));

// Both wasm builds are separate instances; load them once.
const modules = {};
test.before(async () => {
  modules.libktx = await createKtxModule();
  modules.libktx_read = await createKtxReadModule();
});

for (const name of ['libktx', 'libktx_read']) {
  const write = name === 'libktx';

  test(`${name}: module surface`, () => {
    const ktx = modules[name];
    for (const e of ['error_code', 'transcode_fmt', 'transcode_flag_bits', 'basis_codec',
      'SupercmpScheme', 'OrientationX', 'OrientationY', 'OrientationZ', 'khr_df_model',
      'khr_df_primaries', 'khr_df_transfer', 'texture']) {
      assert.ok(e in ktx, e);
    }
    // class_compat.js aliases.
    assert.strictEqual(ktx.ktxTexture, ktx.texture);
    assert.strictEqual(ktx.ErrorCode, ktx.error_code);
    assert.strictEqual(ktx.TranscodeTarget, ktx.transcode_fmt);
    assert.strictEqual(ktx.TranscodeFlags, ktx.transcode_flag_bits);

    assert.strictEqual(ktx.error_code.SUCCESS.value, 0);
    assert.strictEqual(ktx.error_code.FILE_UNEXPECTED_ERROR.value, 7);
    assert.strictEqual(ktx.error_code.DECOMPRESS_CHECKSUM_ERROR.value, 20);
    assert.strictEqual(ktx.transcode_fmt.RGBA8888.value, ktx.transcode_fmt.RGBA32.value);
    assert.strictEqual(ktx.transcode_fmt.BC1_OR_3.value, 23);
    assert.strictEqual(ktx.transcode_flag_bits.TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS.value, 4);
    assert.strictEqual(ktx.OrientationX.LEFT.value, 'l'.charCodeAt(0));
    assert.strictEqual(ktx.khr_df_primaries.SRGB.value, ktx.khr_df_primaries.BT709.value);

    const writeOnly = ['textureCreateInfo', 'astcParams', 'basisParams', 'VkFormat',
      'TextureCreateStorageEnum', 'pack_astc_quality_levels', 'pack_astc_block_dimension',
      'pack_astc_encoder_mode', 'pack_uastc_flag_bits', 'ANIMDATA_KEY', 'ORIENTATION_KEY',
      'SWIZZLE_KEY', 'WRITER_KEY', 'WRITER_SCPARAMS_KEY', 'FACESLICE_WHOLE_lEVEL',
      'ETC1S_DEFAULT_COMPRESSION_LEVEL'];
    for (const w of writeOnly) {
      assert.strictEqual(w in ktx, write, w);
    }
    for (const m of ['compressAstc', 'compressBasis', 'deflateZstd', 'deflateZLIB',
      'addKVPairString', 'addKVPairByte', 'deleteKVPair', 'setImageFromMemory',
      'writeToMemory']) {
      assert.strictEqual(typeof ktx.texture.prototype[m] === 'function', write, m);
    }
    if (write) {
      assert.strictEqual(Object.keys(ktx.VkFormat).filter((k) => k !== 'values').length, 230);
      assert.strictEqual(ktx.VkFormat.R8G8B8A8_SRGB.value, 43);
      assert.strictEqual(ktx.VkFormat.A8_UNORM_KHR.value, 1000470001);
      assert.strictEqual(ktx.pack_astc_block_dimension.D6x6x6.value, 23);
      assert.strictEqual(ktx.pack_astc_quality_levels.THOROUGH.value, 98);
      assert.strictEqual(ktx.FACESLICE_WHOLE_lEVEL, 0xffffffff);
      assert.strictEqual(ktx.ORIENTATION_KEY, 'KTXorientation');
      assert.strictEqual(ktx.ETC1S_DEFAULT_COMPRESSION_LEVEL, 2);
    }
  });

  test(`${name}: texture properties`, () => {
    const ktx = modules[name];
    const t = new ktx.texture(load('tests/resources/ktx2/kodim17_blze.ktx2'));
    assert.deepStrictEqual([t.baseWidth, t.baseHeight, t.numComponents, t.vkFormat],
      [512, 768, 3, 0]);
    assert.strictEqual(t.supercompressScheme, ktx.SupercmpScheme.BASIS_LZ);
    assert.strictEqual(t.oetf, ktx.khr_df_transfer.SRGB);
    assert.strictEqual(t.transferFunction, ktx.khr_df_transfer.SRGB);
    // BT709 and SRGB share value 1; like embind, values[1] is the last registered (SRGB).
    assert.strictEqual(t.primaries, ktx.khr_df_primaries.SRGB);
    // ETC1S (163) is not a registered khr_df_model value.
    assert.strictEqual(t.colorModel, undefined);
    assert.deepStrictEqual([t.isSrgb, t.isHDR, t.isPremultiplied, t.isTranscodable,
      t.needsTranscoding], [true, false, false, true, true]);
    assert.deepStrictEqual(t.orientation,
      { x: ktx.OrientationX.RIGHT, y: ktx.OrientationY.DOWN, z: ktx.OrientationZ.OUT });
    const writer = new TextDecoder().decode(t.findKeyValue('KTXwriter'));
    assert.ok(writer.endsWith('\0'));
    assert.strictEqual(t.findKeyValue('nokey'), null);

    const copy = t.createCopy();
    assert.ok(copy instanceof ktx.texture);
    assert.strictEqual(copy.baseHeight, 768);
    assert.strictEqual(copy.transcodeBasis(ktx.transcode_fmt.RGBA32, 0), ktx.error_code.SUCCESS);
    assert.strictEqual(copy.vkFormat, 43);
    assert.strictEqual(copy.needsTranscoding, false);
    assert.strictEqual(t.needsTranscoding, true);
    copy.delete();
    assert.ok(copy.isDeleted());
    t.delete();

    const u = new ktx.texture(load('bindings/js/tests/resources/r8g8b8a8_srgb_mip_uastc.ktx2'));
    assert.strictEqual(u.colorModel, ktx.khr_df_model.KHR_DF_MODEL_UASTC_LDR_4x4);
    assert.strictEqual(u.supercompressScheme, ktx.SupercmpScheme.NONE);
    u.delete();

    // KTX1: KTX2-only queries fall back like the C++.
    const k1 = new ktx.texture(load('tests/resources/ktx/etc2_rgba8.ktx'));
    assert.strictEqual(k1.vkFormat, 0);
    assert.strictEqual(k1.numComponents, 0);
    assert.strictEqual(k1.supercompressScheme, ktx.SupercmpScheme.NONE);
    assert.strictEqual(k1.transcodeBasis(ktx.transcode_fmt.RGBA32, 0),
      ktx.error_code.INVALID_OPERATION);
    assert.ok(k1.getImage(0, 0, 0).length > 0);
    k1.delete();

    // A failed load leaves an empty texture (C++ keeps a null pointer).
    const bad = new ktx.texture(new Uint8Array(12));
    assert.strictEqual(bad.baseWidth, 0);
    assert.strictEqual(bad.getImage(0, 0, 0), null);
    bad.delete();

    assert.throws(() => new ktx.texture(), /invalid number of parameters/);

    // Module.GL (Emscripten GL emulation); glUpload needs a current context (run in a browser
    // by tests/browser).
    for (const f of ['createContext', 'makeContextCurrent', 'registerContext', 'getContext']) {
      assert.strictEqual(typeof ktx.GL[f], 'function', f);
    }
    assert.ok(Array.isArray(ktx.GL.textures));
    const g = new ktx.texture(load('tests/resources/ktx2/r8g8b8a8_srgb_mip.ktx2'));
    assert.throws(() => g.glUpload(), /no current WebGL context/);
    g.delete();
    if (!write) {
      const r = new ktx.texture(load('tests/resources/ktx2/kodim17_blze.ktx2'));
      assert.throws(() => { r.oetf = ktx.khr_df_transfer.LINEAR; }, /read-only/);
      r.delete();
    }
  });

  test(`${name}: transcodeBasis matches libktx`, () => {
    const ktx = modules[name];
    for (const [file, byTarget] of readGolden()) {
      const data = load(file);
      for (const [target, want] of byTarget) {
        const t = new ktx.texture(data);
        const r = t.transcodeBasis(ktx.transcode_fmt[target], 0);
        if (want === 'fail') {
          assert.notStrictEqual(r, ktx.error_code.SUCCESS, `${file} ${target}`);
          t.delete();
          continue;
        }
        assert.strictEqual(r, ktx.error_code.SUCCESS, `${file} ${target}`);
        for (const w of want) {
          const [level, layer, face] = w.image.split('.').map(Number);
          const image = t.getImage(level, layer, face);
          assert.strictEqual(image.length, w.len, `${file} ${target} ${w.image}`);
          assert.strictEqual(fnv1a64(image), w.hash, `${file} ${target} ${w.image}`);
        }
        t.delete();
      }
    }
  });
}

test('libktx: write scenarios match the native library', () => {
  const ktx = modules.libktx;
  const golden = new Map(fs.readFileSync(path.join(__dirname, 'golden', 'write.tsv'), 'utf8')
    .split('\n').filter(Boolean).map((l) => {
      const [name, len, hash] = l.split('\t');
      return [name, { len: Number(len), hash }];
    }));
  const check = (name, t) => {
    const data = t.writeToMemory();
    assert.strictEqual(data.length, golden.get(name).len, name);
    assert.strictEqual(fnv1a64(data), golden.get(name).hash, name);
    t.delete();
  };
  const SUCCESS = ktx.error_code.SUCCESS;
  const raw = () => {
    const info = new ktx.textureCreateInfo();
    info.vkFormat = ktx.VkFormat.R8G8B8A8_SRGB;
    assert.strictEqual(info.vkFormat, 43);
    Object.assign(info, { baseWidth: 32, baseHeight: 32, baseDepth: 1, numDimensions: 2,
      numLevels: 1, numLayers: 1, numFaces: 1, isArray: false, generateMipmaps: false });
    const t = new ktx.texture(info, ktx.TextureCreateStorageEnum.ALLOC_STORAGE);
    info.delete();
    const image = new Uint8Array(32 * 32 * 4).map((_, i) => (i * 7 + (i >> 7)) & 0xff);
    assert.strictEqual(t.setImageFromMemory(0, 0, 0, image), SUCCESS);
    assert.strictEqual(t.addKVPairString(ktx.ORIENTATION_KEY, 'rd'), SUCCESS);
    return t;
  };
  check('raw', raw());

  let t = raw();
  let p = new ktx.basisParams();
  assert.deepStrictEqual([p.codec, p.qualityLevel, p.uastcFlags, p.inputSwizzle],
    [0, 0, 0, '\0\0\0\0']);
  p.codec = ktx.basis_codec.ETC1S.value;
  p.qualityLevel = 128;
  p.etc1sCompressionLevel = ktx.ETC1S_DEFAULT_COMPRESSION_LEVEL;
  assert.strictEqual(t.compressBasis(p), SUCCESS);
  p.delete();
  assert.strictEqual(t.supercompressScheme, ktx.SupercmpScheme.BASIS_LZ);
  check('etc1s', t);

  t = raw();
  p = new ktx.basisParams();
  p.codec = ktx.basis_codec.UASTC_LDR_4x4.value;
  p.uastcFlags = ktx.pack_uastc_flag_bits.LEVEL_FASTEST;
  assert.strictEqual(p.uastcFlags, 0);
  assert.strictEqual(t.compressBasis(p), SUCCESS);
  assert.strictEqual(t.deflateZstd(5), SUCCESS);
  p.delete();
  check('uastc_zstd', t);

  t = raw();
  const a = new ktx.astcParams();
  a.blockDimension = ktx.pack_astc_block_dimension.D6x6;
  a.mode = ktx.pack_astc_encoder_mode.LDR;
  a.qualityLevel = ktx.pack_astc_quality_levels.FAST;
  assert.deepStrictEqual([a.blockDimension, a.mode, a.qualityLevel], [4, 1, 10]);
  a.inputSwizzle = 'rgba';
  assert.strictEqual(a.inputSwizzle, 'rgba');
  a.inputSwizzle = '';
  assert.strictEqual(t.compressAstc(a), SUCCESS);
  assert.strictEqual(t.deflateZLIB(3), SUCCESS);
  a.delete();
  check('astc_zlib', t);

  t = raw();
  assert.strictEqual(t.addKVPairByte('mykey', new Uint8Array([1, 2, 3])), SUCCESS);
  assert.strictEqual(t.deleteKVPair(ktx.ORIENTATION_KEY), SUCCESS);
  t.oetf = ktx.khr_df_transfer.LINEAR;
  t.primaries = ktx.khr_df_primaries.DISPLAYP3;
  assert.strictEqual(t.primaries, ktx.khr_df_primaries.DISPLAYP3);
  assert.deepStrictEqual(Array.from(t.findKeyValue('mykey')), [1, 2, 3]);
  check('kv_colorspace', t);
});
