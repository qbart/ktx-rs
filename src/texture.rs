//! Port of lib/src/texture.c / texture.h: the `ktxTexture` base class.
//!
//! `ktxTexture1` and `ktxTexture2` embed [`Texture`] and deref to it, mirroring the
//! `KTXTEXTURECLASSDEFN` macro. Virtual functions live in the [`KtxTexture`] trait.

use crate::defs::{Orientation, ktx_pad_unpack_align_len};
use crate::error::{ErrorCode, KtxResult};
use crate::formatsize::{FormatSize, KTX_FORMAT_SIZE_COMPRESSED_BIT};
use crate::hashlist::HashList;
use crate::stream::Stream;
use crate::vkformat::VkFormat;

/// Port of `ktxTextureCreateFlagBits`.
pub mod create_flags {
    pub const KTX_TEXTURE_CREATE_NO_FLAGS: u32 = 0x00;
    pub const KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT: u32 = 0x01;
    pub const KTX_TEXTURE_CREATE_RAW_KVDATA_BIT: u32 = 0x02;
    pub const KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT: u32 = 0x04;
    pub const KTX_TEXTURE_CREATE_CHECK_GLTF_BASISU_BIT: u32 = 0x08;
}
pub use create_flags::*;

/// Port of `ktxTextureCreateFlags`.
pub type TextureCreateFlags = u32;

/// Port of `ktxTextureCreateStorageEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureCreateStorage {
    NoStorage = 0,
    AllocStorage = 1,
}

/// Port of `class_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassId {
    Texture1 = 1,
    Texture2 = 2,
}

/// Port of `ktxFormatVersionEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatVersion {
    One = 1,
    Two = 2,
}

/// Port of `ktxTextureCreateInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextureCreateInfo {
    /// Internal format for the texture, e.g. GL_RGB8. Ignored for ktxTexture2.
    pub gl_internalformat: u32,
    /// VkFormat for texture. Ignored for ktxTexture1.
    pub vk_format: VkFormat,
    /// DFD; used only for ktxTexture2 when `vk_format` is UNDEFINED.
    pub dfd: Option<Vec<u32>>,
    pub base_width: u32,
    pub base_height: u32,
    pub base_depth: u32,
    pub num_dimensions: u32,
    pub num_levels: u32,
    pub num_layers: u32,
    pub num_faces: u32,
    pub is_array: bool,
    pub generate_mipmaps: bool,
}

/// Port of `ktxTexture_protected` (minus the vtable).
#[derive(Default)]
pub struct TextureProtected {
    pub format_size: FormatSize,
    pub type_size: u32,
    /// Source stream while image data has not been loaded yet.
    pub stream: Option<Box<dyn Stream>>,
}

impl std::fmt::Debug for TextureProtected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureProtected")
            .field("format_size", &self.format_size)
            .field("type_size", &self.type_size)
            .field("stream", &self.stream.as_ref().map(|s| s.stream_type()))
            .finish()
    }
}

/// Port of `struct ktxTexture` (`KTXTEXTURECLASSDEFN`).
#[derive(Debug)]
pub struct Texture {
    pub class_id: ClassId,
    pub is_array: bool,
    pub is_cubemap: bool,
    pub is_compressed: bool,
    pub generate_mipmaps: bool,
    pub base_width: u32,
    pub base_height: u32,
    pub base_depth: u32,
    pub num_dimensions: u32,
    pub num_levels: u32,
    pub num_layers: u32,
    pub num_faces: u32,
    pub orientation: Orientation,
    /// Parsed key/value data.
    pub kv_data_head: HashList,
    /// Length of raw `kv_data` (only when created with RAW_KVDATA).
    pub kv_data_len: u32,
    /// Raw key/value data (only when created with RAW_KVDATA).
    pub kv_data: Option<Vec<u8>>,
    /// Size of the image data in bytes.
    pub data_size: usize,
    /// Image data. `None` until loaded or allocated.
    pub p_data: Option<Vec<u8>>,
    pub(crate) protected: TextureProtected,
}

/// Port of `PFNKTXITERCB`: `(miplevel, face, width, height, depth, faceLodSize, pixels)`.
pub type IterCb<'a> = dyn FnMut(u32, u32, u32, u32, u32, u64, &[u8]) -> KtxResult<()> + 'a;

impl Texture {
    /// Empty texture used by stream constructors (`ktxTexture_constructFromStream`).
    pub(crate) fn empty(class_id: ClassId) -> Texture {
        Texture {
            class_id,
            is_array: false,
            is_cubemap: false,
            is_compressed: false,
            generate_mipmaps: false,
            base_width: 0,
            base_height: 0,
            base_depth: 0,
            num_dimensions: 0,
            num_levels: 0,
            num_layers: 0,
            num_faces: 0,
            orientation: Orientation::default(),
            kv_data_head: HashList::new(),
            kv_data_len: 0,
            kv_data: None,
            data_size: 0,
            p_data: None,
            protected: TextureProtected::default(),
        }
    }

    /// Port of `ktxTexture_construct`.
    pub(crate) fn construct(
        class_id: ClassId,
        create_info: &TextureCreateInfo,
        format_size: &FormatSize,
    ) -> KtxResult<Texture> {
        let mut this = Texture::empty(class_id);
        this.protected.format_size = *format_size;
        this.is_compressed = (format_size.flags & KTX_FORMAT_SIZE_COMPRESSED_BIT) != 0;

        if create_info.num_dimensions < 1 || create_info.num_dimensions > 3 {
            return Err(ErrorCode::InvalidValue);
        }
        if create_info.base_width == 0 || create_info.base_height == 0 || create_info.base_depth == 0 {
            return Err(ErrorCode::InvalidValue);
        }
        match create_info.num_dimensions {
            1 => {
                if create_info.base_height > 1 || create_info.base_depth > 1 {
                    return Err(ErrorCode::InvalidOperation);
                }
            }
            2 => {
                if create_info.base_depth > 1 {
                    return Err(ErrorCode::InvalidOperation);
                }
            }
            _ => {
                // 3D array textures and 3D cubemaps are not supported.
                if create_info.is_array || create_info.num_faces != 1 || create_info.num_layers != 1 {
                    return Err(ErrorCode::InvalidOperation);
                }
            }
        }
        this.num_dimensions = create_info.num_dimensions;
        this.base_width = create_info.base_width;
        this.base_depth = create_info.base_depth;
        this.base_height = create_info.base_height;

        if create_info.num_layers == 0 {
            return Err(ErrorCode::InvalidValue);
        }
        this.num_layers = create_info.num_layers;
        this.is_array = create_info.is_array;

        if create_info.num_faces == 6 {
            if this.num_dimensions != 2 {
                return Err(ErrorCode::InvalidOperation);
            }
            if create_info.base_width != create_info.base_height {
                return Err(ErrorCode::InvalidOperation);
            }
            this.is_cubemap = true;
        } else if create_info.num_faces != 1 {
            return Err(ErrorCode::InvalidValue);
        }
        this.num_faces = create_info.num_faces;

        if create_info.num_levels == 0 {
            return Err(ErrorCode::InvalidValue);
        }
        this.num_levels = create_info.num_levels;
        this.generate_mipmaps = create_info.generate_mipmaps;

        if create_info.num_levels > 1 {
            let max_dim = create_info.base_width.max(create_info.base_height).max(create_info.base_depth);
            if this.num_levels > 32 || max_dim < (1u32 << (this.num_levels - 1)) {
                // Can't have more mip levels than 1 + log2(max(width, height, depth))
                return Err(ErrorCode::InvalidOperation);
            }
        }
        Ok(this)
    }

    /// Format size information (`_protected->_formatSize`).
    pub fn format_size(&self) -> &FormatSize {
        &self.protected.format_size
    }

    /// Type size (`_protected->_typeSize`).
    pub fn type_size(&self) -> u32 {
        self.protected.type_size
    }

    /// Port of `ktxTexture_GetData`.
    pub fn get_data(&self) -> Option<&[u8]> {
        self.p_data.as_deref()
    }

    /// Mutable access to the image data.
    pub fn get_data_mut(&mut self) -> Option<&mut [u8]> {
        self.p_data.as_deref_mut()
    }

    /// Port of `ktxTexture_GetDataSize`.
    pub fn get_data_size(&self) -> usize {
        self.data_size
    }

    /// Port of `ktxTexture_GetElementSize`.
    pub fn get_element_size(&self) -> u32 {
        self.protected.format_size.block_size_in_bits / 8
    }

    /// Port of `ktxTexture_isActiveStream`.
    pub fn is_active_stream(&self) -> bool {
        self.protected.stream.is_some()
    }

    fn block_counts(&self, level: u32) -> (u32, u32) {
        let fs = &self.protected.format_size;
        let level_width = self.base_width.checked_shr(level).unwrap_or(0);
        let level_height = self.base_height.checked_shr(level).unwrap_or(0);
        // Round up to next whole block (C uses ceilf on floats).
        let bx = level_width.div_ceil(fs.block_width.max(1));
        let by = level_height.div_ceil(fs.block_height.max(1));
        (fs.min_blocks_x.max(bx), fs.min_blocks_y.max(by))
    }

    /// Port of `ktxTexture_calcImageSize`.
    pub fn calc_image_size(&self, level: u32, fv: FormatVersion) -> usize {
        let fs = &self.protected.format_size;
        let (bx, by) = self.block_counts(level);
        let block_size_in_bytes = (fs.block_size_in_bits / 8) as usize;
        if fs.flags & KTX_FORMAT_SIZE_COMPRESSED_BIT != 0 {
            bx as usize * by as usize * block_size_in_bytes
        } else {
            let mut row_bytes = bx as usize * block_size_in_bytes;
            if fv == FormatVersion::One {
                row_bytes += ktx_pad_unpack_align_len(row_bytes as u64) as usize;
            }
            row_bytes * by as usize
        }
    }

    /// Port of `ktxTexture_layerSize`.
    pub fn layer_size(&self, level: u32, fv: FormatVersion) -> usize {
        let fs = &self.protected.format_size;
        let block_depth = fs.block_depth.max(1);
        let level_depth = self.base_depth.checked_shr(level).unwrap_or(0);
        let block_count_z = level_depth.div_ceil(block_depth).max(1);
        let image_size = self.calc_image_size(level, fv);
        let layer_size = image_size * block_count_z as usize;
        // KTX_GL_UNPACK_ALIGNMENT is 4 so no cube padding is needed for KTX1.
        layer_size * self.num_faces as usize
    }

    /// Port of `ktxTexture_calcLevelSize`.
    pub fn calc_level_size(&self, level: u32, fv: FormatVersion) -> usize {
        self.layer_size(level, fv) * self.num_layers as usize
    }

    /// Port of `ktxTexture_doCalcFaceLodSize`.
    pub fn do_calc_face_lod_size(&self, level: u32, fv: FormatVersion) -> usize {
        if self.is_cubemap && !self.is_array {
            self.calc_image_size(level, fv)
        } else {
            self.calc_level_size(level, fv)
        }
    }

    /// Port of `ktxTexture_rowInfo`. Returns `(numRows, rowLengthBytes, rowPadding)`.
    pub fn row_info(&self, level: u32) -> (u32, u32, u32) {
        let fs = &self.protected.format_size;
        let bx = (self.base_width / fs.block_width.max(1)).checked_shr(level).unwrap_or(0).max(1);
        let num_rows = (self.base_height / fs.block_height.max(1)).checked_shr(level).unwrap_or(0).max(1);
        let row_length = bx * fs.block_size_in_bits / 8;
        let padding = ktx_pad_unpack_align_len(row_length as u64) as u32;
        (num_rows, row_length + padding, padding)
    }

    /// Port of `ktxTexture_GetRowPitch`.
    pub fn get_row_pitch(&self, level: u32) -> u32 {
        let fs = &self.protected.format_size;
        let bx = (self.base_width / fs.block_width.max(1)).checked_shr(level).unwrap_or(0).max(1);
        let mut pitch = bx * fs.block_size_in_bits / 8;
        if self.class_id == ClassId::Texture1 {
            pitch += ktx_pad_unpack_align_len(pitch as u64) as u32;
        }
        pitch
    }

    /// Level dimensions `max(1, base >> level)`.
    pub fn level_dims(&self, level: u32) -> (u32, u32, u32) {
        let d = |v: u32| v.checked_shr(level).unwrap_or(0).max(1);
        (d(self.base_width), d(self.base_height), d(self.base_depth))
    }
}

/// Virtual interface of `ktxTexture` (`ktxTexture_vtbl` + `ktxTexture_vtblInt`).
pub trait KtxTexture {
    /// Base class data.
    fn base(&self) -> &Texture;
    /// Mutable base class data.
    fn base_mut(&mut self) -> &mut Texture;

    /// `calcDataSizeLevels`.
    fn calc_data_size_levels(&self, levels: u32) -> usize;
    /// `calcFaceLodSize`.
    fn calc_face_lod_size(&self, level: u32) -> usize;
    /// `calcLevelOffset`.
    fn calc_level_offset(&self, level: u32) -> usize;

    /// Port of `ktxTexture_GetImageOffset`.
    fn get_image_offset(&self, level: u32, layer: u32, face_slice: u32) -> KtxResult<usize>;
    /// Port of `ktxTexture_GetDataSizeUncompressed`.
    fn get_data_size_uncompressed(&self) -> usize;
    /// Port of `ktxTexture_GetImageSize`.
    fn get_image_size(&self, level: u32) -> usize;
    /// Port of `ktxTexture_GetLevelSize`.
    fn get_level_size(&self, level: u32) -> usize;
    /// Port of `ktxTexture_IterateLevels`.
    fn iterate_levels(&self, iter_cb: &mut IterCb<'_>) -> KtxResult<()>;
    /// Port of `ktxTexture_IterateLoadLevelFaces`.
    fn iterate_load_level_faces(&mut self, iter_cb: &mut IterCb<'_>) -> KtxResult<()>;
    /// Port of `ktxTexture_LoadImageData`. With `buffer` the data is loaded there
    /// instead of into the texture.
    fn load_image_data(&mut self, buffer: Option<&mut [u8]>) -> KtxResult<()>;
    /// Port of `ktxTexture_NeedsTranscoding`.
    fn needs_transcoding(&self) -> bool;

    /// Port of `ktxTexture_calcDataSizeTexture`.
    fn calc_data_size_texture(&self) -> usize {
        self.calc_data_size_levels(self.base().num_levels)
    }

    /// Port of `ktxTexture_IterateLevelFaces`.
    fn iterate_level_faces(&self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        let this = self.base();
        for miplevel in 0..this.num_levels {
            let (width, height, depth) = this.level_dims(miplevel);
            let face_lod_size = self.calc_face_lod_size(miplevel);
            // All array layers are passed in a group.
            let inner_iterations = if this.is_cubemap && !this.is_array { this.num_faces } else { 1 };
            for face in 0..inner_iterations {
                let offset = self.get_image_offset(miplevel, 0, face)?;
                // The sizes come from the format and extents, not the data.
                if offset > this.data_size || face_lod_size > this.data_size - offset {
                    return Err(ErrorCode::FileDataError);
                }
                let data = this.p_data.as_deref().unwrap_or(&[]);
                let pixels = data.get(offset..offset + face_lod_size).ok_or(ErrorCode::FileDataError)?;
                iter_cb(miplevel, face, width, height, depth, face_lod_size as u64, pixels)?;
            }
        }
        Ok(())
    }
}
