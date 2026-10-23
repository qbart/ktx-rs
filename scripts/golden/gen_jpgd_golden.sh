#!/usr/bin/env bash
# Regenerates tests/resources/golden/jpgd.txt from basis_universal's jpgd.
# Requires g++ with AddressSanitizer and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ENC=$ROOT/ref/ktx/external/basis_universal/encoder
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -std=c++17 -O1 -g -w -DNDEBUG -fsanitize=address -fno-omit-frame-pointer -I"$ENC" \
  "$ROOT/scripts/golden/jpgd_golden.cpp" "$ENC/jpgd.cpp" -o "$WORK/jpgd_golden"
cd "$ROOT"
"$WORK/jpgd_golden" $(ls tests/resources/imageio/jpg/*.jpg tests/resources/input/jpg/*.jpg | LC_ALL=C sort) \
  > tests/resources/golden/jpgd.txt
echo "wrote tests/resources/golden/jpgd.txt ($(wc -l < tests/resources/golden/jpgd.txt) cases)"
