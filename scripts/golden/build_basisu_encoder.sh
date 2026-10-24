#!/usr/bin/env bash
# Builds the reference Basis Universal encoder + transcoder as $WORK/libbasisu_enc.a with the
# configuration libktx uses on x86_64 (SSE 4.1 kernels, zstd, no OpenCL). Sourced by the
# basisu encoder golden generators.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
REF=$ROOT/ref/ktx
BU=$REF/external/basis_universal
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK/basisu_obj"
BASISU_CXXFLAGS="-O2 -std=c++17 -w -msse4.1 -DBASISU_SUPPORT_SSE=1 -DBASISU_SUPPORT_OPENCL=0 -DBASISD_SUPPORT_KTX2=1 -DBASISD_SUPPORT_KTX2_ZSTD=1 -I$BU -I$BU/transcoder -I$BU/encoder -I$BU/zstd"
if [ ! -f "$WORK/libbasisu_enc.a" ]; then
  srcs=$(ls "$BU"/encoder/*.cpp | grep -v -e wasm -e pvpngreader)
  pids=()
  for s in $srcs "$BU/transcoder/basisu_transcoder.cpp" "$BU/encoder/pvpngreader.cpp" "$BU"/encoder/3rdparty/*.cpp; do
    o="$WORK/basisu_obj/$(basename "$s" .cpp).o"
    g++ $BASISU_CXXFLAGS -c "$s" -o "$o" &
    pids+=($!)
  done
  gcc -O2 -w -c "$BU/zstd/zstd.c" -o "$WORK/basisu_obj/zstd.o" &
  pids+=($!)
  for p in "${pids[@]}"; do wait "$p"; done
  ar rcs "$WORK/libbasisu_enc.a" "$WORK"/basisu_obj/*.o
fi
