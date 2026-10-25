#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_uastc_hdr_4x4_enc.txt from the reference basisu UASTC HDR 4x4 encoder.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_uastc_hdr_4x4_enc_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_uastc_hdr_4x4_enc_golden"
"$WORK/basisu_uastc_hdr_4x4_enc_golden" > "$ROOT/tests/resources/golden/basisu_uastc_hdr_4x4_enc.txt"
echo "wrote tests/resources/golden/basisu_uastc_hdr_4x4_enc.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_uastc_hdr_4x4_enc.txt") cases)"
