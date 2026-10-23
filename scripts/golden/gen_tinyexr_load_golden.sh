#!/usr/bin/env bash
# Regenerates tests/resources/golden/tinyexr_load_corpus.bin and
# tinyexr_load.txt from the tinyexr bundled with basis_universal
# (encoder/3rdparty). Requires g++ with AddressSanitizer and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TINYEXR=$ROOT/ref/ktx/external/basis_universal/encoder/3rdparty
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O1 -g -w -fsanitize=address -fno-omit-frame-pointer -I"$TINYEXR" -I"$TINYEXR/../.." \
  "$ROOT/scripts/golden/tinyexr_load_golden.cpp" -o "$WORK/tinyexr_load_golden"
G=$ROOT/tests/resources/golden
"$WORK/tinyexr_load_golden" gen "$G/tinyexr_load_corpus.bin"
cd "$ROOT"
"$WORK/tinyexr_load_golden" run "$G/tinyexr_load_corpus.bin" \
  tests/resources/input/exr/Desk.exr tests/resources/cts/input/exr/*.exr tests/resources/cts/input/exr_sample/*.exr \
  > "$G/tinyexr_load.txt"
echo "wrote $G/tinyexr_load.txt ($(wc -l < "$G/tinyexr_load.txt") cases)"
