#!/usr/bin/env bash
# Regenerates tests/resources/golden/zstd.txt from the libzstd 1.5.7
# amalgamation bundled with basisu (the zstd libktx links). Requires gcc and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ZSTD=$ROOT/ref/ktx/external/basis_universal/zstd
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
gcc -O2 -w -I"$ZSTD" "$ROOT/scripts/golden/zstd_golden.c" "$ZSTD/zstd.c" -o "$WORK/zstd_golden" -lpthread
R=$ROOT/tests/resources
"$WORK/zstd_golden" "$R/ktx2/ktx_app_astc_8x8.ktx2" "$R/ktx2/bc3_unorm_array_7.ktx2" "$R/ktx2/r8g8b8_srgb_mip.ktx2" \
  "$R/ktx2/r8g8b8a8_srgb_array_7_mip.ktx2" "$R/ktx2/r8g8b8a8_srgb_3d_7.ktx2" > "$R/golden/zstd.txt"
echo "wrote tests/resources/golden/zstd.txt"
