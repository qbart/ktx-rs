#!/usr/bin/env bash
# Regenerates tests/resources/golden/basisu_hdr6x6_parts.txt from the reference basisu ASTC HDR 6x6
# encoder helpers (the harness includes basisu_astc_hdr_6x6_enc.cpp, so it links a library without
# that object and without basisu_comp.o, which references it).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$(mktemp -d)}
export WORK
source "$ROOT/scripts/golden/build_basisu_encoder.sh"
LIB="$WORK/libbasisu_enc_no6x6.a"
if [ ! -f "$LIB" ]; then
  # basisu_comp.cpp holds the miniz implementation; build it on its own.
  echo '#include "basisu_miniz.h"' > "$WORK/miniz_impl.cpp"
  g++ $BASISU_CXXFLAGS -c "$WORK/miniz_impl.cpp" -o "$WORK/miniz_impl.o"
  ar rcs "$LIB" $(ls "$WORK"/basisu_obj/*.o | grep -v -e basisu_astc_hdr_6x6_enc.o -e basisu_comp.o) "$WORK/miniz_impl.o"
fi
g++ $BASISU_CXXFLAGS -I"$ROOT/ref/ktx/external/basis_universal/encoder" "$ROOT/scripts/golden/basisu_hdr6x6_parts_golden.cpp" "$LIB" -lpthread -o "$WORK/basisu_hdr6x6_parts_golden"
"$WORK/basisu_hdr6x6_parts_golden" > "$ROOT/tests/resources/golden/basisu_hdr6x6_parts.txt"
echo "wrote tests/resources/golden/basisu_hdr6x6_parts.txt ($(wc -l < "$ROOT/tests/resources/golden/basisu_hdr6x6_parts.txt") cases)"
