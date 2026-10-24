#!/usr/bin/env bash
# Regenerates tests/resources/golden/image.txt from tools/imageio/image.hpp
# (with basis_universal's resampler, glm and fmt). Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
ENC=$REF/external/basis_universal/encoder
INC=(-I"$REF/tools/imageio" -I"$REF/utils" -I"$REF/lib/include" -I"$REF/external/dfdutils" -I"$REF/external/fmt/include"
     -I"$REF/external/basis_universal" -I"$REF/other_include")
g++ -std=c++17 -O1 -w -DFMT_HEADER_ONLY -DNDEBUG "${INC[@]}" "$ROOT/scripts/golden/image_golden.cpp" \
  "$ENC/basisu_resampler.cpp" "$ENC/basisu_resample_filters.cpp" -o "$WORK/image_golden"
"$WORK/image_golden" > "$ROOT/tests/resources/golden/image.txt"
echo "wrote tests/resources/golden/image.txt ($(wc -l < "$ROOT/tests/resources/golden/image.txt") cases)"
