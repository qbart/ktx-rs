// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// texture.glUpload on a real WebGL 2 context (run by run.js in headless Chrome). Each case
// uploads, checks target / error / WebGLTexture, and reads every level / layer / face back
// through a framebuffer, comparing with texture.getImage. Sets window.__result.
'use strict';

const GL_TEXTURE_2D = 0x0DE1;
const GL_TEXTURE_3D = 0x806F;
const GL_TEXTURE_2D_ARRAY = 0x8C1A;
const GL_TEXTURE_CUBE_MAP = 0x8513;
const GL_TEXTURE_CUBE_MAP_POSITIVE_X = 0x8515;

async function fetchBytes(path) {
  const r = await fetch('/' + path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
}

// readPixels of one level / layer (or face) of an RGBA8 texture.
function readBack(gl, target, texture, level, layerOrFace, width, height) {
  const fb = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  if (target === GL_TEXTURE_2D_ARRAY || target === GL_TEXTURE_3D) {
    gl.framebufferTextureLayer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, texture, level,
      layerOrFace);
  } else {
    const t = target === GL_TEXTURE_CUBE_MAP ? GL_TEXTURE_CUBE_MAP_POSITIVE_X + layerOrFace
      : target;
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, t, texture, level);
  }
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  const out = new Uint8Array(width * height * 4);
  if (status === gl.FRAMEBUFFER_COMPLETE) {
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, out);
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.deleteFramebuffer(fb);
  return status === gl.FRAMEBUFFER_COMPLETE ? out : null;
}

function equal(a, b) {
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

const log = [];

async function run() {
  const check = (cond, msg) => {
    log.push((cond ? 'ok   ' : 'FAIL ') + msg);
    if (!cond) log.failed = true;
  };

  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2');
  check(!!gl, 'WebGL 2 context');
  const ktx = await createKtxModule({ preinitializedWebGLContext: gl });
  const ktxRead = await createKtxReadModule({ preinitializedWebGLContext: gl });

  // Like tests/webgl: make the existing context current for the GL emulation.
  check(ktx.GL.makeContextCurrent(ktx.GL.createContext(canvas, { majorVersion: 2.0 })),
    'GL.makeContextCurrent');
  check(ktx.GL.currentContext.version === 2, 'preinitialized context is WebGL 2');
  check(ktxRead.GL.makeContextCurrent(ktxRead.GL.createContext(canvas, { majorVersion: 2.0 })),
    'libktx_read GL.makeContextCurrent');

  // Uploads `t`, checks the result and, for RGBA8 data (`readable`: levels and layers, faces
  // or depth), every image read back.
  const upload = (M, name, t, wantTarget, readable) => {
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 8);
    const r = t.glUpload();
    check(r.error === 0, `${name}: error ${r.error}`);
    check(r.target === wantTarget, `${name}: target 0x${r.target.toString(16)}`);
    check(r.object instanceof WebGLTexture, `${name}: WebGLTexture`);
    check(gl.getParameter(gl.UNPACK_ALIGNMENT) === 8, `${name}: UNPACK_ALIGNMENT restored`);
    check(M.GL.textures[r.object.name] === r.object, `${name}: GL.textures[name]`);
    if (!readable) return r;
    const levels = readable.levels;
    let images = 0;
    let same = 0;
    for (let level = 0; level < levels; level++) {
      const w = Math.max(1, t.baseWidth >> level);
      const h = Math.max(1, t.baseHeight >> level);
      const layers = wantTarget === GL_TEXTURE_3D ? Math.max(1, readable.depth >> level)
        : readable.layers;
      for (let l = 0; l < layers; l++) {
        const got = readBack(gl, wantTarget, r.object, level, l, w, h);
        const want = wantTarget === GL_TEXTURE_3D
          ? t.getImage(level, 0, ktx.FACESLICE_WHOLE_lEVEL).subarray(l * w * h * 4,
            (l + 1) * w * h * 4)
          : wantTarget === GL_TEXTURE_CUBE_MAP ? t.getImage(level, 0, l)
          : t.getImage(level, l, 0);
        images++;
        if (equal(got, want)) same++;
      }
    }
    check(images > 0 && same === images, `${name}: ${same}/${images} images read back equal`);
    return r;
  };

  const tex = async (M, path) => new M.texture(await fetchBytes(path));

  // 2D with mipmaps (KTX2: unpack alignment 1).
  let t = await tex(ktx, 'tests/resources/ktx2/r8g8b8a8_srgb_mip.ktx2');
  upload(ktx, '2D mipmapped', t, GL_TEXTURE_2D, { levels: 7, layers: 1 });
  t.delete();

  // 2D array, 3D.
  t = await tex(ktx, 'tests/resources/ktx2/r8g8b8a8_srgb_array_7_mip.ktx2');
  upload(ktx, '2D array', t, GL_TEXTURE_2D_ARRAY, { levels: 5, layers: 7 });
  t.delete();
  t = await tex(ktx, 'tests/resources/ktx2/r8g8b8a8_srgb_3d_7.ktx2');
  upload(ktx, '3D', t, GL_TEXTURE_3D, { levels: 1, depth: 7 });
  t.delete();

  // Cube map built with the write API.
  const info = new ktx.textureCreateInfo();
  info.vkFormat = ktx.VkFormat.R8G8B8A8_UNORM;
  Object.assign(info, { baseWidth: 8, baseHeight: 8, baseDepth: 1, numDimensions: 2,
    numLevels: 1, numLayers: 1, numFaces: 6 });
  t = new ktx.texture(info, ktx.TextureCreateStorageEnum.ALLOC_STORAGE);
  info.delete();
  for (let face = 0; face < 6; face++) {
    const img = new Uint8Array(8 * 8 * 4).map((_, i) => (i * 3 + face * 41) & 0xff);
    t.setImageFromMemory(0, 0, face, img);
  }
  upload(ktx, 'cube map', t, GL_TEXTURE_CUBE_MAP, { levels: 1, layers: 6 });
  t.delete();

  // BasisLZ/ETC1S transcoded to RGBA32, with libktx_read.js.
  t = await tex(ktxRead, 'tests/resources/ktx2/kodim17_blze.ktx2');
  check(t.transcodeBasis(ktxRead.transcode_fmt.RGBA32, 0) === ktxRead.error_code.SUCCESS,
    'ETC1S transcode');
  upload(ktxRead, 'ETC1S->RGBA32 (libktx_read)', t, GL_TEXTURE_2D, { levels: 1, layers: 1 });
  t.delete();

  // KTX1 (unpack alignment 4).
  t = await tex(ktx, 'tests/resources/ktx/r8g8b8a8_srgb.ktx');
  upload(ktx, 'KTX1', t, GL_TEXTURE_2D, { levels: 1, layers: 1 });
  t.delete();

  // ETC2 without WEBGL_compressed_texture_etc enabled: glCompressedTexImage2D fails with
  // INVALID_ENUM and libktx unpacks in software.
  t = await tex(ktx, 'tests/resources/ktx2/r8g8b8a8_srgb_mip_etc2.ktx2');
  upload(ktx, 'ETC2 software unpack', t, GL_TEXTURE_2D, null);
  t.delete();

  // Not transcoded: KTX_INVALID_OPERATION, no texture.
  t = await tex(ktx, 'tests/resources/ktx2/kodim17_blze.ktx2');
  const r = t.glUpload();
  check(!r.object && r.target === 0 && r.error === 0, 'untranscoded BasisLZ is refused');
  t.delete();

  // Discovered capabilities: an ES 3.0 context.
  check(ktx.GL.getContext(ktx.GL.currentContext.handle) === ktx.GL.currentContext,
    'GL.getContext');

  window.__result = { failed: !!log.failed, log };
}

run().catch((e) => {
  log.push('EXCEPTION ' + (e && e.stack || e));
  window.__result = { failed: true, log };
});
