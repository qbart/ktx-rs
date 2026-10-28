#!/bin/sh
# SPDX-FileCopyrightText: 2026 KTX Rust port contributors
# SPDX-License-Identifier: Apache-2.0
#
# Builds one JS module into bindings/js/dist: <name>.js (wasm-bindgen no-modules glue +
# js/embind.js + js/<name>.js inside the factory scope) and <name>.wasm.
# Usage: build.sh <cargo feature> <module name> <factory name>
set -eu
feature=$1 name=$2 factory=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=$here/dist
tmp=$out/.tmp-$name

cargo build -p ktx-rs-wasm --release --target wasm32-unknown-unknown --features "$feature" \
  --manifest-path "$root/Cargo.toml"
rm -rf "$tmp" && mkdir -p "$tmp"
wasm-bindgen --target no-modules --no-typescript --out-dir "$tmp" --out-name "$name" \
  "$root/target/wasm32-unknown-unknown/release/ktx_rs_wasm.wasm"
{
  echo "// $name.js: ktx-rs-wasm $(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1), port of KTX-Software interface/js_binding."
  echo "// SPDX-License-Identifier: Apache-2.0"
  echo "var $factory = (() => {"
  cat "$tmp/$name.js" "$here/js/embind.js" "$here/js/$name.js"
  echo "})();"
  echo "if (typeof exports === 'object' && typeof module === 'object') {"
  echo "  module.exports = $factory;"
  echo "  module.exports.default = $factory;"
  echo "} else if (typeof define === 'function' && define['amd']) {"
  echo "  define([], () => $factory);"
  echo "}"
} > "$out/$name.js"
cp "$tmp/${name}_bg.wasm" "$out/$name.wasm"
rm -rf "$tmp"
