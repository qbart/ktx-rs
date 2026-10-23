#!/usr/bin/env bash
# Regenerates tests/resources/golden/tinyexr.txt from the tinyexr bundled with
# basis_universal (encoder/3rdparty). Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TINYEXR=$ROOT/ref/ktx/external/basis_universal/encoder/3rdparty
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O2 -w -I"$TINYEXR" -I"$TINYEXR/../.." "$ROOT/scripts/golden/tinyexr_golden.cpp" "$TINYEXR/tinyexr.cpp" \
  -o "$WORK/tinyexr_golden" -lpthread
"$WORK/tinyexr_golden" > "$ROOT/tests/resources/golden/tinyexr.txt"
echo "wrote tests/resources/golden/tinyexr.txt"
