#!/usr/bin/env bash
# Regenerates tests/resources/golden/etcunpack.txt from the reference _ktxUnpackETC + etcdec.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
g++ -O1 -w -DSUPPORT_SOFTWARE_ETC_UNPACK=1 -I"$REF/lib/src" -I"$REF/lib/include" -I"$REF/other_include" -I"$REF/external/dfdutils" \
  "$ROOT/scripts/golden/etcunpack_golden.cpp" "$REF/lib/src/etcunpack.cxx" "$REF/external/etcdec/etcdec.cxx" \
  -o "$WORK/etcunpack_golden"
"$WORK/etcunpack_golden" > "$ROOT/tests/resources/golden/etcunpack.txt"
echo "wrote tests/resources/golden/etcunpack.txt ($(wc -l < "$ROOT/tests/resources/golden/etcunpack.txt") cases)"
