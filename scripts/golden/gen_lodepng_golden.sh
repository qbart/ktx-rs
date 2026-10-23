#!/usr/bin/env bash
# Regenerates tests/resources/golden/lodepng.txt and lodepng_decode.txt from the original lodepng
# (external/lodepng of KTX-Software). Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LODEPNG=$ROOT/ref/ktx/external/lodepng
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O2 -w -I"$LODEPNG" "$ROOT/scripts/golden/lodepng_golden.cpp" "$LODEPNG/lodepng.cpp" -o "$WORK/lodepng_golden"
"$WORK/lodepng_golden" > "$ROOT/tests/resources/golden/lodepng.txt"
echo "wrote tests/resources/golden/lodepng.txt"
g++ -O2 -w -I"$LODEPNG" "$ROOT/scripts/golden/lodepng_decode_golden.cpp" "$LODEPNG/lodepng.cpp" -o "$WORK/lodepng_decode_golden"
"$WORK/lodepng_decode_golden" > "$ROOT/tests/resources/golden/lodepng_decode.txt"
echo "wrote tests/resources/golden/lodepng_decode.txt"
