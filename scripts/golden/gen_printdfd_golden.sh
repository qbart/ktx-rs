#!/usr/bin/env bash
# Regenerates tests/resources/golden/printdfd.txt from the original printdfd.c.
# Requires gcc and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
gcc -O2 -w -I"$REF/external/dfdutils" -I"$REF/include" -I"$REF/lib/include" -I"$REF/external" \
  "$ROOT/scripts/golden/printdfd_golden.c" "$REF/external/dfdutils/printdfd.c" \
  -o "$WORK/printdfd_golden"
"$WORK/printdfd_golden" $(ls "$ROOT"/tests/resources/ktx2/*.ktx2 | LC_ALL=C sort) \
  > "$ROOT/tests/resources/golden/printdfd.txt"
echo "wrote tests/resources/golden/printdfd.txt"
