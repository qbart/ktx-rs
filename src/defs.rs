//! Public definitions from `ktx.h` that are not tied to a specific object.

/// Key string for animation metadata.
pub const KTX_ANIMDATA_KEY: &str = "KTXanimData";
/// Key string for orientation metadata.
pub const KTX_ORIENTATION_KEY: &str = "KTXorientation";
/// Key string for swizzle metadata.
pub const KTX_SWIZZLE_KEY: &str = "KTXswizzle";
/// Key string for map range metadata.
pub const KTX_MAP_RANGE_KEY: &str = "KTXmapRange";
/// Key string for writer metadata.
pub const KTX_WRITER_KEY: &str = "KTXwriter";
/// Key string for writer supercompression parameters.
pub const KTX_WRITER_SCPARAMS_KEY: &str = "KTXwriterScParams";
/// Required unpack alignment.
pub const KTX_GL_UNPACK_ALIGNMENT: u32 = 4;
/// Port of `KTX_FACESLICE_WHOLE_LEVEL`.
pub const KTX_FACESLICE_WHOLE_LEVEL: u32 = u32::MAX;

/// KTX 1 file identifier.
pub const KTX_IDENTIFIER_REF: [u8; 12] =
    [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
/// KTX 2 file identifier.
pub const KTX2_IDENTIFIER_REF: [u8; 12] =
    [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
/// Endianness marker as written by a little/big matching writer.
pub const KTX_ENDIAN_REF: u32 = 0x04030201;
/// Endianness marker from an opposite-endian writer.
pub const KTX_ENDIAN_REF_REV: u32 = 0x01020304;
/// Size of a KTX 1 header.
pub const KTX_HEADER_SIZE: usize = 64;
/// Size of a KTX 2 header.
pub const KTX2_HEADER_SIZE: usize = 80;

/// Port of `ktxOrientationX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrientationX {
    Left = b'l',
    Right = b'r',
}

/// Port of `ktxOrientationY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrientationY {
    Up = b'u',
    Down = b'd',
}

/// Port of `ktxOrientationZ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrientationZ {
    In = b'i',
    Out = b'o',
}

/// Port of `struct ktxOrientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orientation {
    pub x: OrientationX,
    pub y: OrientationY,
    pub z: OrientationZ,
}

impl Default for Orientation {
    fn default() -> Self {
        Orientation { x: OrientationX::Right, y: OrientationY::Down, z: OrientationZ::Out }
    }
}

/// Port of `ktxSupercmpScheme`. Transparent `u32` because files may hold any value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct SupercmpScheme(pub u32);

impl SupercmpScheme {
    pub const NONE: SupercmpScheme = SupercmpScheme(0);
    pub const BASIS_LZ: SupercmpScheme = SupercmpScheme(1);
    pub const ZSTD: SupercmpScheme = SupercmpScheme(2);
    pub const ZLIB: SupercmpScheme = SupercmpScheme(3);
    pub const UASTC_HDR_6X6_INTERMEDIATE: SupercmpScheme = SupercmpScheme(4);
    pub const BEGIN_RANGE: SupercmpScheme = SupercmpScheme::NONE;
    pub const END_RANGE: SupercmpScheme = SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE;
    pub const BEGIN_VENDOR_RANGE: SupercmpScheme = SupercmpScheme(0x10000);
    pub const END_VENDOR_RANGE: SupercmpScheme = SupercmpScheme(0x1ffff);
    pub const BEGIN_RESERVED: SupercmpScheme = SupercmpScheme(0x20000);
}

/// Port of `ktx_transcode_fmt_e`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TranscodeFmt(pub u32);

#[allow(non_upper_case_globals)]
impl TranscodeFmt {
    pub const ETC1_RGB: TranscodeFmt = TranscodeFmt(0);
    pub const ETC2_RGBA: TranscodeFmt = TranscodeFmt(1);
    pub const BC1_RGB: TranscodeFmt = TranscodeFmt(2);
    pub const BC3_RGBA: TranscodeFmt = TranscodeFmt(3);
    pub const BC4_R: TranscodeFmt = TranscodeFmt(4);
    pub const BC5_RG: TranscodeFmt = TranscodeFmt(5);
    pub const BC7_RGBA: TranscodeFmt = TranscodeFmt(6);
    pub const PVRTC1_4_RGB: TranscodeFmt = TranscodeFmt(8);
    pub const PVRTC1_4_RGBA: TranscodeFmt = TranscodeFmt(9);
    pub const ASTC_LDR_4x4_RGBA: TranscodeFmt = TranscodeFmt(10);
    pub const ASTC_4x4_RGBA: TranscodeFmt = TranscodeFmt::ASTC_LDR_4x4_RGBA;
    pub const PVRTC2_4_RGB: TranscodeFmt = TranscodeFmt(18);
    pub const PVRTC2_4_RGBA: TranscodeFmt = TranscodeFmt(19);
    pub const ETC2_EAC_R11: TranscodeFmt = TranscodeFmt(20);
    pub const ETC2_EAC_RG11: TranscodeFmt = TranscodeFmt(21);
    pub const RGBA32: TranscodeFmt = TranscodeFmt(13);
    pub const RGB565: TranscodeFmt = TranscodeFmt(14);
    pub const BGR565: TranscodeFmt = TranscodeFmt(15);
    pub const RGBA4444: TranscodeFmt = TranscodeFmt(16);
    pub const ETC: TranscodeFmt = TranscodeFmt(22);
    pub const BC1_OR_3: TranscodeFmt = TranscodeFmt(23);
    pub const RGB_HALF: TranscodeFmt = TranscodeFmt(24);
    pub const RGBA_HALF: TranscodeFmt = TranscodeFmt(25);
    pub const RGB_9E5: TranscodeFmt = TranscodeFmt(26);
    pub const ASTC_HDR_4x4_RGBA: TranscodeFmt = TranscodeFmt(29);
    pub const ASTC_HDR_6x6_RGBA: TranscodeFmt = TranscodeFmt(30);
    pub const BC6HU_RGB: TranscodeFmt = TranscodeFmt(31);
    pub const BC6HU: TranscodeFmt = TranscodeFmt::BC6HU_RGB;
    pub const NOSELECTION: TranscodeFmt = TranscodeFmt(0x7fffffff);
}

/// Port of `ktx_transcode_flag_bits_e`.
pub mod transcode_flags {
    pub const PVRTC_DECODE_TO_NEXT_POW2: u32 = 2;
    pub const TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS: u32 = 4;
    pub const HIGH_QUALITY: u32 = 32;
    pub const NO_ETC1S_CHROMA_FILTERING: u32 = 64;
}

/// Port of `ktxTranscodeFormatString`.
pub fn ktx_transcode_format_string(format: TranscodeFmt) -> &'static str {
    match format {
        TranscodeFmt::ETC1_RGB => "ETC1_RGB",
        TranscodeFmt::ETC2_RGBA => "ETC2_RGBA",
        TranscodeFmt::BC1_RGB => "BC1_RGB",
        TranscodeFmt::BC3_RGBA => "BC3_RGBA",
        TranscodeFmt::BC4_R => "BC4_R",
        TranscodeFmt::BC5_RG => "BC5_RG",
        TranscodeFmt::BC7_RGBA => "BC7_RGBA",
        TranscodeFmt::PVRTC1_4_RGB => "PVRTC1_4_RGB",
        TranscodeFmt::PVRTC1_4_RGBA => "PVRTC1_4_RGBA",
        TranscodeFmt::ASTC_4x4_RGBA => "ASTC_4x4_RGBA",
        TranscodeFmt::ASTC_HDR_4x4_RGBA => "KTX_TTF_ASTC_HDR_4x4_RGBA",
        TranscodeFmt::ASTC_HDR_6x6_RGBA => "KTX_TTF_ASTC_HDR_6x6_RGBA",
        TranscodeFmt::RGBA32 => "RGBA32",
        TranscodeFmt::RGBA_HALF => "KTX_TTF_RGBA_HALF",
        TranscodeFmt::RGB565 => "RGB565",
        TranscodeFmt::BGR565 => "BGR565",
        TranscodeFmt::RGBA4444 => "RGBA4444",
        TranscodeFmt::PVRTC2_4_RGB => "PVRTC2_4_RGB",
        TranscodeFmt::PVRTC2_4_RGBA => "PVRTC2_4_RGBA",
        TranscodeFmt::ETC2_EAC_R11 => "ETC2_EAC_R11",
        TranscodeFmt::ETC2_EAC_RG11 => "ETC2_EAC_RG11",
        TranscodeFmt::ETC => "ETC",
        TranscodeFmt::BC1_OR_3 => "BC1 or BC3",
        _ => "Unrecognized format",
    }
}

/// Port of `_KTX_PADN`: round `nbytes` up to a multiple of `n`.
pub const fn ktx_padn(n: u64, nbytes: u64) -> u64 {
    if n == 0 { nbytes } else { nbytes.div_ceil(n) * n }
}

/// Port of `_KTX_PADN_LEN`: padding bytes required to reach a multiple of `n`.
pub const fn ktx_padn_len(n: u64, nbytes: u64) -> u64 {
    ktx_padn(n, nbytes) - nbytes
}

/// Port of `_KTX_PAD4`.
pub const fn ktx_pad4(nbytes: u64) -> u64 {
    ktx_padn(4, nbytes)
}

/// Port of `_KTX_PAD4_LEN`.
pub const fn ktx_pad4_len(nbytes: u64) -> u64 {
    ktx_padn_len(4, nbytes)
}

/// Port of `_KTX_PAD8`.
pub const fn ktx_pad8(nbytes: u64) -> u64 {
    ktx_padn(8, nbytes)
}

/// Port of `_KTX_PAD8_LEN`.
pub const fn ktx_pad8_len(nbytes: u64) -> u64 {
    ktx_padn_len(8, nbytes)
}

/// Port of `_KTX_PAD_UNPACK_ALIGN`.
pub const fn ktx_pad_unpack_align(nbytes: u64) -> u64 {
    ktx_padn(KTX_GL_UNPACK_ALIGNMENT as u64, nbytes)
}

/// Port of `_KTX_PAD_UNPACK_ALIGN_LEN`.
pub const fn ktx_pad_unpack_align_len(nbytes: u64) -> u64 {
    ktx_padn_len(KTX_GL_UNPACK_ALIGNMENT as u64, nbytes)
}
