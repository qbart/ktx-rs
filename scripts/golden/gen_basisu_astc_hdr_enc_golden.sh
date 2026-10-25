#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_astc_hdr_enc.txt from the reference basisu ASTC HDR common encoder.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_astc_hdr_enc_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_astc_hdr_enc_golden"
"$WORK/basisu_astc_hdr_enc_golden" > "$ROOT/tests/resources/golden/basisu_astc_hdr_enc.txt"
echo "wrote tests/resources/golden/basisu_astc_hdr_enc.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_astc_hdr_enc.txt") cases)"
