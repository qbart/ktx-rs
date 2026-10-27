// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `huffman_decoding_table`, `bitwise_decoder` and `approx_move_to_front`
//! from basisu_transcoder_internal.h, plus the Huffman constants of basisu.h.
//!
//! Reads past the end of the buffer yield zero bits (as in C); corrupt tree
//! references decode to symbol 0 instead of reading out of bounds.

pub const HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE: usize = 31;
pub const HUFFMAN_FAST_LOOKUP_BITS: u32 = 10;
pub const HUFFMAN_MAX_SYMS_LOG2: u32 = 14;
pub const HUFFMAN_MAX_SYMS: u32 = 1 << HUFFMAN_MAX_SYMS_LOG2;
pub const HUFFMAN_SMALL_ZERO_RUN_SIZE_MIN: u32 = 3;
pub const HUFFMAN_SMALL_ZERO_RUN_EXTRA_BITS: u32 = 3;
pub const HUFFMAN_BIG_ZERO_RUN_SIZE_MIN: u32 = 11;
pub const HUFFMAN_BIG_ZERO_RUN_EXTRA_BITS: u32 = 7;
pub const HUFFMAN_SMALL_REPEAT_SIZE_MIN: u32 = 3;
pub const HUFFMAN_SMALL_REPEAT_EXTRA_BITS: u32 = 2;
pub const HUFFMAN_BIG_REPEAT_SIZE_MIN: u32 = 7;
pub const HUFFMAN_BIG_REPEAT_EXTRA_BITS: u32 = 7;
pub const HUFFMAN_TOTAL_CODELENGTH_CODES: usize = 21;
pub const HUFFMAN_SMALL_ZERO_RUN_CODE: u32 = 17;
pub const HUFFMAN_BIG_ZERO_RUN_CODE: u32 = 18;
pub const HUFFMAN_SMALL_REPEAT_CODE: u32 = 19;
pub const HUFFMAN_BIG_REPEAT_CODE: u32 = 20;

/// `g_huffman_sorted_codelength_codes`.
pub const HUFFMAN_SORTED_CODELENGTH_CODES: [u8; 21] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 0xA, 5, 0xB, 4, 0xC, 3, 0xD, 2, 0xE, 1, 0xF, 0x10,
];

/// Port of `huffman_decoding_table`.
#[derive(Debug, Clone, Default)]
pub struct HuffmanDecodingTable {
    code_sizes: Vec<u8>,
    lookup: Vec<i32>,
    tree: Vec<i16>,
}

impl HuffmanDecodingTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.code_sizes.clear();
        self.lookup.clear();
        self.tree.clear();
    }

    pub fn code_sizes(&self) -> &[u8] {
        &self.code_sizes
    }

    pub fn is_valid(&self) -> bool {
        !self.code_sizes.is_empty()
    }

    /// Port of `init`.
    pub fn init(&mut self, code_sizes: &[u8], fast_lookup_bits: u32) -> bool {
        let total_syms = code_sizes.len();
        if total_syms == 0 {
            self.clear();
            return true;
        }
        self.code_sizes = code_sizes.to_vec();
        let fast_size = 1usize << fast_lookup_bits;
        self.lookup = vec![0; fast_size];
        self.tree = vec![0; total_syms * 2];

        let mut syms_using_codesize = [0u32; HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE + 1];
        for &cs in code_sizes {
            if cs as usize > HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE {
                return false;
            }
            syms_using_codesize[cs as usize] += 1;
        }
        let mut next_code = [0u32; HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE + 1];
        let mut used_syms = 0u32;
        let mut total = 0u32;
        for i in 1..HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE {
            used_syms += syms_using_codesize[i];
            total = (total.wrapping_add(syms_using_codesize[i])) << 1;
            next_code[i + 1] = total;
        }
        if (1u32 << HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE) != total && used_syms != 1 {
            return false;
        }

        let mut tree_next: i32 = -1;
        for (sym_index, &cs) in code_sizes.iter().enumerate() {
            let code_size = cs as u32;
            if code_size == 0 {
                continue;
            }
            let mut cur_code = next_code[code_size as usize];
            next_code[code_size as usize] = cur_code.wrapping_add(1);
            let mut rev_code: u32 = 0;
            for _ in 0..code_size {
                rev_code = (rev_code << 1) | (cur_code & 1);
                cur_code >>= 1;
            }
            if code_size <= fast_lookup_bits {
                let k = ((code_size << 16) | sym_index as u32) as i32;
                while (rev_code as usize) < fast_size {
                    if self.lookup[rev_code as usize] != 0 {
                        return false;
                    }
                    self.lookup[rev_code as usize] = k;
                    rev_code += 1 << code_size;
                }
                continue;
            }
            let idx = (rev_code as usize) & (fast_size - 1);
            let mut tree_cur = self.lookup[idx];
            if tree_cur == 0 {
                self.lookup[idx] = tree_next;
                tree_cur = tree_next;
                tree_next -= 2;
            }
            if tree_cur >= 0 {
                return false;
            }
            rev_code >>= fast_lookup_bits - 1;
            let mut j = code_size as i32;
            while j > fast_lookup_bits as i32 + 1 {
                rev_code >>= 1;
                tree_cur -= (rev_code & 1) as i32;
                let tidx = -tree_cur - 1;
                if tidx < 0 {
                    return false;
                }
                let tidx = tidx as usize;
                if tidx >= self.tree.len() {
                    self.tree.resize(tidx + 1, 0);
                }
                if self.tree[tidx] == 0 {
                    self.tree[tidx] = tree_next as i16;
                    tree_cur = tree_next;
                    tree_next -= 2;
                } else {
                    tree_cur = self.tree[tidx] as i32;
                    if tree_cur >= 0 {
                        return false;
                    }
                }
                j -= 1;
            }
            rev_code >>= 1;
            tree_cur -= (rev_code & 1) as i32;
            let tidx = -tree_cur - 1;
            if tidx < 0 {
                return false;
            }
            let tidx = tidx as usize;
            if tidx >= self.tree.len() {
                self.tree.resize(tidx + 1, 0);
            }
            if self.tree[tidx] != 0 {
                return false;
            }
            self.tree[tidx] = sym_index as i16;
        }
        true
    }
}

/// Port of `bitwise_decoder`.
#[derive(Debug, Clone, Default)]
pub struct BitwiseDecoder<'a> {
    buf: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_buf_size: u32,
}

impl<'a> BitwiseDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        BitwiseDecoder {
            buf,
            pos: 0,
            bit_buf: 0,
            bit_buf_size: 0,
        }
    }

    /// Port of `init`.
    pub fn init(&mut self, buf: &'a [u8]) -> bool {
        *self = Self::new(buf);
        true
    }

    fn next_byte(&mut self) -> u32 {
        match self.buf.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                c as u32
            }
            None => 0,
        }
    }

    pub fn peek_bits(&mut self, num_bits: u32) -> u32 {
        if num_bits == 0 {
            return 0;
        }
        while self.bit_buf_size < num_bits {
            let c = self.next_byte();
            self.bit_buf |= c << self.bit_buf_size;
            self.bit_buf_size += 8;
        }
        self.bit_buf & ((1u32 << num_bits) - 1)
    }

    pub fn remove_bits(&mut self, num_bits: u32) {
        let n = num_bits.min(self.bit_buf_size);
        self.bit_buf = self.bit_buf.checked_shr(n).unwrap_or(0);
        self.bit_buf_size -= n;
    }

    pub fn get_bits(&mut self, mut num_bits: u32) -> u32 {
        if num_bits > 25 {
            let bits0 = self.peek_bits(25);
            self.remove_bits(25);
            num_bits = num_bits.min(32) - 25;
            let bits = self.peek_bits(num_bits);
            self.remove_bits(num_bits);
            return bits0 | (bits << 25);
        }
        let bits = self.peek_bits(num_bits);
        self.remove_bits(num_bits);
        bits
    }

    /// Port of `decode_truncated_binary`.
    pub fn decode_truncated_binary(&mut self, n: u32) -> u32 {
        let k = 31 - n.max(1).leading_zeros();
        let u = (1u32 << (k + 1)).wrapping_sub(n);
        let mut result = self.get_bits(k);
        if result >= u {
            result = ((result << 1) | self.get_bits(1)).wrapping_sub(u);
        }
        result
    }

    /// Port of `decode_rice`.
    pub fn decode_rice(&mut self, m: u32) -> u32 {
        let mut q = 0u32;
        loop {
            let mut k = self.peek_bits(16);
            let mut l = 0;
            while k & 1 != 0 {
                l += 1;
                k >>= 1;
            }
            q = q.wrapping_add(l);
            self.remove_bits(l);
            if l < 16 {
                break;
            }
        }
        (q << m).wrapping_add(self.get_bits(m + 1) >> 1)
    }

    /// Port of `decode_vlc`.
    pub fn decode_vlc(&mut self, chunk_bits: u32) -> u32 {
        let chunk_size = 1u32 << chunk_bits;
        let chunk_mask = chunk_size - 1;
        let mut v = 0u32;
        let mut ofs = 0u32;
        loop {
            let s = self.get_bits(chunk_bits + 1);
            v |= (s & chunk_mask).checked_shl(ofs).unwrap_or(0);
            ofs += chunk_bits;
            if s & chunk_size == 0 || ofs >= 32 {
                break;
            }
        }
        v
    }

    /// Port of `decode_huffman`.
    pub fn decode_huffman(&mut self, ct: &HuffmanDecodingTable) -> u32 {
        self.decode_huffman_bits(ct, HUFFMAN_FAST_LOOKUP_BITS)
    }

    pub fn decode_huffman_bits(&mut self, ct: &HuffmanDecodingTable, fast_lookup_bits: u32) -> u32 {
        let fast_size = 1u32 << fast_lookup_bits;
        while self.bit_buf_size < 16 {
            let c = self.next_byte();
            self.bit_buf |= c << self.bit_buf_size;
            self.bit_buf_size += 8;
        }
        let mut sym = ct
            .lookup
            .get((self.bit_buf & (fast_size - 1)) as usize)
            .copied()
            .unwrap_or(0);
        let code_len;
        if sym >= 0 {
            code_len = (sym >> 16) as u32;
            sym &= 0xFFFF;
        } else {
            let mut cl = fast_lookup_bits;
            loop {
                let bit = (self.bit_buf.checked_shr(cl).unwrap_or(0) & 1) as i32;
                cl += 1;
                sym = ct
                    .tree
                    .get((!sym + bit) as usize)
                    .copied()
                    .map(i32::from)
                    .unwrap_or(0);
                if sym >= 0 || cl > 32 {
                    break;
                }
            }
            if sym < 0 {
                sym = 0;
            }
            code_len = cl;
        }
        self.remove_bits(code_len);
        sym as u32
    }

    /// Port of `read_huffman_table`.
    pub fn read_huffman_table(&mut self, ct: &mut HuffmanDecodingTable) -> bool {
        ct.clear();
        let total_used_syms = self.get_bits(HUFFMAN_MAX_SYMS_LOG2);
        if total_used_syms == 0 {
            return true;
        }
        if total_used_syms > HUFFMAN_MAX_SYMS {
            return false;
        }
        let mut code_length_code_sizes = [0u8; HUFFMAN_TOTAL_CODELENGTH_CODES];
        let num_codelength_codes = self.get_bits(5) as usize;
        if !(1..=HUFFMAN_TOTAL_CODELENGTH_CODES).contains(&num_codelength_codes) {
            return false;
        }
        for &code in HUFFMAN_SORTED_CODELENGTH_CODES
            .iter()
            .take(num_codelength_codes)
        {
            code_length_code_sizes[code as usize] = self.get_bits(3) as u8;
        }
        let mut code_length_table = HuffmanDecodingTable::new();
        if !code_length_table.init(&code_length_code_sizes, HUFFMAN_FAST_LOOKUP_BITS) {
            return false;
        }
        if !code_length_table.is_valid() {
            return false;
        }
        let total = total_used_syms as usize;
        let mut code_sizes = vec![0u8; total];
        let mut cur = 0usize;
        while cur < total {
            let c = self.decode_huffman(&code_length_table);
            if c <= 16 {
                code_sizes[cur] = c as u8;
                cur += 1;
            } else if c == HUFFMAN_SMALL_ZERO_RUN_CODE {
                cur += (self.get_bits(HUFFMAN_SMALL_ZERO_RUN_EXTRA_BITS)
                    + HUFFMAN_SMALL_ZERO_RUN_SIZE_MIN) as usize;
            } else if c == HUFFMAN_BIG_ZERO_RUN_CODE {
                cur += (self.get_bits(HUFFMAN_BIG_ZERO_RUN_EXTRA_BITS)
                    + HUFFMAN_BIG_ZERO_RUN_SIZE_MIN) as usize;
            } else {
                if cur == 0 {
                    return false;
                }
                let mut l = if c == HUFFMAN_SMALL_REPEAT_CODE {
                    self.get_bits(HUFFMAN_SMALL_REPEAT_EXTRA_BITS) + HUFFMAN_SMALL_REPEAT_SIZE_MIN
                } else {
                    self.get_bits(HUFFMAN_BIG_REPEAT_EXTRA_BITS) + HUFFMAN_BIG_REPEAT_SIZE_MIN
                };
                let prev = code_sizes[cur - 1];
                if prev == 0 {
                    return false;
                }
                loop {
                    if cur >= total {
                        return false;
                    }
                    code_sizes[cur] = prev;
                    cur += 1;
                    l -= 1;
                    if l == 0 {
                        break;
                    }
                }
            }
        }
        if cur != total {
            return false;
        }
        ct.init(&code_sizes, HUFFMAN_FAST_LOOKUP_BITS)
    }

    /// Port of `get_bits_remaining`.
    pub fn get_bits_remaining(&self) -> usize {
        (self.buf.len().saturating_sub(self.pos)) * 8 + self.bit_buf_size as usize
    }
}

/// Port of `approx_move_to_front`.
#[derive(Debug, Clone)]
pub struct ApproxMoveToFront {
    values: Vec<i32>,
    rover: usize,
}

impl ApproxMoveToFront {
    pub fn new(n: u32) -> Self {
        ApproxMoveToFront {
            values: vec![0; n as usize],
            rover: n as usize / 2,
        }
    }

    pub fn size(&self) -> usize {
        self.values.len()
    }

    pub fn get(&self, index: usize) -> i32 {
        self.values.get(index).copied().unwrap_or(0)
    }

    pub fn add(&mut self, new_value: i32) {
        if let Some(v) = self.values.get_mut(self.rover) {
            *v = new_value;
        }
        self.rover += 1;
        if self.rover == self.values.len() {
            self.rover = self.values.len() / 2;
        }
    }

    pub fn reset(&mut self) {
        let n = self.values.len();
        self.values = vec![0; n];
        self.rover = n / 2;
    }

    pub fn use_index(&mut self, index: usize) {
        if index != 0 && index < self.values.len() {
            self.values.swap(index / 2, index);
        }
    }
}
