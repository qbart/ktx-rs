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

## astcenc vecmath (safe Rust)
- `round` uses the add/subtract 2^23 trick (`f32::round_ties_even` is a libm call without
  SSE4.1); `select` and `cvtt` are branch free; `load` / `store` / `gatherf_byte_inds` have a
  full-vector fast path without per-lane bounds branches. Unit tests check `round_even` against
  `round_ties_even` (1M values) and `cvtt` edge cases.
- Building with `-C target-cpu=x86-64-v3` does not help (the lane loops barely vectorize).
- Real SIMD (`std::arch`, as upstream's `astcenc_vecmathlib_sse_4.h` / `avx2_8.h`) needs
  `unsafe` at the intrinsic call boundary, even for baseline SSE2; not done (crate denies
  `unsafe` outside the GPU loaders).

## Status (24 threads, `scripts/bench/bench.sh`, Rust time / C++ time)
| case | before | now |
|---|---|---|
| rgba8 + mipmaps / zstd 20 | 1.0 / 1.2 | 1.05 / 1.16 |
| UASTC 2 / UASTC 4 + RDO | 5.3 / 10.4 | 0.76 / 1.10 |
| Basis-LZ / q255 + mips | 1.4 / 1.6 | 0.69 / 0.96 |
| Basis-LZ clevel 5 (2048²) | 4.2 | 1.4 |
| ASTC 4x4 thorough / 6x6 medium / 6x6 2048² | 21.6 / 7.7 / 6.3 | 1.84 / 1.34 / 1.46 |

Single threaded: UASTC at parity, Basis-LZ clevel 5 1.35x (C++ SSE `find_lowest_error`
kernels), ASTC 4x4 thorough 2.4x (1.45 s vs 0.60 s; scalar vecmath).
