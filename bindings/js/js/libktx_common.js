// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// EMSCRIPTEN_BINDINGS(ktx) of ktx_wrapper.cpp, shared by libktx.js and libktx_read.js. Runs
// inside the factory scope after the wasm-bindgen glue and embind.js.

function __ktxLibktxPopulate(Module) {
  const g = wasm_bindgen;
  const write = typeof g.__constants === 'function';
  const enums = g.__ktxEnums();
  for (const name of Object.keys(enums)) {
    Module[name] = __ktxEnum(name, enums[name]);
  }
  const err = (code) => Module.error_code.values[code];
  // Rust None comes back as undefined; the C++ returns val::null().
  const orNull = (v) => (v === undefined ? null : v);
  // Module.GL (Emscripten GL emulation, js/gl.js) and the commands glUpload runs on. A WebGL
  // exception is kept until the upload returns so it never unwinds through the wasm frames.
  const { GL, commands } = __ktxGL(Module);
  Module.GL = GL;
  let pendingException = null;
  const guardedCommands = {};
  for (const [name, f] of Object.entries(commands)) {
    guardedCommands[name] = (...args) => {
      try {
        return f(...args);
      } catch (e) {
        if (pendingException === null) pendingException = e;
        GL.recordError(0x502 /* GL_INVALID_OPERATION */);
        return undefined;
      }
    };
  }

  const readOnly = (name) => () => {
    throw new Error(`texture.${name} is a read-only property`);
  };

  class texture extends __ktxClass(g.texture) {
    constructor(...args) {
      super();
      if (args.length === 1) {
        this._load(args[0]);
      } else if (write && args.length === 2) {
        this._create(args[0], __ktxVal(args[1]));
      } else {
        throw new Error('Tried to invoke ctor of texture with invalid number of parameters (' +
          args.length + ') - expected (' + (write ? '1 or 2' : '1') + ') parameters instead!');
      }
    }

    createCopy() {
      return Object.setPrototypeOf(super.createCopy(), texture.prototype);
    }

    get colorModel() {
      return Module.khr_df_model.values[this._colorModel];
    }
    get transferFunction() {
      return Module.khr_df_transfer.values[this._transferFunction];
    }
    set transferFunction(v) {
      write ? this._set_transfer_function(__ktxVal(v)) : readOnly('transferFunction')();
    }
    get oetf() {
      return Module.khr_df_transfer.values[this._oetf];
    }
    set oetf(v) {
      write ? this._set_oetf(__ktxVal(v)) : readOnly('oetf')();
    }
    get primaries() {
      return Module.khr_df_primaries.values[this._primaries];
    }
    set primaries(v) {
      write ? this._set_primaries(__ktxVal(v)) : readOnly('primaries')();
    }
    get supercompressScheme() {
      return Module.SupercmpScheme.values[this._supercompressScheme];
    }
    // value_object<ktxOrientation>: a fresh plain object of enum entries.
    get orientation() {
      const [x, y, z] = this._orientation;
      return {
        x: Module.OrientationX.values[x],
        y: Module.OrientationY.values[y],
        z: Module.OrientationZ.values[z],
      };
    }

    findKeyValue(key) {
      return orNull(super.findKeyValue(key));
    }
    getImage(level, layer, faceSlice) {
      return orNull(super.getImage(level, layer, faceSlice));
    }

    // NOTE: WebGLTexture objects are completely opaque so the option of passing in the
    // texture to use is not viable.
    glUpload() {
      if (!GL.currentContext) {
        throw new Error('glUpload: no current WebGL context; call GL.makeContextCurrent(' +
          'GL.createContext(canvas, attributes)) first');
      }
      pendingException = null;
      const [texname, target, error] = this._gl_upload(guardedCommands);
      if (pendingException !== null) {
        const e = pendingException;
        pendingException = null;
        throw e;
      }
      return { object: GL.textures[texname], target, error };
    }

    transcodeBasis(targetFormat, decodeFlags) {
      return err(this._transcode_basis(__ktxVal(targetFormat), __ktxVal(decodeFlags)));
    }
    decodeAstc() {
      return err(this._decode_astc());
    }
  }

  if (write) {
    Object.assign(texture.prototype, {
      compressAstc(params) {
        return err(this._compress_astc(params));
      },
      compressBasis(params) {
        return err(this._compress_basis(params));
      },
      deflateZstd(level) {
        return err(this._deflate_zstd(level));
      },
      deflateZLIB(level) {
        return err(this._deflate_zlib(level));
      },
      addKVPairString(key, value) {
        return err(this._add_kv_pair_string(key, value));
      },
      addKVPairByte(key, value) {
        return err(this._add_kv_pair_byte(key, value));
      },
      deleteKVPair(key) {
        return err(this._delete_kv_pair(key));
      },
      writeToMemory() {
        return orNull(g.texture.prototype.writeToMemory.call(this));
      },
      setImageFromMemory(level, layer, faceSlice, imageData) {
        return err(g.texture.prototype.setImageFromMemory.call(this, level, layer, faceSlice,
          imageData));
      },
    });

    class textureCreateInfo extends __ktxClass(g.textureCreateInfo) {
      get vkFormat() {
        return this._vkFormat;
      }
      set vkFormat(v) {
        this._vkFormat = __ktxVal(v);
      }
    }
    Module.textureCreateInfo = textureCreateInfo;

    // Enum typed fields: set with an entry (or number), read back as a number.
    const enumProps = (C, names) => {
      for (const name of names) {
        const base = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(C.prototype), name);
        Object.defineProperty(C.prototype, name, {
          get: base.get,
          set(v) {
            this['_' + name] = __ktxVal(v);
          },
          configurable: true,
        });
      }
      return C;
    };
    Module.astcParams = enumProps(class astcParams extends __ktxClass(g.astcParams) {},
      ['blockDimension', 'mode', 'qualityLevel']);
    Module.basisParams = enumProps(class basisParams extends __ktxClass(g.basisParams) {},
      ['uastcFlags']);

    Object.assign(Module, g.__constants());
  }
  Module.texture = texture;

  // class_compat.js (--pre-js): replaces any user onRuntimeInitialized, as upstream does.
  Module.onRuntimeInitialized = function () {
    Module['ktxTexture'] = Module.texture;
    Module['ErrorCode'] = Module.error_code;
    Module['TranscodeTarget'] = Module.transcode_fmt;
    Module['TranscodeFlags'] = Module.transcode_flag_bits;
  };
}
