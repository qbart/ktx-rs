#!/usr/bin/env bash
# Regenerates tests/resources/golden/info.txt from the original lib/src/info.c.
# Requires gcc and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
S=$REF/lib/src
gcc -O2 -w -DKTX_FEATURE_KTX1 -DKTX_FEATURE_KTX2 -DLIBKTX \
  -I"$REF/include" -I"$S" -I"$REF/lib/include" -I"$REF/external" -I"$REF/other_include" -I"$REF/utils" \
  -I"$REF/external/dfdutils" -I"$REF/external/basis_universal/zstd" \
  "$ROOT/scripts/golden/info_golden.c" "$S/info.c" "$S/hashlist.c" "$S/checkheader.c" \
  "$S/swap.c" "$S/memstream.c" "$S/filestream.c" "$S/strings.c" "$S/glformat_str.c" \
  "$REF/external/dfdutils/printdfd.c" "$S/vkformat_str.c" "$S/vkformat_check.c" \
  -o "$WORK/info_golden"
R=$ROOT/tests/resources
"$WORK/info_golden" "$R/ktx/r8g8b8a8_srgb.ktx" "$R/ktx/orient_up_metadata.ktx" "$R/ktx2/r8g8b8a8_srgb.ktx2" $( (ls "$ROOT"/tests/resources/ktx/*.ktx; ls "$ROOT"/tests/resources/ktx2/*.ktx2) | LC_ALL=C sort) \
  > "$ROOT/tests/resources/golden/info.txt"
echo "wrote tests/resources/golden/info.txt"
