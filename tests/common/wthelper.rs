//! Port of tests/unittests/wthelper.h (`WriterTestHelper`) plus the
//! `TextureWriterTestHelper` additions from texturetests.cc.

use ktx::defs::{KTX_GL_UNPACK_ALIGNMENT, KTX_ORIENTATION_KEY, KTX_WRITER_KEY, ktx_padn_len};
use ktx::gl_format::{
    gl_get_format_from_internal_format, gl_get_type_from_internal_format,
    gl_get_type_size_from_type,
};
use ktx::ktxint::{KtxHeader, KtxHeader2, LevelIndexEntry};
use ktx::texture2::lcm4;
use ktx::vk_format::{RgbaAstcMapping, vk_get_format_from_opengl_internal_format};
use ktx::writer2::append_lib_id;
use ktx::{HashList, KtxResult, SupercmpScheme, Texture1, Texture2, TextureCreateInfo};

/// A color component type (`component_type` in C).
pub trait Component: Copy + Default {
    const SIZE: usize;
    fn from_u32(v: u32) -> Self;
    fn push_le(self, out: &mut Vec<u8>);
}

impl Component for u8 {
    const SIZE: usize = 1;
    fn from_u32(v: u32) -> Self {
        v as u8
    }
    fn push_le(self, out: &mut Vec<u8>) {
        out.push(self);
    }
}

impl Component for u16 {
    const SIZE: usize = 2;
    fn from_u32(v: u32) -> Self {
        v as u16
    }
    fn push_le(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

pub const E_NONE: u32 = 0x00;
pub const E_MIPMAPPED: u32 = 0x01;
pub const E_GENERATE_MIPMAPS: u32 = 0x02;
pub const E_ARRAY: u32 = 0x04;

/// Port of `wthImageInfo`.
#[derive(Debug, Clone)]
pub struct WthImageInfo {
    pub size: usize,
    pub data: Vec<u8>,
}

/// Port of `wthTexInfo` (extends `ktxTextureCreateInfo`).
#[derive(Debug, Clone, Default)]
pub struct WthTexInfo {
    pub ci: TextureCreateInfo,
    pub gl_type_size: u32,
    pub gl_type: u32,
    pub gl_format: u32,
    pub gl_base_internalformat: u32,
    pub header_pixel_height: u32,
    pub header_pixel_depth: u32,
    pub header_num_layers: u32,
}

impl WthTexInfo {
    fn new(internalformat: u32) -> WthTexInfo {
        let gl_type = gl_get_type_from_internal_format(internalformat);
        let gl_format = gl_get_format_from_internal_format(internalformat);
        WthTexInfo {
            ci: TextureCreateInfo {
                gl_internalformat: internalformat,
                ..Default::default()
            },
            gl_type,
            gl_type_size: gl_get_type_size_from_type(gl_type),
            gl_format,
            gl_base_internalformat: gl_format,
            ..Default::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resize(
        &mut self,
        levels: u32,
        layers: u32,
        faces: u32,
        dimensions: u32,
        array: bool,
        w: u32,
        h: u32,
        d: u32,
    ) {
        self.ci.num_layers = layers;
        self.ci.num_faces = faces;
        self.ci.num_levels = levels;
        self.ci.num_dimensions = dimensions;
        self.ci.generate_mipmaps = false;
        self.ci.is_array = array;
        self.ci.base_width = w;
        self.ci.base_height = h;
        self.ci.base_depth = d;
        self.header_num_layers = if array { layers } else { 0 };
        self.header_pixel_height = if dimensions >= 2 { h } else { 0 };
        self.header_pixel_depth = if dimensions == 3 { d } else { 0 };
    }

    pub fn compare_header1(&self, h: &KtxHeader) -> bool {
        h.gl_type == self.gl_type
            && h.gl_type_size == self.gl_type_size
            && h.gl_format == self.gl_format
            && h.gl_internalformat == self.ci.gl_internalformat
            && h.gl_base_internalformat == self.gl_base_internalformat
            && h.pixel_width == self.ci.base_width
            && h.pixel_height == self.header_pixel_height
            && h.pixel_depth == self.header_pixel_depth
            && h.number_of_array_elements == self.header_num_layers
            && h.number_of_faces == self.ci.num_faces
            && h.number_of_mip_levels == self.ci.num_levels
    }

    pub fn compare_header2(&self, h: &KtxHeader2) -> bool {
        let format = vk_get_format_from_opengl_internal_format(
            self.ci.gl_internalformat,
            RgbaAstcMapping::ToHdr,
        );
        h.vk_format == format
            && h.pixel_width == self.ci.base_width
            && h.pixel_height == self.header_pixel_height
            && h.pixel_depth == self.header_pixel_depth
            && h.layer_count == self.header_num_layers
            && h.face_count == self.ci.num_faces
            && h.level_count == self.ci.num_levels
            && h.supercompression_scheme.0 <= SupercmpScheme::END_RANGE.0
    }

    pub fn compare_texture2(&self, t: &Texture2) -> bool {
        let format = vk_get_format_from_opengl_internal_format(
            self.ci.gl_internalformat,
            RgbaAstcMapping::ToHdr,
        );
        t.vk_format == format
            && t.base_width == self.ci.base_width
            && t.base_height == self.ci.base_height
            && t.base_depth == self.ci.base_depth
            && t.num_layers == self.ci.num_layers
            && t.num_faces == self.ci.num_faces
            && t.num_levels == self.ci.num_levels
            && t.supercompression_scheme.0 <= SupercmpScheme::END_RANGE.0
    }
}

/// Port of `WriterTestHelper<component_type, numComponents, internalformat>` +
/// `TextureWriterTestHelper`.
pub struct WriterTestHelper<C: Component, const NC: u32, const IF: u32> {
    pub num_levels: u32,
    pub num_layers: u32,
    pub num_faces: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub is_array: bool,
    pub kv_data: Vec<u8>,
    pub kv_data_len: u32,
    pub orientation: String,
    pub kv_data_writer_ktx2: Vec<u8>,
    pub kv_data_len_writer_ktx2: u32,
    pub kv_data_all_ktx2: Vec<u8>,
    pub kv_data_len_all_ktx2: u32,
    pub kv_hash: HashList,
    pub kv_hash_ktx2: HashList,
    pub orientation_ktx2: [u8; 4],
    pub writer_ktx2: String,
    pub image_data_size: usize,
    /// `images[level][layer][faceSlice]` as little-endian bytes.
    pub images: Vec<Vec<Vec<Vec<u8>>>>,
    pub image_list: Vec<WthImageInfo>,
    pub texinfo: WthTexInfo,
    /// `TextureWriterTestHelper::createInfo`.
    pub create_info: TextureCreateInfo,
    _c: std::marker::PhantomData<C>,
}

impl<C: Component, const NC: u32, const IF: u32> Default for WriterTestHelper<C, NC, IF> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Component, const NC: u32, const IF: u32> WriterTestHelper<C, NC, IF> {
    pub fn new() -> Self {
        WriterTestHelper {
            num_levels: 0,
            num_layers: 0,
            num_faces: 0,
            width: 0,
            height: 0,
            depth: 0,
            is_array: false,
            kv_data: Vec::new(),
            kv_data_len: 0,
            orientation: String::new(),
            kv_data_writer_ktx2: Vec::new(),
            kv_data_len_writer_ktx2: 0,
            kv_data_all_ktx2: Vec::new(),
            kv_data_len_all_ktx2: 0,
            kv_hash: HashList::new(),
            kv_hash_ktx2: HashList::new(),
            orientation_ktx2: [0; 4],
            writer_ktx2: "WriteTestHelper 1.0 __default__".to_string(),
            image_data_size: 0,
            images: Vec::new(),
            image_list: Vec::new(),
            texinfo: WthTexInfo::new(IF),
            create_info: TextureCreateInfo {
                gl_internalformat: IF,
                ..Default::default()
            },
            _c: std::marker::PhantomData,
        }
    }

    pub fn levels_from_size(width: u32, height: u32, depth: u32) -> u32 {
        let mut max_dim = width.max(height).max(depth);
        let mut mip_levels = 1;
        while max_dim != 1 {
            mip_levels += 1;
            max_dim >>= 1;
        }
        mip_levels
    }

    /// Port of `resize` (both helper levels), default color.
    #[allow(clippy::too_many_arguments)]
    pub fn resize(
        &mut self,
        flags: u32,
        layers: u32,
        faces: u32,
        dimensions: u32,
        w: u32,
        h: u32,
        d: u32,
    ) {
        self.resize_with_color(flags, layers, faces, dimensions, w, h, d, None);
    }

    /// Port of `WriterTestHelper::resize` with an optional requested color.
    #[allow(clippy::too_many_arguments)]
    pub fn resize_with_color(
        &mut self,
        flags: u32,
        layers: u32,
        faces: u32,
        dimensions: u32,
        w: u32,
        h: u32,
        d: u32,
        requested_color: Option<&[C]>,
    ) {
        self.width = w;
        self.height = h;
        self.depth = d;
        self.num_levels = if flags & E_MIPMAPPED != 0 {
            Self::levels_from_size(w, h, d)
        } else {
            1
        };
        self.num_layers = layers;
        self.num_faces = faces;
        self.is_array = flags & E_ARRAY != 0;
        self.texinfo.resize(
            self.num_levels,
            layers,
            faces,
            dimensions,
            self.is_array,
            w,
            h,
            d,
        );

        self.image_data_size = 0;
        self.images = Vec::new();
        self.image_list = Vec::new();
        let nc = NC as usize;
        let mut color: Vec<C> = vec![C::default(); nc];
        if let Some(rc) = requested_color {
            color[..nc].copy_from_slice(&rc[..nc]);
        }
        for level in 0..self.num_levels {
            let lw = (w >> level).max(1);
            let lh = (h >> level).max(1);
            let ld = (d >> level).max(1);
            let mut level_images = Vec::new();
            for layer in 0..layers {
                let num_images = if faces == 6 { faces } else { ld };
                let mut layer_images = Vec::new();
                for face_slice in 0..num_images {
                    let pixel_count = (lw * lh) as usize;
                    if requested_color.is_none() {
                        // (component_type)0.5 truncates to 0 for integer types.
                        if nc >= 4 {
                            color[3] = C::from_u32(0);
                        }
                        if nc >= 3 {
                            color[2] = C::from_u32(face_slice);
                        }
                        if nc >= 2 {
                            color[1] = C::from_u32(layer);
                        }
                        color[0] = C::from_u32(level);
                    }
                    let mut bytes = Vec::with_capacity(pixel_count * nc * C::SIZE);
                    for _ in 0..pixel_count {
                        for c in color.iter().take(nc) {
                            c.push_le(&mut bytes);
                        }
                    }
                    self.image_data_size += bytes.len();
                    self.image_list.push(WthImageInfo {
                        size: bytes.len(),
                        data: bytes.clone(),
                    });
                    layer_images.push(bytes);
                }
                level_images.push(layer_images);
            }
            self.images.push(level_images);
        }

        self.orientation = match dimensions {
            1 => "S=r".to_string(),
            2 => "S=r,T=d".to_string(),
            _ => "S=r,T=d,R=i".to_string(),
        };
        self.orientation_ktx2 = [b'r', b'd', b'i', 0];
        if (dimensions as usize) < 4 {
            self.orientation_ktx2[dimensions as usize] = 0;
        }

        let mut orient = self.orientation.as_bytes().to_vec();
        orient.push(0);
        self.kv_hash = HashList::new();
        let _ = self.kv_hash.add_kv_pair(KTX_ORIENTATION_KEY, &orient);
        self.kv_data = self.kv_hash.serialize().unwrap_or_default();
        self.kv_data_len = self.kv_data.len() as u32;

        self.kv_hash_ktx2 = HashList::new();
        let _ = self
            .kv_hash_ktx2
            .add_kv_pair(KTX_WRITER_KEY, self.writer_ktx2.as_bytes());
        // Get the library to add its Id to the writer key.
        let _ = append_lib_id(&mut self.kv_hash_ktx2);
        self.kv_data_writer_ktx2 = self.kv_hash_ktx2.serialize().unwrap_or_default();
        self.kv_data_len_writer_ktx2 = self.kv_data_writer_ktx2.len() as u32;
        let _ = self.kv_hash_ktx2.add_kv_pair(
            KTX_ORIENTATION_KEY,
            &self.orientation_ktx2[..dimensions as usize + 1],
        );
        let _ = self.kv_hash_ktx2.sort();
        self.kv_data_all_ktx2 = self.kv_hash_ktx2.serialize().unwrap_or_default();
        self.kv_data_len_all_ktx2 = self.kv_data_all_ktx2.len() as u32;

        // TextureWriterTestHelper::createInfo.resize
        self.create_info.base_width = w;
        self.create_info.base_height = h;
        self.create_info.base_depth = d;
        self.create_info.num_dimensions = dimensions;
        self.create_info.generate_mipmaps = flags & E_GENERATE_MIPMAPS != 0;
        self.create_info.is_array = flags & E_ARRAY != 0;
        self.create_info.num_faces = faces;
        self.create_info.num_layers = layers;
        self.create_info.num_levels = self.num_levels;
    }

    fn row_padding(row_bytes: usize) -> usize {
        let a = KTX_GL_UNPACK_ALIGNMENT as usize;
        3 - ((row_bytes + a - 1) % a)
    }

    /// Port of `compareRawImages(ktx_uint8_t*)` (KTX 1 file image section).
    pub fn compare_raw_images(&self, data: &[u8]) -> bool {
        let mut p = 0usize;
        let rd = |p: usize| {
            data.get(p..p + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        for level in 0..self.num_levels {
            let Some(face_lod_size) = rd(p) else {
                return false;
            };
            let lw = (self.width >> level).max(1) as usize;
            let lh = (self.height >> level).max(1) as usize;
            let ld = (self.depth >> level).max(1);
            let row_bytes = lw * C::SIZE * NC as usize;
            let row_padding = Self::row_padding(row_bytes);
            let padded_row_bytes = row_bytes + row_padding;
            let padded_image_bytes = padded_row_bytes * lh;
            let (num_images, expected) = if self.num_faces == 6 && !self.is_array {
                (self.num_faces, padded_image_bytes)
            } else {
                let n = if self.num_faces == 6 {
                    self.num_faces
                } else {
                    ld
                };
                (
                    n,
                    padded_image_bytes * n as usize * self.num_layers as usize,
                )
            };
            if face_lod_size as usize != expected {
                return false;
            }
            p += 4;
            for layer in 0..self.num_layers as usize {
                for face_slice in 0..num_images as usize {
                    let img = &self.images[level as usize][layer][face_slice];
                    if row_padding == 0 {
                        if data.get(p..p + img.len()) != Some(&img[..]) {
                            return false;
                        }
                        p += padded_image_bytes;
                    } else {
                        for row in 0..lh {
                            let src = &img[row * row_bytes..(row + 1) * row_bytes];
                            if data.get(p..p + row_bytes) != Some(src) {
                                return false;
                            }
                            p += padded_row_bytes;
                        }
                    }
                }
            }
        }
        true
    }

    /// Port of `compareRawImages(ktxLevelIndexEntry[], ktx_uint8_t*)` (KTX 2).
    pub fn compare_raw_images2(&self, level_index: &[LevelIndexEntry], base: &[u8]) -> bool {
        for level in 0..self.num_levels as usize {
            let Some(e) = level_index.get(level) else {
                return false;
            };
            let ld = (self.depth >> level).max(1);
            let image_bytes = self.images[level][0][0].len();
            let num_images = if self.num_faces == 6 {
                self.num_faces
            } else {
                ld
            } as usize;
            let expected = image_bytes * num_images * self.num_layers as usize;
            if e.uncompressed_byte_length as usize != expected {
                return false;
            }
            let mut p = e.byte_offset as usize;
            for layer in 0..self.num_layers as usize {
                for face_slice in 0..num_images {
                    let img = &self.images[level][layer][face_slice];
                    if base.get(p..p + image_bytes) != Some(&img[..]) {
                        return false;
                    }
                    p += image_bytes;
                }
            }
        }
        true
    }

    /// Port of `TextureWriterTestHelper::compareTexture1Images`.
    pub fn compare_texture1_images(&self, data: &[u8]) -> bool {
        let mut p = 0usize;
        for level in 0..self.images.len() {
            let lw = (self.width >> level).max(1) as usize;
            let lh = (self.height >> level).max(1) as usize;
            let row_bytes = lw * C::SIZE * NC as usize;
            let row_padding = Self::row_padding(row_bytes);
            let padded_image_bytes = (row_bytes + row_padding) * lh;
            for layer in 0..self.images[0].len() {
                for img in &self.images[level][layer] {
                    if row_padding == 0 {
                        if data.get(p..p + img.len()) != Some(&img[..]) {
                            return false;
                        }
                        p += padded_image_bytes;
                    } else {
                        for row in 0..lh {
                            if data.get(p..p + row_bytes)
                                != Some(&img[row * row_bytes..(row + 1) * row_bytes])
                            {
                                return false;
                            }
                            p += row_bytes + row_padding;
                        }
                    }
                }
            }
        }
        true
    }

    /// Port of `TextureWriterTestHelper::compareTexture2Images`.
    pub fn compare_texture2_images(&self, data: &[u8]) -> bool {
        let mut p = 0usize;
        for level in (0..self.images.len()).rev() {
            let lw = (self.width >> level).max(1) as usize;
            let lh = (self.height >> level).max(1) as usize;
            let texel_block_size = C::SIZE * NC as usize;
            let required = lcm4(texel_block_size as u32) as u64;
            let image_bytes = lw * texel_block_size * lh;
            for layer in 0..self.images[0].len() {
                for img in &self.images[level][layer] {
                    if data.get(p..p + img.len()) != Some(&img[..]) {
                        return false;
                    }
                    p += image_bytes;
                }
            }
            p += ktx_padn_len(required, image_bytes as u64) as usize;
        }
        true
    }

    /// Port of `copyImagesToTexture` for `ktxTexture1`.
    pub fn copy_images_to_texture1(&self, texture: &mut Texture1) -> KtxResult<()> {
        for (level, l) in self.images.iter().enumerate() {
            for (layer, ly) in l.iter().enumerate() {
                for (face_slice, img) in ly.iter().enumerate() {
                    texture.set_image_from_memory(
                        level as u32,
                        layer as u32,
                        face_slice as u32,
                        img,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Port of `copyImagesToTexture` for `ktxTexture2`.
    pub fn copy_images_to_texture2(&self, texture: &mut Texture2) -> KtxResult<()> {
        for (level, l) in self.images.iter().enumerate() {
            for (layer, ly) in l.iter().enumerate() {
                for (face_slice, img) in ly.iter().enumerate() {
                    texture.set_image_from_memory(
                        level as u32,
                        layer as u32,
                        face_slice as u32,
                        img,
                    )?;
                }
            }
        }
        Ok(())
    }
}
