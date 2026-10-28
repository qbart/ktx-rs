// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// Emscripten / embind compatible surface over the wasm-bindgen exports, shared by every
// module (docs/spec/js_binding.md). Concatenated after the wasm-bindgen no-modules glue and
// before the module's own file, inside the module's factory scope.

// Directory of this script: the default location of the .wasm file.
const __ktxScriptDir = (() => {
  if (typeof document !== 'undefined' && document.currentScript) {
    return document.currentScript.src.replace(/[^/]*$/, '');
  }
  if (typeof __dirname === 'string') {
    return __dirname + '/';
  }
  return '';
})();

const __ktxIsNode = typeof process === 'object' && typeof process.versions === 'object' &&
  typeof process.versions.node === 'string' && typeof window === 'undefined';

// Instantiates the wasm (once; wasm-bindgen keeps a single instance) like Emscripten does:
// Module.wasmBinary, then Module.locateFile(name, scriptDirectory), then <scriptDir>/<name>.
async function __ktxInstantiate(name, moduleArg) {
  let source = moduleArg.wasmBinary;
  if (source === undefined) {
    const file = name + '.wasm';
    let path = typeof moduleArg.locateFile === 'function'
      ? moduleArg.locateFile(file, __ktxScriptDir)
      : __ktxScriptDir + file;
    if (__ktxIsNode && !/^(https?|data|blob):/.test(path)) {
      source = require('fs').readFileSync(path);
    } else {
      source = path;
    }
  }
  await wasm_bindgen({ module_or_path: source });
}

// embind enum_: a function object holding one frozen {value} entry per name and the
// value -> entry map in `values`.
function __ktxEnum(name, values) {
  const E = function () {};
  Object.defineProperty(E, 'name', { value: name });
  E.values = {};
  for (const key of Object.keys(values)) {
    const entry = Object.create(E.prototype);
    entry.value = values[key];
    Object.freeze(entry);
    E[key] = entry;
    E.values[entry.value] = entry;
  }
  return E;
}

// The numeric value of an embind enum entry; plain numbers pass through, undefined is 0.
function __ktxVal(x) {
  if (x !== null && typeof x === 'object' && 'value' in x) {
    return x.value;
  }
  return x === undefined ? 0 : x;
}

// embind ClassHandle methods on a wasm-bindgen class: delete() frees the Rust object.
function __ktxClass(C) {
  C.prototype.delete = function () {
    if (this.__wbg_ptr !== 0) {
      this.free();
    }
  };
  C.prototype.isDeleted = function () {
    return this.__wbg_ptr === 0;
  };
  return C;
}

// Emscripten MODULARIZE: the factory resolves with moduleArg filled in, after calling
// Module.onRuntimeInitialized.
async function __ktxModule(name, moduleArg, populate) {
  const Module = moduleArg || {};
  await __ktxInstantiate(name, Module);
  populate(Module);
  if (typeof Module.onRuntimeInitialized === 'function') {
    Module.onRuntimeInitialized();
  }
  return Module;
}
