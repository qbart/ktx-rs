#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_comp.txt from the reference basisu basis_compressor.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_comp_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_comp_golden"
"$WORK/basisu_comp_golden" > "$ROOT/tests/resources/golden/basisu_comp.txt"
echo "wrote tests/resources/golden/basisu_comp.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_comp.txt") cases)"
