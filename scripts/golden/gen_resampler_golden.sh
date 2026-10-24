#!/usr/bin/env bash
# Regenerates tests/resources/golden/resampler.txt from basis_universal's resampler.
# Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ENC=$ROOT/ref/ktx/external/basis_universal/encoder
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -std=c++17 -O2 -w -DNDEBUG -I"$ENC" "$ROOT/scripts/golden/resampler_golden.cpp" \
  "$ENC/basisu_resampler.cpp" "$ENC/basisu_resample_filters.cpp" -o "$WORK/resampler_golden"
"$WORK/resampler_golden" > "$ROOT/tests/resources/golden/resampler.txt"
echo "wrote tests/resources/golden/resampler.txt ($(wc -l < "$ROOT/tests/resources/golden/resampler.txt") cases)"
