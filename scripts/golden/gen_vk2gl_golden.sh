#!/usr/bin/env bash
# Regenerates tests/resources/golden/vk2gl.txt: vkFormat2glInternalFormat / glFormat / glType
# of every VkFormat in vkformat_enum.h (plus 0..200), from the reference vk2gl.h.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=$ROOT/ref/ktx
WORK=${WORK:-$(mktemp -d)}
mkdir -p "$WORK"
VALUES=$( (seq 0 200; grep -oE '= [0-9]+' "$REF/lib/src/vkformat_enum.h" | grep -oE '[0-9]+') | sort -nu | tr '\n' ',')
cat > "$WORK/vk2gl_golden.c" <<C
#include <stdio.h>
#include "vk2gl.h"
int main(void) {
    static const long long values[] = { ${VALUES} };
    for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); i++) {
        VkFormat f = (VkFormat)values[i];
        printf("%lld %#x %#x %#x\n", values[i], vkFormat2glInternalFormat(f), vkFormat2glFormat(f), vkFormat2glType(f));
    }
    return 0;
}
C
gcc -O1 -w -I"$REF/lib/src" -I"$REF/other_include" -I"$REF/lib/include" "$WORK/vk2gl_golden.c" -o "$WORK/vk2gl_golden"
"$WORK/vk2gl_golden" > "$ROOT/tests/resources/golden/vk2gl.txt"
echo "wrote tests/resources/golden/vk2gl.txt ($(wc -l < "$ROOT/tests/resources/golden/vk2gl.txt") formats)"
