//! Port of lib/src/texture1.c / texture1.h: `ktxTexture1` (KTX 1 files).

use std::ops::{Deref, DerefMut};

use crate::defs::{
    KTX_ENDIAN_REF_REV, KTX_HEADER_SIZE, KTX_ORIENTATION_KEY, OrientationX, OrientationY,
    OrientationZ,
};
use crate::error::{ErrorCode, KtxResult};
use crate::formatsize::KTX_FORMAT_SIZE_COMPRESSED_BIT;
use crate::gl_format::{
    GL_INVALID_VALUE, gl_get_format_from_internal_format, gl_get_format_size,
    gl_get_type_from_internal_format, gl_get_type_size_from_type,
};
use crate::ktxint::{KtxHeader, ktx_check_header1};
use crate::stream::{FileStream, MemStream, Stream};
use crate::swap::ktx_swap_endian_bytes;
use crate::texture::{
    ClassId, FormatVersion, IterCb, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
    KTX_TEXTURE_CREATE_RAW_KVDATA_BIT, KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT, KtxTexture, Texture,
    TextureCreateFlags, TextureCreateInfo, TextureCreateStorage,
};
use crate::vk_format::RgbaAstcMapping;

/// Port of `ktxTexture1_private`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Texture1Private {
    pub need_swap: bool,
    pub rgba_astc_mapping: RgbaAstcMapping,
    /// Stream offset of the first `imageSize` field (not in C; the C stream is
    /// left positioned there). Used to support deferred loading from a copy.
    pub data_offset: u64,
}

/// Port of `ktxTexture1`.
#[derive(Debug)]
pub struct Texture1 {
    base: Texture,
    pub gl_format: u32,
    pub gl_internalformat: u32,
    pub gl_base_internalformat: u32,
    pub gl_type: u32,
    pub(crate) private: Texture1Private,
}

impl Deref for Texture1 {
    type Target = Texture;
    fn deref(&self) -> &Texture {
        &self.base
    }
}

impl DerefMut for Texture1 {
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

/// Emulates `sscanf(s, "S=%c,T=%c,R=%c", ...)`: returns the converted chars.
pub(crate) fn scan_ktx1_orientation(s: &[u8]) -> Vec<u8> {
    let s = match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    };
    let mut out = Vec::new();
    let mut i = 0usize;
    for (k, prefix) in [&b"S="[..], &b",T="[..], &b",R="[..]].iter().enumerate() {
        let _ = k;
        if s.get(i..i + prefix.len()) != Some(prefix) {
            break;
        }
        i += prefix.len();
        match s.get(i) {
            Some(&c) => {
                out.push(c);
                i += 1;
            }
            None => break,
        }
    }
    out
}

impl Texture1 {
    fn empty() -> Texture1 {
        Texture1 {
            base: Texture::empty(ClassId::Texture1),
            gl_format: 0,
            gl_internalformat: 0,
            gl_base_internalformat: 0,
            gl_type: 0,
            // HDR is the best default mapping to resolve the ASTC ambiguity.
            private: Texture1Private {
                rgba_astc_mapping: RgbaAstcMapping::ToHdr,
                ..Default::default()
            },
        }
    }

    /// Private data.
    pub fn private(&self) -> &Texture1Private {
        &self.private
    }

    /// Port of `ktxTexture1_construct` / `ktxTexture1_Create`.
    pub fn create(
        create_info: &TextureCreateInfo,
        storage_allocation: TextureCreateStorage,
    ) -> KtxResult<Texture1> {
        let format_size = gl_get_format_size(create_info.gl_internalformat);
        if format_size.block_size_in_bits == 0 {
            // Most likely a deprecated legacy format.
            return Err(ErrorCode::UnsupportedTextureType);
        }
        let gl_format = gl_get_format_from_internal_format(create_info.gl_internalformat);
        if gl_format == GL_INVALID_VALUE {
            return Err(ErrorCode::InvalidValue);
        }
        let mut this = Texture1::empty();
        this.base = Texture::construct(ClassId::Texture1, create_info, &format_size)?;
        this.gl_internalformat = create_info.gl_internalformat;
        this.base.is_compressed = format_size.flags & KTX_FORMAT_SIZE_COMPRESSED_BIT != 0;
        if this.base.is_compressed {
            this.gl_format = 0;
            this.gl_base_internalformat = gl_format;
            this.gl_type = 0;
            this.base.protected.type_size = 1;
        } else {
            this.gl_base_internalformat = gl_format;
            this.gl_format = gl_format;
            this.gl_type = gl_get_type_from_internal_format(create_info.gl_internalformat);
            if this.gl_type == GL_INVALID_VALUE {
                return Err(ErrorCode::InvalidValue);
            }
            let type_size = gl_get_type_size_from_type(this.gl_type);
            // Only 8, 16, and 32-bit types are supported for byte-swapping.
            if type_size != 1 && type_size != 2 && type_size != 4 {
                return Err(ErrorCode::InvalidValue);
            }
            this.base.protected.type_size = type_size;
        }
        if storage_allocation == TextureCreateStorage::AllocStorage {
            this.base.data_size = this.calc_data_size_texture();
            this.base.p_data = Some(try_alloc(this.base.data_size)?);
        }
        Ok(this)
    }

    /// Port of `ktxTexture1_constructFromStreamAndHeader` (without the final load).
    pub(crate) fn construct_from_stream_and_header(
        stream: &mut dyn Stream,
        header: &mut KtxHeader,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
        let mut this = Texture1::empty();
        let supp_info = ktx_check_header1(header)?;

        this.gl_format = header.gl_format;
        this.gl_internalformat = header.gl_internalformat;
        this.gl_type = header.gl_type;
        let format_size = gl_get_format_size(this.gl_internalformat);
        if format_size.block_size_in_bits == 0 {
            return Err(ErrorCode::UnsupportedTextureType);
        }
        this.base.protected.format_size = format_size;
        this.gl_base_internalformat = header.gl_base_internalformat;
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
        if header.number_of_array_elements > 0 {
            this.base.num_layers = header.number_of_array_elements;
            this.base.is_array = true;
        } else {
            this.base.num_layers = 1;
            this.base.is_array = false;
        }
        this.base.num_faces = header.number_of_faces;
        this.base.is_cubemap = header.number_of_faces == 6;
        this.base.num_levels = header.number_of_mip_levels;
        this.base.is_compressed = supp_info.compressed != 0;
        this.base.generate_mipmaps = supp_info.generate_mipmaps != 0;
        if header.endianness == KTX_ENDIAN_REF_REV {
            this.private.need_swap = true;
        }
        this.base.protected.type_size = header.gl_type_size;

        // Load KVData.
        if header.bytes_of_key_value_data > 0 {
            if create_flags & KTX_TEXTURE_CREATE_SKIP_KVDATA_BIT == 0 {
                let kvd_len = header.bytes_of_key_value_data;
                let mut kvd = try_alloc(kvd_len as usize)?;
                stream.read(&mut kvd)?;
                if this.private.need_swap {
                    // Swap the counts inside the key & value data.
                    // (C omits the 4-byte size field when advancing.)
                    let mut src = 0usize;
                    while src + 4 <= kvd.len() {
                        kvd[src..src + 4].reverse();
                        let sz = u32::from_le_bytes([
                            kvd[src],
                            kvd[src + 1],
                            kvd[src + 2],
                            kvd[src + 3],
                        ]);
                        src += 4 + crate::defs::ktx_pad4(sz as u64) as usize;
                    }
                }
                if create_flags & KTX_TEXTURE_CREATE_RAW_KVDATA_BIT == 0 {
                    this.base.kv_data_head.deserialize(&kvd)?;
                    if let Ok(orientation) = this.base.kv_data_head.find_value(KTX_ORIENTATION_KEY)
                    {
                        let mut orient = scan_ktx1_orientation(orientation);
                        orient.resize(3, 0);
                        let nd = this.base.num_dimensions;
                        if nd >= 3 {
                            this.base.orientation.z = OrientationZ(orient[2]);
                        }
                        if nd >= 2 {
                            this.base.orientation.y = OrientationY(orient[1]);
                        }
                        this.base.orientation.x = OrientationX(orient[0]);
                    }
                } else {
                    this.base.kv_data_len = kvd_len;
                    this.base.kv_data = Some(kvd);
                }
            } else {
                stream.skip(header.bytes_of_key_value_data as u64)?;
            }
        }

        // Get the size of the image data.
        let size = stream.getsize()?;
        let pos = stream.getpos()?;
        this.private.data_offset = pos;
        // Remove space for faceLodSize fields
        let data_size = size
            .checked_sub(pos)
            .and_then(|v| v.checked_sub(this.base.num_levels as u64 * 4))
            .ok_or(ErrorCode::FileUnexpectedEof)?;
        this.base.data_size = usize::try_from(data_size).map_err(|_| ErrorCode::FileOverflow)?;
        Ok(this)
    }

    pub(crate) fn create_from_borrowed_stream(
        stream: &mut dyn Stream,
        header: Option<KtxHeader>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
        let mut header = match header {
            Some(h) => h,
            None => {
                let mut hb = [0u8; KTX_HEADER_SIZE];
                stream.read(&mut hb)?;
                KtxHeader::from_bytes(&hb)
            }
        };
        let mut this = Self::construct_from_stream_and_header(stream, &mut header, create_flags)?;
        if create_flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT != 0 {
            this.load_image_data_int(stream, None)?;
        }
        Ok(this)
    }

    /// Port of `ktxTexture1_CreateFromStream`.
    pub fn create_from_stream(
        mut stream: Box<dyn Stream>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
        let mut this = Self::create_from_borrowed_stream(&mut *stream, None, create_flags)?;
        if create_flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT == 0 {
            this.base.protected.stream = Some(stream);
        }
        Ok(this)
    }

    /// Port of `ktxTexture1_CreateFromStdioStream`.
    pub fn create_from_stdio_stream(
        file: std::fs::File,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
        Self::create_from_stream(Box::new(FileStream::construct(file)?), create_flags)
    }

    /// Port of `ktxTexture1_CreateFromNamedFile`.
    pub fn create_from_named_file(
        filename: impl AsRef<std::path::Path>,
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
        Self::create_from_stream(Box::new(FileStream::open(filename)?), create_flags)
    }

    /// Port of `ktxTexture1_CreateFromMemory`.
    pub fn create_from_memory(
        bytes: &[u8],
        create_flags: TextureCreateFlags,
    ) -> KtxResult<Texture1> {
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

    /// Port of `ktxTexture1_Destroy`.
    pub fn destroy(self) {}

    /// Port of `ktxTexture1_glTypeSize`.
    pub fn gl_type_size(&self) -> u32 {
        self.base.protected.type_size
    }

    /// Port of `ktxTexture1_IsTranscodable`.
    pub fn is_transcodable(&self) -> bool {
        false
    }

    /// Port of `ktxTexture1_IsHDR`.
    pub fn is_hdr(&self) -> bool {
        false
    }

    /// Port of `ktxTexture1_SetRgbaAstcMapping`.
    pub fn set_rgba_astc_mapping(&mut self, mapping: RgbaAstcMapping) {
        self.private.rgba_astc_mapping = mapping;
    }

    /// Port of `ktxTexture1_GetRgbaAstcMapping`.
    pub fn get_rgba_astc_mapping(&self) -> RgbaAstcMapping {
        self.private.rgba_astc_mapping
    }

    fn seek_to_data(&self, stream: &mut dyn Stream) -> KtxResult<()> {
        if stream.getpos()? != self.private.data_offset {
            stream.setpos(self.private.data_offset)?;
        }
        Ok(())
    }

    fn swap_image(&self, data: &mut [u8], face_lod_size: usize) {
        if self.private.need_swap {
            let w = self.base.protected.type_size as usize;
            if (w == 2 || w == 4)
                && let Some(d) = data.get_mut(..face_lod_size.min(data.len()))
            {
                ktx_swap_endian_bytes(d, w);
            }
        }
    }

    fn read_face_lod_size(&self, stream: &mut dyn Stream) -> KtxResult<u32> {
        let mut b = [0u8; 4];
        stream.read(&mut b)?;
        let v = u32::from_le_bytes(b);
        Ok(if self.private.need_swap {
            v.swap_bytes()
        } else {
            v
        })
    }

    /// Port of `ktxTexture1_LoadImageData` body using `stream`.
    fn load_image_data_int(
        &mut self,
        stream: &mut dyn Stream,
        buffer: Option<&mut [u8]>,
    ) -> KtxResult<()> {
        let use_own = buffer.is_none();
        let mut own = if use_own {
            try_alloc(self.base.data_size)?
        } else {
            Vec::new()
        };
        let dest: &mut [u8] = match buffer {
            None => &mut own,
            Some(b) => {
                if b.len() < self.base.data_size {
                    return Err(ErrorCode::InvalidValue);
                }
                b
            }
        };
        self.seek_to_data(stream)?;
        let mut p = 0usize;
        for _miplevel in 0..self.base.num_levels {
            let face_lod_size = self.read_face_lod_size(stream)? as usize;
            // KTX_GL_UNPACK_ALIGNMENT is 4 so no padding.
            let inner = if self.base.is_cubemap && !self.base.is_array {
                self.base.num_faces
            } else {
                1
            };
            for _face in 0..inner {
                let d = dest
                    .get_mut(p..p.saturating_add(face_lod_size))
                    .ok_or(ErrorCode::InvalidValue)?;
                stream.read(d)?;
                self.swap_image(d, face_lod_size);
                p += face_lod_size;
            }
        }
        if use_own {
            self.base.p_data = Some(own);
        }
        Ok(())
    }

    fn iterate_load_level_faces_int(
        &self,
        stream: &mut dyn Stream,
        iter_cb: &mut IterCb<'_>,
    ) -> KtxResult<()> {
        self.seek_to_data(stream)?;
        let mut data: Vec<u8> = Vec::new();
        for miplevel in 0..self.base.num_levels {
            let (width, height, depth) = self.base.level_dims(miplevel);
            let face_lod_size = self.read_face_lod_size(stream)? as usize;
            if data.is_empty() {
                // allocate memory sufficient for the base miplevel
                data = try_alloc(face_lod_size)?;
            } else if data.len() < face_lod_size {
                // subsequent miplevels cannot be larger than the base miplevel
                return Err(ErrorCode::FileDataError);
            }
            let inner = if self.base.is_cubemap && !self.base.is_array {
                self.base.num_faces
            } else {
                1
            };
            for face in 0..inner {
                let d = &mut data[..face_lod_size];
                stream.read(d)?;
                self.swap_image(d, face_lod_size);
                iter_cb(
                    miplevel,
                    face,
                    width,
                    height,
                    depth,
                    face_lod_size as u64,
                    d,
                )?;
            }
        }
        Ok(())
    }
}

impl KtxTexture for Texture1 {
    fn base(&self) -> &Texture {
        &self.base
    }

    fn base_mut(&mut self) -> &mut Texture {
        &mut self.base
    }

    /// Port of `ktxTexture1_calcDataSizeLevels`.
    fn calc_data_size_levels(&self, levels: u32) -> usize {
        // KTX_GL_UNPACK_ALIGNMENT == 4 so no mip padding.
        (0..levels)
            .map(|i| self.calc_level_size(i, FormatVersion::One))
            .sum()
    }

    /// Port of `ktxTexture1_calcFaceLodSize`.
    fn calc_face_lod_size(&self, level: u32) -> usize {
        self.do_calc_face_lod_size(level, FormatVersion::One)
    }

    /// Port of `ktxTexture1_calcLevelOffset`.
    fn calc_level_offset(&self, level: u32) -> usize {
        self.calc_data_size_levels(level)
    }

    /// Port of `ktxTexture1_GetImageOffset`.
    fn get_image_offset(&self, level: u32, layer: u32, face_slice: u32) -> KtxResult<usize> {
        if level >= self.num_levels || layer >= self.num_layers {
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
        let mut offset = self.calc_data_size_levels(level);
        if layer != 0 {
            offset += layer as usize * self.layer_size(level, FormatVersion::One);
        }
        if face_slice != 0 {
            offset += face_slice as usize * self.get_image_size(level);
        }
        Ok(offset)
    }

    /// Port of `ktxTexture1_GetDataSizeUncompressed`.
    fn get_data_size_uncompressed(&self) -> usize {
        self.data_size
    }

    /// Port of `ktxTexture1_GetImageSize`.
    fn get_image_size(&self, level: u32) -> usize {
        self.calc_image_size(level, FormatVersion::One)
    }

    /// Port of `ktxTexture1_GetLevelSize`.
    fn get_level_size(&self, level: u32) -> usize {
        self.calc_level_size(level, FormatVersion::One)
    }

    /// Port of `ktxTexture1_IterateLevels`.
    fn iterate_levels(&self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        let data = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        for miplevel in 0..self.num_levels {
            let (width, height, depth) = self.level_dims(miplevel);
            let level_size = self.calc_level_size(miplevel, FormatVersion::One);
            let offset = self.get_image_offset(miplevel, 0, 0)?;
            let pixels = data
                .get(offset..offset + level_size)
                .ok_or(ErrorCode::FileDataError)?;
            iter_cb(miplevel, 0, width, height, depth, level_size as u64, pixels)?;
        }
        Ok(())
    }

    /// Port of `ktxTexture1_IterateLoadLevelFaces`. The stream is consumed.
    fn iterate_load_level_faces(&mut self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        let mut stream = self
            .base
            .protected
            .stream
            .take()
            .ok_or(ErrorCode::InvalidOperation)?;
        self.iterate_load_level_faces_int(&mut *stream, iter_cb)
    }

    /// Port of `ktxTexture1_LoadImageData`. The stream is consumed (as in C).
    fn load_image_data(&mut self, buffer: Option<&mut [u8]>) -> KtxResult<()> {
        if self.p_data.is_some() {
            return Err(ErrorCode::InvalidOperation);
        }
        let mut stream = self
            .base
            .protected
            .stream
            .take()
            .ok_or(ErrorCode::InvalidOperation)?;
        self.load_image_data_int(&mut *stream, buffer)
    }

    /// Port of `ktxTexture1_NeedsTranscoding`.
    fn needs_transcoding(&self) -> bool {
        false
    }
}
