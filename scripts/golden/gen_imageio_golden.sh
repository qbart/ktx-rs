#!/usr/bin/env bash
# Regenerates tests/resources/golden/imageio.txt from KTX-Software's imageio
# readers (tools/imageio, lodepng, tinyexr, dfdutils, fmt). Requires g++ with
# AddressSanitizer and ./ref/ktx.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
INC=(-I"$REF/tools/imageio" -I"$REF/utils" -I"$REF/external" -I"$REF/external/dfdutils" -I"$REF/lib/include"
     -I"$REF/lib/src" -I"$REF/external/fmt/include" -I"$REF/external/basis_universal" -I"$REF/other_include")
FLAGS=(-O1 -g -w -fsanitize=address -fno-omit-frame-pointer -DFMT_HEADER_ONLY -DDISABLE_NPBM -DNDEBUG)
echo '#include "encoder/basisu_miniz.h"' > "$WORK/miniz_impl.cpp"
for f in imageinput.cc imageoutput.cc imageio.cc png.imageio/pnginput.cc png.imageio/pngoutput.cc exr.imageio/exrinput.cc jpg.imageio/jpginput.cc; do
  g++ -std=c++17 "${FLAGS[@]}" "${INC[@]}" -c "$REF/tools/imageio/$f" -o "$WORK/$(basename "$f").o"
done
g++ -std=c++17 "${FLAGS[@]}" "${INC[@]}" -c "$REF/external/lodepng/lodepng.cpp" -o "$WORK/lodepng.o"
g++ -std=c++17 "${FLAGS[@]}" "${INC[@]}" -c "$WORK/miniz_impl.cpp" -o "$WORK/miniz.o"
g++ -std=c++17 "${FLAGS[@]}" "${INC[@]}" -c "$REF/external/basis_universal/encoder/jpgd.cpp" -o "$WORK/jpgd.o"
for f in colourspaces.c vk2dfd.c createdfd.c; do
  gcc "${FLAGS[@]}" "${INC[@]}" -c "$REF/external/dfdutils/$f" -o "$WORK/$f.o"
done
g++ -std=c++17 "${FLAGS[@]}" "${INC[@]}" "$ROOT/scripts/golden/imageio_golden.cpp" "$WORK"/*.o -o "$WORK/imageio_golden"
cd "$ROOT"
FILES=$(ls tests/resources/cts/input/png/*.png tests/resources/cts/input/png_sample/*.png \
  tests/resources/cts/input/exr/*.exr tests/resources/cts/input/exr_sample/*.exr tests/resources/input/exr/Desk.exr \
  tests/resources/imageio/exr/* tests/resources/imageio/jpg/*.jpg tests/resources/input/jpg/*.jpg | LC_ALL=C sort)
"$WORK/imageio_golden" $FILES > tests/resources/golden/imageio.txt
echo "wrote tests/resources/golden/imageio.txt ($(wc -l < tests/resources/golden/imageio.txt) cases)"
