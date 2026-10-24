#!/usr/bin/env bash
# Regenerates tests/resources/golden/astcenc_compress.txt from the astc-encoder in
# ./ref/ktx, built like KTX-Software builds it on x86-64 (AVX2, F16C, POPCNT,
# no FP contraction). Requires g++.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SRC=$ROOT/ref/ktx/external/astc-encoder/Source
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O2 -w -std=c++17 -mavx2 -mpopcnt -mf16c -ffp-contract=off \
  -DASTCENC_NEON=0 -DASTCENC_SVE=0 -DASTCENC_SSE=41 -DASTCENC_AVX=2 -DASTCENC_X86_GATHERS=1 \
  -DASTCENC_POPCNT=1 -DASTCENC_F16C=1 -I"$SRC" "$ROOT/scripts/golden/astcenc_compress_golden.cpp" \
  $(ls "$SRC"/astcenc_*.cpp) -o "$WORK/astcenc_compress_golden" -lpthread
"$WORK/astcenc_compress_golden" > "$ROOT/tests/resources/golden/astcenc_compress.txt"
echo "wrote tests/resources/golden/astcenc_compress.txt ($(wc -l < "$ROOT/tests/resources/golden/astcenc_compress.txt") cases)"
