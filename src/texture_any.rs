//! Port of the class-agnostic `ktxTexture_CreateFrom*` functions (texture.c).
//!
//! C returns a `ktxTexture*` whose `classId` tells KTX 1 from KTX 2; here an enum.

use crate::defs::{KTX_HEADER_SIZE, KTX_IDENTIFIER_REF, KTX2_HEADER_SIZE, KTX2_IDENTIFIER_REF};
use crate::error::{ErrorCode, KtxResult};
use crate::ktxint::{KtxHeader, KtxHeader2};
use crate::stream::{FileStream, MemStream, Stream};
use crate::texture::{
    ClassId, IterCb, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, KtxTexture, Texture, TextureCreateFlags,
};
use crate::texture1::Texture1;
use crate::texture2::Texture2;

/// A `ktxTexture` of either class.
#[derive(Debug)]
pub enum AnyTexture {
    Ktx1(Texture1),
    Ktx2(Texture2),
}

enum Header {
    Ktx1(KtxHeader),
    Ktx2(KtxHeader2),
}

/// Port of `ktxDetermineFileType_`.
fn determine_file_type(stream: &mut dyn Stream) -> KtxResult<Header> {
    let mut id = [0u8; 12];
    stream.read(&mut id)?;
    if id == KTX_IDENTIFIER_REF {
        let mut hb = [0u8; KTX_HEADER_SIZE];
        hb[..12].copy_from_slice(&id);
        stream.read(&mut hb[12..])?;
        Ok(Header::Ktx1(KtxHeader::from_bytes(&hb)))
    } else if id == KTX2_IDENTIFIER_REF {
        let mut hb = [0u8; KTX2_HEADER_SIZE];
        hb[..12].copy_from_slice(&id);
        stream.read(&mut hb[12..])?;
        Ok(Header::Ktx2(KtxHeader2::from_bytes(&hb)))
    } else {
        Err(ErrorCode::UnknownFileFormat)
    }
}

fn create_borrowed(stream: &mut dyn Stream, flags: TextureCreateFlags) -> KtxResult<AnyTexture> {
    match determine_file_type(stream)? {
        Header::Ktx1(h) => Texture1::create_from_borrowed_stream(stream, Some(h), flags).map(AnyTexture::Ktx1),
        Header::Ktx2(h) => Texture2::create_from_borrowed_stream(stream, Some(h), flags).map(AnyTexture::Ktx2),
    }
}

impl AnyTexture {
    /// Port of `ktxTexture_CreateFromStream`.
    pub fn create_from_stream(mut stream: Box<dyn Stream>, flags: TextureCreateFlags) -> KtxResult<AnyTexture> {
        let mut tex = create_borrowed(&mut *stream, flags)?;
        if flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT == 0 {
            tex.base_mut().protected.stream = Some(stream);
        }
        Ok(tex)
    }

    /// Port of `ktxTexture_CreateFromStdioStream`.
    pub fn create_from_stdio_stream(file: std::fs::File, flags: TextureCreateFlags) -> KtxResult<AnyTexture> {
        Self::create_from_stream(Box::new(FileStream::construct(file)?), flags)
    }

    /// Port of `ktxTexture_CreateFromNamedFile`.
    pub fn create_from_named_file(
        filename: impl AsRef<std::path::Path>,
        flags: TextureCreateFlags,
    ) -> KtxResult<AnyTexture> {
        Self::create_from_stream(Box::new(FileStream::open(filename)?), flags)
    }

    /// Port of `ktxTexture_CreateFromMemory`.
    pub fn create_from_memory(bytes: &[u8], flags: TextureCreateFlags) -> KtxResult<AnyTexture> {
        if bytes.is_empty() {
            return Err(ErrorCode::InvalidValue);
        }
        let mut stream = MemStream::construct_ro(bytes)?;
        let mut tex = create_borrowed(&mut stream, flags)?;
        if flags & KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT == 0 {
            tex.base_mut().protected.stream = Some(Box::new(MemStream::construct_ro_owned(bytes.to_vec())?));
        }
        Ok(tex)
    }

    /// Port of `ktxTexture_Destroy`.
    pub fn destroy(self) {}

    /// `classId`.
    pub fn class_id(&self) -> ClassId {
        match self {
            AnyTexture::Ktx1(_) => ClassId::Texture1,
            AnyTexture::Ktx2(_) => ClassId::Texture2,
        }
    }

    fn inner(&self) -> &dyn KtxTexture {
        match self {
            AnyTexture::Ktx1(t) => t,
            AnyTexture::Ktx2(t) => t,
        }
    }

    fn inner_mut(&mut self) -> &mut dyn KtxTexture {
        match self {
            AnyTexture::Ktx1(t) => t,
            AnyTexture::Ktx2(t) => t,
        }
    }

    /// Port of `ktxTexture_WriteToStream`.
    pub fn write_to_stream(&mut self, dststr: &mut dyn Stream) -> KtxResult<()> {
        match self {
            AnyTexture::Ktx1(t) => t.write_to_stream(dststr),
            AnyTexture::Ktx2(t) => t.write_to_stream(dststr),
        }
    }

    /// Port of `ktxTexture_WriteToNamedFile`.
    pub fn write_to_named_file(&mut self, dstname: impl AsRef<std::path::Path>) -> KtxResult<()> {
        match self {
            AnyTexture::Ktx1(t) => t.write_to_named_file(dstname),
            AnyTexture::Ktx2(t) => t.write_to_named_file(dstname),
        }
    }

    /// Port of `ktxTexture_WriteToMemory`.
    pub fn write_to_memory(&mut self) -> KtxResult<Vec<u8>> {
        match self {
            AnyTexture::Ktx1(t) => t.write_to_memory(),
            AnyTexture::Ktx2(t) => t.write_to_memory(),
        }
    }

    /// Port of `ktxTexture_SetImageFromMemory`.
    pub fn set_image_from_memory(&mut self, level: u32, layer: u32, face_slice: u32, src: &[u8]) -> KtxResult<()> {
        match self {
            AnyTexture::Ktx1(t) => t.set_image_from_memory(level, layer, face_slice, src),
            AnyTexture::Ktx2(t) => t.set_image_from_memory(level, layer, face_slice, src),
        }
    }

    /// Port of `ktxTexture_IsTranscodable`.
    pub fn is_transcodable(&self) -> bool {
        match self {
            AnyTexture::Ktx1(t) => t.is_transcodable(),
            AnyTexture::Ktx2(t) => t.is_transcodable(),
        }
    }

    /// Port of `ktxTexture_IsHDR`.
    pub fn is_hdr(&self) -> bool {
        match self {
            AnyTexture::Ktx1(t) => t.is_hdr(),
            AnyTexture::Ktx2(t) => t.is_hdr(),
        }
    }
}

impl KtxTexture for AnyTexture {
    fn base(&self) -> &Texture {
        self.inner().base()
    }
    fn base_mut(&mut self) -> &mut Texture {
        self.inner_mut().base_mut()
    }
    fn calc_data_size_levels(&self, levels: u32) -> usize {
        self.inner().calc_data_size_levels(levels)
    }
    fn calc_face_lod_size(&self, level: u32) -> usize {
        self.inner().calc_face_lod_size(level)
    }
    fn calc_level_offset(&self, level: u32) -> usize {
        self.inner().calc_level_offset(level)
    }
    fn get_image_offset(&self, level: u32, layer: u32, face_slice: u32) -> KtxResult<usize> {
        self.inner().get_image_offset(level, layer, face_slice)
    }
    fn get_data_size_uncompressed(&self) -> usize {
        self.inner().get_data_size_uncompressed()
    }
    fn get_image_size(&self, level: u32) -> usize {
        self.inner().get_image_size(level)
    }
    fn get_level_size(&self, level: u32) -> usize {
        self.inner().get_level_size(level)
    }
    fn iterate_levels(&self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        self.inner().iterate_levels(iter_cb)
    }
    fn iterate_load_level_faces(&mut self, iter_cb: &mut IterCb<'_>) -> KtxResult<()> {
        self.inner_mut().iterate_load_level_faces(iter_cb)
    }
    fn load_image_data(&mut self, buffer: Option<&mut [u8]>) -> KtxResult<()> {
        self.inner_mut().load_image_data(buffer)
    }
    fn needs_transcoding(&self) -> bool {
        self.inner().needs_transcoding()
    }
}

impl std::ops::Deref for AnyTexture {
    type Target = Texture;
    fn deref(&self) -> &Texture {
        self.base()
    }
}
