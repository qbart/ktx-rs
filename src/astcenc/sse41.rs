//! SSE4.1 + POPCNT copy.

// The same `enc/` files are compiled once per instruction set copy on purpose.
#![allow(clippy::duplicate_mod)]
// # Safety (every public function): the CPU must have this copy's target features;
// `astcenc_context_alloc` only calls into a copy after `AstcencIsa::detect` found them.
#![allow(clippy::missing_safety_doc)]

/// Instruction set level of this copy (0 SSE2 / scalar, 1 SSE4.1, 2 AVX2) selecting the
/// `sse` kernels.
pub const ISA: u8 = 1;
macro_rules! isa_fn {
    ($($item:tt)*) => { #[target_feature(enable = "sse4.1,sse4.2,popcnt")] $($item)* };
}
pub use super::*;
#[path = "enc/averages_and_directions.rs"]
pub mod averages_and_directions;
#[path = "enc/block_sizes.rs"]
pub mod block_sizes;
#[path = "enc/color_quantize.rs"]
pub mod color_quantize;
#[path = "enc/compress_symbolic.rs"]
pub mod compress_symbolic;
#[path = "enc/compressor.rs"]
pub mod compressor;
#[path = "enc/decompress_symbolic.rs"]
pub mod decompress_symbolic;
#[path = "enc/find_best_partitioning.rs"]
pub mod find_best_partitioning;
#[path = "enc/ideal_endpoints_and_weights.rs"]
pub mod ideal_endpoints_and_weights;
#[path = "enc/image.rs"]
pub mod image;
#[path = "enc/internal.rs"]
pub mod internal;
#[path = "enc/mathlib.rs"]
pub mod mathlib;
#[path = "enc/partition_tables.rs"]
pub mod partition_tables;
#[path = "enc/pick_best_endpoint_format.rs"]
pub mod pick_best_endpoint_format;
#[path = "enc/sse.rs"]
pub mod sse;
#[path = "enc/symbolic_physical.rs"]
pub mod symbolic_physical;
#[path = "enc/vecmath.rs"]
pub mod vecmath;
#[path = "enc/weight_align.rs"]
pub mod weight_align;
