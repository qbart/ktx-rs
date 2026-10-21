//! Port of lib/src/writer1.c: writing `ktxTexture1` as KTX 1 or KTX 2.

use crate::defs::{
    KTX_ENDIAN_REF, KTX_IDENTIFIER_REF, KTX_ORIENTATION_KEY, KTX_WRITER_KEY, KTX2_HEADER_SIZE, KTX2_IDENTIFIER_REF,
    ktx_padn, ktx_padn_len,
};
use crate::dfd::vk2dfd;
use crate::error::{ErrorCode, KtxResult};
use crate::ktxint::{IndexEntry32, KtxHeader, KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry};
use crate::stream::{FileStream, MemStream, Stream};
use crate::texture::{FormatVersion, KtxTexture};
use crate::texture1::{Texture1, scan_ktx1_orientation};
use crate::texture2::lcm4;
use crate::vk_format::vk_get_format_from_opengl_internal_format;
use crate::vkformat::VkFormat;
use crate::writer2::append_lib_id;

impl Texture1 {
    /// Port of `ktxTexture1_setImageFromStream`.
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
        let image_offset = self.get_image_offset(level, layer, face_slice)?;
        let (packed_bytes, row_padding, packed_row_bytes, row_bytes, num_rows) = if self.is_compressed {
            (self.get_image_size(level), 0u32, 0u32, 0u32, 0u32)
        } else {
            let (num_rows, row_bytes, row_padding) = self.row_info(level);
            let packed_row_bytes = row_bytes - row_padding;
            (packed_row_bytes as usize * num_rows as usize, row_padding, packed_row_bytes, row_bytes, num_rows)
        };
        if src_size != packed_bytes {
            return Err(ErrorCode::InvalidOperation);
        }
        let data = self.p_data.as_deref_mut().ok_or(ErrorCode::InvalidOperation)?;
        if row_padding == 0 {
            // Can copy whole image at once
            let dst = data.get_mut(image_offset..image_offset + src_size).ok_or(ErrorCode::InvalidOperation)?;
            let _ = src.read(dst);
        } else {
            // Copy the rows individually, padding each one
            for row in 0..num_rows as usize {
                let row_offset = image_offset + row_bytes as usize * row;
                let dst = data
                    .get_mut(row_offset..row_offset + row_bytes as usize)
                    .ok_or(ErrorCode::InvalidOperation)?;
                let _ = src.read(&mut dst[..packed_row_bytes as usize]);
                dst[packed_row_bytes as usize..].fill(0);
            }
        }
        Ok(())
    }

    /// Port of `ktxTexture1_SetImageFromStdioStream`.
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

    /// Port of `ktxTexture1_SetImageFromMemory`.
    pub fn set_image_from_memory(&mut self, level: u32, layer: u32, face_slice: u32, src: &[u8]) -> KtxResult<()> {
        let mut s = MemStream::construct_ro(src)?;
        self.set_image_from_stream(level, layer, face_slice, &mut s, src.len())
    }

    fn num_images(&self, level: u32) -> u32 {
        let level_depth = self.base_depth.checked_shr(level).unwrap_or(0).max(1);
        if self.is_cubemap { self.num_faces } else { level_depth }
    }

    /// Port of `ktxTexture1_WriteToStream`.
    pub fn write_to_stream(&mut self, dststr: &mut dyn Stream) -> KtxResult<()> {
        let data = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        if !self.kv_data_head.is_empty() && self.kv_data.is_some() {
            return Err(ErrorCode::InvalidOperation);
        }
        let kvd: Vec<u8> = if !self.kv_data_head.is_empty() {
            self.kv_data_head.serialize()?
        } else if let Some(k) = &self.kv_data {
            k.get(..self.kv_data_len as usize).unwrap_or(k).to_vec()
        } else {
            Vec::new()
        };
        let header = KtxHeader {
            identifier: KTX_IDENTIFIER_REF,
            endianness: KTX_ENDIAN_REF,
            gl_type: self.gl_type,
            gl_type_size: self.gl_type_size(),
            gl_format: self.gl_format,
            gl_internalformat: self.gl_internalformat,
            gl_base_internalformat: self.gl_base_internalformat,
            pixel_width: self.base_width,
            pixel_height: if self.num_dimensions > 1 { self.base_height } else { 0 },
            pixel_depth: if self.num_dimensions > 2 { self.base_depth } else { 0 },
            number_of_array_elements: if self.is_array { self.num_layers } else { 0 },
            number_of_faces: self.num_faces,
            number_of_mip_levels: if self.generate_mipmaps { 0 } else { self.num_levels },
            bytes_of_key_value_data: kvd.len() as u32,
        };
        let hb = header.to_bytes();
        dststr.write(&hb, hb.len(), 1)?;
        if !kvd.is_empty() {
            dststr.write(&kvd, 1, kvd.len())?;
        }
        // Write the image data
        let mut level_offset = 0usize;
        for level in 0..self.num_levels {
            let face_lod_size = self.do_calc_face_lod_size(level, FormatVersion::One) as u32;
            let image_size = self.get_image_size(level);
            let num_images = self.num_images(level);
            dststr.write(&face_lod_size.to_le_bytes(), 4, 1)?;
            for _layer in 0..self.num_layers {
                for _face_slice in 0..num_images {
                    let src = data.get(level_offset..level_offset + image_size).ok_or(ErrorCode::InvalidOperation)?;
                    dststr.write(src, image_size, 1)?;
                    level_offset += image_size;
                }
            }
        }
        Ok(())
    }

    /// Port of `ktxTexture1_WriteToStdioStream`.
    pub fn write_to_stdio_stream(&mut self, dst: std::fs::File) -> KtxResult<()> {
        let mut s = FileStream::construct(dst)?;
        self.write_to_stream(&mut s)
    }

    /// Port of `ktxTexture1_WriteToNamedFile`.
    pub fn write_to_named_file(&mut self, dstname: impl AsRef<std::path::Path>) -> KtxResult<()> {
        let mut s = FileStream::create(dstname)?;
        self.write_to_stream(&mut s)
    }

    /// Port of `ktxTexture1_WriteToMemory`.
    pub fn write_to_memory(&mut self) -> KtxResult<Vec<u8>> {
        let mut s = MemStream::construct()?;
        self.write_to_stream(&mut s)?;
        Ok(s.into_data())
    }

    /// Port of `ktxTexture1_WriteKTX2ToStream`.
    pub fn write_ktx2_to_stream(&mut self, dststr: &mut dyn Stream) -> KtxResult<()> {
        if self.p_data.is_none() {
            return Err(ErrorCode::InvalidOperation);
        }
        let vk_format = vk_get_format_from_opengl_internal_format(self.gl_internalformat, self.get_rgba_astc_mapping());
        if vk_format == VkFormat::UNDEFINED {
            return Err(ErrorCode::UnsupportedTextureType);
        }
        let num_levels = self.num_levels;
        let mut header = KtxHeader2 {
            identifier: KTX2_IDENTIFIER_REF,
            vk_format,
            type_size: self.gl_type_size(),
            pixel_width: self.base_width,
            pixel_height: if self.num_dimensions > 1 { self.base_height } else { 0 },
            pixel_depth: if self.num_dimensions > 2 { self.base_depth } else { 0 },
            layer_count: if self.is_array { self.num_layers } else { 0 },
            face_count: self.num_faces,
            level_count: if self.generate_mipmaps { 0 } else { num_levels },
            ..Default::default()
        };
        let level_index_size = LEVEL_INDEX_ENTRY_SIZE * num_levels as usize;
        let mut offset = (KTX2_HEADER_SIZE + level_index_size) as u64;
        let dfd = vk2dfd(vk_format).ok_or(ErrorCode::UnsupportedTextureType)?;
        let dfd_len = dfd.first().copied().unwrap_or(0);
        header.data_format_descriptor = IndexEntry32 { byte_offset: offset as u32, byte_length: dfd_len };
        offset += dfd_len as u64;

        // Check for invalid metadata. Writer metadata is allowed as we write KTX2.
        for entry in &self.kv_data_head {
            let (_, key) = entry.get_key();
            if key.len() >= 3
                && key[..3].eq_ignore_ascii_case(b"KTX")
                && key != KTX_ORIENTATION_KEY.as_bytes()
                && key != KTX_WRITER_KEY.as_bytes()
            {
                return Err(ErrorCode::InvalidOperation);
            }
        }

        // Rewrite the orientation value in the KTX2 form.
        if let Ok(orientation) = self.kv_data_head.find_value(KTX_ORIENTATION_KEY) {
            let mut new_orient = scan_ktx1_orientation(orientation);
            let nd = self.num_dimensions as usize;
            if new_orient.len() < nd {
                return Err(ErrorCode::FileDataError);
            }
            new_orient.truncate(nd);
            new_orient.push(0);
            self.kv_data_head.delete_kv_pair(KTX_ORIENTATION_KEY)?;
            self.kv_data_head.add_kv_pair(KTX_ORIENTATION_KEY, &new_orient)?;
        }

        append_lib_id(&mut self.kv_data_head)?;
        self.kv_data_head.sort()?; // KTX2 requires sorted metadata.
        let kvd = self.kv_data_head.serialize()?;
        let kvd_len = kvd.len() as u32;
        header.key_value_data =
            IndexEntry32 { byte_offset: if kvd_len != 0 { offset as u32 } else { 0 }, byte_length: kvd_len };
        offset += kvd_len as u64;

        let required_level_alignment = lcm4(self.format_size().block_size_in_bits / 8) as u64;
        let initial_level_pad_len = ktx_padn_len(required_level_alignment, offset);
        offset += initial_level_pad_len;

        let mut level_index = vec![LevelIndexEntry::default(); num_levels as usize];
        for level in (0..num_levels).rev() {
            let level_size = self.calc_level_size(level, FormatVersion::Two) as u64;
            level_index[level as usize] =
                LevelIndexEntry { uncompressed_byte_length: level_size, byte_length: level_size, byte_offset: offset };
            offset += ktx_padn(required_level_alignment, level_size);
        }

        let hb = header.to_bytes();
        dststr.write(&hb, hb.len(), 1)?;
        let li: Vec<u8> = level_index.iter().flat_map(|e| e.to_bytes()).collect();
        dststr.write(&li, 1, li.len())?;
        let mut dfd_bytes: Vec<u8> = dfd.iter().flat_map(|w| w.to_le_bytes()).collect();
        dfd_bytes.truncate(dfd_len as usize);
        dststr.write(&dfd_bytes, 1, dfd_bytes.len())?;
        if kvd_len != 0 {
            dststr.write(&kvd, 1, kvd.len())?;
        }
        let padding = [0u8; 32];
        if initial_level_pad_len != 0 {
            dststr.write(&padding, 1, initial_level_pad_len as usize)?;
        }

        let data = self.p_data.as_deref().unwrap_or(&[]);
        for level in (0..num_levels).rev() {
            let image_size = self.calc_image_size(level, FormatVersion::Two);
            let num_images = self.num_images(level);
            let (num_rows, row_bytes, row_padding) =
                if !self.is_compressed { self.row_info(level) } else { (0, 0, 0) };
            let mut src_offset = self.calc_level_offset(level);
            let mut dst_level_size: u64 = 0;
            // The KTX1 image size includes row padding; advance by it.
            let src_image_size = self.calc_image_size(level, FormatVersion::One);
            for _layer in 0..self.num_layers {
                for _face_slice in 0..num_images {
                    if row_padding == 0 {
                        let src = data.get(src_offset..src_offset + image_size).ok_or(ErrorCode::InvalidOperation)?;
                        dststr.write(src, 1, image_size)?;
                        dst_level_size += image_size as u64;
                    } else {
                        // Copy the rows individually, removing padding.
                        let packed_row_bytes = (row_bytes - row_padding) as usize;
                        for row in 0..num_rows as usize {
                            let ro = src_offset + row_bytes as usize * row;
                            let src = data.get(ro..ro + packed_row_bytes).ok_or(ErrorCode::InvalidOperation)?;
                            dststr.write(src, 1, packed_row_bytes)?;
                            dst_level_size += packed_row_bytes as u64;
                        }
                    }
                    src_offset += src_image_size;
                }
            }
            if level != 0 {
                let level_pad_len = ktx_padn_len(required_level_alignment, dst_level_size);
                if level_pad_len != 0 {
                    dststr.write(&padding, 1, level_pad_len as usize)?;
                }
            }
        }
        Ok(())
    }

    /// Port of `ktxTexture1_WriteKTX2ToStdioStream`.
    pub fn write_ktx2_to_stdio_stream(&mut self, dst: std::fs::File) -> KtxResult<()> {
        let mut s = FileStream::construct(dst)?;
        self.write_ktx2_to_stream(&mut s)
    }

    /// Port of `ktxTexture1_WriteKTX2ToNamedFile`.
    pub fn write_ktx2_to_named_file(&mut self, dstname: impl AsRef<std::path::Path>) -> KtxResult<()> {
        let mut s = FileStream::create(dstname)?;
        self.write_ktx2_to_stream(&mut s)
    }

    /// Port of `ktxTexture1_WriteKTX2ToMemory`.
    pub fn write_ktx2_to_memory(&mut self) -> KtxResult<Vec<u8>> {
        let mut s = MemStream::construct()?;
        self.write_ktx2_to_stream(&mut s)?;
        Ok(s.into_data())
    }
}
