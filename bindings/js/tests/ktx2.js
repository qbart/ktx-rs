// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// Test helpers: a minimal KTX2 reader (what msc_basis_transcoder users write themselves, see
// the transcoder_wrapper.cpp documentation), FNV-1a 64 and the golden file.
'use strict';

const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..', '..', '..');

function readKtx2(file) {
  const buf = fs.readFileSync(path.join(ROOT, file));
  const data = new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const u32 = (o) => dv.getUint32(o, true);
  const u64 = (o) => Number(dv.getBigUint64(o, true));
  const h = {
    vkFormat: u32(12),
    pixelWidth: u32(20),
    pixelHeight: u32(24),
    pixelDepth: u32(28),
    layerCount: u32(32),
    faceCount: u32(36),
    levelCount: u32(40),
    supercompressionScheme: u32(44),
    dfdByteOffset: u32(48),
    sgdByteOffset: u64(64),
    sgdByteLength: u64(72),
  };
  h.levels = [];
  for (let i = 0; i < Math.max(1, h.levelCount); i++) {
    const o = 80 + i * 24;
    h.levels.push({ byteOffset: u64(o), byteLength: u64(o + 8) });
  }
  // DFD basic block: colorModel and the first sample's channel id.
  const bdb = h.dfdByteOffset + 4;
  h.colorModel = data[bdb + 8];
  h.channelId0 = data[bdb + 24 + 3] & 0x0f;
  return { data, dv, header: h };
}

// BasisLZ supercompression global data (KTX2 spec 3.11.1).
function readBasisLzSgd(ktx) {
  const { dv, header: h } = ktx;
  const o = h.sgdByteOffset;
  const sgd = {
    endpointCount: dv.getUint16(o, true),
    selectorCount: dv.getUint16(o + 2, true),
    endpointsByteLength: dv.getUint32(o + 4, true),
    selectorsByteLength: dv.getUint32(o + 8, true),
    tablesByteLength: dv.getUint32(o + 12, true),
    imageDescs: [],
  };
  let p = o + 20;
  const layers = Math.max(1, h.layerCount);
  for (let level = 0; level < h.levels.length; level++) {
    const depth = Math.max(1, (h.pixelDepth || 1) >> level);
    for (let i = 0; i < layers * h.faceCount * depth; i++) {
      sgd.imageDescs.push({
        imageFlags: dv.getUint32(p, true),
        rgbSliceByteOffset: dv.getUint32(p + 4, true),
        rgbSliceByteLength: dv.getUint32(p + 8, true),
        alphaSliceByteOffset: dv.getUint32(p + 12, true),
        alphaSliceByteLength: dv.getUint32(p + 16, true),
      });
      p += 20;
    }
  }
  sgd.endpointsStart = p;
  sgd.selectorsStart = sgd.endpointsStart + sgd.endpointsByteLength;
  sgd.tablesStart = sgd.selectorsStart + sgd.selectorsByteLength;
  return sgd;
}

function fnv1a64(bytes) {
  let h = 0xcbf29ce484222325n;
  for (const b of bytes) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16).padStart(16, '0');
}

// golden/transcode.tsv: file -> target -> [{image: "level.layer.face", len, hash}] or 'fail'.
function readGolden() {
  const golden = new Map();
  const text = fs.readFileSync(path.join(__dirname, 'golden', 'transcode.tsv'), 'utf8');
  for (const line of text.split('\n')) {
    if (!line) continue;
    const [file, target, image, len, hash] = line.split('\t');
    if (!golden.has(file)) golden.set(file, new Map());
    const byTarget = golden.get(file);
    if (image === 'fail') {
      byTarget.set(target, 'fail');
    } else {
      if (!byTarget.has(target)) byTarget.set(target, []);
      byTarget.get(target).push({ image, len: Number(len), hash });
    }
  }
  return golden;
}

module.exports = { ROOT, readKtx2, readBasisLzSgd, fnv1a64, readGolden };
