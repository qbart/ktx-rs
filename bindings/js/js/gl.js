// SPDX-FileCopyrightText: 2010-2026 Emscripten authors
// SPDX-License-Identifier: MIT
// Derived from Emscripten src/library_webgl.js (MIT / University of Illinois NCSA) and
// modified for the Rust port of KTX-Software; see NOTICE.md.
//
// The part of Emscripten's GL emulation (library_webgl.js, built with
// GL_PREINITIALIZED_CONTEXT and EXPORTED_RUNTIME_METHODS=['GL']) that libktx.js uses: Module.GL
// with its context and texture name tables, and the glX commands ktxTexture_GLUpload calls on
// the current context. Runs inside the factory scope after embind.js.

function __ktxGL(Module) {
  const GL = {
    counter: 1,
    contexts: [],
    textures: [],
    currentContext: null,
    lastError: 0,
    stringCache: {},
    stringiCache: {},

    getNewId(table) {
      const ret = GL.counter++;
      for (let i = table.length; i < ret; i++) {
        table[i] = null;
      }
      return ret;
    },

    recordError(errorCode) {
      if (!GL.lastError) {
        GL.lastError = errorCode;
      }
    },

    // With GL_PREINITIALIZED_CONTEXT the context given as Module.preinitializedWebGLContext is
    // used instead of creating one on the canvas.
    createContext(canvas, webGLContextAttributes) {
      const attributes = Object.assign({ majorVersion: 1 }, webGLContextAttributes);
      let ctx = Module.preinitializedWebGLContext;
      if (ctx) {
        attributes.majorVersion = typeof WebGL2RenderingContext !== 'undefined' &&
          ctx instanceof WebGL2RenderingContext ? 2 : 1;
      } else {
        ctx = attributes.majorVersion > 1
          ? canvas.getContext('webgl2', attributes)
          : canvas.getContext('webgl', attributes);
      }
      if (!ctx) {
        return 0;
      }
      return GL.registerContext(ctx, attributes);
    },

    registerContext(ctx, webGLContextAttributes) {
      const handle = GL.getNewId(GL.contexts);
      const context = {
        handle,
        attributes: webGLContextAttributes,
        version: webGLContextAttributes.majorVersion,
        GLctx: ctx,
      };
      if (ctx.canvas) {
        ctx.canvas.GLctxObject = context;
      }
      GL.contexts[handle] = context;
      return handle;
    },

    makeContextCurrent(contextHandle) {
      GL.currentContext = GL.contexts[contextHandle] || null;
      Module.ctx = GL.currentContext ? GL.currentContext.GLctx : undefined;
      return !(contextHandle && !Module.ctx);
    },

    getContext(contextHandle) {
      return GL.contexts[contextHandle];
    },

    deleteContext(contextHandle) {
      if (GL.currentContext === GL.contexts[contextHandle]) {
        GL.currentContext = null;
      }
      GL.contexts[contextHandle] = null;
    },
  };

  const ctx = () => GL.currentContext.GLctx;

  // webglGetExtensions: each extension unprefixed and again with "GL_".
  const extensions = () => {
    const exts = ctx().getSupportedExtensions() || [];
    return exts.concat(exts.map((e) => 'GL_' + e));
  };

  // heapObjectForWebGLType: the typed array type WebGL 2 expects for a pixel type.
  const viewForType = (type, bytes) => {
    let C;
    switch (type - 0x1400) {
      case 0: C = Int8Array; break;
      case 1: C = Uint8Array; break;
      case 2: C = Int16Array; break;
      case 4: C = Int32Array; break;
      case 6: C = Float32Array; break;
      case 5: case 28922: case 28520: case 30779: case 30782: C = Uint32Array; break;
      default: C = Uint16Array; break;
    }
    if (bytes.byteOffset % C.BYTES_PER_ELEMENT !== 0) {
      bytes = bytes.slice();
    }
    return new C(bytes.buffer, bytes.byteOffset,
      Math.floor(bytes.byteLength / C.BYTES_PER_ELEMENT));
  };

  // emscripten_GetProcAddress resolves every GLES 2 / 3 command; GLES has no 1D textures.
  const missing = new Set(['glTexImage1D', 'glCompressedTexImage1D', 'glTexStorage1D',
    'glTexSubImage1D', 'glCompressedTexSubImage1D']);

  const commands = {
    hasCommand: (name) => !missing.has(name),

    glBindTexture: (target, texture) => ctx().bindTexture(target, GL.textures[texture] || null),

    glGenTexture: () => {
      const texture = ctx().createTexture();
      if (!texture) {
        GL.recordError(0x502 /* GL_INVALID_OPERATION */);
        return 0;
      }
      const id = GL.getNewId(GL.textures);
      texture.name = id;
      GL.textures[id] = texture;
      return id;
    },

    glDeleteTexture: (id) => {
      const texture = GL.textures[id];
      if (!texture) return;
      ctx().deleteTexture(texture);
      texture.name = 0;
      GL.textures[id] = null;
    },

    glGetError: () => {
      const error = ctx().getError() || GL.lastError;
      GL.lastError = 0;
      return error;
    },

    // emscriptenWebGLGet for one integer; undefined when an error is recorded.
    glGetIntegerv: (name) => {
      switch (name) {
        case 0x821D: // GL_NUM_EXTENSIONS
          if (GL.currentContext.version < 2) {
            GL.recordError(0x502 /* GL_INVALID_OPERATION */);
            return undefined;
          }
          return 2 * (ctx().getSupportedExtensions() || []).length;
        case 0x821B: // GL_MAJOR_VERSION
        case 0x821C: // GL_MINOR_VERSION
          if (GL.currentContext.version < 2) {
            GL.recordError(0x500 /* GL_INVALID_ENUM */);
            return undefined;
          }
          return name === 0x821B ? 3 : 0;
      }
      const result = ctx().getParameter(name);
      switch (typeof result) {
        case 'number': return result | 0;
        case 'boolean': return result ? 1 : 0;
        default:
          GL.recordError(0x500 /* GL_INVALID_ENUM */);
          return undefined;
      }
    },

    glGetString: (name) => {
      let ret = GL.stringCache[name];
      if (ret === undefined) {
        switch (name) {
          case 0x1F03: // GL_EXTENSIONS
            ret = extensions().join(' ');
            break;
          case 0x1F00: // GL_VENDOR
          case 0x1F01: // GL_RENDERER
            ret = ctx().getParameter(name) || null;
            if (!ret) GL.recordError(0x500 /* GL_INVALID_ENUM */);
            break;
          case 0x1F02: { // GL_VERSION
            const webGLVersion = ctx().getParameter(0x1F02);
            ret = GL.currentContext.version >= 2
              ? `OpenGL ES 3.0 (${webGLVersion})` : `OpenGL ES 2.0 (${webGLVersion})`;
            break;
          }
          default:
            GL.recordError(0x500 /* GL_INVALID_ENUM */);
            ret = null;
        }
        GL.stringCache[name] = ret;
      }
      return ret === null ? undefined : ret;
    },

    glGetStringi: (name, index) => {
      if (GL.currentContext.version < 2) {
        GL.recordError(0x502 /* GL_INVALID_OPERATION */);
        return undefined;
      }
      let cache = GL.stringiCache[name];
      if (!cache) {
        if (name !== 0x1F03 /* GL_EXTENSIONS */) {
          GL.recordError(0x500 /* GL_INVALID_ENUM */);
          return undefined;
        }
        cache = GL.stringiCache[name] = extensions();
      }
      if (index < 0 || index >= cache.length) {
        GL.recordError(0x501 /* GL_INVALID_VALUE */);
        return undefined;
      }
      return cache[index];
    },

    glPixelStorei: (pname, param) => {
      if (pname === 0xCF5 /* GL_UNPACK_ALIGNMENT */) {
        GL.unpackAlignment = param;
      }
      ctx().pixelStorei(pname, param);
    },

    glTexParameteri: (target, pname, param) => ctx().texParameteri(target, pname, param),

    glGenerateMipmap: (target) => ctx().generateMipmap(target),

    glTexImage2D: (target, level, internalFormat, width, height, border, format, type,
      pixels) => {
      ctx().texImage2D(target, level, internalFormat, width, height, border, format, type,
        pixels.length ? viewForType(type, pixels) : null);
    },

    glCompressedTexImage2D: (target, level, internalFormat, width, height, border, imageSize,
      data) => {
      ctx().compressedTexImage2D(target, level, internalFormat, width, height, border,
        data.subarray(0, imageSize));
    },

    glTexImage3D: (target, level, internalFormat, width, height, depth, border, format, type,
      pixels) => {
      ctx().texImage3D(target, level, internalFormat, width, height, depth, border, format,
        type, pixels.length ? viewForType(type, pixels) : null);
    },

    glCompressedTexImage3D: (target, level, internalFormat, width, height, depth, border,
      imageSize, data) => {
      ctx().compressedTexImage3D(target, level, internalFormat, width, height, depth, border,
        data.subarray(0, imageSize));
    },
  };

  return { GL, commands };
}
