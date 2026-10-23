//! Port of the astc-encoder (astcenc) decompressor: `astcenc_config_init`,
//! `astcenc_context_alloc`, `astcenc_decompress_image` and the code they use
//! (block size descriptors, partition tables, integer sequence decoding,
//! physical to symbolic block conversion, color endpoint unpacking, weight
//! infill, texel decode and image store).
//!
//! Matches the x86-64 AVX2 + F16C build KTX-Software uses (fp-contract off):
//! IEEE float math, `_mm_min_ps` / `_mm_max_ps` NaN rules (second operand
//! wins), round-to-nearest-even float16 conversion. The compressor is not
//! ported; `config_init` keeps only the fields the decompressor needs.

mod tables;

use tables::*;

/// `astcenc_error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstcencError {
    Success = 0,
    OutOfMem,
    BadCpuFloat,
    BadParam,
    BadBlockSize,
    BadProfile,
    BadQuality,
    BadSwizzle,
    BadFlags,
    BadContext,
    NotImplemented,
    BadDecodeMode,
}

/// Port of `astcenc_get_error_string`.
pub fn astcenc_get_error_string(status: AstcencError) -> &'static str {
    match status {
        AstcencError::Success => "ASTCENC_SUCCESS",
        AstcencError::OutOfMem => "ASTCENC_ERR_OUT_OF_MEM",
        AstcencError::BadCpuFloat => "ASTCENC_ERR_BAD_CPU_FLOAT",
        AstcencError::BadParam => "ASTCENC_ERR_BAD_PARAM",
        AstcencError::BadBlockSize => "ASTCENC_ERR_BAD_BLOCK_SIZE",
        AstcencError::BadProfile => "ASTCENC_ERR_BAD_PROFILE",
        AstcencError::BadQuality => "ASTCENC_ERR_BAD_QUALITY",
        AstcencError::BadSwizzle => "ASTCENC_ERR_BAD_SWIZZLE",
        AstcencError::BadFlags => "ASTCENC_ERR_BAD_FLAGS",
        AstcencError::BadContext => "ASTCENC_ERR_BAD_CONTEXT",
        AstcencError::NotImplemented => "ASTCENC_ERR_NOT_IMPLEMENTED",
        AstcencError::BadDecodeMode => "ASTCENC_ERR_BAD_DECODE_MODE",
    }
}

/// `astcenc_profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstcencProfile {
    LdrSrgb = 0,
    Ldr,
    HdrRgbLdrA,
    Hdr,
}

pub const ASTCENC_FLG_MAP_NORMAL: u32 = 1 << 0;
pub const ASTCENC_FLG_USE_DECODE_UNORM8: u32 = 1 << 1;
pub const ASTCENC_FLG_USE_ALPHA_WEIGHT: u32 = 1 << 2;
pub const ASTCENC_FLG_USE_PERCEPTUAL: u32 = 1 << 3;
pub const ASTCENC_FLG_DECOMPRESS_ONLY: u32 = 1 << 4;
pub const ASTCENC_FLG_SELF_DECOMPRESS_ONLY: u32 = 1 << 5;
pub const ASTCENC_FLG_MAP_RGBM: u32 = 1 << 6;
const ASTCENC_ALL_FLAGS: u32 = ASTCENC_FLG_MAP_NORMAL
    | ASTCENC_FLG_MAP_RGBM
    | ASTCENC_FLG_USE_ALPHA_WEIGHT
    | ASTCENC_FLG_USE_PERCEPTUAL
    | ASTCENC_FLG_USE_DECODE_UNORM8
    | ASTCENC_FLG_DECOMPRESS_ONLY
    | ASTCENC_FLG_SELF_DECOMPRESS_ONLY;

pub const ASTCENC_PRE_FASTEST: f32 = 0.0;
pub const ASTCENC_PRE_FAST: f32 = 10.0;
pub const ASTCENC_PRE_MEDIUM: f32 = 60.0;
pub const ASTCENC_PRE_THOROUGH: f32 = 98.0;
pub const ASTCENC_PRE_VERYTHOROUGH: f32 = 99.0;
pub const ASTCENC_PRE_EXHAUSTIVE: f32 = 100.0;

/// `astcenc_swz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstcencSwz {
    R = 0,
    G = 1,
    B = 2,
    A = 3,
    Zero = 4,
    One = 5,
    Z = 6,
}

/// `astcenc_swizzle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AstcencSwizzle {
    pub r: AstcencSwz,
    pub g: AstcencSwz,
    pub b: AstcencSwz,
    pub a: AstcencSwz,
}

impl AstcencSwizzle {
    pub const RGBA: AstcencSwizzle = AstcencSwizzle {
        r: AstcencSwz::R,
        g: AstcencSwz::G,
        b: AstcencSwz::B,
        a: AstcencSwz::A,
    };
}

/// `astcenc_config` (decompression relevant fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AstcencConfig {
    pub profile: AstcencProfile,
    pub flags: u32,
    pub block_x: u32,
    pub block_y: u32,
    pub block_z: u32,
}

const BLOCK_MAX_TEXELS: usize = 216;
const BLOCK_MAX_WEIGHTS: u32 = 64;
const BLOCK_MIN_WEIGHT_BITS: u32 = 24;
const BLOCK_MAX_WEIGHT_BITS: u32 = 96;
const BLOCK_MAX_PARTITIONINGS: usize = 1024;
const PARTITION_INDEX_BITS: u32 = 10;
const WEIGHTS_PLANE2_OFFSET: usize = 32;
const QUANT_6: usize = 4;

/// `is_legal_2d_block_size`.
fn is_legal_2d_block_size(x: u32, y: u32) -> bool {
    matches!(
        (x << 8) | y,
        0x0404
            | 0x0504
            | 0x0505
            | 0x0605
            | 0x0606
            | 0x0805
            | 0x0806
            | 0x0808
            | 0x0A05
            | 0x0A06
            | 0x0A08
            | 0x0A0A
            | 0x0C0A
            | 0x0C0C
    )
}

/// `is_legal_3d_block_size`.
fn is_legal_3d_block_size(x: u32, y: u32, z: u32) -> bool {
    matches!(
        (x << 16) | (y << 8) | z,
        0x030303
            | 0x040303
            | 0x040403
            | 0x040404
            | 0x050404
            | 0x050504
            | 0x050505
            | 0x060505
            | 0x060605
            | 0x060606
    )
}

/// Port of `astcenc_config_init`. Only the decompression relevant fields are
/// kept; the encoder tuning presets are not ported.
pub fn astcenc_config_init(
    profile: AstcencProfile,
    block_x: u32,
    block_y: u32,
    block_z: u32,
    quality: f32,
    flags: u32,
) -> Result<AstcencConfig, AstcencError> {
    // Zero init all config fields; although most of will be over written
    let block_z = block_z.max(1);
    // validate_block_size
    let is_legal = (block_z <= 1 && is_legal_2d_block_size(block_x, block_y))
        || (block_z >= 2 && is_legal_3d_block_size(block_x, block_y, block_z));
    if !is_legal {
        return Err(AstcencError::BadBlockSize);
    }
    if !(ASTCENC_PRE_FASTEST..=ASTCENC_PRE_EXHAUSTIVE).contains(&quality) {
        return Err(AstcencError::BadQuality);
    }
    // validate_flags
    if flags & !ASTCENC_ALL_FLAGS != 0 {
        return Err(AstcencError::BadFlags);
    }
    if (flags & (ASTCENC_FLG_MAP_NORMAL | ASTCENC_FLG_MAP_RGBM)).count_ones() > 1 {
        return Err(AstcencError::BadFlags);
    }
    let is_hdr = matches!(profile, AstcencProfile::Hdr | AstcencProfile::HdrRgbLdrA);
    if flags & ASTCENC_FLG_USE_DECODE_UNORM8 != 0 && is_hdr {
        return Err(AstcencError::BadDecodeMode);
    }
    Ok(AstcencConfig {
        profile,
        flags,
        block_x,
        block_y,
        block_z,
    })
}

/// A decoded block mode (`block_mode` + its `decimation_info` index).
#[derive(Debug, Clone, Copy)]
struct BlockMode {
    decimation_mode: usize,
    quant_mode: usize,
    is_dual_plane: bool,
}

/// The decoding part of `decimation_info`: per texel up to 4 (weight index,
/// contribution) pairs.
#[derive(Debug, Clone)]
struct DecimationInfo {
    weight_count: usize,
    texel_weight_count: Vec<u8>,
    texel_weights: Vec<[u8; 4]>,
    texel_weight_contribs: Vec<[u8; 4]>,
}

/// The decoding part of `block_size_descriptor`.
#[derive(Debug, Clone)]
struct BlockSizeDescriptor {
    dim_x: u32,
    dim_y: u32,
    dim_z: u32,
    texel_count: usize,
    /// Indexed by the 11-bit block mode.
    block_modes: Vec<Option<BlockMode>>,
    decimation_tables: Vec<DecimationInfo>,
    /// `partition_of_texel` of partitionings [partition_count - 1][index]
    /// (1 partition: a single entry).
    partition_of_texel: [Vec<Vec<u8>>; 4],
}

/// Port of `get_ise_sequence_bitcount`.
fn get_ise_sequence_bitcount(character_count: u32, quant_level: usize) -> u32 {
    // (scale, divisor parameter) per quant level
    const ISE_SIZES: [(u32, u32); 21] = [
        (1, 0),
        (8, 2),
        (2, 0),
        (7, 1),
        (13, 2),
        (3, 0),
        (10, 1),
        (18, 2),
        (4, 0),
        (13, 1),
        (23, 2),
        (5, 0),
        (16, 1),
        (28, 2),
        (6, 0),
        (19, 1),
        (33, 2),
        (7, 0),
        (22, 1),
        (38, 2),
        (8, 0),
    ];
    // Cope with out-of bounds values - input might be invalid
    let Some(&(scale, div)) = ISE_SIZES.get(quant_level) else {
        return 1024;
    };
    let divisor = (div << 1) + 1;
    (scale * character_count).div_ceil(divisor)
}

/// `decode_block_mode_2d` / `decode_block_mode_3d`: (weights x, y, z, dual
/// plane, quant mode, weight bits) of a valid block mode.
fn decode_block_mode(block_mode: u32, is_3d: bool) -> Option<(u32, u32, u32, bool, usize, u32)> {
    let mut base_quant_mode = (block_mode >> 4) & 1;
    let mut h = (block_mode >> 9) & 1;
    let mut d = (block_mode >> 10) & 1;
    let a = (block_mode >> 5) & 0x3;
    let (wx, wy, wz);
    if !is_3d {
        if block_mode & 3 != 0 {
            base_quant_mode |= (block_mode & 3) << 1;
            let b = (block_mode >> 7) & 3;
            (wx, wy) = match (block_mode >> 2) & 3 {
                0 => (b + 4, a + 2),
                1 => (b + 8, a + 2),
                2 => (a + 2, b + 8),
                _ => {
                    let b = b & 1;
                    if block_mode & 0x100 != 0 {
                        (b + 2, a + 2)
                    } else {
                        (a + 2, b + 6)
                    }
                }
            };
        } else {
            base_quant_mode |= ((block_mode >> 2) & 3) << 1;
            if (block_mode >> 2) & 3 == 0 {
                return None;
            }
            let b = (block_mode >> 9) & 3;
            (wx, wy) = match (block_mode >> 7) & 3 {
                0 => (12, a + 2),
                1 => (a + 2, 12),
                2 => {
                    d = 0;
                    h = 0;
                    (a + 6, b + 6)
                }
                _ => match (block_mode >> 5) & 3 {
                    0 => (6, 10),
                    1 => (10, 6),
                    _ => return None,
                },
            };
        }
        wz = 1;
    } else if block_mode & 3 != 0 {
        base_quant_mode |= (block_mode & 3) << 1;
        let b = (block_mode >> 7) & 3;
        let c = (block_mode >> 2) & 0x3;
        (wx, wy, wz) = (a + 2, b + 2, c + 2);
    } else {
        base_quant_mode |= ((block_mode >> 2) & 3) << 1;
        if (block_mode >> 2) & 3 == 0 {
            return None;
        }
        let b = (block_mode >> 9) & 3;
        if (block_mode >> 7) & 3 != 3 {
            d = 0;
            h = 0;
        }
        (wx, wy, wz) = match (block_mode >> 7) & 3 {
            0 => (6, b + 2, a + 2),
            1 => (a + 2, 6, b + 2),
            2 => (a + 2, b + 2, 6),
            _ => match (block_mode >> 5) & 3 {
                0 => (6, 2, 2),
                1 => (2, 6, 2),
                2 => (2, 2, 6),
                _ => return None,
            },
        };
    }

    let weight_count = wx * wy * wz * (d + 1);
    let quant_mode = (base_quant_mode as usize).wrapping_sub(2) + 6 * h as usize;
    let weight_bits = get_ise_sequence_bitcount(weight_count, quant_mode);
    let valid = weight_count <= BLOCK_MAX_WEIGHTS
        && (BLOCK_MIN_WEIGHT_BITS..=BLOCK_MAX_WEIGHT_BITS).contains(&weight_bits);
    valid.then_some((wx, wy, wz, d != 0, quant_mode, weight_bits))
}

/// `init_decimation_info_2d` / `init_decimation_info_3d` (decoding tables).
fn init_decimation_info(texels: (u32, u32, u32), weights: (u32, u32, u32)) -> DecimationInfo {
    let (tx, ty, tz) = texels;
    let (wx, wy, wz) = weights;
    let texel_count = (tx * ty * tz) as usize;
    let mut di = DecimationInfo {
        weight_count: (wx * wy * wz) as usize,
        texel_weight_count: vec![0; texel_count],
        texel_weights: vec![[0; 4]; texel_count],
        texel_weight_contribs: vec![[0; 4]; texel_count],
    };
    let step = |t: u32, i: u32, w: u32| -> i32 {
        ((((1024 + t / 2) / (t - 1)) * i * (w - 1) + 32) >> 6) as i32
    };
    for z in 0..tz {
        for y in 0..ty {
            for x in 0..tx {
                let texel = ((z * ty + y) * tx + x) as usize;
                let mut qweight = [0i32; 4];
                let mut weight = [0i32; 4];
                if tz == 1 {
                    let x_weight = step(tx, x, wx);
                    let y_weight = step(ty, y, wy);
                    let (x_frac, y_frac) = (x_weight & 0xF, y_weight & 0xF);
                    qweight[0] = (x_weight >> 4) + (y_weight >> 4) * wx as i32;
                    qweight[1] = qweight[0] + 1;
                    qweight[2] = qweight[0] + wx as i32;
                    qweight[3] = qweight[2] + 1;
                    // Truncated-precision bilinear interpolation
                    let prod = x_frac * y_frac;
                    weight[3] = (prod + 8) >> 4;
                    weight[1] = x_frac - weight[3];
                    weight[2] = y_frac - weight[3];
                    weight[0] = 16 - x_frac - y_frac + weight[3];
                } else {
                    let x_weight = step(tx, x, wx);
                    let y_weight = step(ty, y, wy);
                    let z_weight = step(tz, z, wz);
                    let (fs, ft, fp) = (x_weight & 0xF, y_weight & 0xF, z_weight & 0xF);
                    let (xi, yi, zi) = (x_weight >> 4, y_weight >> 4, z_weight >> 4);
                    let (n, nm) = (wx as i32, (wx * wy) as i32);
                    qweight[0] = (zi * wy as i32 + yi) * n + xi;
                    qweight[3] = ((zi + 1) * wy as i32 + (yi + 1)) * n + (xi + 1);
                    // simplex interpolation
                    let cas =
                        (i32::from(fs > ft) << 2) + (i32::from(ft > fp) << 1) + i32::from(fs > fp);
                    let (s1, s2, w) = match cas {
                        7 => (1, n, [16 - fs, fs - ft, ft - fp, fp]),
                        3 => (n, 1, [16 - ft, ft - fs, fs - fp, fp]),
                        5 => (1, nm, [16 - fs, fs - fp, fp - ft, ft]),
                        4 => (nm, 1, [16 - fp, fp - fs, fs - ft, ft]),
                        2 => (n, nm, [16 - ft, ft - fp, fp - fs, fs]),
                        _ => (nm, n, [16 - fp, fp - ft, ft - fs, fs]),
                    };
                    qweight[1] = qweight[0] + s1;
                    qweight[2] = qweight[1] + s2;
                    weight = w;
                }
                for i in 0..4 {
                    if weight[i] != 0 {
                        let n = di.texel_weight_count[texel] as usize;
                        di.texel_weights[texel][n] = qweight[i] as u8;
                        di.texel_weight_contribs[texel][n] = weight[i] as u8;
                        di.texel_weight_count[texel] += 1;
                    }
                }
            }
        }
    }
    di
}

/// `hash52`.
fn hash52(mut inp: u32) -> u32 {
    inp ^= inp >> 15;
    // (2^4 + 1) * (2^7 + 1) * (2^17 - 1)
    inp = inp.wrapping_mul(0xEEDE0891);
    inp ^= inp >> 5;
    inp = inp.wrapping_add(inp << 16);
    inp ^= inp >> 7;
    inp ^= inp >> 3;
    inp ^= inp << 6;
    inp ^= inp >> 17;
    inp
}

/// `select_partition`: the partition of texel (x, y, z).
fn select_partition(
    mut seed: u32,
    mut x: u32,
    mut y: u32,
    mut z: u32,
    partition_count: u32,
    small_block: bool,
) -> u8 {
    // For small blocks bias the coordinates to get better distribution
    if small_block {
        x <<= 1;
        y <<= 1;
        z <<= 1;
    }
    seed += (partition_count - 1) * 1024;
    let rnum = hash52(seed);
    let s = |shift: u32| {
        let v = (rnum >> shift) & 0xF;
        v * v
    };
    let mut seeds = [
        s(0),
        s(4),
        s(8),
        s(12),
        s(16),
        s(20),
        s(24),
        s(28),
        s(18),
        s(22),
        s(26),
        {
            let v = rnum.rotate_left(2) & 0xF;
            v * v
        },
    ];
    let (sh1, sh2) = if seed & 1 != 0 {
        (
            if seed & 2 != 0 { 4 } else { 5 },
            if partition_count == 3 { 6 } else { 5 },
        )
    } else {
        (
            if partition_count == 3 { 6 } else { 5 },
            if seed & 2 != 0 { 4 } else { 5 },
        )
    };
    let sh3 = if seed & 0x10 != 0 { sh1 } else { sh2 };
    for (i, v) in seeds.iter_mut().enumerate() {
        *v >>= match i {
            0 | 2 | 4 | 6 => sh1,
            1 | 3 | 5 | 7 => sh2,
            _ => sh3,
        };
    }
    let [s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, s12] = seeds;
    let mut a = (s1 * x + s2 * y + s11 * z).wrapping_add(rnum >> 14) & 0x3F;
    let mut b = (s3 * x + s4 * y + s12 * z).wrapping_add(rnum >> 10) & 0x3F;
    let mut c = (s5 * x + s6 * y + s9 * z).wrapping_add(rnum >> 6) & 0x3F;
    let mut d = (s7 * x + s8 * y + s10 * z).wrapping_add(rnum >> 2) & 0x3F;

    // Remove some of the components if we are to output < 4 partitions.
    if partition_count <= 3 {
        d = 0;
    }
    if partition_count <= 2 {
        c = 0;
    }
    if partition_count <= 1 {
        b = 0;
    }
    if partition_count == 0 {
        a = 0;
    }
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

impl BlockSizeDescriptor {
    /// `init_block_size_descriptor` without mode / partitioning omission (the
    /// configuration of a non SELF_DECOMPRESS_ONLY context).
    fn new(dim_x: u32, dim_y: u32, dim_z: u32) -> BlockSizeDescriptor {
        let is_3d = dim_z > 1;
        let texel_count = (dim_x * dim_y * dim_z) as usize;
        let mut decimation_index: std::collections::HashMap<(u32, u32, u32), usize> =
            std::collections::HashMap::new();
        let mut decimation_tables = Vec::new();
        let mut block_modes = vec![None; 2048];
        for (i, mode) in block_modes.iter_mut().enumerate() {
            let Some((wx, wy, wz, is_dual_plane, quant_mode, weight_bits)) =
                decode_block_mode(i as u32, is_3d)
            else {
                continue;
            };
            // Always skip invalid encodings for the current block size
            if wx > dim_x || wy > dim_y || wz > dim_z {
                continue;
            }
            // Always skip encodings we can't physically encode based on
            // generic encoding bit availability
            let limit = if is_dual_plane { 109 } else { 111 };
            if weight_bits >= limit {
                continue;
            }
            let decimation_mode = *decimation_index.entry((wx, wy, wz)).or_insert_with(|| {
                decimation_tables.push(init_decimation_info((dim_x, dim_y, dim_z), (wx, wy, wz)));
                decimation_tables.len() - 1
            });
            *mode = Some(BlockMode {
                decimation_mode,
                quant_mode,
                is_dual_plane,
            });
        }

        // init_partition_tables: every partitioning is kept
        let small_block = texel_count < 32;
        let partitioning = |partition_count: u32, index: u32| -> Vec<u8> {
            let mut p = Vec::with_capacity(texel_count);
            for z in 0..dim_z {
                for y in 0..dim_y {
                    for x in 0..dim_x {
                        p.push(select_partition(
                            index,
                            x,
                            y,
                            z,
                            partition_count,
                            small_block,
                        ));
                    }
                }
            }
            p
        };
        let partition_of_texel = [
            vec![partitioning(1, 0)],
            (0..BLOCK_MAX_PARTITIONINGS as u32)
                .map(|i| partitioning(2, i))
                .collect(),
            (0..BLOCK_MAX_PARTITIONINGS as u32)
                .map(|i| partitioning(3, i))
                .collect(),
            (0..BLOCK_MAX_PARTITIONINGS as u32)
                .map(|i| partitioning(4, i))
                .collect(),
        ];

        BlockSizeDescriptor {
            dim_x,
            dim_y,
            dim_z,
            texel_count,
            block_modes,
            decimation_tables,
            partition_of_texel,
        }
    }
}

/// Port of `astcenc_context` (decompression state).
#[derive(Debug, Clone)]
pub struct AstcencContext {
    pub config: AstcencConfig,
    pub thread_count: u32,
    bsd: BlockSizeDescriptor,
}

/// Port of `astcenc_context_alloc`.
pub fn astcenc_context_alloc(
    config: &AstcencConfig,
    thread_count: u32,
) -> Result<Box<AstcencContext>, AstcencError> {
    if thread_count == 0 {
        return Err(AstcencError::BadParam);
    }
    Ok(Box::new(AstcencContext {
        config: *config,
        thread_count,
        bsd: BlockSizeDescriptor::new(config.block_x, config.block_y, config.block_z),
    }))
}

/// Port of `astcenc_context_free`.
pub fn astcenc_context_free(context: Box<AstcencContext>) {
    drop(context);
}

/// Port of `astcenc_decompress_reset` (no state to reset single threaded).
pub fn astcenc_decompress_reset(_context: &mut AstcencContext) -> Result<(), AstcencError> {
    Ok(())
}

/// `astcenc_image` data planes (one per z slice, 4 components per texel).
#[derive(Debug, Clone, PartialEq)]
pub enum AstcencImageData {
    U8(Vec<Vec<u8>>),
    F16(Vec<Vec<u16>>),
    F32(Vec<Vec<f32>>),
}

/// Port of `astcenc_image`.
#[derive(Debug, Clone, PartialEq)]
pub struct AstcencImage {
    pub dim_x: u32,
    pub dim_y: u32,
    pub dim_z: u32,
    pub data: AstcencImageData,
}

/// `symbolic_compressed_block` block types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockType {
    Error,
    ConstF16,
    ConstU16,
    NonConst,
}

/// Port of `symbolic_compressed_block` (decoding fields).
struct SymbolicBlock {
    block_type: BlockType,
    partition_count: usize,
    partition_index: usize,
    block_mode: usize,
    plane2_component: i32,
    color_formats: [u8; 4],
    color_values: [[u8; 8]; 4],
    constant_color: [i32; 4],
    weights: [u8; 64],
}

/// `read_bits` (bits past the 16 byte block read as zero).
fn read_bits(bitcount: u32, bitoffset: u32, data: &[u8]) -> u32 {
    let mask = (1u32 << bitcount) - 1;
    let p = (bitoffset >> 3) as usize;
    let value = u32::from(data.get(p).copied().unwrap_or(0))
        | (u32::from(data.get(p + 1).copied().unwrap_or(0)) << 8);
    (value >> (bitoffset & 7)) & mask
}

/// `bitrev8`.
fn bitrev8(p: u8) -> u8 {
    p.reverse_bits()
}

/// Port of `decode_ise`.
fn decode_ise(
    quant_level: usize,
    character_count: usize,
    input: &[u8],
    output: &mut [u8],
    mut bit_offset: u32,
) {
    // (bits, trits, quints) per quant level
    const BTQ_COUNTS: [(u32, bool, bool); 21] = [
        (1, false, false),
        (0, true, false),
        (2, false, false),
        (0, false, true),
        (1, true, false),
        (3, false, false),
        (1, false, true),
        (2, true, false),
        (4, false, false),
        (2, false, true),
        (3, true, false),
        (5, false, false),
        (3, false, true),
        (4, true, false),
        (6, false, false),
        (4, false, true),
        (5, true, false),
        (7, false, false),
        (5, false, true),
        (6, true, false),
        (8, false, false),
    ];
    let mut results = [0u8; 68];
    let mut tq_blocks = [0u8; 22];
    let (bits, trits, quints) = BTQ_COUNTS[quant_level];
    let mut lcounter = 0usize;
    let mut hcounter = 0usize;

    // Collect bits for each element, as well as bits for any trit-blocks and quint-blocks.
    for r in results.iter_mut().take(character_count) {
        *r = read_bits(bits, bit_offset, input) as u8;
        bit_offset += bits;
        if trits {
            const BITS_TO_READ: [u32; 5] = [2, 2, 1, 2, 1];
            const BLOCK_SHIFT: [u32; 5] = [0, 2, 4, 5, 7];
            const NEXT_LCOUNTER: [usize; 5] = [1, 2, 3, 4, 0];
            const HCOUNTER_INCR: [usize; 5] = [0, 0, 0, 0, 1];
            let tdata = read_bits(BITS_TO_READ[lcounter], bit_offset, input);
            bit_offset += BITS_TO_READ[lcounter];
            tq_blocks[hcounter] |= (tdata << BLOCK_SHIFT[lcounter]) as u8;
            hcounter += HCOUNTER_INCR[lcounter];
            lcounter = NEXT_LCOUNTER[lcounter];
        }
        if quints {
            const BITS_TO_READ: [u32; 3] = [3, 2, 2];
            const BLOCK_SHIFT: [u32; 3] = [0, 3, 5];
            const NEXT_LCOUNTER: [usize; 3] = [1, 2, 0];
            const HCOUNTER_INCR: [usize; 3] = [0, 0, 1];
            let tdata = read_bits(BITS_TO_READ[lcounter], bit_offset, input);
            bit_offset += BITS_TO_READ[lcounter];
            tq_blocks[hcounter] |= (tdata << BLOCK_SHIFT[lcounter]) as u8;
            hcounter += HCOUNTER_INCR[lcounter];
            lcounter = NEXT_LCOUNTER[lcounter];
        }
    }

    // Unpack trit-blocks or quint-blocks as needed
    if trits {
        for i in 0..character_count.div_ceil(5) {
            let t = TRITS_OF_INTEGER[tq_blocks[i] as usize];
            for k in 0..5 {
                results[5 * i + k] |= t[k] << bits;
            }
        }
    }
    if quints {
        for i in 0..character_count.div_ceil(3) {
            let q = QUINTS_OF_INTEGER[tq_blocks[i] as usize];
            for k in 0..3 {
                results[3 * i + k] |= q[k] << bits;
            }
        }
    }
    output[..character_count].copy_from_slice(&results[..character_count]);
}

/// Port of `physical_to_symbolic`.
fn physical_to_symbolic(bsd: &BlockSizeDescriptor, pcb: &[u8]) -> SymbolicBlock {
    let mut scb = SymbolicBlock {
        block_type: BlockType::NonConst,
        partition_count: 0,
        partition_index: 0,
        block_mode: 0,
        plane2_component: -1,
        color_formats: [0; 4],
        color_values: [[0; 8]; 4],
        constant_color: [0; 4],
        weights: [0; 64],
    };
    let error = |mut scb: SymbolicBlock| {
        scb.block_type = BlockType::Error;
        scb
    };

    // Extract header fields
    let block_mode = read_bits(11, 0, pcb);
    if block_mode & 0x1FF == 0x1FC {
        // Constant color block: check what format the data has
        scb.block_type = if block_mode & 0x200 != 0 {
            BlockType::ConstF16
        } else {
            BlockType::ConstU16
        };
        for i in 0..4 {
            scb.constant_color[i] = i32::from(pcb[2 * i + 8]) | (i32::from(pcb[2 * i + 9]) << 8);
        }
        // Additionally, check that the void-extent
        if bsd.dim_z == 1 {
            // 2D void-extent
            if read_bits(2, 10, pcb) != 3 {
                return error(scb);
            }
            // Low values span 3 bytes so need two read_bits calls
            let vx_low_s = read_bits(8, 12, pcb) | (read_bits(5, 12 + 8, pcb) << 8);
            let vx_high_s = read_bits(13, 25, pcb);
            let vx_low_t = read_bits(8, 38, pcb) | (read_bits(5, 38 + 8, pcb) << 8);
            let vx_high_t = read_bits(13, 51, pcb);
            let all_ones = vx_low_s == 0x1FFF
                && vx_high_s == 0x1FFF
                && vx_low_t == 0x1FFF
                && vx_high_t == 0x1FFF;
            if (vx_low_s >= vx_high_s || vx_low_t >= vx_high_t) && !all_ones {
                return error(scb);
            }
        } else {
            // 3D void-extent
            let v = |o: u32| read_bits(9, o, pcb);
            let (ls, hs, lt, ht, lr, hr) = (v(10), v(19), v(28), v(37), v(46), v(55));
            let all_ones = [ls, hs, lt, ht, lr, hr].iter().all(|&x| x == 0x1FF);
            if (ls >= hs || lt >= ht || lr >= hr) && !all_ones {
                return error(scb);
            }
        }
        return scb;
    }

    let Some(bm) = bsd.block_modes[block_mode as usize] else {
        return error(scb);
    };
    let di = &bsd.decimation_tables[bm.decimation_mode];
    let weight_count = di.weight_count;
    let weight_quant_method = bm.quant_mode;
    let is_dual_plane = bm.is_dual_plane;
    let real_weight_count = if is_dual_plane {
        2 * weight_count
    } else {
        weight_count
    };

    let partition_count = read_bits(2, 11, pcb) as usize + 1;
    scb.block_mode = block_mode as usize;
    scb.partition_count = partition_count;

    let mut bswapped = [0u8; 16];
    for (i, b) in bswapped.iter_mut().enumerate() {
        *b = bitrev8(pcb[15 - i]);
    }

    let bits_for_weights = get_ise_sequence_bitcount(real_weight_count as u32, weight_quant_method);
    let mut below_weights_pos = 128 - bits_for_weights as i32;

    let mut indices = [0u8; 64];
    decode_ise(
        weight_quant_method,
        real_weight_count,
        &bswapped,
        &mut indices,
        0,
    );
    let map = &WEIGHT_UNSCRAMBLE_AND_UNQUANT[weight_quant_method];
    if is_dual_plane {
        for i in 0..weight_count {
            scb.weights[i] = map[indices[2 * i] as usize];
            scb.weights[i + WEIGHTS_PLANE2_OFFSET] = map[indices[2 * i + 1] as usize];
        }
    } else {
        for i in 0..weight_count {
            scb.weights[i] = map[indices[i] as usize];
        }
    }

    if is_dual_plane && partition_count == 4 {
        return error(scb);
    }

    // Determine the format of each endpoint pair
    let mut color_formats = [0u32; 4];
    let mut encoded_type_highpart_size: i32 = 0;
    if partition_count == 1 {
        color_formats[0] = read_bits(4, 13, pcb);
        scb.partition_index = 0;
    } else {
        encoded_type_highpart_size = (3 * partition_count as i32) - 4;
        below_weights_pos -= encoded_type_highpart_size;
        let encoded_type = read_bits(6, 13 + PARTITION_INDEX_BITS, pcb)
            | (read_bits(
                encoded_type_highpart_size as u32,
                below_weights_pos.max(0) as u32,
                pcb,
            ) << 6);
        let mut baseclass = encoded_type & 0x3;
        if baseclass == 0 {
            for f in color_formats.iter_mut().take(partition_count) {
                *f = (encoded_type >> 2) & 0xF;
            }
            below_weights_pos += encoded_type_highpart_size;
            encoded_type_highpart_size = 0;
        } else {
            let mut bitpos = 2;
            baseclass -= 1;
            for f in color_formats.iter_mut().take(partition_count) {
                *f = (((encoded_type >> bitpos) & 1) + baseclass) << 2;
                bitpos += 1;
            }
            for f in color_formats.iter_mut().take(partition_count) {
                *f |= (encoded_type >> bitpos) & 3;
                bitpos += 2;
            }
        }
        scb.partition_index = read_bits(10, 13, pcb) as usize;
    }

    for (dst, &f) in scb
        .color_formats
        .iter_mut()
        .zip(&color_formats[..partition_count])
    {
        *dst = f as u8;
    }

    // Determine number of color endpoint integers
    let color_integer_count: usize = color_formats[..partition_count]
        .iter()
        .map(|f| ((f >> 2) as usize + 1) * 2)
        .sum();
    if color_integer_count > 18 {
        return error(scb);
    }

    // Determine the color endpoint format to use
    const COLOR_BITS_ARR: [i32; 5] = [
        -1,
        115 - 4,
        113 - 4 - PARTITION_INDEX_BITS as i32,
        113 - 4 - PARTITION_INDEX_BITS as i32,
        113 - 4 - PARTITION_INDEX_BITS as i32,
    ];
    let mut color_bits =
        COLOR_BITS_ARR[partition_count] - bits_for_weights as i32 - encoded_type_highpart_size;
    if is_dual_plane {
        color_bits -= 2;
    }
    let color_bits = color_bits.max(0) as usize;

    let color_quant_level = QUANT_MODE_TABLE[color_integer_count >> 1][color_bits.min(127)];
    if color_quant_level < QUANT_6 as i8 {
        return error(scb);
    }
    let color_quant_level = color_quant_level as usize;

    // Unpack the integer color values and assign to endpoints
    let mut values_to_decode = [0u8; 32];
    decode_ise(
        color_quant_level,
        color_integer_count,
        pcb,
        &mut values_to_decode,
        if partition_count == 1 {
            17
        } else {
            19 + PARTITION_INDEX_BITS
        },
    );
    let unpack_table = COLOR_SCRAMBLED_PQUANT_TO_UQUANT[color_quant_level - QUANT_6];
    let mut valuecount_to_decode = 0;
    for (values, &f) in scb
        .color_values
        .iter_mut()
        .zip(&color_formats[..partition_count])
    {
        let vals = 2 * (f >> 2) as usize + 2;
        for (j, v) in values.iter_mut().take(vals).enumerate() {
            *v = unpack_table[values_to_decode[j + valuecount_to_decode] as usize];
        }
        valuecount_to_decode += vals;
    }

    // Fetch component for second-plane in the case of dual plane of weights.
    scb.plane2_component = -1;
    if is_dual_plane {
        scb.plane2_component = read_bits(2, (below_weights_pos - 2) as u32, pcb) as i32;
    }
    scb
}

type Vint4 = [i32; 4];

/// `uncontract_color`.
fn uncontract_color(input: Vint4) -> Vint4 {
    let b = input[2];
    [(input[0] + b) >> 1, (input[1] + b) >> 1, input[2], input[3]]
}

/// `rgba_delta_unpack`.
fn rgba_delta_unpack(mut input0: Vint4, mut input1: Vint4) -> (Vint4, Vint4) {
    // Apply bit transfer: bit_transfer_signed(input1, input0)
    for i in 0..4 {
        let (c0, c1) = (input0[i], input1[i]);
        input0[i] = ((c0 as u32 >> 1) | (c1 as u32 & 0x80)) as i32;
        let mut d = ((c1 as u32 >> 1) & 0x3F) as i32;
        if d & 0x20 != 0 {
            d -= 0x40;
        }
        input1[i] = d;
    }

    // Apply blue-uncontraction if needed
    let rgb_sum = input1[0] + input1[1] + input1[2];
    for i in 0..4 {
        input1[i] += input0[i];
    }
    if rgb_sum < 0 {
        input0 = uncontract_color(input0);
        input1 = uncontract_color(input1);
        std::mem::swap(&mut input0, &mut input1);
    }
    (
        input0.map(|v| v.clamp(0, 255)),
        input1.map(|v| v.clamp(0, 255)),
    )
}

/// `rgba_unpack`.
fn rgba_unpack(mut input0: Vint4, mut input1: Vint4) -> (Vint4, Vint4) {
    // Apply blue-uncontraction if needed
    if input0[0] + input0[1] + input0[2] > input1[0] + input1[1] + input1[2] {
        input0 = uncontract_color(input0);
        input1 = uncontract_color(input1);
        std::mem::swap(&mut input0, &mut input1);
    }
    (input0, input1)
}

/// `safe_signed_lsh`.
fn safe_signed_lsh(val: i32, shift: i32) -> i32 {
    ((val as u32).wrapping_shl(shift as u32)) as i32
}

/// `hdr_rgbo_unpack`.
fn hdr_rgbo_unpack(input: &[u8]) -> (Vint4, Vint4) {
    let (v0, v1, v2, v3) = (
        i32::from(input[0]),
        i32::from(input[1]),
        i32::from(input[2]),
        i32::from(input[3]),
    );
    let modeval = ((v0 & 0xC0) >> 6) | (((v1 & 0x80) >> 7) << 2) | (((v2 & 0x80) >> 7) << 3);
    let (majcomp, mode) = if modeval & 0xC != 0xC {
        (modeval >> 2, modeval & 3)
    } else if modeval != 0xF {
        (modeval & 3, 4)
    } else {
        (0, 5)
    };

    let mut red = v0 & 0x3F;
    let mut green = v1 & 0x1F;
    let mut blue = v2 & 0x1F;
    let mut scale = v3 & 0x1F;

    let bit0 = (v1 >> 6) & 1;
    let bit1 = (v1 >> 5) & 1;
    let bit2 = (v2 >> 6) & 1;
    let bit3 = (v2 >> 5) & 1;
    let bit4 = (v3 >> 7) & 1;
    let bit5 = (v3 >> 6) & 1;
    let bit6 = (v3 >> 5) & 1;

    let ohcomp = 1 << mode;
    if ohcomp & 0x30 != 0 {
        green |= bit0 << 6;
    }
    if ohcomp & 0x3A != 0 {
        green |= bit1 << 5;
    }
    if ohcomp & 0x30 != 0 {
        blue |= bit2 << 6;
    }
    if ohcomp & 0x3A != 0 {
        blue |= bit3 << 5;
    }
    if ohcomp & 0x3D != 0 {
        scale |= bit6 << 5;
    }
    if ohcomp & 0x2D != 0 {
        scale |= bit5 << 6;
    }
    if ohcomp & 0x04 != 0 {
        scale |= bit4 << 7;
    }
    if ohcomp & 0x3B != 0 {
        red |= bit4 << 6;
    }
    if ohcomp & 0x04 != 0 {
        red |= bit3 << 6;
    }
    if ohcomp & 0x10 != 0 {
        red |= bit5 << 7;
    }
    if ohcomp & 0x0F != 0 {
        red |= bit2 << 7;
    }
    if ohcomp & 0x05 != 0 {
        red |= bit1 << 8;
    }
    if ohcomp & 0x0A != 0 {
        red |= bit0 << 8;
    }
    if ohcomp & 0x05 != 0 {
        red |= bit0 << 9;
    }
    if ohcomp & 0x02 != 0 {
        red |= bit6 << 9;
    }
    if ohcomp & 0x01 != 0 {
        red |= bit3 << 10;
    }
    if ohcomp & 0x02 != 0 {
        red |= bit5 << 10;
    }

    // expand to 12 bits.
    const SHAMTS: [i32; 6] = [1, 1, 2, 3, 4, 5];
    let shamt = SHAMTS[mode as usize];
    red <<= shamt;
    green <<= shamt;
    blue <<= shamt;
    scale <<= shamt;

    // on modes 0 to 4, the values stored for "green" and "blue" are differentials,
    // not absolute values.
    if mode != 5 {
        green = red - green;
        blue = red - blue;
    }

    // switch around components.
    match majcomp {
        1 => std::mem::swap(&mut red, &mut green),
        2 => std::mem::swap(&mut red, &mut blue),
        _ => {}
    }

    let red0 = (red - scale).max(0);
    let green0 = (green - scale).max(0);
    let blue0 = (blue - scale).max(0);
    let red = red.max(0);
    let green = green.max(0);
    let blue = blue.max(0);
    (
        [red0 << 4, green0 << 4, blue0 << 4, 0x7800],
        [red << 4, green << 4, blue << 4, 0x7800],
    )
}

/// `hdr_rgb_unpack`.
fn hdr_rgb_unpack(input: &[u8]) -> (Vint4, Vint4) {
    let v: [i32; 6] = [0, 1, 2, 3, 4, 5].map(|i| i32::from(input[i]));
    let [v0, v1, v2, v3, v4, v5] = v;

    // extract all the fixed-placement bitfields
    let modeval = ((v1 & 0x80) >> 7) | (((v2 & 0x80) >> 7) << 1) | (((v3 & 0x80) >> 7) << 2);
    let majcomp = ((v4 & 0x80) >> 7) | (((v5 & 0x80) >> 7) << 1);

    if majcomp == 3 {
        return (
            [v0 << 8, v2 << 8, (v4 & 0x7F) << 9, 0x7800],
            [v1 << 8, v3 << 8, (v5 & 0x7F) << 9, 0x7800],
        );
    }

    let mut a = v0 | ((v1 & 0x40) << 2);
    let mut b0 = v2 & 0x3f;
    let mut b1 = v3 & 0x3f;
    let mut c = v1 & 0x3f;
    let mut d0 = v4 & 0x7f;
    let mut d1 = v5 & 0x7f;

    // get hold of the number of bits in 'd0' and 'd1'
    const DBITS_TAB: [i32; 8] = [7, 6, 7, 6, 5, 6, 5, 6];
    let dbits = DBITS_TAB[modeval as usize];

    // extract six variable-placement bits
    let bit0 = (v2 >> 6) & 1;
    let bit1 = (v3 >> 6) & 1;
    let bit2 = (v4 >> 6) & 1;
    let bit3 = (v5 >> 6) & 1;
    let bit4 = (v4 >> 5) & 1;
    let bit5 = (v5 >> 5) & 1;

    // and prepend the variable-placement bits depending on mode.
    let ohmod = 1 << modeval; // one-hot-mode
    if ohmod & 0xA4 != 0 {
        a |= bit0 << 9;
    }
    if ohmod & 0x8 != 0 {
        a |= bit2 << 9;
    }
    if ohmod & 0x50 != 0 {
        a |= bit4 << 9;
    }
    if ohmod & 0x50 != 0 {
        a |= bit5 << 10;
    }
    if ohmod & 0xA0 != 0 {
        a |= bit1 << 10;
    }
    if ohmod & 0xC0 != 0 {
        a |= bit2 << 11;
    }
    if ohmod & 0x4 != 0 {
        c |= bit1 << 6;
    }
    if ohmod & 0xE8 != 0 {
        c |= bit3 << 6;
    }
    if ohmod & 0x20 != 0 {
        c |= bit2 << 7;
    }
    if ohmod & 0x5B != 0 {
        b0 |= bit0 << 6;
        b1 |= bit1 << 6;
    }
    if ohmod & 0x12 != 0 {
        b0 |= bit2 << 7;
        b1 |= bit3 << 7;
    }
    if ohmod & 0xAF != 0 {
        d0 |= bit4 << 5;
        d1 |= bit5 << 5;
    }
    if ohmod & 0x5 != 0 {
        d0 |= bit2 << 6;
        d1 |= bit3 << 6;
    }

    // sign-extend 'd0' and 'd1'
    let sx_shamt = 32 - dbits;
    d0 = safe_signed_lsh(d0, sx_shamt) >> sx_shamt;
    d1 = safe_signed_lsh(d1, sx_shamt) >> sx_shamt;

    // expand all values to 12 bits, with left-shift as needed.
    let val_shamt = (modeval >> 1) ^ 3;
    a = safe_signed_lsh(a, val_shamt);
    b0 = safe_signed_lsh(b0, val_shamt);
    b1 = safe_signed_lsh(b1, val_shamt);
    c = safe_signed_lsh(c, val_shamt);
    d0 = safe_signed_lsh(d0, val_shamt);
    d1 = safe_signed_lsh(d1, val_shamt);

    // then compute the actual color values.
    let clamp12 = |v: i32| v.clamp(0, 4095);
    let mut red1 = clamp12(a);
    let mut green1 = clamp12(a - b0);
    let mut blue1 = clamp12(a - b1);
    let mut red0 = clamp12(a - c);
    let mut green0 = clamp12(a - b0 - c - d0);
    let mut blue0 = clamp12(a - b1 - c - d1);

    // switch around the color components
    match majcomp {
        1 => {
            std::mem::swap(&mut red0, &mut green0);
            std::mem::swap(&mut red1, &mut green1);
        }
        2 => {
            std::mem::swap(&mut red0, &mut blue0);
            std::mem::swap(&mut red1, &mut blue1);
        }
        _ => {}
    }
    (
        [red0 << 4, green0 << 4, blue0 << 4, 0x7800],
        [red1 << 4, green1 << 4, blue1 << 4, 0x7800],
    )
}

/// `hdr_luminance_small_range_unpack`.
fn hdr_luminance_small_range_unpack(input: &[u8]) -> (Vint4, Vint4) {
    let (v0, v1) = (i32::from(input[0]), i32::from(input[1]));
    let (y0, mut y1);
    if v0 & 0x80 != 0 {
        y0 = ((v1 & 0xE0) << 4) | ((v0 & 0x7F) << 2);
        y1 = (v1 & 0x1F) << 2;
    } else {
        y0 = ((v1 & 0xF0) << 4) | ((v0 & 0x7F) << 1);
        y1 = (v1 & 0xF) << 1;
    }
    y1 += y0;
    if y1 > 0xFFF {
        y1 = 0xFFF;
    }
    (
        [y0 << 4, y0 << 4, y0 << 4, 0x7800],
        [y1 << 4, y1 << 4, y1 << 4, 0x7800],
    )
}

/// `hdr_luminance_large_range_unpack`.
fn hdr_luminance_large_range_unpack(input: &[u8]) -> (Vint4, Vint4) {
    let (v0, v1) = (i32::from(input[0]), i32::from(input[1]));
    let (y0, y1) = if v1 >= v0 {
        (v0 << 4, v1 << 4)
    } else {
        ((v1 << 4) + 8, (v0 << 4) - 8)
    };
    (
        [y0 << 4, y0 << 4, y0 << 4, 0x7800],
        [y1 << 4, y1 << 4, y1 << 4, 0x7800],
    )
}

/// `hdr_alpha_unpack`.
fn hdr_alpha_unpack(input: &[u8]) -> (i32, i32) {
    let mut v6 = i32::from(input[0]);
    let mut v7 = i32::from(input[1]);
    let modeval = ((v6 >> 7) & 1) | ((v7 >> 6) & 2);
    v6 &= 0x7F;
    v7 &= 0x7F;
    let (o0, o1) = if modeval == 3 {
        // Directly specified 12-bit alpha values
        (v6 << 5, v7 << 5)
    } else {
        // Packed base + delta alpha values
        // Transfer 1-3 high bits of v7 to make an 8-10 bit base
        v6 |= (v7 << (modeval + 1)) & 0x780;
        // Extract remaining 4-6 bits and unbias to make signed delta
        v7 &= 0x3f >> modeval;
        v7 ^= 32 >> modeval;
        v7 -= 32 >> modeval;
        // Scale unsigned base to 12 bits
        v6 <<= 4 - modeval;
        // Scale signed delta to 6-10 bits
        v7 = safe_signed_lsh(v7, 4 - modeval);
        // Clamp computed base+delta value to valid range
        v7 = (v6 + v7).clamp(0, 0xFFF);
        (v6, v7)
    };
    (o0 << 4, o1 << 4)
}

const FMT_LUMINANCE: u8 = 0;
const FMT_LUMINANCE_DELTA: u8 = 1;
const FMT_HDR_LUMINANCE_LARGE_RANGE: u8 = 2;
const FMT_HDR_LUMINANCE_SMALL_RANGE: u8 = 3;
const FMT_LUMINANCE_ALPHA: u8 = 4;
const FMT_LUMINANCE_ALPHA_DELTA: u8 = 5;
const FMT_RGB_SCALE: u8 = 6;
const FMT_HDR_RGB_SCALE: u8 = 7;
const FMT_RGB: u8 = 8;
const FMT_RGB_DELTA: u8 = 9;
const FMT_RGB_SCALE_ALPHA: u8 = 10;
const FMT_HDR_RGB: u8 = 11;
const FMT_RGBA: u8 = 12;
const FMT_RGBA_DELTA: u8 = 13;
const FMT_HDR_RGB_LDR_ALPHA: u8 = 14;
const FMT_HDR_RGBA: u8 = 15;

/// Port of `unpack_color_endpoints`: (endpoint 0, endpoint 1, rgb_hdr, alpha_hdr).
fn unpack_color_endpoints(
    decode_mode: AstcencProfile,
    format: u8,
    input: &[u8; 8],
) -> (Vint4, Vint4, bool, bool) {
    let i = |k: usize| i32::from(input[k]);
    // Assume no NaNs and LDR endpoints unless set later
    let mut rgb_hdr = false;
    let mut alpha_hdr = false;
    let mut alpha_hdr_default = false;

    let (mut output0, mut output1) = match format {
        FMT_LUMINANCE => ([i(0), i(0), i(0), 255], [i(1), i(1), i(1), 255]),
        FMT_LUMINANCE_DELTA => {
            let l0 = (i(0) >> 2) | (i(1) & 0xC0);
            let l1 = (l0 + (i(1) & 0x3F)).min(255);
            ([l0, l0, l0, 255], [l1, l1, l1, 255])
        }
        FMT_HDR_LUMINANCE_SMALL_RANGE => {
            rgb_hdr = true;
            alpha_hdr_default = true;
            hdr_luminance_small_range_unpack(input)
        }
        FMT_HDR_LUMINANCE_LARGE_RANGE => {
            rgb_hdr = true;
            alpha_hdr_default = true;
            hdr_luminance_large_range_unpack(input)
        }
        FMT_LUMINANCE_ALPHA => ([i(0), i(0), i(0), i(2)], [i(1), i(1), i(1), i(3)]),
        FMT_LUMINANCE_ALPHA_DELTA => {
            let (mut lum0, mut lum1, mut alpha0, mut alpha1) = (i(0), i(1), i(2), i(3));
            lum0 |= (lum1 & 0x80) << 1;
            alpha0 |= (alpha1 & 0x80) << 1;
            lum1 &= 0x7F;
            alpha1 &= 0x7F;
            if lum1 & 0x40 != 0 {
                lum1 -= 0x80;
            }
            if alpha1 & 0x40 != 0 {
                alpha1 -= 0x80;
            }
            lum0 >>= 1;
            lum1 >>= 1;
            alpha0 >>= 1;
            alpha1 >>= 1;
            lum1 = (lum1 + lum0).clamp(0, 255);
            alpha1 = (alpha1 + alpha0).clamp(0, 255);
            ([lum0, lum0, lum0, alpha0], [lum1, lum1, lum1, alpha1])
        }
        FMT_RGB_SCALE => {
            // rgb_scale_unpack
            let input0 = [i(0), i(1), i(2), 0];
            let scale = i(3);
            let mut o1 = input0;
            o1[3] = 255;
            let mut o0 = input0.map(|v| (v * scale) >> 8);
            o0[3] = 255;
            (o0, o1)
        }
        FMT_RGB_SCALE_ALPHA => {
            // rgb_scale_alpha_unpack
            let input0 = [i(0), i(1), i(2), i(4)];
            let mut o1 = input0;
            o1[3] = i(5);
            let mut o0 = input0.map(|v| (v * i(3)) >> 8);
            o0[3] = input0[3];
            (o0, o1)
        }
        FMT_HDR_RGB_SCALE => {
            rgb_hdr = true;
            alpha_hdr_default = true;
            hdr_rgbo_unpack(input)
        }
        FMT_RGB | FMT_RGB_DELTA => {
            let input0 = [i(0), i(2), i(4), 0];
            let input1 = [i(1), i(3), i(5), 0];
            let (mut o0, mut o1) = if format == FMT_RGB {
                rgba_unpack(input0, input1)
            } else {
                rgba_delta_unpack(input0, input1)
            };
            o0[3] = 255;
            o1[3] = 255;
            (o0, o1)
        }
        FMT_HDR_RGB => {
            rgb_hdr = true;
            alpha_hdr_default = true;
            hdr_rgb_unpack(input)
        }
        FMT_RGBA | FMT_RGBA_DELTA => {
            let input0 = [i(0), i(2), i(4), i(6)];
            let input1 = [i(1), i(3), i(5), i(7)];
            if format == FMT_RGBA {
                rgba_unpack(input0, input1)
            } else {
                rgba_delta_unpack(input0, input1)
            }
        }
        FMT_HDR_RGB_LDR_ALPHA => {
            rgb_hdr = true;
            let (mut o0, mut o1) = hdr_rgb_unpack(input);
            o0[3] = i(6);
            o1[3] = i(7);
            (o0, o1)
        }
        FMT_HDR_RGBA => {
            rgb_hdr = true;
            alpha_hdr = true;
            let (mut o0, mut o1) = hdr_rgb_unpack(input);
            let (a0, a1) = hdr_alpha_unpack(&input[6..]);
            o0[3] = a0;
            o1[3] = a1;
            (o0, o1)
        }
        // Formats are 4 bits; C leaves the outputs unset otherwise.
        _ => ([0; 4], [0; 4]),
    };

    // Assign a correct default alpha
    if alpha_hdr_default {
        if decode_mode == AstcencProfile::Hdr {
            output0[3] = 0x7800;
            output1[3] = 0x7800;
            alpha_hdr = true;
        } else {
            output0[3] = 0x00FF;
            output1[3] = 0x00FF;
            alpha_hdr = false;
        }
    }

    // Handle endpoint errors and expansion
    match decode_mode {
        AstcencProfile::Ldr | AstcencProfile::LdrSrgb => {
            // Error color - HDR endpoint in an LDR encoding
            if rgb_hdr || alpha_hdr {
                output0 = [0xFF, 0x00, 0xFF, 0xFF];
                output1 = [0xFF, 0x00, 0xFF, 0xFF];
                rgb_hdr = false;
                alpha_hdr = false;
            }
            if decode_mode == AstcencProfile::Ldr {
                // Linear LDR 8-bit endpoints are expanded to 16-bit by replication
                output0 = output0.map(|v| v * 257);
                output1 = output1.map(|v| v * 257);
            } else {
                // sRGB LDR 8-bit endpoints are expanded to 16 bit by shifting
                // left by 8 bits and OR with 0x80
                output0 = output0.map(|v| (v << 8) | 0x80);
                output1 = output1.map(|v| (v << 8) | 0x80);
            }
        }
        _ => {
            // An HDR profile decode, but may be using linear LDR endpoints
            let hdr_lanes = [rgb_hdr, rgb_hdr, rgb_hdr, alpha_hdr];
            for k in 0..4 {
                let s = if hdr_lanes[k] { 1 } else { 257 };
                output0[k] *= s;
                output1[k] *= s;
            }
        }
    }
    (output0, output1, rgb_hdr, alpha_hdr)
}

/// `unorm16_to_sf16`: unorm16 [0, 65535] to float16 in range [0, 1].
fn unorm16_to_sf16(p: i32) -> i32 {
    if p == 0xFFFF {
        return 0x3C00;
    }
    if p < 4 {
        return p << 8;
    }
    let lz = (p as u32).leading_zeros() as i32 - 16;
    let mut v = p.wrapping_mul(1 << (lz + 1));
    v &= 0xFFFF;
    v = ((v as u32) >> 6) as i32;
    v | ((14 - lz) << 10)
}

/// `lns_to_sf16`: 16-bit LNS to float16.
fn lns_to_sf16(p: i32) -> i32 {
    let mc = p & 0x7FF;
    let ec = (p as u32 >> 11) as i32;
    let mt = if mc < 512 {
        mc * 3
    } else if mc < 1536 {
        mc * 4 - 512
    } else {
        mc * 5 - 2048
    };
    let res = (ec << 10) | ((mt as u32) >> 3) as i32;
    res.min(0x7BFF)
}

/// `float16_to_float` (F16C `vcvtph2ps`: exact, NaNs quietened).
pub fn float16_to_float(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1F);
    let mant = u32::from(h & 0x3FF);
    if exp == 0x1F {
        let nan_bit = if mant != 0 { 0x40_0000 } else { 0 };
        return f32::from_bits(sign | 0x7F80_0000 | nan_bit | (mant << 13));
    }
    if exp == 0 {
        // zero / denormal: exact as mant * 2^-24
        let v = mant as f32 * f32::from_bits(0x3380_0000);
        return f32::from_bits(sign | v.to_bits());
    }
    f32::from_bits(sign | ((exp + 112) << 23) | (mant << 13))
}

/// `float_to_float16` (F16C `vcvtps2ph` with round to nearest even; NaNs
/// quietened).
pub fn float_to_float16(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x7F_FFFF;
    if exp == 0xFF {
        if mant != 0 {
            return sign | 0x7C00 | 0x200 | (mant >> 13) as u16;
        }
        return sign | 0x7C00;
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        // denormal or zero half
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut r = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && r & 1 != 0) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 != 0) {
        r += 1;
    }
    sign | r as u16
}

/// Port of `image_block` (decoded texels).
struct ImageBlock {
    data: [[f32; 4]; BLOCK_MAX_TEXELS],
    decode_unorm8: bool,
}

/// `error_color_nan`: turns into 0xFFFF when converted to an FP16 NaN.
fn error_color_nan() -> f32 {
    f32::from_bits(0xFFFF_E000)
}

/// Port of `decompress_symbolic_block` (with `unpack_weights`).
fn decompress_symbolic_block(
    decode_mode: AstcencProfile,
    bsd: &BlockSizeDescriptor,
    scb: &SymbolicBlock,
    blk: &mut ImageBlock,
) {
    let texel_count = bsd.texel_count;
    // Decode mode or sRGB forces writing to unorm8 output value
    let u8_mask = blk.decode_unorm8 || decode_mode == AstcencProfile::LdrSrgb;

    // If we detected an error-block, blow up immediately.
    if scb.block_type == BlockType::Error {
        for t in blk.data.iter_mut().take(texel_count) {
            *t = [error_color_nan(); 4];
        }
        return;
    }

    if scb.block_type == BlockType::ConstF16 || scb.block_type == BlockType::ConstU16 {
        let color: [f32; 4] = if scb.block_type == BlockType::ConstU16 {
            // UNORM16 constant color block: the real decoder would just use the
            // top 8 bits, but we rescale into a 16-bit value that rounds correctly.
            scb.constant_color.map(|c| {
                let c = if u8_mask { (c >> 8) * 257 } else { c };
                float16_to_float(unorm16_to_sf16(c) as u16)
            })
        } else {
            // FLOAT16 constant color block
            match decode_mode {
                AstcencProfile::LdrSrgb | AstcencProfile::Ldr => [error_color_nan(); 4],
                // Constant-color block; unpack from FP16 to FP32. The F16C vector
                // conversion packs with signed saturation (`_mm_packs_epi32`), so
                // values >= 0x8000 (negative halves) become 0x7FFF, a NaN.
                _ => scb
                    .constant_color
                    .map(|c| float16_to_float(c.clamp(-0x8000, 0x7FFF) as u16)),
            }
        };
        for t in blk.data.iter_mut().take(texel_count) {
            *t = color;
        }
        return;
    }

    let Some(bm) = bsd.block_modes[scb.block_mode] else {
        return;
    };
    let di = &bsd.decimation_tables[bm.decimation_mode];
    let partition_of_texel: &[u8] = if scb.partition_count == 1 {
        &bsd.partition_of_texel[0][0]
    } else {
        &bsd.partition_of_texel[scb.partition_count - 1][scb.partition_index]
    };

    // Unquantize and undecimate the weights
    let infill = |offset: usize, t: usize| -> i32 {
        let mut sum = 8i32;
        for j in 0..di.texel_weight_count[t] as usize {
            sum += i32::from(scb.weights[offset + di.texel_weights[t][j] as usize])
                * i32::from(di.texel_weight_contribs[t][j]);
        }
        sum >> 4
    };

    let endpoints: Vec<(Vint4, Vint4, bool, bool)> = (0..scb.partition_count)
        .map(|i| unpack_color_endpoints(decode_mode, scb.color_formats[i], &scb.color_values[i]))
        .collect();

    for (t, &part) in partition_of_texel.iter().enumerate().take(texel_count) {
        let part = part as usize;
        let Some(&(ep0, ep1, rgb_lns, a_lns)) = endpoints.get(part) else {
            continue;
        };
        let w1 = infill(0, t);
        let w2 = if bm.is_dual_plane {
            infill(WEIGHTS_PLANE2_OFFSET, t)
        } else {
            0
        };
        let mut out = [0f32; 4];
        for c in 0..4 {
            let weight = if c as i32 == scb.plane2_component {
                w2
            } else {
                w1
            };
            // lerp_color_int
            let mut color = (ep0[c] * (64 - weight) + ep1[c] * weight + 32) >> 6;
            if u8_mask {
                // For decode_unorm8 values force the codec to bit replicate.
                color = (color >> 8) * 257;
            }
            // decode_texel
            let lns = if c == 3 { a_lns } else { rgb_lns };
            let h = if lns {
                lns_to_sf16(color)
            } else {
                unorm16_to_sf16(color)
            };
            out[c] = float16_to_float(h as u16);
        }
        blk.data[t] = out;
    }
}

/// SSE `_mm_max_ps`: the second operand is returned if either is NaN.
fn sse_max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// SSE `_mm_min_ps`: the second operand is returned if either is NaN.
fn sse_min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `float_to_int_rtn`: (x + 0.5) truncated (`_mm_cvttps_epi32`).
fn float_to_int_rtn(a: f32) -> i32 {
    let v = a + 0.5;
    if v.is_nan() || v >= 2147483648.0 || v < -2147483648.0 {
        i32::MIN
    } else {
        v as i32
    }
}

/// The swizzled value of `swz` in a texel.
fn swizzle_value(swz: AstcencSwz, data: &[f32; 7]) -> f32 {
    data[swz as usize]
}

/// Port of `store_image_block`.
fn store_image_block(
    img: &mut AstcencImage,
    blk: &ImageBlock,
    bsd: &BlockSizeDescriptor,
    pos: (usize, usize, usize),
    swz: &AstcencSwizzle,
) {
    let (pos_x, pos_y, pos_z) = pos;
    let size_x = img.dim_x as usize;
    let end_x = size_x.min(pos_x + bsd.dim_x as usize);
    let end_y = (img.dim_y as usize).min(pos_y + bsd.dim_y as usize);
    let end_z = (img.dim_z as usize).min(pos_z + bsd.dim_z as usize);
    let needs_swz = *swz != AstcencSwizzle::RGBA;
    let needs_z = [swz.r, swz.g, swz.b, swz.a].contains(&AstcencSwz::Z);

    // The swizzle source table of a texel (needs_z: reconstructed Z).
    let table = |t: &[f32; 4], u8_out: bool| -> [f32; 7] {
        let mut d = [t[0], t[1], t[2], t[3], 0.0, 1.0, 0.0];
        if needs_z {
            let xn = t[0] * 2.0 - 1.0;
            let yn = t[3] * 2.0 - 1.0;
            let zn = 1.0 - xn * xn - yn * yn;
            if u8_out {
                d[6] = sse_max(zn, 0.0).sqrt() * 0.5 + 0.5;
            } else {
                d[6] = (if zn < 0.0 { 0.0 } else { zn }).sqrt() * 0.5 + 0.5;
            }
        }
        d
    };

    for z in pos_z..end_z {
        for y in pos_y..end_y {
            for x in pos_x..end_x {
                let idx = ((z - pos_z) * bsd.dim_y as usize + (y - pos_y)) * bsd.dim_x as usize
                    + (x - pos_x);
                let texel = &blk.data[idx];
                let o = 4 * (size_x * y + x);
                match &mut img.data {
                    AstcencImageData::U8(planes) => {
                        let Some(plane) = planes.get_mut(z) else {
                            continue;
                        };
                        // Clamp values to [0.0, 1.0] range before unorm conversion
                        let to_u8 =
                            |v: f32| float_to_int_rtn(sse_min(sse_max(v, 0.0), 1.0) * 255.0);
                        let mut rgba = texel.map(to_u8);
                        if needs_swz {
                            let d = table(texel, true);
                            let pick = |s: AstcencSwz| match s {
                                AstcencSwz::Zero => 0,
                                AstcencSwz::One => 255,
                                AstcencSwz::Z => float_to_int_rtn(sse_min(d[6], 1.0) * 255.0),
                                _ => rgba[s as usize],
                            };
                            rgba = [pick(swz.r), pick(swz.g), pick(swz.b), pick(swz.a)];
                        }
                        // Errors are NaN encoded - convert to magenta error color
                        if texel[0].is_nan() {
                            rgba = [0xFF, 0x00, 0xFF, 0xFF];
                        }
                        if let Some(dst) = plane.get_mut(o..o + 4) {
                            for (d, v) in dst.iter_mut().zip(rgba) {
                                *d = v as u8;
                            }
                        }
                    }
                    AstcencImageData::F16(planes) => {
                        let Some(plane) = planes.get_mut(z) else {
                            continue;
                        };
                        let colorf = if needs_swz {
                            let d = table(texel, false);
                            [swz.r, swz.g, swz.b, swz.a].map(|s| swizzle_value(s, &d))
                        } else {
                            *texel
                        };
                        if let Some(dst) = plane.get_mut(o..o + 4) {
                            for (d, v) in dst.iter_mut().zip(colorf) {
                                *d = float_to_float16(v);
                            }
                        }
                    }
                    AstcencImageData::F32(planes) => {
                        let Some(plane) = planes.get_mut(z) else {
                            continue;
                        };
                        let colorf = if needs_swz {
                            let d = table(texel, false);
                            [swz.r, swz.g, swz.b, swz.a].map(|s| swizzle_value(s, &d))
                        } else {
                            *texel
                        };
                        if let Some(dst) = plane.get_mut(o..o + 4) {
                            dst.copy_from_slice(&colorf);
                        }
                    }
                }
            }
        }
    }
}

/// Port of `astcenc_decompress_image`.
pub fn astcenc_decompress_image(
    context: &mut AstcencContext,
    data: &[u8],
    image_out: &mut AstcencImage,
    swizzle: &AstcencSwizzle,
    thread_index: u32,
) -> Result<(), AstcencError> {
    // Today this doesn't matter (working set on stack) but might in future ...
    if thread_index >= context.thread_count {
        return Err(AstcencError::BadParam);
    }
    let dim_x = image_out.dim_x as usize;
    let dim_y = image_out.dim_y as usize;
    let dim_z = image_out.dim_z as usize;
    if dim_x == 0 || dim_y == 0 || dim_z == 0 {
        return Err(AstcencError::BadParam);
    }
    let config = context.config;
    let (block_x, block_y, block_z) = (
        config.block_x as usize,
        config.block_y as usize,
        config.block_z as usize,
    );
    let blocks_x = dim_x.div_ceil(block_x);
    let blocks_y = dim_y.div_ceil(block_y);
    let blocks_z = dim_z.div_ceil(block_z);
    let block_count = blocks_x * blocks_y * blocks_z;

    // Check we have enough output space
    if data.len() < block_count * 16 {
        return Err(AstcencError::OutOfMem);
    }

    let mut blk = ImageBlock {
        data: [[0.0; 4]; BLOCK_MAX_TEXELS],
        // Decode mode inferred from the output data type
        decode_unorm8: matches!(image_out.data, AstcencImageData::U8(_)),
    };
    let bsd = &context.bsd;
    for i in 0..block_count {
        // Decode i into x, y, z block indices
        let z = i / (blocks_x * blocks_y);
        let rem = i - z * blocks_x * blocks_y;
        let y = rem / blocks_x;
        let x = rem - y * blocks_x;
        let offset = ((z * blocks_y + y) * blocks_x + x) * 16;
        let pcb = &data[offset..offset + 16];
        let scb = physical_to_symbolic(bsd, pcb);
        decompress_symbolic_block(config.profile, bsd, &scb, &mut blk);
        store_image_block(
            image_out,
            &blk,
            bsd,
            (x * block_x, y * block_y, z * block_z),
            swizzle,
        );
    }
    Ok(())
}
