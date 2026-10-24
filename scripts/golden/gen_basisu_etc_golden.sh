#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_etc.txt from the reference basisu ETC1 encoder.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_etc_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_etc_golden"
"$WORK/basisu_etc_golden" > "$ROOT/tests/resources/golden/basisu_etc.txt"
echo "wrote tests/resources/golden/basisu_etc.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_etc.txt") cases)"
