// SPDX-FileCopyrightText: 2019-2020 The Khronos Group Inc.
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software dfdutils and modified; see NOTICE.md.
//! Port of the `KHR_DFDVAL`, `KHR_DFDSETVAL`, `KHR_DFDSVAL`, `KHR_DFDSETSVAL`,
//! `KHR_DFDSAMPLECOUNT` and `KHR_DFDSIZEWORDS` macros from `khr_df.h`.
//!
//! `bdb` is the basic descriptor block (i.e. `DFD + 1`). Out-of-range reads
//! return 0 and out-of-range writes are ignored, so malformed input never panics.

use super::khr_df::*;

/// A basic-descriptor-block field: word index, shift and mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfdField {
    pub word: usize,
    pub shift: u32,
    pub mask: u32,
}

/// A sample field: word within the sample, shift and mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfdSampleField {
    pub word: usize,
    pub shift: u32,
    pub mask: u32,
}

const fn f(word: u32, shift: u32, mask: u32) -> DfdField {
    DfdField {
        word: word as usize,
        shift,
        mask,
    }
}

const fn sf(word: u32, shift: u32, mask: u32) -> DfdSampleField {
    DfdSampleField {
        word: word as usize,
        shift,
        mask,
    }
}

pub const VENDORID: DfdField = f(
    KHR_DF_WORD_VENDORID,
    KHR_DF_SHIFT_VENDORID,
    KHR_DF_MASK_VENDORID,
);
pub const DESCRIPTORTYPE: DfdField = f(
    KHR_DF_WORD_DESCRIPTORTYPE,
    KHR_DF_SHIFT_DESCRIPTORTYPE,
    KHR_DF_MASK_DESCRIPTORTYPE,
);
pub const VERSIONNUMBER: DfdField = f(
    KHR_DF_WORD_VERSIONNUMBER,
    KHR_DF_SHIFT_VERSIONNUMBER,
    KHR_DF_MASK_VERSIONNUMBER,
);
pub const DESCRIPTORBLOCKSIZE: DfdField = f(
    KHR_DF_WORD_DESCRIPTORBLOCKSIZE,
    KHR_DF_SHIFT_DESCRIPTORBLOCKSIZE,
    KHR_DF_MASK_DESCRIPTORBLOCKSIZE,
);
pub const MODEL: DfdField = f(KHR_DF_WORD_MODEL, KHR_DF_SHIFT_MODEL, KHR_DF_MASK_MODEL);
pub const PRIMARIES: DfdField = f(
    KHR_DF_WORD_PRIMARIES,
    KHR_DF_SHIFT_PRIMARIES,
    KHR_DF_MASK_PRIMARIES,
);
pub const TRANSFER: DfdField = f(
    KHR_DF_WORD_TRANSFER,
    KHR_DF_SHIFT_TRANSFER,
    KHR_DF_MASK_TRANSFER,
);
pub const FLAGS: DfdField = f(KHR_DF_WORD_FLAGS, KHR_DF_SHIFT_FLAGS, KHR_DF_MASK_FLAGS);
pub const TEXELBLOCKDIMENSION0: DfdField = f(
    KHR_DF_WORD_TEXELBLOCKDIMENSION0,
    KHR_DF_SHIFT_TEXELBLOCKDIMENSION0,
    KHR_DF_MASK_TEXELBLOCKDIMENSION0,
);
pub const TEXELBLOCKDIMENSION1: DfdField = f(
    KHR_DF_WORD_TEXELBLOCKDIMENSION1,
    KHR_DF_SHIFT_TEXELBLOCKDIMENSION1,
    KHR_DF_MASK_TEXELBLOCKDIMENSION1,
);
pub const TEXELBLOCKDIMENSION2: DfdField = f(
    KHR_DF_WORD_TEXELBLOCKDIMENSION2,
    KHR_DF_SHIFT_TEXELBLOCKDIMENSION2,
    KHR_DF_MASK_TEXELBLOCKDIMENSION2,
);
pub const TEXELBLOCKDIMENSION3: DfdField = f(
    KHR_DF_WORD_TEXELBLOCKDIMENSION3,
    KHR_DF_SHIFT_TEXELBLOCKDIMENSION3,
    KHR_DF_MASK_TEXELBLOCKDIMENSION3,
);
pub const BYTESPLANE0: DfdField = f(
    KHR_DF_WORD_BYTESPLANE0,
    KHR_DF_SHIFT_BYTESPLANE0,
    KHR_DF_MASK_BYTESPLANE0,
);
pub const BYTESPLANE1: DfdField = f(
    KHR_DF_WORD_BYTESPLANE1,
    KHR_DF_SHIFT_BYTESPLANE1,
    KHR_DF_MASK_BYTESPLANE1,
);
pub const BYTESPLANE2: DfdField = f(
    KHR_DF_WORD_BYTESPLANE2,
    KHR_DF_SHIFT_BYTESPLANE2,
    KHR_DF_MASK_BYTESPLANE2,
);
pub const BYTESPLANE3: DfdField = f(
    KHR_DF_WORD_BYTESPLANE3,
    KHR_DF_SHIFT_BYTESPLANE3,
    KHR_DF_MASK_BYTESPLANE3,
);
pub const BYTESPLANE4: DfdField = f(
    KHR_DF_WORD_BYTESPLANE4,
    KHR_DF_SHIFT_BYTESPLANE4,
    KHR_DF_MASK_BYTESPLANE4,
);
pub const BYTESPLANE5: DfdField = f(
    KHR_DF_WORD_BYTESPLANE5,
    KHR_DF_SHIFT_BYTESPLANE5,
    KHR_DF_MASK_BYTESPLANE5,
);
pub const BYTESPLANE6: DfdField = f(
    KHR_DF_WORD_BYTESPLANE6,
    KHR_DF_SHIFT_BYTESPLANE6,
    KHR_DF_MASK_BYTESPLANE6,
);
pub const BYTESPLANE7: DfdField = f(
    KHR_DF_WORD_BYTESPLANE7,
    KHR_DF_SHIFT_BYTESPLANE7,
    KHR_DF_MASK_BYTESPLANE7,
);

pub const BITOFFSET: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_BITOFFSET,
    KHR_DF_SAMPLESHIFT_BITOFFSET,
    KHR_DF_SAMPLEMASK_BITOFFSET,
);
pub const BITLENGTH: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_BITLENGTH,
    KHR_DF_SAMPLESHIFT_BITLENGTH,
    KHR_DF_SAMPLEMASK_BITLENGTH,
);
pub const CHANNELID: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_CHANNELID,
    KHR_DF_SAMPLESHIFT_CHANNELID,
    KHR_DF_SAMPLEMASK_CHANNELID,
);
pub const QUALIFIERS: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_QUALIFIERS,
    KHR_DF_SAMPLESHIFT_QUALIFIERS,
    KHR_DF_SAMPLEMASK_QUALIFIERS,
);
pub const SAMPLEPOSITION0: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEPOSITION0,
    KHR_DF_SAMPLESHIFT_SAMPLEPOSITION0,
    KHR_DF_SAMPLEMASK_SAMPLEPOSITION0,
);
pub const SAMPLEPOSITION1: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEPOSITION1,
    KHR_DF_SAMPLESHIFT_SAMPLEPOSITION1,
    KHR_DF_SAMPLEMASK_SAMPLEPOSITION1,
);
pub const SAMPLEPOSITION2: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEPOSITION2,
    KHR_DF_SAMPLESHIFT_SAMPLEPOSITION2,
    KHR_DF_SAMPLEMASK_SAMPLEPOSITION2,
);
pub const SAMPLEPOSITION3: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEPOSITION3,
    KHR_DF_SAMPLESHIFT_SAMPLEPOSITION3,
    KHR_DF_SAMPLEMASK_SAMPLEPOSITION3,
);
pub const SAMPLEPOSITION_ALL: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEPOSITION_ALL,
    KHR_DF_SAMPLESHIFT_SAMPLEPOSITION_ALL,
    KHR_DF_SAMPLEMASK_SAMPLEPOSITION_ALL,
);
pub const SAMPLELOWER: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLELOWER,
    KHR_DF_SAMPLESHIFT_SAMPLELOWER,
    KHR_DF_SAMPLEMASK_SAMPLELOWER,
);
pub const SAMPLEUPPER: DfdSampleField = sf(
    KHR_DF_SAMPLEWORD_SAMPLEUPPER,
    KHR_DF_SAMPLESHIFT_SAMPLEUPPER,
    KHR_DF_SAMPLEMASK_SAMPLEUPPER,
);

/// `KHR_DFDVAL(BDB, X)`.
pub fn dfdval(bdb: &[u32], x: DfdField) -> u32 {
    bdb.get(x.word).map_or(0, |w| (w >> x.shift) & x.mask)
}

/// `KHR_DFDSETVAL(BDB, X, val)`.
pub fn dfdsetval(bdb: &mut [u32], x: DfdField, val: u32) {
    if let Some(w) = bdb.get_mut(x.word) {
        *w = (*w & !(x.mask << x.shift)) | ((val & x.mask) << x.shift);
    }
}

fn sample_word(s: u32, x: DfdSampleField) -> usize {
    KHR_DF_WORD_SAMPLESTART as usize + s as usize * KHR_DF_WORD_SAMPLEWORDS as usize + x.word
}

/// `KHR_DFDSVAL(BDB, S, X)`.
pub fn dfdsval(bdb: &[u32], s: u32, x: DfdSampleField) -> u32 {
    bdb.get(sample_word(s, x))
        .map_or(0, |w| (w >> x.shift) & x.mask)
}

/// `KHR_DFDSETSVAL(BDB, S, X, val)`.
pub fn dfdsetsval(bdb: &mut [u32], s: u32, x: DfdSampleField, val: u32) {
    if let Some(w) = bdb.get_mut(sample_word(s, x)) {
        *w = (*w & !(x.mask << x.shift)) | ((val & x.mask) << x.shift);
    }
}

/// `KHR_DFDSAMPLECOUNT(BDB)`.
pub fn dfdsamplecount(bdb: &[u32]) -> u32 {
    ((dfdval(bdb, DESCRIPTORBLOCKSIZE) >> 2).wrapping_sub(KHR_DF_WORD_SAMPLESTART))
        / KHR_DF_WORD_SAMPLEWORDS
}

/// `KHR_DFDSIZEWORDS(S)`.
pub const fn dfdsizewords(s: u32) -> u32 {
    KHR_DF_WORD_SAMPLESTART + s * KHR_DF_WORD_SAMPLEWORDS
}

/// Basic descriptor block of a full DFD (`DFD + 1`). Empty if `dfd` is empty.
pub fn bdb(dfd: &[u32]) -> &[u32] {
    dfd.get(1..).unwrap_or(&[])
}

/// Mutable basic descriptor block of a full DFD.
pub fn bdb_mut(dfd: &mut [u32]) -> &mut [u32] {
    dfd.get_mut(1..).unwrap_or(&mut [])
}
