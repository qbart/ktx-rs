// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_basis_file.cpp` (`basisu_file`): serializes a
//! `basisu_backend_output` into a `.basis` file (header, slice descriptors, ETC1S codebooks and
//! Huffman tables, slice data, CRC16s). The packed little endian `basis_file_header` /
//! `basis_slice_desc` layouts of `transcoder/basisu_file_headers.h` are written field by field.

use super::BasisTexFormat;
use super::backend::BasisuBackendOutput;
use super::enc::crc16;

/// The output file version. Keep in sync with `BASISD_SUPPORTED_BASIS_VERSION`.
pub const BASIS_FILE_VERSION: u32 = 0x13;

/// `basis_file_header::cBASISSigValue`.
pub const C_BASIS_SIG_VALUE: u32 = ((b'B' as u32) << 8) | b's' as u32;
/// `sizeof(basis_file_header)`.
pub const BASIS_FILE_HEADER_SIZE: u32 = 77;
/// `sizeof(basis_slice_desc)`.
pub const BASIS_SLICE_DESC_SIZE: u32 = 23;
/// `BASISU_OFFSETOF(basis_file_header, m_data_size)`.
const HEADER_DATA_SIZE_OFS: usize = 8;

pub const C_BASIS_HEADER_FLAG_ETC1S: u32 = 1;
pub const C_BASIS_HEADER_FLAG_Y_FLIPPED: u32 = 2;
pub const C_BASIS_HEADER_FLAG_HAS_ALPHA_SLICES: u32 = 4;
pub const C_BASIS_HEADER_FLAG_USES_GLOBAL_CODEBOOK: u32 = 8;
pub const C_BASIS_HEADER_FLAG_SRGB: u32 = 16;

pub const C_SLICE_DESC_FLAGS_HAS_ALPHA: u32 = 1;
pub const C_SLICE_DESC_FLAGS_FRAME_IS_I_FRAME: u32 = 2;

/// `cBASISMaxUSPerFrame`.
pub const C_BASIS_MAX_US_PER_FRAME: u32 = 0xFF_FFFF;

/// `basist::basis_file_header` (unpacked).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasisFileHeader {
    pub sig: u32,
    pub ver: u32,
    pub header_size: u32,
    pub header_crc16: u32,
    pub data_size: u32,
    pub data_crc16: u32,
    pub total_slices: u32,
    pub total_images: u32,
    pub tex_format: u32,
    pub flags: u32,
    pub tex_type: u32,
    pub us_per_frame: u32,
    pub reserved: u32,
    pub userdata0: u32,
    pub userdata1: u32,
    pub total_endpoints: u32,
    pub endpoint_cb_file_ofs: u32,
    pub endpoint_cb_file_size: u32,
    pub total_selectors: u32,
    pub selector_cb_file_ofs: u32,
    pub selector_cb_file_size: u32,
    pub tables_file_ofs: u32,
    pub tables_file_size: u32,
    pub slice_desc_file_ofs: u32,
    pub extended_file_ofs: u32,
    pub extended_file_size: u32,
}

/// Field widths of the packed `basis_file_header`, in declaration order.
const HEADER_FIELD_BYTES: [usize; 26] = [
    2, 2, 2, 2, 4, 2, 3, 3, 1, 2, 1, 3, 4, 4, 4, 2, 4, 3, 2, 4, 3, 4, 4, 4, 4, 4,
];

fn put_packed(out: &mut Vec<u8>, v: u32, n: usize) {
    for i in 0..n {
        out.push((v >> (8 * i)) as u8);
    }
}

fn get_packed(b: &[u8], n: usize) -> u32 {
    b.iter()
        .take(n)
        .enumerate()
        .fold(0, |v, (i, &x)| v | ((x as u32) << (8 * i)))
}

impl BasisFileHeader {
    fn fields(&self) -> [u32; 26] {
        [
            self.sig,
            self.ver,
            self.header_size,
            self.header_crc16,
            self.data_size,
            self.data_crc16,
            self.total_slices,
            self.total_images,
            self.tex_format,
            self.flags,
            self.tex_type,
            self.us_per_frame,
            self.reserved,
            self.userdata0,
            self.userdata1,
            self.total_endpoints,
            self.endpoint_cb_file_ofs,
            self.endpoint_cb_file_size,
            self.total_selectors,
            self.selector_cb_file_ofs,
            self.selector_cb_file_size,
            self.tables_file_ofs,
            self.tables_file_size,
            self.slice_desc_file_ofs,
            self.extended_file_ofs,
            self.extended_file_size,
        ]
    }

    /// Serializes the packed header (77 bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BASIS_FILE_HEADER_SIZE as usize);
        for (v, n) in self.fields().iter().zip(HEADER_FIELD_BYTES) {
            put_packed(&mut out, *v, n);
        }
        out
    }

    /// Parses a packed header; `None` if `b` is too short.
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < BASIS_FILE_HEADER_SIZE as usize {
            return None;
        }
        let mut f = [0u32; 26];
        let mut ofs = 0;
        for (v, n) in f.iter_mut().zip(HEADER_FIELD_BYTES) {
            *v = get_packed(&b[ofs..], n);
            ofs += n;
        }
        Some(BasisFileHeader {
            sig: f[0],
            ver: f[1],
            header_size: f[2],
            header_crc16: f[3],
            data_size: f[4],
            data_crc16: f[5],
            total_slices: f[6],
            total_images: f[7],
            tex_format: f[8],
            flags: f[9],
            tex_type: f[10],
            us_per_frame: f[11],
            reserved: f[12],
            userdata0: f[13],
            userdata1: f[14],
            total_endpoints: f[15],
            endpoint_cb_file_ofs: f[16],
            endpoint_cb_file_size: f[17],
            total_selectors: f[18],
            selector_cb_file_ofs: f[19],
            selector_cb_file_size: f[20],
            tables_file_ofs: f[21],
            tables_file_size: f[22],
            slice_desc_file_ofs: f[23],
            extended_file_ofs: f[24],
            extended_file_size: f[25],
        })
    }
}

/// `basist::basis_slice_desc` (unpacked).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasisSliceDesc {
    pub image_index: u32,
    pub level_index: u32,
    pub flags: u32,
    pub orig_width: u32,
    pub orig_height: u32,
    pub num_blocks_x: u32,
    pub num_blocks_y: u32,
    pub file_ofs: u32,
    pub file_size: u32,
    pub slice_data_crc16: u32,
}

const SLICE_FIELD_BYTES: [usize; 10] = [3, 1, 1, 2, 2, 2, 2, 4, 4, 2];

impl BasisSliceDesc {
    fn fields(&self) -> [u32; 10] {
        [
            self.image_index,
            self.level_index,
            self.flags,
            self.orig_width,
            self.orig_height,
            self.num_blocks_x,
            self.num_blocks_y,
            self.file_ofs,
            self.file_size,
            self.slice_data_crc16,
        ]
    }

    /// Appends the packed descriptor (23 bytes).
    pub fn write(&self, out: &mut Vec<u8>) {
        for (v, n) in self.fields().iter().zip(SLICE_FIELD_BYTES) {
            put_packed(out, *v, n);
        }
    }

    /// Parses a packed descriptor; `None` if `b` is too short.
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < BASIS_SLICE_DESC_SIZE as usize {
            return None;
        }
        let mut f = [0u32; 10];
        let mut ofs = 0;
        for (v, n) in f.iter_mut().zip(SLICE_FIELD_BYTES) {
            *v = get_packed(&b[ofs..], n);
            ofs += n;
        }
        Some(BasisSliceDesc {
            image_index: f[0],
            level_index: f[1],
            flags: f[2],
            orig_width: f[3],
            orig_height: f[4],
            num_blocks_x: f[5],
            num_blocks_y: f[6],
            file_ofs: f[7],
            file_size: f[8],
            slice_data_crc16: f[9],
        })
    }
}

/// `basisu_file`.
#[derive(Debug, Clone, Default)]
pub struct BasisuFile {
    header: BasisFileHeader,
    images_descs: Vec<BasisSliceDesc>,
    comp_data: Vec<u8>,
    header_file_ofs: u32,
    slice_descs_file_ofs: u32,
    endpoint_cb_file_ofs: u32,
    selector_cb_file_ofs: u32,
    tables_file_ofs: u32,
    first_image_file_ofs: u32,
    total_file_size: u32,
}

impl BasisuFile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn get_compressed_data(&self) -> &[u8] {
        &self.comp_data
    }

    fn create_header(
        &mut self,
        encoder_output: &BasisuBackendOutput,
        tex_type: u32,
        userdata0: u32,
        userdata1: u32,
        y_flipped: bool,
        us_per_frame: u32,
    ) {
        let h = &mut self.header;
        h.header_size = BASIS_FILE_HEADER_SIZE;
        h.data_size = self.total_file_size.wrapping_sub(BASIS_FILE_HEADER_SIZE);
        h.total_slices = encoder_output.slice_desc.len() as u32;
        h.total_images = 0;
        for d in &encoder_output.slice_desc {
            h.total_images = h.total_images.max(d.source_file_index + 1);
        }
        h.tex_format = encoder_output.tex_format as u32;
        h.flags = 0;
        if encoder_output.etc1s {
            h.flags |= C_BASIS_HEADER_FLAG_ETC1S;
        }
        if y_flipped {
            h.flags |= C_BASIS_HEADER_FLAG_Y_FLIPPED;
        }
        if encoder_output.uses_global_codebooks {
            h.flags |= C_BASIS_HEADER_FLAG_USES_GLOBAL_CODEBOOK;
        }
        if encoder_output.srgb {
            h.flags |= C_BASIS_HEADER_FLAG_SRGB;
        }
        if encoder_output.slice_desc.iter().any(|d| d.alpha) {
            h.flags |= C_BASIS_HEADER_FLAG_HAS_ALPHA_SLICES;
        }
        h.tex_type = tex_type & 0xFF;
        h.us_per_frame = us_per_frame.min(C_BASIS_MAX_US_PER_FRAME);
        h.userdata0 = userdata0;
        h.userdata1 = userdata1;

        h.total_endpoints = encoder_output.num_endpoints;
        if !encoder_output.uses_global_codebooks {
            h.endpoint_cb_file_ofs = self.endpoint_cb_file_ofs;
            h.endpoint_cb_file_size = encoder_output.endpoint_palette.len() as u32;
        }
        h.total_selectors = encoder_output.num_selectors;
        if !encoder_output.uses_global_codebooks {
            h.selector_cb_file_ofs = self.selector_cb_file_ofs;
            h.selector_cb_file_size = encoder_output.selector_palette.len() as u32;
        }
        h.tables_file_ofs = self.tables_file_ofs;
        h.tables_file_size = encoder_output.slice_image_tables.len() as u32;
        h.slice_desc_file_ofs = self.slice_descs_file_ofs;
    }

    fn create_image_descs(&mut self, encoder_output: &BasisuBackendOutput) -> bool {
        let slice_descs = &encoder_output.slice_desc;
        self.images_descs = vec![BasisSliceDesc::default(); slice_descs.len()];
        let mut cur_slice_file_ofs = self.first_image_file_ofs as u64;
        for (i, (d, s)) in self.images_descs.iter_mut().zip(slice_descs).enumerate() {
            d.image_index = s.source_file_index;
            d.level_index = s.mip_index;
            if s.alpha {
                d.flags |= C_SLICE_DESC_FLAGS_HAS_ALPHA;
            }
            if s.iframe {
                d.flags |= C_SLICE_DESC_FLAGS_FRAME_IS_I_FRAME;
            }
            d.orig_width = s.orig_width;
            d.orig_height = s.orig_height;
            d.num_blocks_x = s.num_blocks_x;
            d.num_blocks_y = s.num_blocks_y;
            d.slice_data_crc16 =
                encoder_output.slice_image_crcs.get(i).copied().unwrap_or(0) as u32;

            let Some(data) = encoder_output.slice_image_data.get(i) else {
                return false;
            };
            if data.len() as u64 > u32::MAX as u64 {
                return false;
            }
            let image_size = data.len() as u32;
            d.file_ofs = cur_slice_file_ofs as u32;
            d.file_size = image_size;
            cur_slice_file_ofs += image_size as u64;
            if cur_slice_file_ofs > u32::MAX as u64 {
                return false;
            }
        }
        true
    }

    fn create_comp_data(&mut self, encoder_output: &BasisuBackendOutput) {
        self.comp_data.extend_from_slice(&self.header.to_bytes());
        for d in &self.images_descs {
            d.write(&mut self.comp_data);
        }
        if !encoder_output.uses_global_codebooks {
            self.comp_data
                .extend_from_slice(&encoder_output.endpoint_palette);
            self.comp_data
                .extend_from_slice(&encoder_output.selector_palette);
        }
        self.comp_data
            .extend_from_slice(&encoder_output.slice_image_tables);
        for data in &encoder_output.slice_image_data {
            self.comp_data.extend_from_slice(data);
        }
    }

    fn fixup_crcs(&mut self) {
        let hs = BASIS_FILE_HEADER_SIZE as usize;
        let data_size = self.total_file_size.wrapping_sub(BASIS_FILE_HEADER_SIZE);
        let data_crc = crc16(self.comp_data.get(hs..).unwrap_or(&[]), 0);
        self.header.data_size = data_size;
        self.header.data_crc16 = data_crc as u32;
        // The header CRC covers everything from m_data_size on, so patch those fields first.
        let mut bytes = self.header.to_bytes();
        let header_crc = crc16(&bytes[HEADER_DATA_SIZE_OFS..], 0);
        self.header.header_crc16 = header_crc as u32;
        self.header.sig = C_BASIS_SIG_VALUE;
        self.header.ver = BASIS_FILE_VERSION;
        bytes = self.header.to_bytes();
        self.comp_data[..hs].copy_from_slice(&bytes);
    }

    /// `basisu_file::init`.
    pub fn init(
        &mut self,
        encoder_output: &BasisuBackendOutput,
        tex_type: u32,
        userdata0: u32,
        userdata1: u32,
        y_flipped: bool,
        us_per_frame: u32,
    ) -> bool {
        self.clear();
        let slice_descs = &encoder_output.slice_desc;
        let hs = BASIS_FILE_HEADER_SIZE as u64;
        let sds = BASIS_SLICE_DESC_SIZE as u64 * slice_descs.len() as u64;

        // The Basis file uses 32-bit fields for lots of stuff, so make sure it's not too large.
        let check_size = if !encoder_output.uses_global_codebooks {
            hs + sds
                + encoder_output.endpoint_palette.len() as u64
                + encoder_output.selector_palette.len() as u64
                + encoder_output.slice_image_tables.len() as u64
        } else {
            hs + sds + encoder_output.slice_image_tables.len() as u64
        };
        if check_size >= 0xFFFF_0000 {
            return false;
        }

        self.header_file_ofs = 0;
        self.slice_descs_file_ofs = BASIS_FILE_HEADER_SIZE;
        let after_descs = self.slice_descs_file_ofs + sds as u32;
        if encoder_output.tex_format == BasisTexFormat::Etc1s {
            if encoder_output.uses_global_codebooks {
                self.endpoint_cb_file_ofs = 0;
                self.selector_cb_file_ofs = 0;
                self.tables_file_ofs = after_descs;
            } else {
                self.endpoint_cb_file_ofs = after_descs;
                self.selector_cb_file_ofs =
                    self.endpoint_cb_file_ofs + encoder_output.endpoint_palette.len() as u32;
                self.tables_file_ofs =
                    self.selector_cb_file_ofs + encoder_output.selector_palette.len() as u32;
            }
            self.first_image_file_ofs =
                self.tables_file_ofs + encoder_output.slice_image_tables.len() as u32;
        } else {
            self.endpoint_cb_file_ofs = 0;
            self.selector_cb_file_ofs = 0;
            self.tables_file_ofs = 0;
            self.first_image_file_ofs = after_descs;
        }

        let mut total_file_size = self.first_image_file_ofs as u64;
        for data in &encoder_output.slice_image_data {
            total_file_size += data.len() as u64;
        }
        if total_file_size >= 0xFFFF_0000 {
            return false;
        }
        self.total_file_size = total_file_size as u32;

        self.create_header(
            encoder_output,
            tex_type,
            userdata0,
            userdata1,
            y_flipped,
            us_per_frame,
        );
        if !self.create_image_descs(encoder_output) {
            return false;
        }
        self.create_comp_data(encoder_output);
        if self.comp_data.len() != self.total_file_size as usize {
            return false;
        }
        self.fixup_crcs();
        true
    }
}
