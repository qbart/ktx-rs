#!/usr/bin/env bash
# Wall-clock comparison of the Rust `ktx` against a C++ reference `ktx` build.
# Usage: scripts/bench/bench.sh <cpp-ktx> [rust-ktx]
set -u
CPP=${1:?C++ ktx binary}
RUST=${2:-$(dirname "$0")/../../target/release/ktx}
R=$(cd "$(dirname "$0")/../../tests/resources" && pwd)
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
IMG=$R/input/png/kodim17.png
BIG=$R/input/png/FlightHelmet_baseColor.png
run() { # name args...
  local name=$1; shift
  local t0 t1 t2
  t0=$(date +%s.%N); "$CPP" "$@" "$OUT/c.ktx2" >/dev/null 2>&1 || echo "  cpp failed: $name"
  t1=$(date +%s.%N); "$RUST" "$@" "$OUT/r.ktx2" >/dev/null 2>&1 || echo "  rust failed: $name"
  t2=$(date +%s.%N)
  local same=same; cmp -s "$OUT/c.ktx2" "$OUT/r.ktx2" || same=DIFF
  awk -v n="$name" -v c="$(echo "$t1-$t0" | bc)" -v r="$(echo "$t2-$t1" | bc)" -v s=$same \
    'BEGIN { printf "%-34s cpp %7.2fs  rust %7.2fs  x%5.2f  %s\n", n, c, r, r/c, s }'
}
run "rgba8 + mipmaps"        create --testrun --format R8G8B8A8_UNORM --generate-mipmap "$BIG"
run "zstd 20"                create --testrun --format R8G8B8A8_UNORM --zstd 20 "$BIG"
run "astc 6x6 medium"        create --testrun --format ASTC_6x6_UNORM_BLOCK --astc-quality medium "$IMG"
run "astc 4x4 thorough"      create --testrun --format ASTC_4x4_UNORM_BLOCK --astc-quality thorough "$IMG"
run "astc 6x6 medium 2048"   create --testrun --format ASTC_6x6_UNORM_BLOCK --astc-quality medium "$BIG"
run "basis-lz"               create --testrun --format R8G8B8A8_SRGB --encode basis-lz "$IMG"
run "basis-lz q255 mips"     create --testrun --format R8G8B8A8_SRGB --encode basis-lz --qlevel 255 --generate-mipmap "$IMG"
run "uastc 2"                create --testrun --format R8G8B8A8_SRGB --encode uastc --uastc-quality 2 "$IMG"
run "uastc 4 rdo"            create --testrun --format R8G8B8A8_SRGB --encode uastc --uastc-quality 4 --uastc-rdo "$IMG"
