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
- ETC1S frontend, same job sizes as C++: `init_etc1_images` (4096),
  `compute_endpoint_subblock_error_vec` (512, sorted after), `generate_endpoint_codebook` (128),
  `refine_endpoint_clusterization` (1024), `create_initial_packed_texture` (4096),
  `generate_selector_clusters` training vectors (4096), `create_optimized_selector_codebook`
  (256), `find_optimal_selector_clusters_for_each_block` (2048), `reoptimize_remapped_endpoints`
  (256). Not threaded (cheap): `init_endpoint_training_vectors`, `init_global_codebooks`
  (unused by libktx).

## Status (24 threads, kodim17 512x768 / FlightHelmet 2048², Rust/C++ time)
| case | before | now |
|---|---|---|
| rgba8 + mipmaps / zstd 20 | 1.0 / 1.2 | |
| UASTC 2 | 5.3 | 0.8 |
| UASTC 4 + RDO | 10.4 | 1.1 |
| Basis-LZ (q128 / clevel 5, 2048²) | 1.6 / 4.2 | 1.1 / 1.4 |
| ASTC 4x4 thorough | 21.6 | |

Single-threaded, UASTC is at parity and Basis-LZ clevel 5 is 1.35x slower (C++ uses the SSE
`find_lowest_error` kernels); ASTC is ~3.5x slower (scalar emulation of
the AVX2 `vecmathlib`) and astcenc worker threads are not used yet.
