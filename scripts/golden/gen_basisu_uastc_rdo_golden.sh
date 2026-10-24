#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_uastc_rdo.txt from the reference basisu UASTC encoder.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_uastc_rdo_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_uastc_rdo_golden"
"$WORK/basisu_uastc_rdo_golden" > "$ROOT/tests/resources/golden/basisu_uastc_rdo.txt"
echo "wrote tests/resources/golden/basisu_uastc_rdo.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_uastc_rdo.txt") cases)"
