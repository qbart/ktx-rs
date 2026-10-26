#!/usr/bin/env bash
# Regenerates tests/resources/golden/transcode_basis.txt (sha256 + size of the
# reference Basis Universal transcodes). Requires g++ and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
BU=$REF/external/basis_universal
# BASISD_SUPPORT_FXT1=0 as in lib/CMakeLists.txt.
g++ -O2 -std=c++17 -w \
  -DBASISD_SUPPORT_KTX2=0 -DBASISD_SUPPORT_KTX2_ZSTD=0 -DBASISD_SUPPORT_FXT1=0 \
  -I"$BU" -I"$BU/transcoder" -I"$REF/interface/basisu_c_binding/inc" -I"$REF/lib/src" -I"$REF/lib/include" -I"$REF/other_include" -I"$REF/utils" \
  "$ROOT/scripts/golden/transcode_golden.cpp" \
  "$REF/interface/basisu_c_binding/src/basisu_c_binding.cpp" \
  "$BU/transcoder/basisu_transcoder.cpp" \
  -o "$WORK/transcode_golden"
mkdir -p "$WORK/out"
"$WORK/transcode_golden" "$REF/tests/resources/basis" "$WORK/out"
mkdir -p "$ROOT/tests/resources/golden"
( cd "$WORK/out" && for f in $(ls | sort); do
    echo "$f $(stat -c %s "$f") $(sha256sum "$f" | cut -d' ' -f1)"
  done ) > "$ROOT/tests/resources/golden/transcode_basis.txt"
echo "wrote tests/resources/golden/transcode_basis.txt"

# UASTC LDR 4x4 (libktx transcodeUastcLDR4x4 call path).
gcc -O2 -w -c "$BU/zstd/zstddeclib.c" -o "$WORK/zstddeclib.o"
g++ -O2 -std=c++17 -w \
  -DBASISD_SUPPORT_KTX2=0 -DBASISD_SUPPORT_KTX2_ZSTD=0 \
  -I"$BU" -I"$BU/transcoder" -I"$BU/zstd" \
  "$ROOT/scripts/golden/uastc_golden.cpp" \
  "$BU/transcoder/basisu_transcoder.cpp" "$WORK/zstddeclib.o" \
  -o "$WORK/uastc_golden"
rm -rf "$WORK/uout" && mkdir -p "$WORK/uout"
"$WORK/uastc_golden" "$REF/tests/resources/ktx2" "$WORK/uout"
( cd "$WORK/uout" && for f in $(ls | sort); do
    echo "$f $(stat -c %s "$f") $(sha256sum "$f" | cut -d' ' -f1)"
  done ) > "$ROOT/tests/resources/golden/transcode_uastc.txt"
echo "wrote tests/resources/golden/transcode_uastc.txt"

# UASTC HDR (libktx transcodeUastcHDR4x4 call path).
g++ -O2 -std=c++17 -w \
  -DBASISD_SUPPORT_KTX2=0 -DBASISD_SUPPORT_KTX2_ZSTD=0 \
  -I"$BU" -I"$BU/transcoder" -I"$BU/zstd" \
  "$ROOT/scripts/golden/hdr_golden.cpp" \
  "$BU/transcoder/basisu_transcoder.cpp" "$WORK/zstddeclib.o" \
  -o "$WORK/hdr_golden"
rm -rf "$WORK/hout" && mkdir -p "$WORK/hout"
"$WORK/hdr_golden" "$REF/tests/resources/ktx2" "$WORK/hout"
( cd "$WORK/hout" && for f in $(ls | sort); do
    echo "$f $(stat -c %s "$f") $(sha256sum "$f" | cut -d' ' -f1)"
  done ) > "$ROOT/tests/resources/golden/transcode_uastc_hdr.txt"
echo "wrote tests/resources/golden/transcode_uastc_hdr.txt"
