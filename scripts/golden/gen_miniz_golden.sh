#!/usr/bin/env bash
# Regenerates tests/resources/golden/miniz.txt from the original basisu_miniz.h
# (x86-64 configuration). Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O2 -w -I"$REF/external/basis_universal" "$ROOT/scripts/golden/miniz_golden.cpp" -o "$WORK/miniz_golden"
R=$ROOT/tests/resources
"$WORK/miniz_golden" "$R/ktx2/r8g8b8a8_srgb.ktx2" "$R/ktx2/bc3_unorm_array_7.ktx2" "$R/ktx2/astc_8x8_unorm_array_7.ktx2" \
  "$R/ktx/orient_up_metadata.ktx" > "$R/golden/miniz.txt"
echo "wrote tests/resources/golden/miniz.txt"
