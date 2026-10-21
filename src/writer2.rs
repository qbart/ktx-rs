//! Port of lib/src/writer2.c: writing `ktxTexture2` and deflating it.

use std::cell::Cell;

use crate::defs::{
    KTX_FACESLICE_WHOLE_LEVEL, KTX_WRITER_KEY, KTX2_IDENTIFIER_REF, SupercmpScheme, ktx_pad8_len,
    ktx_padn_len,
};
use crate::error::{ErrorCode, KtxResult};
use crate::hashlist::HashList;
use crate::ktxint::{
    IndexEntry32, IndexEntry64, KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry,
};
use crate::stream::{FileStream, MemStream, Stream};
use crate::strings::strnstr;
use crate::texture::{FormatVersion, KtxTexture};
use crate::texture2::Texture2;
use crate::version::{LIBKTX_DEFAULT_VERSION, LIBKTX_VERSION};
use crate::zlib::{ktx_compress_zlib_bounds, ktx_compress_zlib_int};

thread_local! {
    static DISABLE_WRITER_METADATA: Cell<bool> = const { Cell::new(false) };
}

/// Port of `__disableWriterMetadata__` (test hook, `TestNoMetadata`). Thread-local.
#[doc(hidden)]
pub fn set_disable_writer_metadata(disable: bool) {
    DISABLE_WRITER_METADATA.with(|c| c.set(disable));
}

fn disable_writer_metadata() -> bool {
    DISABLE_WRITER_METADATA.with(|c| c.get())
}

/// Port of `appendLibId`: append (or update) " / libktx <ver>" in `KTXwriter`.
pub fn append_lib_id(head: &mut HashList) -> KtxResult<()> {
    const LIB_ID_INTRO: &[u8] = b" / libktx ";
    const UNIDENTIFIED: &[u8] = b"Unidentified app";
    const DEFAULT_ID: &[u8] = b"__default__";

    let writer: Option<Vec<u8>> = head.find_value(KTX_WRITER_KEY).ok().map(|v| v.to_vec());
    let id: Vec<u8> = match &writer {
        Some(v) => v.clone(),
        None => UNIDENTIFIED.to_vec(),
    };
    let mut id_len = match &writer {
        Some(v) => v.len(),
        None => 17,
    };

    // strnstr needed because KTXwriter values may not be NUL terminated.
    let lib_ver = if strnstr(&id, DEFAULT_ID, id_len).is_some() {
        LIBKTX_DEFAULT_VERSION
    } else {
        LIBKTX_VERSION
    };
    let mut lib_id = LIB_ID_INTRO.to_vec();
    lib_id.extend_from_slice(lib_ver.as_bytes());

    if strnstr(&id, &lib_id, id_len).is_some() {
        // This lib id is already in the writer value.
        return Ok(());
    }
    if let Some(pos) = strnstr(&id, LIB_ID_INTRO, id_len) {
        // There is a libktx version but not the current version.
        id_len = pos;
    } else if id_len > 0 && id.get(id_len - 1) == Some(&0) {
        id_len -= 1;
    }
    let mut full_id = id.get(..id_len.min(id.len())).unwrap_or(&[]).to_vec();
    // Pad with NULs if id was shorter than claimed (strncpy semantics).
    full_id.resize(id_len, 0);
    full_id.extend_from_slice(&lib_id);
    full_id.push(0);
    if full_id.len() > u32::MAX as usize {
        return Err(ErrorCode::InvalidOperation);
    }
    if writer.is_some() {
        head.delete_kv_pair(KTX_WRITER_KEY)?;
    }
    head.add_kv_pair(KTX_WRITER_KEY, &full_id)
}

const KNOWN_KEYS: [&str; 11] = [
    "KTXcubemapIncomplete",
    "KTXorientation",
    "KTXglFormat",
    "KTXdxgiFormat__",
    "KTXmetalPixelFormat",
    "KTXswizzle",
    "KTXwriter",
    "KTXwriterScParams",
    "KTXastcDecodeMode",
    "KTXanimData",
    "KTXmapRange",
];

impl Texture2 {
    /// Port of `ktxTexture2_setImageFromStream`.
    pub fn set_image_from_stream(
        &mut self,
        level: u32,
        layer: u32,
        face_slice: u32,
        src: &mut dyn Stream,
        src_size: usize,
    ) -> KtxResult<()> {
        if self.p_data.is_none() {
            return Err(ErrorCode::InvalidOperation);
        }
        let (image_byte_offset, image_byte_length) = if face_slice == KTX_FACESLICE_WHOLE_LEVEL {
            (
                self.get_image_offset(level, layer, 0)?,
                self.calc_level_size(level, FormatVersion::Two),
            )
        } else {
            (
                self.get_image_offset(level, layer, face_slice)?,
                self.get_image_size(level),
            )
        };
        if src_size != image_byte_length {
            return Err(ErrorCode::InvalidOperation);
        }
        let data = self
            .p_data
            .as_deref_mut()
            .ok_or(ErrorCode::InvalidOperation)?;
        let dst = data
            .get_mut(image_byte_offset..image_byte_offset + src_size)
            .ok_or(ErrorCode::InvalidOperation)?;
        // C ignores the read result here.
        let _ = src.read(dst);
        Ok(())
    }

    /// Port of `ktxTexture2_SetImageFromStdioStream`.
    pub fn set_image_from_stdio_stream(
        &mut self,
        level: u32,
        layer: u32,
        face_slice: u32,
        src: std::fs::File,
        src_size: usize,
    ) -> KtxResult<()> {
        let mut s = FileStream::construct(src)?;
        self.set_image_from_stream(level, layer, face_slice, &mut s, src_size)
    }

    /// Port of `ktxTexture2_SetImageFromMemory`.
    pub fn set_image_from_memory(
        &mut self,
        level: u32,
        layer: u32,
        face_slice: u32,
        src: &[u8],
    ) -> KtxResult<()> {
        let mut s = MemStream::construct_ro(src)?;
        self.set_image_from_stream(level, layer, face_slice, &mut s, src.len())
    }

    /// Port of `ktxTexture2_WriteToStream`.
    pub fn write_to_stream(&mut self, dststr: &mut dyn Stream) -> KtxResult<()> {
        if self.p_data.is_none() {
            return Err(ErrorCode::InvalidOperation);
        }
        let num_levels = self.num_levels;
        let mut header = KtxHeader2 {
            identifier: KTX2_IDENTIFIER_REF,
            vk_format: self.vk_format,
            type_size: self.type_size(),
            pixel_width: self.base_width,
            pixel_height: if self.num_dimensions > 1 {
                self.base_height
            } else {
                0
            },
            pixel_depth: if self.num_dimensions > 2 {
                self.base_depth
            } else {
                0
            },
            layer_count: if self.is_array { self.num_layers } else { 0 },
            face_count: self.num_faces,
            level_count: if self.generate_mipmaps { 0 } else { num_levels },
            supercompression_scheme: self.supercompression_scheme,
            ..Default::default()
        };
        let level_index_size = LEVEL_INDEX_ENTRY_SIZE as u64 * num_levels as u64;
        let mut base_offset: u64 = crate::defs::KTX2_HEADER_SIZE as u64 + level_index_size;
        let dfd_len = self.p_dfd.first().copied().unwrap_or(0);
        header.data_format_descriptor = IndexEntry32 {
            byte_offset: base_offset as u32,
            byte_length: dfd_len,
        };
        base_offset += dfd_len as u64;

        // Check for invalid metadata.
        for entry in &self.kv_data_head {
            let (_, key) = entry.get_key();
            if key.len() >= 3 && key[..3].eq_ignore_ascii_case(b"KTX") {
                if &key[..3] == b"ktx" {
                    return Err(ErrorCode::InvalidOperation);
                }
                if !KNOWN_KEYS.iter().any(|k| k.as_bytes() == key) {
                    return Err(ErrorCode::InvalidOperation);
                }
            }
        }

        if !disable_writer_metadata() {
            append_lib_id(&mut self.kv_data_head)?;
        }
        self.kv_data_head.sort()?; // KTX2 requires sorted metadata.
        let kvd = self.kv_data_head.serialize()?;
        let kvd_len = kvd.len() as u32;
        header.key_value_data = IndexEntry32 {
            byte_offset: if kvd_len != 0 { base_offset as u32 } else { 0 },
            byte_length: kvd_len,
        };
        base_offset += kvd_len as u64;

        let sgd_len = self.private().sgd_byte_length;
        let mut align8_pad_len = 0u64;
        if sgd_len != 0 {
            align8_pad_len = ktx_pad8_len(base_offset);
            base_offset += align8_pad_len;
        }
        header.supercompression_global_data = IndexEntry64 {
            byte_offset: if sgd_len != 0 { base_offset } else { 0 },
            byte_length: sgd_len,
        };
        base_offset += sgd_len;
        let required = self.private().required_level_alignment as u64;
        let initial_level_pad_len = ktx_padn_len(required, base_offset);
        base_offset += initial_level_pad_len;

        // write header and indices
        let hb = header.to_bytes();
        dststr.write(&hb, hb.len(), 1)?;

        let mut li = Vec::with_capacity(level_index_size as usize);
        for e in self.level_index() {
            let fe = LevelIndexEntry {
                byte_offset: e.byte_offset + base_offset,
                byte_length: e.byte_length,
                uncompressed_byte_length: e.uncompressed_byte_length,
            };
            li.extend_from_slice(&fe.to_bytes());
        }
        dststr.write(&li, 1, li.len())?;

        // write data format descriptor
        let mut dfd_bytes: Vec<u8> = self.p_dfd.iter().flat_map(|w| w.to_le_bytes()).collect();
        dfd_bytes.truncate(dfd_len as usize);
        dststr.write(&dfd_bytes, 1, dfd_bytes.len())?;

        if kvd_len != 0 {
            dststr.write(&kvd, 1, kvd.len())?;
        }
        let padding = [0u8; 32];
        if sgd_len != 0 {
            if align8_pad_len != 0 {
                dststr.write(&padding, 1, align8_pad_len as usize)?;
            }
            let sgd = self.supercompression_global_data();
            dststr.write(sgd, 1, sgd.len())?;
        }
        if initial_level_pad_len != 0 {
            dststr.write(&padding, 1, initial_level_pad_len as usize)?;
        }

        // write the image data
        let data = self.p_data.as_deref().unwrap_or(&[]);
        for level in (0..num_levels).rev() {
            let e = self
                .level_index()
                .get(level as usize)
                .copied()
                .unwrap_or_default();
            let start = e.byte_offset as usize;
            let len = e.byte_length as usize;
            let src = data
                .get(start..start + len)
                .ok_or(ErrorCode::InvalidOperation)?;
            dststr.write(src, 1, len)?;
            if level > 0 {
                // No padding at end.
                let level_pad_len = ktx_padn_len(required, e.byte_length);
                if level_pad_len != 0 {
                    dststr.write(&padding, 1, level_pad_len as usize)?;
                }
            }
        }
        Ok(())
    }

    /// Port of `ktxTexture2_WriteToStdioStream`.
    pub fn write_to_stdio_stream(&mut self, dst: std::fs::File) -> KtxResult<()> {
        let mut s = FileStream::construct(dst)?;
        self.write_to_stream(&mut s)
    }

    /// Port of `ktxTexture2_WriteToNamedFile`.
    pub fn write_to_named_file(&mut self, dstname: impl AsRef<std::path::Path>) -> KtxResult<()> {
        let mut s = FileStream::create(dstname)?;
        self.write_to_stream(&mut s)
    }

    /// Port of `ktxTexture2_WriteToMemory`. Returns the serialized file.
    pub fn write_to_memory(&mut self) -> KtxResult<Vec<u8>> {
        let mut s = MemStream::construct()?;
        self.write_to_stream(&mut s)?;
        Ok(s.into_data())
    }

    fn deflate_with(
        &mut self,
        scheme: SupercmpScheme,
        compress: &dyn Fn(&[u8]) -> KtxResult<Vec<u8>>,
    ) -> KtxResult<()> {
        if self.supercompression_scheme != SupercmpScheme::NONE {
            return Err(ErrorCode::InvalidOperation);
        }
        let data = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        let num_levels = self.num_levels as usize;
        let cindex = self.level_index().to_vec();
        let mut nindex = vec![LevelIndexEntry::default(); num_levels];
        let mut cmp_data: Vec<u8> = Vec::new();
        for level in (0..num_levels).rev() {
            let c = cindex.get(level).copied().unwrap_or_default();
            let start = c.byte_offset as usize;
            let src = data
                .get(start..start + c.byte_length as usize)
                .ok_or(ErrorCode::InvalidOperation)?;
            let out = compress(src)?;
            nindex[level] = LevelIndexEntry {
                byte_offset: cmp_data.len() as u64,
                uncompressed_byte_length: c.byte_length,
                byte_length: out.len() as u64,
            };
            cmp_data
                .try_reserve(out.len())
                .map_err(|_| ErrorCode::OutOfMemory)?;
            cmp_data.extend_from_slice(&out);
        }
        // Now modify the texture.
        self.data_size = cmp_data.len();
        self.p_data = Some(cmp_data);
        self.private_mut().level_index = nindex;
        self.supercompression_scheme = scheme;
        self.private_mut().required_level_alignment = 1;
        Ok(())
    }

    /// Port of `ktxTexture2_DeflateZstd`.
    ///
    /// Note: the pure Rust `ruzstd` encoder currently implements only its fastest
    /// level, so every valid `compression_level` (1..=22) produces a valid zstd
    /// stream at that level.
    pub fn deflate_zstd(&mut self, compression_level: u32) -> KtxResult<()> {
        if compression_level > 22 {
            return Err(ErrorCode::InvalidValue);
        }
        self.deflate_with(SupercmpScheme::ZSTD, &|src: &[u8]| {
            Ok(ruzstd::encoding::compress_to_vec(
                src,
                ruzstd::encoding::CompressionLevel::Fastest,
            ))
        })
    }

    /// Port of `ktxTexture2_DeflateZLIB`.
    pub fn deflate_zlib(&mut self, compression_level: u32) -> KtxResult<()> {
        self.deflate_with(SupercmpScheme::ZLIB, &|src: &[u8]| {
            ktx_compress_zlib_int(src, ktx_compress_zlib_bounds(src.len()), compression_level)
        })
    }
}
