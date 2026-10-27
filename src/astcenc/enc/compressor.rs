//! Per instruction set compressor state (`astcenc_context` block size descriptor and working
//! buffers) and the `compress_image` block loop.

use std::sync::Mutex;

use super::internal::{BlockSizeDescriptor, CompressionWorkingBuffers, ImageBlock};
use super::*;
use ktx_macros::isa;

/// The compressor half of `astcenc_context`.
pub struct Compressor {
    /// The compressor's block size descriptor.
    pub bsd: Box<BlockSizeDescriptor>,
    /// Per thread compressor scratch buffers (`working_buffers`), one per `thread_index`.
    pub working_buffers: Vec<Mutex<CompressionWorkingBuffers>>,
}

/// The compressor part of `astcenc_context_alloc`.
#[isa]
pub fn compressor_alloc(cfg: &AstcencConfig, thread_count: u32) -> Compressor {
    let mut bsd = BlockSizeDescriptor::empty();
    let can_omit_modes = cfg.flags & ASTCENC_FLG_SELF_DECOMPRESS_ONLY != 0;
    block_sizes::init_block_size_descriptor(
        cfg.block_x,
        cfg.block_y,
        cfg.block_z,
        can_omit_modes,
        cfg.tune_partition_count_limit,
        cfg.tune_block_mode_limit as f32 / 100.0,
        &mut bsd,
    );
    Compressor {
        bsd,
        working_buffers: (0..thread_count)
            .map(|_| Mutex::new(CompressionWorkingBuffers::default()))
            .collect(),
    }
}

/// `compress_image`: compresses the batches of 16 blocks the task manager hands to this
/// thread. Each batch is compressed locally and copied into the shared output.
#[isa]
pub fn compress_image(
    compressor: &Compressor,
    config: &AstcencConfig,
    manage_compress: &ParallelManager,
    thread_index: usize,
    image: &AstcencImage,
    swizzle: &AstcencSwizzle,
    buffer: &AstcencOutput,
) -> Result<(), AstcencError> {
    let config = *config;
    let bsd = &*compressor.bsd;
    let Some(temp_buffers) = compressor.working_buffers.get(thread_index) else {
        return Err(AstcencError::BadContext);
    };
    let Ok(mut temp_buffers) = temp_buffers.lock() else {
        return Err(AstcencError::BadContext);
    };
    let temp_buffers = &mut *temp_buffers;
    let decode_mode = config.profile;
    let mut blk = ImageBlock::default();

    let block_x = bsd.dim_x as usize;
    let block_y = bsd.dim_y as usize;
    let block_z = bsd.dim_z as usize;
    blk.texel_count = (block_x * block_y * block_z) as u8;

    let dim_x = image.dim_x as usize;
    let dim_y = image.dim_y as usize;
    let dim_z = image.dim_z as usize;

    let blocks_x = dim_x.div_ceil(block_x);
    let blocks_y = dim_y.div_ceil(block_y);
    let blocks_z = dim_z.div_ceil(block_z);
    let block_count = blocks_x * blocks_y * blocks_z;

    let row_blocks = blocks_x;
    let plane_blocks = blocks_x * blocks_y;

    blk.decode_unorm8 = config.flags & ASTCENC_FLG_USE_DECODE_UNORM8 != 0;

    // Populate the block channel weights
    blk.channel_weight = vecmath::Vfloat4::new(
        config.cw_r_weight,
        config.cw_g_weight,
        config.cw_b_weight,
        config.cw_a_weight,
    );

    // Determine if we can use an optimized load function
    let needs_swz = *swizzle != AstcencSwizzle::RGBA;
    let needs_hdr = matches!(
        decode_mode,
        AstcencProfile::Hdr | AstcencProfile::HdrRgbLdrA
    );
    let use_fast_load =
        !needs_swz && !needs_hdr && block_z == 1 && matches!(image.data, AstcencImageData::U8(_));

    // Only the first thread actually runs the initializer
    manage_compress.init(block_count);

    // All threads run this processing loop until there is no work remaining
    const GRANULE: usize = 16;
    let mut local = [0u8; GRANULE * 16];
    loop {
        let (base, count) = manage_compress.get_task_assignment(GRANULE);
        if count == 0 {
            break;
        }

        for i in base..base + count {
            // Decode i into x, y, z block indices
            let z = i / plane_blocks;
            let rem = i - (z * plane_blocks);
            let y = rem / row_blocks;
            let x = rem - (y * row_blocks);

            // Fetch the full block for compression
            if use_fast_load {
                image::load_image_block_fast_ldr(image, &mut blk, bsd, x * block_x, y * block_y);
            } else {
                image::load_image_block(
                    decode_mode,
                    image,
                    &mut blk,
                    bsd,
                    x * block_x,
                    y * block_y,
                    z * block_z,
                    swizzle,
                );
            }

            // Scale RGB error contribution by the maximum alpha in the block. This encourages
            // preserving alpha accuracy in regions with high transparency.
            if config.flags & ASTCENC_FLG_USE_ALPHA_WEIGHT != 0 {
                let alpha_scale = blk.data_max.lane(3) * (1.0 / 65535.0);
                blk.channel_weight = vecmath::Vfloat4::new(
                    config.cw_r_weight * alpha_scale,
                    config.cw_g_weight * alpha_scale,
                    config.cw_b_weight * alpha_scale,
                    config.cw_a_weight,
                );
            }

            // ((z * blocks_y + y) * blocks_x + x) * 16 == i * 16: a batch is contiguous
            let Some(bp) = local
                .get_mut((i - base) * 16..(i - base) * 16 + 16)
                .and_then(|b| <&mut [u8; 16]>::try_from(b).ok())
            else {
                return Err(AstcencError::OutOfMem);
            };
            compress_symbolic::compress_block(&config, bsd, &blk, bp, temp_buffers);
        }

        let Ok(mut out) = buffer.lock() else {
            return Err(AstcencError::BadContext);
        };
        let Some(dst) = out.get_mut(base * 16..(base + count) * 16) else {
            return Err(AstcencError::OutOfMem);
        };
        dst.copy_from_slice(&local[..count * 16]);
        drop(out);
        manage_compress.complete_task_assignment(count);
    }
    Ok(())
}
