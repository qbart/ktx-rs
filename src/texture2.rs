//! Port of lib/src/texture2.c / texture2.h: `ktxTexture2` (KTX 2 files).

use std::ops::{Deref, DerefMut};

use crate::defs::{
    KTX_ANIMDATA_KEY, KTX_ORIENTATION_KEY, KTX2_HEADER_SIZE, OrientationX, OrientationY,
    OrientationZ, SupercmpScheme, ktx_padn,
};
use crate::dfd::khr_df::*;
use crate::dfd::{
    CHANNELID, DESCRIPTORBLOCKSIZE, FLAGS, MODEL, PRIMARIES, QUALIFIERS, TEXELBLOCKDIMENSION0,
    TEXELBLOCKDIMENSION1, TEXELBLOCKDIMENSION2, TEXELBLOCKDIMENSION3, TRANSFER, bdb, bdb_mut,
    dfdsamplecount, dfdsetval, dfdsval, dfdval, get_dfd_component_info_unpacked,
    get_dfd_num_components, vk2dfd,
};
use crate::error::{ErrorCode, KtxResult};
use crate::formatsize::FormatSize;
use crate::ktxint::{KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry, ktx_check_header2};
use crate::stream::{FileStream, MemStream, Stream};
use crate::swap::ktx_swap_endian_bytes;
use crate::texture::{
    ClassId, FormatVersion, IterCb, KTX_TEXTURE_CREATE_CHECK_GLTF_BASISU_BIT,
    KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KTX_TEXTURE_CREATE_RAW_KVDATA_BIT,
    KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT, KtxTexture, Texture, TextureCreateFlags, TextureCreateInfo,
    TextureCreateStorage,
};
use crate::vkformat::{
    VkFormat, is_not_srgb_format_but_has_srgb_variant, is_prohibited_format, is_srgb_format,
    vk_format_type_size,
};
use crate::zlib::{ktx_uncompress_zlib_int, ktx_zstd_decompress};

/// Port of `ktxTexture2_private`.
#[derive(Debug, Clone, Default)]
pub struct Texture2Private {
    pub supercompression_global_data: Vec<u8>,
    pub required_level_alignment: u32,
    pub sgd_byte_length: u64,
    /// Always 0 unless created from a stream and the image data is not yet loaded.
    pub first_level_file_offset: u64,
    /// Offsets are from the start of the image data.
    pub level_index: Vec<LevelIndexEntry>,
}

/// Port of `ktxTexture2`.
#[derive(Debug)]
pub struct Texture2 {
    pub(crate) base: Texture,
    pub vk_format: VkFormat,
    /// Data format descriptor (`pDfd`), `p_dfd[0]` = total size in bytes.
    pub p_dfd: Vec<u32>,
    pub supercompression_scheme: SupercmpScheme,
    pub is_video: bool,
    pub duration: u32,
    pub timescale: u32,
    pub loopcount: u32,
    pub(crate) private: Texture2Private,
}

impl Deref for Texture2 {
    type Target = Texture;
    fn deref(&self) -> &Texture {
        &self.base
    }
}

impl DerefMut for Texture2 {
    fn deref_mut(&mut self) -> &mut Texture {
        &mut self.base
    }
}

fn try_alloc(len: usize) -> KtxResult<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| ErrorCode::OutOfMemory)?;
    v.resize(len, 0);
    Ok(v)
}

/// Port of `gcd` (texture2.c).
fn gcd(a: u32, b: u32) -> u32 {
    if a == 0 { b } else { gcd(b % a, a) }
}

/// Port of `lcm4`: least common multiple of `a` and 4.
pub fn lcm4(a: u32) -> u32 {
    if a & 0x03 == 0 {
        return a; // a is a multiple of 4.
    }
    (a * 4) / gcd(a, 4)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InflateFlag {
    DontInflateOnLoad,
    InflateOnLoad,
}

impl Texture2 {
    fn empty() -> Texture2 {
        Texture2 {
            base: Texture::empty(ClassId::Texture2),
            vk_format: VkFormat::UNDEFINED,
            p_dfd: Vec::new(),
            supercompression_scheme: SupercmpScheme::NONE,
            is_video: false,
            duration: 0,
            timescale: 0,
            loopcount: 0,
            private: Texture2Private::default(),
        }
    }

    /// Private data (level index, SGD, alignment).
    pub fn private(&self) -> &Texture2Private {
        &self.private
    }

    /// Mutable private data.
    pub fn private_mut(&mut self) -> &mut Texture2Private {
        &mut self.private
    }

    /// Supercompression global data.
    pub fn supercompression_global_data(&self) -> &[u8] {
        &self.private.supercompression_global_data
    }

    /// Level index (offsets relative to the start of the image data).
    pub fn level_index(&self) -> &[LevelIndexEntry] {
        &self.private.level_index
    }

    fn level_entry(&self, level: u32) -> LevelIndexEntry {
        self.private
            .level_index
            .get(level as usize)
            .copied()
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ construction

    /// Port of `ktxTexture2_construct` / `ktxTexture2_Create`.
    pub fn create(
        create_info: &TextureCreateInfo,
        storage_allocation: TextureCreateStorage,
    ) -> KtxResult<Texture2> {
        let mut dfd;

        let format_size = if create_info.vk_format != VkFormat::UNDEFINED {
            if is_prohibited_format(create_info.vk_format) {
                return Err(ErrorCode::UnsupportedTextureType);
            }
            // Format is unknown or unsupported.
            dfd = vk2dfd(create_info.vk_format).ok_or(ErrorCode::InvalidValue)?;
            FormatSize::init_from_dfd(&mut dfd).map_err(|_| ErrorCode::UnsupportedTextureType)?
        } else {
            let src = create_info.dfd.as_ref().ok_or(ErrorCode::InvalidValue)?;
            let total = *src.first().ok_or(ErrorCode::InvalidValue)? as usize;
            if total == 0 || total.div_ceil(4) > src.len() {
                return Err(ErrorCode::InvalidValue);
            }
            dfd = src[..total.div_ceil(4)].to_vec();
            FormatSize::init_from_dfd(&mut dfd).map_err(|_| ErrorCode::UnsupportedTextureType)?
        };

        let base = Texture::construct(ClassId::Texture2, create_info, &format_size)?;
        let mut this = Texture2::empty();
        this.base = base;
        this.p_dfd = dfd;
        this.vk_format = create_info.vk_format;
        // typeSize cannot be reconstructed from the DFD; derive it from vkFormat.
        this.base.protected.type_size = vk_format_type_size(create_info.vk_format);
        this.supercompression_scheme = SupercmpScheme::NONE;
        this.private.required_level_alignment = this.calc_required_level_alignment();

        // Create levelIndex. Offsets are from start of the KTX2 stream.
        this.private.first_level_file_offset = 0;
        this.private.level_index = vec![LevelIndexEntry::default(); this.num_levels as usize];
        for level in 0..this.num_levels {
            let uncompressed = this.calc_level_size(level, FormatVersion::Two) as u64;
            let offset = this.calc_level_offset(level) as u64;
            if let Some(e) = this.private.level_index.get_mut(level as usize) {
                e.uncompressed_byte_length = uncompressed;
                e.byte_length = uncompressed;
                e.byte_offset = offset;
            }
        }

        if storage_allocation == TextureCreateStorage::AllocStorage {
            this.base.data_size = this.calc_data_size_texture();
            this.base.p_data = Some(try_alloc(this.base.data_size)?);
        }
        Ok(this)
    }

    /// Port of `ktxTexture2_constructCopy` / `ktxTexture2_CreateCopy`.
    /// Loads `orig`'s image data first if it is still pending on its stream.
    pub fn create_copy(orig: &mut Texture2) -> KtxResult<Texture2> {
        if orig.p_data.is_none() && orig.is_active_stream() {
            orig.load_image_data(None)?;
        }
        let mut base = Texture::empty(ClassId::Texture2);
        base.is_array = orig.is_array;
        base.is_cubemap = orig.is_cubemap;
        base.is_compressed = orig.is_compressed;
        base.generate_mipmaps = orig.generate_mipmaps;
        base.base_width = orig.base_width;
        base.base_height = orig.base_height;
        base.base_depth = orig.base_depth;
        base.num_dimensions = orig.num_dimensions;
        base.num_levels = orig.num_levels;
        base.num_layers = orig.num_layers;
        base.num_faces = orig.num_faces;
        base.orientation = orig.orientation;
        base.kv_data_head = HashListCopy::copy(&orig.kv_data_head);
        base.kv_data_len = orig.kv_data_len;
        base.kv_data = orig.kv_data.clone();
        base.data_size = orig.data_size;
        base.p_data = orig.p_data.clone();
        base.protected.format_size = orig.base.protected.format_size;
        base.protected.type_size = orig.base.protected.type_size;
        Ok(Texture2 {
            base,
            vk_format: orig.vk_format,
            p_dfd: orig.p_dfd.clone(),
            supercompression_scheme: orig.supercompression_scheme,
            is_video: orig.is_video,
            duration: orig.duration,
            timescale: orig.timescale,
            loopcount: orig.loopcount,
            private: orig.private.clone(),
        })
    }

    /// Port of `ktxTexture2_constructFromStreamAndHeader` excluding the final
    /// `LOAD_IMAGE_DATA` step, which callers perform with the same stream.
    pub(crate) fn construct_from_stream_and_header(
        stream: &mut dyn Stream,
        header: &mut KtxHeader2,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        let mut this = Texture2::empty();
        let supp_info = ktx_check_header2(header)?;

        this.vk_format = header.vk_format;
        this.supercompression_scheme = header.supercompression_scheme;
        this.base.protected.type_size = header.type_size;
        this.base.num_dimensions = supp_info.texture_dimension as u32;
        this.base.base_width = header.pixel_width;
        match supp_info.texture_dimension {
            1 => {
                this.base.base_height = 1;
                this.base.base_depth = 1;
            }
            2 => {
                this.base.base_height = header.pixel_height;
                this.base.base_depth = 1;
            }
            _ => {
                this.base.base_height = header.pixel_height;
                this.base.base_depth = header.pixel_depth;
            }
        }
        if header.layer_count > 0 {
            this.base.num_layers = header.layer_count;
            this.base.is_array = true;
        } else {
            this.base.num_layers = 1;
            this.base.is_array = false;
        }
        this.base.num_faces = header.face_count;
        this.base.is_cubemap = header.face_count == 6;
        this.base.num_levels = header.level_count;
        this.base.generate_mipmaps = supp_info.generate_mipmaps != 0;

        // Read level index
        let num_levels = this.base.num_levels as usize;
        let mut li = vec![0u8; LEVEL_INDEX_ENTRY_SIZE * num_levels];
        stream.read(&mut li)?;
        this.private.level_index = li
            .as_chunks::<LEVEL_INDEX_ENTRY_SIZE>()
            .0
            .iter()
            .map(|c| LevelIndexEntry::from_bytes(c).unwrap_or_default())
            .collect();

        // Check that the data the header and level index declare is within the
        // file before memory is allocated for it. Pipes have no known size.
        if let Ok(file_size) = stream.getsize() {
            let dfd = &header.data_format_descriptor;
            let kvd = &header.key_value_data;
            let sgd = &header.supercompression_global_data;
            if dfd.byte_offset as u64 + dfd.byte_length as u64 > file_size
                || kvd.byte_offset as u64 + kvd.byte_length as u64 > file_size
                || sgd.byte_offset > file_size
                || sgd.byte_length > file_size - sgd.byte_offset
            {
                return Err(ErrorCode::FileUnexpectedEof);
            }
            for entry in &this.private.level_index {
                if entry.byte_offset > file_size
                    || entry.byte_length > file_size - entry.byte_offset
                {
                    return Err(ErrorCode::FileUnexpectedEof);
                }
            }
        }

        // Rebase index to start of data and save file offset.
        let first = this
            .private
            .level_index
            .last()
            .map(|e| e.byte_offset)
            .unwrap_or(0);
        this.private.first_level_file_offset = first;
        let mut level_mismatch = false;
        for e in this.private.level_index.iter_mut() {
            e.byte_offset = e.byte_offset.wrapping_sub(first);
            if this.supercompression_scheme == SupercmpScheme::NONE
                && e.byte_length != e.uncompressed_byte_length
            {
                // For non-supercompressed files the levels must have matching byte lengths
                level_mismatch = true;
            }
        }
        if level_mismatch {
            return Err(ErrorCode::FileDataError);
        }

        // Read DFD
        let dfd_len = header.data_format_descriptor.byte_length;
        if header.data_format_descriptor.byte_offset == 0 || dfd_len < 16 {
            return Err(ErrorCode::FileDataError); // Missing or too small DFD
        }
        let mut dfd_bytes = vec![0u8; dfd_len as usize];
        stream.read(&mut dfd_bytes)?;
        let mut dfd: Vec<u32> = dfd_bytes
            .chunks(4)
            .map(|c| {
                let mut w = [0u8; 4];
                w[..c.len()].copy_from_slice(c);
                u32::from_le_bytes(w)
            })
            .collect();
        if dfd.first().copied() != Some(dfd_len) {
            return Err(ErrorCode::FileDataError); // byteLength does not match dfdTotalSize
        }
        this.base.protected.format_size = FormatSize::init_from_dfd(&mut dfd)?;
        this.p_dfd = dfd;

        let b = bdb(&this.p_dfd);
        let descriptor_block_size = dfdval(b, DESCRIPTORBLOCKSIZE);
        if descriptor_block_size < 24 || !(descriptor_block_size - 24).is_multiple_of(16) {
            return Err(ErrorCode::FileDataError); // BDFD has invalid size
        }
        let transfer = dfdval(b, TRANSFER);
        if transfer > KHR_DF_TRANSFER_HLG_UNNORMALIZED_OETF {
            return Err(ErrorCode::FileDataError); // Invalid transfer function
        }
        if is_srgb_format(this.vk_format) && transfer != KHR_DF_TRANSFER_SRGB {
            return Err(ErrorCode::FileDataError);
        }
        if is_not_srgb_format_but_has_srgb_variant(this.vk_format)
            && transfer == KHR_DF_TRANSFER_SRGB
        {
            return Err(ErrorCode::FileDataError);
        }

        this.base.is_compressed = (this.base.protected.format_size.flags
            & crate::formatsize::KTX_FORMAT_SIZE_COMPRESSED_BIT)
            != 0;

        let model = dfdval(b, MODEL);
        if (this.supercompression_scheme == SupercmpScheme::BASIS_LZ && model != KHR_DF_MODEL_ETC1S)
            || (this.supercompression_scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
                && model != KHR_DF_MODEL_UASTC_HDR_6x6)
        {
            return Err(ErrorCode::FileDataError);
        }

        // Check compatibility with the KHR_texture_basisu glTF extension, if needed.
        if create_flags & KTX_TEXTURE_CREATE_CHECK_GLTF_BASISU_BIT != 0 {
            let max_dim = header
                .pixel_width
                .max(header.pixel_height)
                .max(header.pixel_depth);
            let full_mip_pyramid_level_count = 32 - max_dim.leading_zeros();
            if header.level_count != 1 && header.level_count != full_mip_pyramid_level_count {
                return Err(ErrorCode::FileDataError);
            }
            if this.base.num_dimensions != 2 || this.base.is_array || this.base.is_cubemap {
                return Err(ErrorCode::FileDataError);
            }
            if !this.base.base_width.is_multiple_of(4) || !this.base.base_height.is_multiple_of(4) {
                return Err(ErrorCode::FileDataError);
            }
            if model != KHR_DF_MODEL_ETC1S && model != KHR_DF_MODEL_UASTC {
                return Err(ErrorCode::FileDataError);
            }
            if model == KHR_DF_MODEL_UASTC
                && this.supercompression_scheme != SupercmpScheme::NONE
                && this.supercompression_scheme != SupercmpScheme::ZSTD
            {
                return Err(ErrorCode::FileDataError);
            }
        }

        let sample_count = dfdsamplecount(b);
        if sample_count == 0 {
            return Err(ErrorCode::FileDataError);
        }
        if model == KHR_DF_MODEL_ETC1S || model == KHR_DF_MODEL_UASTC {
            if !(1..=2).contains(&sample_count)
                || (sample_count == 2 && model == KHR_DF_MODEL_UASTC)
            {
                return Err(ErrorCode::FileDataError);
            }
            if dfdval(b, TEXELBLOCKDIMENSION0) != 3
                || dfdval(b, TEXELBLOCKDIMENSION1) != 3
                || dfdval(b, TEXELBLOCKDIMENSION2) != 0
                || dfdval(b, TEXELBLOCKDIMENSION3) != 0
            {
                return Err(ErrorCode::FileDataError); // Texel block dimension must be 4x4x1x1
            }
        }

        this.private.required_level_alignment = this.calc_required_level_alignment();

        // Load KVData.
        if header.key_value_data.byte_length > 0 {
            let expected = (header.data_format_descriptor.byte_offset as u64
                + header.data_format_descriptor.byte_length as u64
                + 3)
                & !0x3;
            if header.key_value_data.byte_offset as u64 != expected {
                return Err(ErrorCode::FileDataError);
            }
            if create_flags & KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT == 0 {
                let kvd_len = header.key_value_data.byte_length;
                let mut kvd = try_alloc(kvd_len as usize)?;
                stream.read(&mut kvd)?;
                if create_flags & KTX_TEXTURE_CREATE_RAW_KVDATA_BIT == 0 {
                    this.base.kv_data_head.deserialize(&kvd)?;
                    this.parse_orientation_and_anim()?;
                } else {
                    this.base.kv_data_len = kvd_len;
                    this.base.kv_data = Some(kvd);
                }
            } else {
                stream.skip(header.key_value_data.byte_length as u64)?;
            }
        } else if header.key_value_data.byte_offset != 0 {
            return Err(ErrorCode::FileDataError); // Non-zero KVD byteOffset with zero byteLength
        }

        let sgd = header.supercompression_global_data;
        if sgd.byte_length > 0 {
            match this.supercompression_scheme {
                SupercmpScheme::BASIS_LZ | SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE => {}
                SupercmpScheme::NONE | SupercmpScheme::ZSTD | SupercmpScheme::ZLIB => {
                    return Err(ErrorCode::FileDataError); // SGD is not allowed
                }
                _ => return Err(ErrorCode::UnsupportedFeature),
            }
            // There could be padding here so seek to the next item.
            stream.setpos(sgd.byte_offset)?;
            let len = usize::try_from(sgd.byte_length).map_err(|_| ErrorCode::OutOfMemory)?;
            let mut buf = try_alloc(len)?;
            stream.read(&mut buf)?;
            this.private.supercompression_global_data = buf;
            this.private.sgd_byte_length = sgd.byte_length;
        } else if sgd.byte_offset != 0 {
            return Err(ErrorCode::FileDataError); // Non-zero SGD byteOffset with zero byteLength
        } else if this.supercompression_scheme == SupercmpScheme::BASIS_LZ
            || this.supercompression_scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
        {
            return Err(ErrorCode::FileDataError); // SGD is required
        }

        // Calculate size of the image data. Level 0 is the last level in the data.
        let l0 = this.level_entry(0);
        // size_t arithmetic: wraps like C for out-of-order level indices.
        this.base.data_size = usize::try_from(l0.byte_offset.wrapping_add(l0.byte_length))
            .map_err(|_| ErrorCode::FileOverflow)?;
        Ok(this)
    }

    fn parse_orientation_and_anim(&mut self) -> KtxResult<()> {
        if let Ok(orientation) = self.base.kv_data_head.find_value(KTX_ORIENTATION_KEY) {
            // Length includes the terminating NUL.
            if orientation.len() != self.base.num_dimensions as usize + 1 {
                return Err(ErrorCode::FileDataError);
            }
            let o: Vec<u8> = orientation.to_vec();
            if self.base.num_dimensions >= 3 {
                self.base.orientation.z = OrientationZ(o[2]);
            }
            if self.base.num_dimensions >= 2 {
                self.base.orientation.y = OrientationY(o[1]);
            }
            self.base.orientation.x = OrientationX(o[0]);
        }
        if let Ok(anim) = self.base.kv_data_head.find_value(KTX_ANIMDATA_KEY) {
            if anim.len() != 12 {
                return Err(ErrorCode::FileDataError);
            }
            if !self.base.is_array {
                // animData is only valid for array textures.
                return Err(ErrorCode::FileDataError);
            }
            let rd =
                |i: usize| u32::from_le_bytes([anim[i], anim[i + 1], anim[i + 2], anim[i + 3]]);
            let (d, t, l) = (rd(0), rd(4), rd(8));
            self.is_video = true;
            self.duration = d;
            self.timescale = t;
            self.loopcount = l;
        }
        Ok(())
    }

    fn read_header(stream: &mut dyn Stream) -> KtxResult<KtxHeader2> {
        let mut hb = [0u8; KTX2_HEADER_SIZE];
        stream.read(&mut hb)?;
        Ok(KtxHeader2::from_bytes(&hb))
    }

    /// Reads header and constructs; loads image data if requested, using `stream`.
    pub(crate) fn create_from_borrowed_stream(
        stream: &mut dyn Stream,
        header: Option<KtxHeader2>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        let mut header = match header {
            Some(h) => h,
            None => Self::read_header(stream)?,
        };
        let mut this = Self::construct_from_stream_and_header(stream, &mut header, create_flags)?;
        if create_flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT != 0 {
            this.load_image_data_int(stream, None, InflateFlag::InflateOnLoad)?;
        }
        Ok(this)
    }

    /// Port of `ktxTexture2_CreateFromStream`. The stream is kept for deferred
    /// loading unless `KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT` is set.
    pub fn create_from_stream(
        mut stream: Box<dyn Stream>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        let mut this = Self::create_from_borrowed_stream(&mut *stream, None, create_flags)?;
        if create_flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT == 0 {
            this.base.protected.stream = Some(stream);
        }
        Ok(this)
    }

    /// Port of `ktxTexture2_CreateFromStdioStream` for an open [`std::fs::File`].
    pub fn create_from_stdio_stream(
        file: std::fs::File,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        Self::create_from_stream(Box::new(FileStream::construct(file)?), create_flags)
    }

    /// Port of `ktxTexture2_CreateFromNamedFile`.
    pub fn create_from_named_file(
        filename: impl AsRef<std::path::Path>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        Self::create_from_stream(Box::new(FileStream::open(filename)?), create_flags)
    }

    /// Port of `ktxTexture2_CreateFromMemory`. When image data is not loaded
    /// immediately a copy of `bytes` is retained for deferred loading.
    pub fn create_from_memory(
        bytes: &[u8],
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture2> {
        if bytes.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        let mut stream = MemStream::construct_ro(bytes)?;
        let mut this = Self::create_from_borrowed_stream(&mut stream, None, create_flags)?;
        if create_flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT == 0 {
            this.base.protected.stream =
                Some(Box::new(MemStream::construct_ro_owned(bytes.to_vec())?));
        }
        Ok(this)
    }

    /// Port of `ktxTexture2_Destroy`.
    pub fn destroy(self) {}

    // ------------------------------------------------------------ sizes

    /// Port of `ktxTexture2_calcRequiredLevelAlignment`.
    pub fn calc_required_level_alignment(&self) -> u32 {
        if self.supercompression_scheme != SupercmpScheme::NONE {
            1
        } else {
            lcm4(self.base.protected.format_size.block_size_in_bits / 8)
        }
    }

    /// Port of `ktxTexture2_calcPostInflationLevelAlignment`.
    pub fn calc_post_inflation_level_alignment(&self) -> u32 {
        if self.vk_format != VkFormat::UNDEFINED {
            lcm4(self.base.protected.format_size.block_size_in_bits / 8)
        } else {
            16
        }
    }

    /// Port of `ktxTexture2_levelFileOffset`.
    pub fn level_file_offset(&self, level: u32) -> u64 {
        self.level_entry(level)
            .byte_offset
            .wrapping_add(self.private.first_level_file_offset)
    }

    /// Port of `ktxTexture2_levelDataOffset`.
    pub fn level_data_offset(&self, level: u32) -> u64 {
        self.level_entry(level).byte_offset
    }

    // ------------------------------------------------------------ queries

    /// Port of `ktxTexture2_GetComponentInfo`. Returns `(numComponents, componentByteLength)`.
    pub fn get_component_info(&self) -> (u32, u32) {
        get_dfd_component_info_unpacked(&self.p_dfd)
    }

    /// Port of `ktxTexture2_GetNumComponents`.
    pub fn get_num_components(&self) -> u32 {
        let b = bdb(&self.p_dfd);
        let dfd_num_components = get_dfd_num_components(&self.p_dfd);
        let color_model = dfdval(b, MODEL);
        if color_model < KHR_DF_MODEL_DXT1A {
            return dfd_num_components;
        }
        match color_model {
            KHR_DF_MODEL_ETC1S => {
                let channel0 = dfdsval(b, 0, CHANNELID);
                if dfd_num_components == 1 {
                    if channel0 == KHR_DF_CHANNEL_ETC1S_RGB {
                        3
                    } else {
                        1
                    }
                } else {
                    let channel1 = dfdsval(b, 1, CHANNELID);
                    if channel0 == KHR_DF_CHANNEL_ETC1S_RGB && channel1 == KHR_DF_CHANNEL_ETC1S_AAA
                    {
                        4
                    } else {
                        2
                    }
                }
            }
            KHR_DF_MODEL_UASTC => match dfdsval(b, 0, CHANNELID) {
                KHR_DF_CHANNEL_UASTC_RRR => 1,
                KHR_DF_CHANNEL_UASTC_RRRG => 2,
                KHR_DF_CHANNEL_UASTC_RGB => 3,
                KHR_DF_CHANNEL_UASTC_RGBA => 4,
                _ => 1,
            },
            _ => dfd_num_components,
        }
    }

    /// Port of `ktxTexture2_GetTransferFunction_e`.
    pub fn get_transfer_function_e(&self) -> u32 {
        dfdval(bdb(&self.p_dfd), TRANSFER)
    }

    /// Port of `ktxTexture2_GetOETF_e`.
    pub fn get_oetf_e(&self) -> u32 {
        self.get_transfer_function_e()
    }

    /// Port of `ktxTexture2_GetOETF`.
    pub fn get_oetf(&self) -> u32 {
        self.get_transfer_function_e()
    }

    /// Port of `ktxTexture2_GetColorModel_e`.
    pub fn get_color_model_e(&self) -> u32 {
        dfdval(bdb(&self.p_dfd), MODEL)
    }

    /// Port of `ktxTexture2_GetPremultipliedAlpha`.
    pub fn get_premultiplied_alpha(&self) -> bool {
        dfdval(bdb(&self.p_dfd), FLAGS) & KHR_DF_FLAG_ALPHA_PREMULTIPLIED != 0
    }

    /// Port of `ktxTexture2_GetPrimaries_e`.
    pub fn get_primaries_e(&self) -> u32 {
        dfdval(bdb(&self.p_dfd), PRIMARIES)
    }

    /// Port of `ktxTexture2_IsTranscodable`.
    pub fn is_transcodable(&self) -> bool {
        self.needs_transcoding() || self.get_color_model_e() == KHR_DF_MODEL_UASTC_HDR_4x4
    }

    /// Port of `ktxTexture2_IsHDR`.
    pub fn is_hdr(&self) -> bool {
        let b = bdb(&self.p_dfd);
        let model = dfdval(b, MODEL);
        for s in 0..dfdsamplecount(b) {
            if dfdsval(b, s, QUALIFIERS) & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0 {
                return true;
            }
        }
        model == KHR_DF_MODEL_BC6H
            || model == KHR_DF_MODEL_UASTC_HDR_4x4
            || model == KHR_DF_MODEL_UASTC_HDR_6x6
    }

    /// Port of `ktxTexture2_SetTransferFunction`.
    pub fn set_transfer_function(&mut self, tf: u32) -> KtxResult<()> {
        if is_srgb_format(self.vk_format) && tf != KHR_DF_TRANSFER_SRGB {
            return Err(ErrorCode::InvalidOperation);
        }
        if is_not_srgb_format_but_has_srgb_variant(self.vk_format) && tf == KHR_DF_TRANSFER_SRGB {
            return Err(ErrorCode::InvalidOperation);
        }
        dfdsetval(bdb_mut(&mut self.p_dfd), TRANSFER, tf);
        Ok(())
    }

    /// Port of `ktxTexture2_SetOETF`.
    pub fn set_oetf(&mut self, tf: u32) -> KtxResult<()> {
        self.set_transfer_function(tf)
    }

    /// Port of `ktxTexture2_SetPrimaries`.
    pub fn set_primaries(&mut self, primaries: u32) -> KtxResult<()> {
        dfdsetval(bdb_mut(&mut self.p_dfd), PRIMARIES, primaries);
        Ok(())
    }

    // ------------------------------------------------------------ loading

    /// Port of `ktxTexture2_LoadDeflatedImageData`: load without inflating.
    pub fn load_deflated_image_data(&mut self, buffer: Option<&mut [u8]>) -> KtxResult<()> {
        self.load_with_stream(buffer, InflateFlag::DontInflateOnLoad)
    }

    fn load_with_stream(
        &mut self,
        buffer: Option<&mut [u8]>,
        inflate: InflateFlag,
    ) -> KtxResult<()> {
        if self.base.p_data.is_some() {
            return Err(ErrorCode::InvalidOperation); // Data already loaded.
        }
        let mut stream = self
            .base
            .protected
            .stream
            .take()
            .ok_or(ErrorCode::InvalidOperation)?;
        match self.load_image_data_int(&mut *stream, buffer, inflate) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.base.protected.stream = Some(stream);
                Err(e)
            }
        }
    }

    /// Port of `ktxTexture2_loadImageDataInt`.
    fn load_image_data_int(
        &mut self,
        stream: &mut dyn Stream,
        buffer: Option<&mut [u8]>,
        inflate: InflateFlag,
    ) -> KtxResult<()> {
        if self.base.p_data.is_some() {
            return Err(ErrorCode::InvalidOperation);
        }
        let mut do_inflate = false;
        let output_capacity = if inflate == InflateFlag::InflateOnLoad {
            if self.supercompression_scheme == SupercmpScheme::ZSTD
                || self.supercompression_scheme == SupercmpScheme::ZLIB
            {
                do_inflate = true;
            }
            self.get_data_size_uncompressed()
        } else {
            self.base.data_size
        };

        let use_own = buffer.is_none();
        let mut own = if use_own {
            try_alloc(output_capacity)?
        } else {
            Vec::new()
        };
        let dest: &mut [u8] = match buffer {
            None => &mut own,
            Some(b) => {
                if b.len() < output_capacity {
                    return Err(ErrorCode::InvalidValue);
                }
                b
            }
        };

        // Seek to first level as there may be padding after metadata/sgd.
        stream.setpos(self.private.first_level_file_offset)?;
        if do_inflate {
            let mut deflated = try_alloc(self.base.data_size)?;
            stream.read(&mut deflated)?;
            if self.supercompression_scheme == SupercmpScheme::ZSTD {
                self.inflate_zstd_int(&deflated, dest)?;
            } else {
                self.inflate_zlib_int(&deflated, dest)?;
            }
        } else {
            let n = self.base.data_size;
            stream.read(dest.get_mut(..n).ok_or(ErrorCode::InvalidValue)?)?;
        }

        if cfg!(target_endian = "big") {
            // Perform endianness conversion on texture data per level.
            let width = self.base.protected.type_size as usize;
            if matches!(width, 2 | 4 | 8) {
                for level in 0..self.base.num_levels {
                    let e = self.level_entry(level);
                    let start = e.byte_offset as usize;
                    let end = start + e.byte_length as usize;
                    if let Some(s) = dest.get_mut(start..end) {
                        ktx_swap_endian_bytes(s, width);
                    }
                }
            }
        }

        if use_own {
            self.base.p_data = Some(own);
        }
        // No further need for stream or file offset.
        self.private.first_level_file_offset = 0;
        Ok(())
    }

    /// Port of `ktxTexture2_inflateZstdInt`.
    pub(crate) fn inflate_zstd_int(
        &mut self,
        deflated: &[u8],
        inflated: &mut [u8],
    ) -> KtxResult<()> {
        if self.supercompression_scheme != SupercmpScheme::ZSTD {
            return Err(ErrorCode::InvalidOperation);
        }
        self.inflate_levels(deflated, inflated, ktx_zstd_decompress)
    }

    /// Port of `ktxTexture2_inflateZLIBInt`.
    pub(crate) fn inflate_zlib_int(
        &mut self,
        deflated: &[u8],
        inflated: &mut [u8],
    ) -> KtxResult<()> {
        if self.supercompression_scheme != SupercmpScheme::ZLIB {
            return Err(ErrorCode::InvalidOperation);
        }
        self.inflate_levels(deflated, inflated, ktx_uncompress_zlib_int)
    }

    fn inflate_levels(
        &mut self,
        deflated: &[u8],
        inflated: &mut [u8],
        decompress: fn(&mut [u8], &[u8]) -> KtxResult<usize>,
    ) -> KtxResult<()> {
        let num_levels = self.base.num_levels as usize;
        let mut nindex = vec![LevelIndexEntry::default(); num_levels];
        let alignment = self.calc_post_inflation_level_alignment();
        let mut capacity = inflated.len();
        let mut level_offset: usize = 0;
        let mut inflated_byte_length: usize = 0;
        for level in (0..num_levels).rev() {
            let c = self
                .private
                .level_index
                .get(level)
                .copied()
                .unwrap_or_default();
            let start = usize::try_from(c.byte_offset).map_err(|_| ErrorCode::FileDataError)?;
            let len = usize::try_from(c.byte_length).map_err(|_| ErrorCode::FileDataError)?;
            let src = deflated
                .get(start..start.saturating_add(len))
                .ok_or(ErrorCode::FileDataError)?;
            let dst = inflated
                .get_mut(level_offset..)
                .ok_or(ErrorCode::DecompressLengthError)?;
            let dst_len = dst.len().min(capacity);
            let level_byte_length = decompress(&mut dst[..dst_len], src)?;
            if c.uncompressed_byte_length != level_byte_length as u64 {
                return Err(ErrorCode::DecompressLengthError);
            }
            let e = &mut nindex[level];
            e.byte_offset = level_offset as u64;
            e.byte_length = level_byte_length as u64;
            e.uncompressed_byte_length = level_byte_length as u64;
            // Level 0 is last in the file so needs no padding.
            let padded = if level == 0 {
                level_byte_length
            } else {
                ktx_padn(alignment as u64, level_byte_length as u64) as usize
            };
            inflated_byte_length += padded;
            level_offset += padded;
            if capacity < padded {
                return Err(ErrorCode::DecompressLengthError);
            }
            capacity -= padded;
        }
        // Now modify the texture.
        self.base.data_size = inflated_byte_length;
        self.supercompression_scheme = SupercmpScheme::NONE;
        self.private.level_index = nindex;
        self.private.required_level_alignment = alignment;
        Ok(())
    }
}

/// Helper so `create_copy` reads naturally.
struct HashListCopy;
impl HashListCopy {
    fn copy(h: &crate::hashlist::HashList) -> crate::hashlist::HashList {
        crate::hashlist::HashList::construct_copy(h)
    }
}

impl KtxTexture for Texture2 {
    fn base(&self) -> &Texture {
        &self.base
    }

    /// Port of `ktxTexture2_GetVkFormat`.
    fn get_vk_format(&self) -> VkFormat {
        self.vk_format
    }

    fn base_mut(&mut self) -> &mut Texture {
        &mut self.base
    }

    /// Port of `ktxTexture2_calcDataSizeLevels`.
    fn calc_data_size_levels(&self, levels: u32) -> usize {
        if levels == 0 {
            return 0;
        }
        let align = self.private.required_level_alignment as u64;
        let mut data_size = 0usize;
        for i in (1..levels).rev() {
            let level_size = self.calc_level_size(i, FormatVersion::Two);
            data_size += ktx_padn(align, level_size as u64) as usize;
        }
        data_size + self.calc_level_size(0, FormatVersion::Two)
    }

    /// Port of `ktxTexture2_calcFaceLodSize`.
    fn calc_face_lod_size(&self, level: u32) -> usize {
        if self.is_cubemap && !self.is_array {
            self.calc_image_size(level, FormatVersion::Two)
        } else {
            self.level_entry(level).uncompressed_byte_length as usize
        }
    }

    /// Port of `ktxTexture2_calcLevelOffset`.
    fn calc_level_offset(&self, level: u32) -> usize {
        let align = self.private.required_level_alignment as u64;
        let mut level_offset = 0usize;
        let mut i = self.num_levels;
        while i > level + 1 {
            i -= 1;
            let level_size = self.calc_level_size(i, FormatVersion::Two);
            level_offset += ktx_padn(align, level_size as u64) as usize;
        }
        level_offset
    }

    /// Port of `ktxTexture2_GetImageOffset`.
    fn get_image_offset(&self, level: u32, layer: u32, face_slice: u32) -> KtxResult<usize> {
        if level >= self.num_levels || layer >= self.num_layers {
            return Err(ErrorCode::InvalidOperation);
        }
        if self.supercompression_scheme != SupercmpScheme::NONE {
            return Err(ErrorCode::InvalidOperation);
        }
        if self.is_cubemap {
            if face_slice >= self.num_faces {
                return Err(ErrorCode::InvalidOperation);
            }
        } else {
            let max_slice = self.base_depth.checked_shr(level).unwrap_or(0).max(1);
            if face_slice >= max_slice {
                return Err(ErrorCode::InvalidOperation);
            }
        }
        let mut offset = self.level_data_offset(level) as usize;
        if layer != 0 {
            offset += layer as usize * self.layer_size(level, FormatVersion::Two);
        }
        if face_slice != 0 {
            offset += face_slice as usize * self.get_image_size(level);
        }
        Ok(offset)
    }

    /// Port of `ktxTexture2_GetDataSizeUncompressed`.
    fn get_data_size_uncompressed(&self) -> usize {
        match self.supercompression_scheme {
            SupercmpScheme::BASIS_LZ
            | SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
            | SupercmpScheme::NONE => self.data_size,
            SupercmpScheme::ZSTD | SupercmpScheme::ZLIB => {
                let alignment = self.calc_post_inflation_level_alignment() as u64;
                let mut size: u64 = 0;
                for level in (1..self.num_levels).rev() {
                    let l = self.level_entry(level).uncompressed_byte_length;
                    size = size.saturating_add(ktx_padn(alignment, l));
                }
                size = size.saturating_add(self.level_entry(0).uncompressed_byte_length);
                usize::try_from(size).unwrap_or(usize::MAX)
            }
            _ => 0,
        }
    }

    /// Port of `ktxTexture2_GetImageSize`.
    fn get_image_size(&self, level: u32) -> usize {
        self.calc_image_size(level, FormatVersion::Two)
    }

    /// Port of `ktxTexture2_GetLevelSize`.
    fn get_level_size(&self, level: u32) -> usize {
        self.calc_level_size(level, FormatVersion::Two)
    }

    /// Port of `ktxTexture2_IterateLevels`.
    fn iterate_levels(&self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        if self.supercompression_scheme != SupercmpScheme::NONE {
            return Err(ErrorCode::InvalidOperation);
        }
        let data = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        for level in (0..self.num_levels).rev() {
            let (width, height, depth) = self.level_dims(level);
            let level_size = self.level_entry(level).uncompressed_byte_length;
            let offset = self.level_data_offset(level) as usize;
            let pixels = data
                .get(offset..offset + level_size as usize)
                .ok_or(ErrorCode::FileDataError)?;
            iter_cb(level, 0, width, height, depth, level_size, pixels)?;
        }
        Ok(())
    }

    /// Port of `ktxTexture2_IterateLoadLevelFaces`.
    fn iterate_load_level_faces(&mut self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        if self.supercompression_scheme != SupercmpScheme::NONE
            && self.supercompression_scheme != SupercmpScheme::ZSTD
            && self.supercompression_scheme != SupercmpScheme::ZLIB
        {
            return Err(ErrorCode::InvalidOperation);
        }
        // Not created from a stream or images are already loaded.
        let mut stream = self
            .base
            .protected
            .stream
            .take()
            .ok_or(ErrorCode::InvalidOperation)?;
        match self.iterate_load_level_faces_int(&mut *stream, iter_cb) {
            Ok(()) => {
                self.private.first_level_file_offset = 0;
                Ok(())
            }
            Err(e) => {
                self.base.protected.stream = Some(stream);
                Err(e)
            }
        }
    }

    /// Port of `ktxTexture2_LoadImageData`.
    fn load_image_data(&mut self, buffer: Option<&mut [u8]>) -> KtxResult<()> {
        self.load_with_stream(buffer, InflateFlag::InflateOnLoad)
    }

    /// Port of `ktxTexture2_NeedsTranscoding`.
    fn needs_transcoding(&self) -> bool {
        let model = self.get_color_model_e();
        model == KHR_DF_MODEL_ETC1S
            || model == KHR_DF_MODEL_UASTC
            || model == KHR_DF_MODEL_UASTC_HDR_6x6
    }
}

impl Texture2 {
    fn iterate_load_level_faces_int(
        &self,
        stream: &mut dyn Stream,
        iter_cb: &mut IterCb<'_>,
    ) -> KtxResult<()> {
        let data_size = self.level_entry(0).byte_length as usize;
        let mut data_buf = try_alloc(data_size)?;
        let scheme = self.supercompression_scheme;
        let inflating = scheme == SupercmpScheme::ZSTD || scheme == SupercmpScheme::ZLIB;
        let uncompressed_data_size = self.level_entry(0).uncompressed_byte_length as usize;
        let mut uncompressed_buf = if inflating {
            try_alloc(uncompressed_data_size)?
        } else {
            Vec::new()
        };
        let fs = self.base.protected.format_size;

        for level in (0..self.num_levels).rev() {
            let (width, height, depth) = self.level_dims(level);
            let mut level_size = self.level_entry(level).byte_length as usize;
            if data_size < level_size {
                // Levels cannot be larger than the base level
                return Err(ErrorCode::FileDataError);
            }
            // Use setpos so we skip any padding.
            stream.setpos(self.level_file_offset(level))?;
            stream.read(&mut data_buf[..level_size])?;
            if scheme == SupercmpScheme::ZSTD {
                level_size = ktx_zstd_decompress(&mut uncompressed_buf, &data_buf[..level_size])?;
            } else if scheme == SupercmpScheme::ZLIB {
                level_size =
                    ktx_uncompress_zlib_int(&mut uncompressed_buf, &data_buf[..level_size])?;
            }
            if self.level_entry(level).uncompressed_byte_length != level_size as u64 {
                return Err(ErrorCode::DecompressLengthError);
            }
            let p_data: &mut [u8] = if inflating {
                &mut uncompressed_buf
            } else {
                &mut data_buf
            };
            if cfg!(target_endian = "big") {
                let w = self.base.protected.type_size as usize;
                if matches!(w, 2 | 4 | 8) {
                    ktx_swap_endian_bytes(&mut p_data[..level_size], w);
                }
            }
            if self.is_cubemap && !self.is_array {
                let bx = width.div_ceil(fs.block_width.max(1)).max(fs.min_blocks_x);
                // C uses minBlocksX for y as well.
                let by = height.div_ceil(fs.block_height.max(1)).max(fs.min_blocks_x);
                let face_size = bx as usize * by as usize * fs.block_size_in_bits as usize / 8;
                for face in 0..self.num_faces {
                    let start = face as usize * face_size;
                    let pixels = p_data
                        .get(start..start + face_size)
                        .ok_or(ErrorCode::FileDataError)?;
                    iter_cb(level, face, width, height, depth, face_size as u64, pixels)?;
                }
            } else {
                iter_cb(
                    level,
                    0,
                    width,
                    height,
                    depth,
                    level_size as u64,
                    &p_data[..level_size],
                )?;
            }
        }
        Ok(())
    }
}
