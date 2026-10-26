# Performance

Goal: match the C++ `ktx` speed while staying byte exact. OpenCL (basisu GPU encoding) is not
ported (C binding).

Benchmark: `scripts/bench/bench.sh <C++ ktx>` runs `ktx create` cases with both binaries and
checks the outputs are identical. Reference build: `ref/ktx`, Release, AVX2 astcenc, no OpenCL.

## Job pool
- `src/basisu/job_pool.rs` replaces basisu `job_pool`: `map_jobs` (results in job order) and
  `for_each_chunk_mut` (disjoint output chunks) on scoped threads; thread count is
  `job_pool_threads` (`--threads`, default hardware concurrency).
- Every C++ job writes disjoint outputs, so the output does not depend on the thread count.
- Ported: `encode_slices_to_uastc_4x4_ldr` (jobs of 256 blocks), `uastc_rdo` job split (≤4,
  output depends on the job count exactly as in C++), hierarchical codebook clusterizers.

## Status (24 threads, kodim17 512x768 / FlightHelmet 2048², Rust/C++ time)
| case | before | now |
|---|---|---|
| rgba8 + mipmaps / zstd 20 | 1.0 / 1.2 | |
| UASTC 2 | 5.3 | 0.8 |
| UASTC 4 + RDO | 10.4 | 1.1 |
| Basis-LZ | 1.4 | |
| ASTC 4x4 thorough | 21.6 | |

Single-threaded, UASTC and Basis-LZ are at parity; ASTC is ~3.5x slower (scalar emulation of
the AVX2 `vecmathlib`) and astcenc worker threads are not used yet.
