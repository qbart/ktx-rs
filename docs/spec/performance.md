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

## astcenc threads
- `ParallelManager` ported for compression (`init` by the first thread, `get_task_assignment`
  batches of 16 blocks, `reset`, `cancel`; no progress callback / `wait` / `term`).
- `astcenc_compress_image(&ctx, .., data_out: &AstcencOutput, thread_index)`: the context is
  shared (`&`), each thread locks its own working buffers, compresses a batch locally and copies
  it into the shared output (`Mutex<&mut [u8]>`; a batch is contiguous in the output).
- `astc_codec.rs` `launch_threads` = `launchThreads` + `compressionWorkloadRunner`.

## astcenc vecmath / SIMD
- `src/astcenc/sse.rs` (the only `unsafe` in astcenc) ports the kernels of
  `astcenc_vecmathlib_sse_4.h` / `avx2_8.h` on `std::arch`; the vector types keep their array
  storage and cast to `__m128` / `__m256` with `bytemuck`.
- Widths: `Lanes` / `LanesI` traits on `[_; 4]` (SSE) and `[_; 8]` (AVX2 when compiled with
  it, else two SSE halves). SSE4.1 ops (`round`, `blendv`, `mullo`, `min/max_epi32`) are used
  when enabled at compile time, otherwise exact SSE2 sequences (round via ±2^23). Non-x86 builds
  use scalar lane loops with the same semantics.
- No runtime CPU dispatch: the ISA is the build's (`-C target-cpu=...`).
- `round_even`, branch-free `cvtt`, full-vector fast paths in `load` / `store` /
  `gatherf_byte_inds`, `#[inline]` on every vecmath function (several were out-of-line calls).
- Tests: `sse::tests::kernels_match_scalar` checks every kernel against scalar semantics
  (NaN, infinities, out-of-range converts, shift counts ≥ 32); the astcenc goldens pass with
  SSE2, x86-64-v2 and x86-64-v3 builds.

## Status (24 threads, `scripts/bench/bench.sh`, Rust time / C++ time)
| case | before | now |
|---|---|---|
| rgba8 + mipmaps / zstd 20 | 1.0 / 1.2 | 1.05 / 1.16 |
| UASTC 2 / UASTC 4 + RDO | 5.3 / 10.4 | 0.76 / 1.10 |
| Basis-LZ / q255 + mips | 1.4 / 1.6 | 0.69 / 0.96 |
| Basis-LZ clevel 5 (2048²) | 4.2 | 1.4 |
| ASTC 4x4 thorough / 6x6 medium / 6x6 2048² | 21.6 / 7.7 / 6.3 | 1.8 / 1.3 / 1.3 |

Single threaded: UASTC at parity, Basis-LZ clevel 5 1.35x (C++ SSE `find_lowest_error`
kernels). ASTC 4x4 thorough: 0.96 s default (SSE2), 0.84 s x86-64-v2, 0.75 s x86-64-v3, C++
(AVX2) 0.59 s; was 2.12 s.
