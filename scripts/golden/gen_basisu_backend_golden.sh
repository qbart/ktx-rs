#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_backend.txt from the reference basisu ETC1S backend.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
g++ $BASISU_CXXFLAGS "$ROOT/scripts/golden/basisu_backend_golden.cpp" "$WORK/libbasisu_enc.a" -lpthread -o "$WORK/basisu_backend_golden"
"$WORK/basisu_backend_golden" > "$ROOT/tests/resources/golden/basisu_backend.txt"
echo "wrote tests/resources/golden/basisu_backend.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_backend.txt") cases)"
