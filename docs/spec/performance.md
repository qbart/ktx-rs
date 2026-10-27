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

## astcenc SIMD and runtime dispatch
- Layout: the compressor lives in `src/astcenc/enc/` and is compiled once per instruction set:
  `base.rs` (SSE2 on x86-64 / scalar elsewhere, plus whatever the build enables), `sse41.rs`
  (SSE4.1, SSE4.2, POPCNT) and `avx2.rs` (AVX, AVX2, SSE4.x, POPCNT, F16C = KTX-Software's
  astcenc build). Each copy declares the same `#[path = "enc/..."]` modules.
- `#[isa]` (`ktx-macros`, a dependency-free proc macro) on each compressor function expands to
  `isa_fn! { .. }`; every copy defines its own `isa_fn!` that adds its `#[target_feature]`.
  Not annotated: trait impl methods, `const fn`s, tests, and the `vecmath` / `sse` helpers, which
  are `#[inline(always)]` and so compile inside their caller's features.
- `src/astcenc/enc/sse.rs` ports `astcenc_vecmathlib_sse_4.h` / `avx2_8.h` on `std::arch`
  (vector types keep array storage, `bytemuck` casts to `__m128` / `__m256`). Each copy's `ISA`
  constant selects SSE4.1 forms (`round`, `blendv`, `mullo`, `min/max_epi32`) and 256-bit AVX2
  kernels; the baseline uses exact SSE2 sequences (round via ±2^23, 8 lanes as two halves).
- Dispatch: `astcenc_context_alloc` picks `AstcencIsa::detect()` (CPUID via
  `is_x86_feature_detected!`); `astcenc_context_alloc_isa` forces one (`BadParam` if the CPU
  lacks it). The context keeps that copy's block size descriptor and working buffers
  (`CompressorCopy`), and `astcenc_compress_image` calls into it (`unsafe`, justified by the
  detection). The scalar decompressor in `mod.rs` is shared.
- Public module paths (`astcenc::internal`, `vecmath`, ...) re-export the baseline copy;
  `AstcencContext::compress_bsd` is only available for baseline contexts.
- Also: `round_even`, branch-free `cvtt`, full-vector fast paths in `load` / `store` /
  `gatherf_byte_inds`, `#[inline]` on every vecmath function (several were out-of-line calls).
- Tests: `sse::tests::kernels_match_scalar` runs in every copy the CPU supports (NaN,
  infinities, out-of-range converts, shift counts >= 32); `astcenc_compress` checks every case
  against the C++ golden with every available copy (a broken AVX2 kernel fails only `Avx2`).

## Status (24 threads, `scripts/bench/bench.sh`, Rust time / C++ time)
| case | before | now |
|---|---|---|
| rgba8 + mipmaps / zstd 20 | 1.0 / 1.2 | 1.05 / 1.16 |
| UASTC 2 / UASTC 4 + RDO | 5.3 / 10.4 | 0.76 / 1.10 |
| Basis-LZ / q255 + mips | 1.4 / 1.6 | 0.69 / 0.96 |
| Basis-LZ clevel 5 (2048²) | 4.2 | 1.4 |
| ASTC 4x4 thorough / 6x6 medium / 6x6 2048² | 21.6 / 7.7 / 6.3 | 1.23 / 1.23 / 1.07 |

Single threaded: UASTC at parity, Basis-LZ clevel 5 1.35x (C++ SSE `find_lowest_error`
kernels). ASTC 4x4 thorough: 0.74 s with a default build on an AVX2 CPU (baseline copy 0.96 s,
SSE4.1 copy 0.84 s), C++ (AVX2) 0.59 s; was 2.12 s.
