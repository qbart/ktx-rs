//! Port of the basisu encoder side canonical Huffman coding (`basisu_enc.h` / `.cpp`):
//! `histogram`, `canonical_huffman_*`, `huffman_encoding_table`, `bitwise_coder` and
//! `palette_index_reorderer`.

use super::bitreader::{
    HUFFMAN_BIG_REPEAT_CODE, HUFFMAN_BIG_REPEAT_EXTRA_BITS, HUFFMAN_BIG_REPEAT_SIZE_MIN,
    HUFFMAN_BIG_ZERO_RUN_CODE, HUFFMAN_BIG_ZERO_RUN_EXTRA_BITS, HUFFMAN_BIG_ZERO_RUN_SIZE_MIN,
    HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE, HUFFMAN_MAX_SYMS, HUFFMAN_MAX_SYMS_LOG2,
    HUFFMAN_SMALL_REPEAT_CODE, HUFFMAN_SMALL_REPEAT_EXTRA_BITS, HUFFMAN_SMALL_REPEAT_SIZE_MIN,
    HUFFMAN_SMALL_ZERO_RUN_CODE, HUFFMAN_SMALL_ZERO_RUN_EXTRA_BITS,
    HUFFMAN_SMALL_ZERO_RUN_SIZE_MIN, HUFFMAN_SORTED_CODELENGTH_CODES,
    HUFFMAN_TOTAL_CODELENGTH_CODES,
};

pub const HUFFMAN_MAX_SUPPORTED_CODE_SIZE: u32 = 16;
pub const HUFFMAN_SMALL_ZERO_RUN_SIZE_MAX: u32 = 10;
pub const HUFFMAN_BIG_ZERO_RUN_SIZE_MAX: u32 = 138;
pub const HUFFMAN_SMALL_REPEAT_SIZE_MAX: u32 = 6;
pub const HUFFMAN_BIG_REPEAT_SIZE_MAX: u32 = 134;

/// `histogram`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Histogram {
    hist: Vec<u32>,
}

impl Histogram {
    pub fn new(size: u32) -> Self {
        Histogram {
            hist: vec![0; size as usize],
        }
    }

    pub fn clear(&mut self) {
        self.hist = Vec::new();
    }

    pub fn init(&mut self, size: u32) {
        self.hist.clear();
        self.hist.resize(size as usize, 0);
    }

    pub fn size(&self) -> u32 {
        self.hist.len() as u32
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.hist
    }

    pub fn as_mut_slice(&mut self) -> &mut [u32] {
        &mut self.hist
    }

    /// `inc(index)`; out of range indices are ignored (C++ writes out of bounds).
    #[inline]
    pub fn inc(&mut self, index: u32) {
        if let Some(h) = self.hist.get_mut(index as usize) {
            *h = h.wrapping_add(1);
        }
    }

    pub fn get_total(&self) -> u64 {
        self.hist.iter().map(|&h| h as u64).sum()
    }

    pub fn get_entropy(&self) -> f64 {
        let total = self.get_total() as f64;
        if total == 0.0 {
            return 0.0;
        }
        let inv_total = 1.0f32 as f64 / total;
        let neg_inv_log2 = (-1.0f32 / 2.0f32.ln()) as f64;
        let mut e = 0.0f64;
        for &h in &self.hist {
            if h != 0 {
                e += (h as f64 * inv_total).ln() * neg_inv_log2 * h as f64;
            }
        }
        e
    }
}

/// `sym_freq`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymFreq {
    pub key: u32,
    pub sym_index: u16,
}

/// `canonical_huffman_radix_sort_syms`: returns true when the result is in `syms1`
/// (C++ returns the pointer of the buffer holding the sorted symbols).
pub fn canonical_huffman_radix_sort_syms(syms0: &mut [SymFreq], syms1: &mut [SymFreq]) -> bool {
    let num_syms = syms0.len();
    let mut hist = [0u32; 256 * 2];
    for s in syms0.iter() {
        let freq = s.key;
        hist[(freq & 0xFF) as usize] += 1;
        hist[256 + ((freq >> 8) & 0xFF) as usize] += 1;
    }
    let mut total_passes = 2usize;
    while total_passes > 1 && num_syms as u32 == hist[(total_passes - 1) * 256] {
        total_passes -= 1;
    }
    let mut in_second = false;
    for pass in 0..total_passes {
        let shift = pass * 8;
        let p_hist = &hist[pass << 8..(pass << 8) + 256];
        let mut offsets = [0u32; 256];
        let mut cur_ofs = 0u32;
        for i in 0..256 {
            offsets[i] = cur_ofs;
            cur_ofs += p_hist[i];
        }
        let (cur, new): (&[SymFreq], &mut [SymFreq]) = if in_second {
            (&*syms1, &mut *syms0)
        } else {
            (&*syms0, &mut *syms1)
        };
        for s in cur.iter() {
            let o = &mut offsets[((s.key >> shift) & 0xFF) as usize];
            new[*o as usize] = *s;
            *o += 1;
        }
        in_second = !in_second;
    }
    in_second
}

/// `canonical_huffman_calculate_minimum_redundancy` (Moffat and Katajainen, in place).
pub fn canonical_huffman_calculate_minimum_redundancy(a: &mut [SymFreq]) {
    let num_syms = a.len() as i32;
    if num_syms == 0 {
        return;
    }
    if num_syms == 1 {
        a[0].key = 1;
        return;
    }
    a[0].key += a[1].key;
    let (mut s, mut r) = (2i32, 0i32);
    let k = |a: &[SymFreq], i: i32| a[i as usize].key;
    let mut next = 1i32;
    while next < num_syms - 1 {
        if s >= num_syms || k(a, r) < k(a, s) {
            a[next as usize].key = k(a, r);
            a[r as usize].key = next as u32;
            r += 1;
        } else {
            a[next as usize].key = k(a, s);
            s += 1;
        }
        if s >= num_syms || (r < next && k(a, r) < k(a, s)) {
            a[next as usize].key = k(a, next).wrapping_add(k(a, r));
            a[r as usize].key = next as u32;
            r += 1;
        } else {
            a[next as usize].key = k(a, next).wrapping_add(k(a, s));
            s += 1;
        }
        next += 1;
    }
    a[(num_syms - 2) as usize].key = 0;
    next = num_syms - 3;
    while next >= 0 {
        let p = k(a, next) as usize;
        a[next as usize].key = 1 + a.get(p).map_or(0, |e| e.key);
        next -= 1;
    }
    let (mut num_avail, mut num_used, mut depth) = (1i32, 0i32, 0i32);
    r = num_syms - 2;
    next = num_syms - 1;
    while num_avail > 0 {
        while r >= 0 && k(a, r) as i32 == depth {
            num_used += 1;
            r -= 1;
        }
        while num_avail > num_used {
            a[next as usize].key = depth as u32;
            next -= 1;
            num_avail -= 1;
        }
        num_avail = 2 * num_used;
        num_used = 0;
        depth += 1;
    }
}

/// `canonical_huffman_enforce_max_code_size`.
pub fn canonical_huffman_enforce_max_code_size(
    num_codes: &mut [i32; HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE + 1],
    code_list_len: usize,
    max_code_size: usize,
) {
    if code_list_len <= 1 {
        return;
    }
    for i in max_code_size + 1..=HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE {
        num_codes[max_code_size] += num_codes[i];
    }
    let mut total = 0u32;
    for i in (1..=max_code_size).rev() {
        total = total.wrapping_add((num_codes[i] as u32) << (max_code_size - i));
    }
    while total != 1u32 << max_code_size {
        num_codes[max_code_size] -= 1;
        for i in (1..max_code_size).rev() {
            if num_codes[i] != 0 {
                num_codes[i] -= 1;
                num_codes[i + 1] += 2;
                break;
            }
        }
        total = total.wrapping_sub(1);
    }
}

/// `huffman_encoding_table`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuffmanEncodingTable {
    codes: Vec<u16>,
    code_sizes: Vec<u8>,
}

impl HuffmanEncodingTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.codes = Vec::new();
        self.code_sizes = Vec::new();
    }

    /// `init(const histogram &h, max_code_size)`.
    pub fn init(&mut self, h: &Histogram, max_code_size: u32) -> bool {
        self.init_u32(h.as_slice(), max_code_size)
    }

    /// `init(num_syms, const uint16_t *pFreq, max_code_size)`.
    pub fn init_u16(&mut self, freq: &[u16], max_code_size: u32) -> bool {
        let num_syms = freq.len();
        if max_code_size > HUFFMAN_MAX_SUPPORTED_CODE_SIZE {
            return false;
        }
        if num_syms == 0 || num_syms > HUFFMAN_MAX_SYMS as usize {
            return false;
        }
        let total_used_syms = freq.iter().filter(|&&f| f != 0).count();
        if total_used_syms == 0 {
            return false;
        }
        let mut sym_freq0 = Vec::with_capacity(total_used_syms);
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                sym_freq0.push(SymFreq {
                    key: f as u32,
                    sym_index: i as u16,
                });
            }
        }
        let mut sym_freq1 = vec![SymFreq::default(); total_used_syms];
        let sorted = if canonical_huffman_radix_sort_syms(&mut sym_freq0, &mut sym_freq1) {
            &mut sym_freq1
        } else {
            &mut sym_freq0
        };
        canonical_huffman_calculate_minimum_redundancy(sorted);

        let mut num_codes = [0i32; HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE + 1];
        for s in sorted.iter() {
            if s.key as usize > HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE {
                return false;
            }
            num_codes[s.key as usize] += 1;
        }
        let max_code_size = max_code_size as usize;
        canonical_huffman_enforce_max_code_size(&mut num_codes, total_used_syms, max_code_size);

        self.code_sizes.clear();
        self.code_sizes.resize(num_syms, 0);
        self.codes.clear();
        self.codes.resize(num_syms, 0);

        let mut j = total_used_syms;
        for (i, &n) in num_codes.iter().enumerate().take(max_code_size + 1).skip(1) {
            let mut l = n;
            while l > 0 {
                if j == 0 {
                    return false;
                }
                j -= 1;
                self.code_sizes[sorted[j].sym_index as usize] = i as u8;
                l -= 1;
            }
        }

        let mut next_code = [0u32; HUFFMAN_MAX_SUPPORTED_INTERNAL_CODE_SIZE + 1];
        let mut jc = 0u32;
        for i in 2..=max_code_size {
            jc = jc.wrapping_add(num_codes[i - 1] as u32) << 1;
            next_code[i] = jc;
        }
        for i in 0..num_syms {
            let code_size = self.code_sizes[i] as usize;
            if code_size == 0 {
                continue;
            }
            let mut code = next_code[code_size];
            next_code[code_size] = code.wrapping_add(1);
            let mut rev_code = 0u32;
            for _ in 0..code_size {
                rev_code = (rev_code << 1) | (code & 1);
                code >>= 1;
            }
            self.codes[i] = rev_code as u16;
        }
        true
    }

    /// `init(num_syms, const uint32_t *pSym_freq, max_code_size)`.
    pub fn init_u32(&mut self, sym_freq: &[u32], max_code_size: u32) -> bool {
        let num_syms = sym_freq.len();
        if num_syms == 0 || num_syms > HUFFMAN_MAX_SYMS as usize {
            return false;
        }
        let max_freq = sym_freq.iter().copied().max().unwrap_or(0);
        let freq: Vec<u16> = if max_freq < u16::MAX as u32 {
            sym_freq.iter().map(|&f| f as u16).collect()
        } else {
            sym_freq
                .iter()
                .map(|&f| {
                    if f == 0 {
                        0
                    } else {
                        let s = (f as u64 * 65534 + (max_freq >> 1) as u64) / max_freq as u64;
                        (s as u32).clamp(1, 65534) as u16
                    }
                })
                .collect()
        };
        self.init_u16(&freq, max_code_size)
    }

    pub fn get_codes(&self) -> &[u16] {
        &self.codes
    }

    pub fn get_code_sizes(&self) -> &[u8] {
        &self.code_sizes
    }

    pub fn get_total_used_codes(&self) -> u32 {
        self.code_sizes
            .iter()
            .rposition(|&c| c != 0)
            .map_or(0, |i| i as u32 + 1)
    }
}

/// `bitwise_coder`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitwiseCoder {
    bytes: Vec<u8>,
    bit_buffer: u32,
    bit_buffer_size: u32,
    total_bits: u64,
}

impl BitwiseCoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.bytes = Vec::new();
        self.bit_buffer = 0;
        self.bit_buffer_size = 0;
        self.total_bits = 0;
    }

    pub fn restart(&mut self) {
        self.bytes.clear();
        self.bit_buffer = 0;
        self.bit_buffer_size = 0;
        self.total_bits = 0;
    }

    pub fn get_bytes(&self) -> &Vec<u8> {
        &self.bytes
    }

    pub fn get_bytes_mut(&mut self) -> &mut Vec<u8> {
        &mut self.bytes
    }

    pub fn reserve(&mut self, size: u32) {
        self.bytes.reserve(size as usize);
    }

    pub fn get_total_bits(&self) -> u64 {
        self.total_bits
    }

    pub fn get_total_bits_u32(&self) -> u32 {
        self.total_bits as u32
    }

    pub fn clear_total_bits(&mut self) {
        self.total_bits = 0;
    }

    pub fn init(&mut self, reserve_size: u32) {
        self.bytes.reserve(reserve_size as usize);
        self.bytes.clear();
        self.bit_buffer = 0;
        self.bit_buffer_size = 0;
        self.total_bits = 0;
    }

    pub fn flush(&mut self) -> u32 {
        if self.bit_buffer_size != 0 {
            self.total_bits += 8 - (self.bit_buffer_size & 7) as u64;
            self.bytes.push(self.bit_buffer as u8);
            self.bit_buffer = 0;
            self.bit_buffer_size = 0;
            return 8;
        }
        0
    }

    pub fn put_bits(&mut self, bits: u32, num_bits: u32) -> u32 {
        if num_bits == 0 {
            return 0;
        }
        self.total_bits += num_bits as u64;
        let mut v = ((bits as u64) << self.bit_buffer_size) | self.bit_buffer as u64;
        self.bit_buffer_size += num_bits;
        while self.bit_buffer_size >= 8 {
            self.bytes.push(v as u8);
            v >>= 8;
            self.bit_buffer_size -= 8;
        }
        self.bit_buffer = v as u8 as u32;
        num_bits
    }

    pub fn put_code(&mut self, sym: u32, tab: &HuffmanEncodingTable) -> u32 {
        let code = tab.codes.get(sym as usize).copied().unwrap_or(0) as u32;
        let code_size = tab.code_sizes.get(sym as usize).copied().unwrap_or(0) as u32;
        self.put_bits(code, code_size);
        code_size
    }

    pub fn put_truncated_binary(&mut self, v: u32, n: u32) -> u32 {
        let k = n.max(1).ilog2();
        let u = (1u32 << (k + 1)).wrapping_sub(n);
        if v < u {
            return self.put_bits(v, k);
        }
        let x = v + u;
        self.put_bits(x >> 1, k);
        self.put_bits(x & 1, 1);
        k + 1
    }

    pub fn put_rice(&mut self, v: u32, m: u32) -> u32 {
        let start_bits = self.total_bits;
        let mut q = v >> m;
        let r = v & ((1u32 << m) - 1);
        while q > 16 {
            self.put_bits(0xFFFF, 16);
            q -= 16;
        }
        self.put_bits((1u32 << q) - 1, q);
        self.put_bits(r << 1, m + 1);
        (self.total_bits - start_bits) as u32
    }

    pub fn put_vlc(&mut self, mut v: u32, chunk_bits: u32) -> u32 {
        let chunk_size = 1u32 << chunk_bits;
        let chunk_mask = chunk_size - 1;
        let mut total_bits = 0;
        loop {
            let next_v = v >> chunk_bits;
            total_bits += self.put_bits(
                (v & chunk_mask) | if next_v != 0 { chunk_size } else { 0 },
                chunk_bits + 1,
            );
            if next_v == 0 {
                break;
            }
            v = next_v;
        }
        total_bits
    }

    fn end_nonzero_run(syms: &mut Vec<u16>, run_size: &mut u32, len: u32) {
        if *run_size != 0 {
            if *run_size < HUFFMAN_SMALL_REPEAT_SIZE_MIN {
                for _ in 0..*run_size {
                    syms.push(len as u16);
                }
            } else if *run_size <= HUFFMAN_SMALL_REPEAT_SIZE_MAX {
                syms.push(
                    (HUFFMAN_SMALL_REPEAT_CODE | ((*run_size - HUFFMAN_SMALL_REPEAT_SIZE_MIN) << 6))
                        as u16,
                );
            } else {
                syms.push(
                    (HUFFMAN_BIG_REPEAT_CODE | ((*run_size - HUFFMAN_BIG_REPEAT_SIZE_MIN) << 6))
                        as u16,
                );
            }
        }
        *run_size = 0;
    }

    fn end_zero_run(syms: &mut Vec<u16>, run_size: &mut u32) {
        if *run_size != 0 {
            if *run_size < HUFFMAN_SMALL_ZERO_RUN_SIZE_MIN {
                for _ in 0..*run_size {
                    syms.push(0);
                }
            } else if *run_size <= HUFFMAN_SMALL_ZERO_RUN_SIZE_MAX {
                syms.push(
                    (HUFFMAN_SMALL_ZERO_RUN_CODE
                        | ((*run_size - HUFFMAN_SMALL_ZERO_RUN_SIZE_MIN) << 6))
                        as u16,
                );
            } else {
                syms.push(
                    (HUFFMAN_BIG_ZERO_RUN_CODE | ((*run_size - HUFFMAN_BIG_ZERO_RUN_SIZE_MIN) << 6))
                        as u16,
                );
            }
        }
        *run_size = 0;
    }

    pub fn emit_huffman_table(&mut self, tab: &HuffmanEncodingTable) -> u32 {
        let start_bits = self.total_bits;
        let code_sizes = tab.get_code_sizes();
        let total_used = tab.get_total_used_codes();
        self.put_bits(total_used, HUFFMAN_MAX_SYMS_LOG2);
        if total_used == 0 {
            return 0;
        }
        let mut syms: Vec<u16> = Vec::with_capacity(total_used as usize + 16);
        let (mut prev_code_len, mut zero_run_size, mut nonzero_run_size) = (u32::MAX, 0u32, 0u32);
        for i in 0..=total_used {
            let code_len = if i == total_used {
                0xFF
            } else {
                code_sizes[i as usize] as u32
            };
            if code_len != 0 {
                Self::end_zero_run(&mut syms, &mut zero_run_size);
                if code_len != prev_code_len {
                    Self::end_nonzero_run(&mut syms, &mut nonzero_run_size, prev_code_len);
                    if code_len != 0xFF {
                        syms.push(code_len as u16);
                    }
                } else {
                    nonzero_run_size += 1;
                    if nonzero_run_size == HUFFMAN_BIG_REPEAT_SIZE_MAX {
                        Self::end_nonzero_run(&mut syms, &mut nonzero_run_size, prev_code_len);
                    }
                }
            } else {
                Self::end_nonzero_run(&mut syms, &mut nonzero_run_size, prev_code_len);
                zero_run_size += 1;
                if zero_run_size == HUFFMAN_BIG_ZERO_RUN_SIZE_MAX {
                    Self::end_zero_run(&mut syms, &mut zero_run_size);
                }
            }
            prev_code_len = code_len;
        }

        let mut h = Histogram::new(HUFFMAN_TOTAL_CODELENGTH_CODES as u32);
        for &s in &syms {
            h.inc((s & 63) as u32);
        }
        let mut ct = HuffmanEncodingTable::new();
        if !ct.init(&h, 7) {
            return 0;
        }
        let mut total_codelength_codes = HUFFMAN_TOTAL_CODELENGTH_CODES;
        while total_codelength_codes > 0 {
            if ct.code_sizes[HUFFMAN_SORTED_CODELENGTH_CODES[total_codelength_codes - 1] as usize]
                != 0
            {
                break;
            }
            total_codelength_codes -= 1;
        }
        self.put_bits(total_codelength_codes as u32, 5);
        for &c in &HUFFMAN_SORTED_CODELENGTH_CODES[..total_codelength_codes] {
            self.put_bits(ct.code_sizes[c as usize] as u32, 3);
        }
        for &s in &syms {
            let (l, e) = ((s & 63) as u32, (s >> 6) as u32);
            self.put_code(l, &ct);
            if l == HUFFMAN_SMALL_ZERO_RUN_CODE {
                self.put_bits(e, HUFFMAN_SMALL_ZERO_RUN_EXTRA_BITS);
            } else if l == HUFFMAN_BIG_ZERO_RUN_CODE {
                self.put_bits(e, HUFFMAN_BIG_ZERO_RUN_EXTRA_BITS);
            } else if l == HUFFMAN_SMALL_REPEAT_CODE {
                self.put_bits(e, HUFFMAN_SMALL_REPEAT_EXTRA_BITS);
            } else if l == HUFFMAN_BIG_REPEAT_CODE {
                self.put_bits(e, HUFFMAN_BIG_REPEAT_EXTRA_BITS);
            }
        }
        (self.total_bits - start_bits) as u32
    }

    pub fn append(&mut self, other: &BitwiseCoder) {
        for &b in &other.bytes {
            self.put_bits(b as u32, 8);
        }
        if other.bit_buffer_size != 0 {
            self.put_bits(other.bit_buffer, other.bit_buffer_size);
        }
    }
}

/// `palette_index_reorderer::pEntry_dist_func`: [0,1] distance of entry i to entry j.
pub type EntryDistFunc<'a> = &'a dyn Fn(u32, u32) -> f32;

#[inline]
fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

/// `palette_index_reorderer`.
#[derive(Debug, Clone, Default)]
pub struct PaletteIndexReorderer {
    hist: Vec<u32>,
    total_count_to_picked: Vec<u32>,
    entries_picked: Vec<u32>,
    entries_to_do: Vec<u32>,
    remap_table: Vec<u32>,
}

impl PaletteIndexReorderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Table remaps old to new symbol indices.
    pub fn get_remap_table(&self) -> &[u32] {
        &self.remap_table
    }

    #[inline]
    fn get_hist(&self, i: u32, j: u32, n: u32) -> u32 {
        let idx = if i > j { j * n + i } else { i * n + j };
        self.hist.get(idx as usize).copied().unwrap_or(0)
    }

    #[inline]
    fn inc_hist(&mut self, i: i32, j: i32, n: u32) {
        if i != j
            && i < j
            && i != -1
            && j != -1
            && let Some(h) = self.hist.get_mut((i as u32 * n + j as u32) as usize)
        {
            *h += 1;
        }
    }

    pub fn init(
        &mut self,
        indices: &[u32],
        num_syms: u32,
        dist_func: Option<EntryDistFunc>,
        dist_func_weight: f32,
    ) {
        self.clear();
        self.remap_table.resize(num_syms as usize, 0);
        self.entries_picked.reserve(num_syms as usize);
        self.total_count_to_picked.resize(num_syms as usize, 0);
        if indices.len() <= 1 {
            return;
        }
        self.prepare_hist(num_syms, indices);
        self.find_initial(num_syms);
        while !self.entries_to_do.is_empty() {
            let (best_entry, _best_count) = self.find_next_entry(dist_func, dist_func_weight);
            let entry_to_move = self.entries_to_do[best_entry];
            let side = self.pick_side(num_syms, entry_to_move, dist_func, dist_func_weight);
            if side <= 0.0 {
                self.entries_picked.push(entry_to_move);
            } else {
                self.entries_picked.insert(0, entry_to_move);
            }
            self.entries_to_do.remove(best_entry);
            for i in 0..self.entries_to_do.len() {
                let e = self.entries_to_do[i];
                let h = self.get_hist(e, entry_to_move, num_syms);
                self.total_count_to_picked[e as usize] += h;
            }
        }
        for i in 0..num_syms as usize {
            if let Some(&p) = self.entries_picked.get(i) {
                self.remap_table[p as usize] = i as u32;
            }
        }
    }

    fn prepare_hist(&mut self, num_syms: u32, indices: &[u32]) {
        self.hist.clear();
        self.hist.resize((num_syms * num_syms) as usize, 0);
        let n = indices.len();
        for i in 0..n {
            let idx = indices[i] as i32;
            let next = if i < n - 1 { indices[i + 1] as i32 } else { -1 };
            self.inc_hist(idx, next, num_syms);
            let prev = if i > 0 { indices[i - 1] as i32 } else { -1 };
            self.inc_hist(idx, prev, num_syms);
        }
    }

    fn find_initial(&mut self, num_syms: u32) {
        let (mut max_count, mut max_index) = (0u32, 0u32);
        for (i, &h) in self.hist.iter().enumerate() {
            if h > max_count {
                max_count = h;
                max_index = i as u32;
            }
        }
        let (a, b) = (max_index / num_syms, max_index % num_syms);
        self.entries_picked.push(a);
        self.entries_picked.push(b);
        for i in 0..num_syms {
            if i != b && i != a {
                self.entries_to_do.push(i);
            }
        }
        for i in 0..self.entries_to_do.len() {
            let e = self.entries_to_do[i];
            for j in 0..self.entries_picked.len() {
                let h = self.get_hist(e, self.entries_picked[j], num_syms);
                self.total_count_to_picked[e as usize] += h;
            }
        }
    }

    fn find_next_entry(
        &self,
        dist_func: Option<EntryDistFunc>,
        dist_func_weight: f32,
    ) -> (usize, f64) {
        let (mut best_entry, mut best_count) = (0usize, 0f64);
        let front = self.entries_picked.first().copied().unwrap_or(0);
        let back = self.entries_picked.last().copied().unwrap_or(0);
        for (i, &u) in self.entries_to_do.iter().enumerate() {
            let mut total_count = self.total_count_to_picked[u as usize] as f64;
            if let Some(f) = dist_func {
                let w = f(u, front).max(f(u, back));
                total_count = (total_count + 1.0)
                    * lerp(1.0 - dist_func_weight, 1.0 + dist_func_weight, w) as f64;
            }
            if total_count <= best_count {
                continue;
            }
            best_entry = i;
            best_count = total_count;
        }
        (best_entry, best_count)
    }

    fn pick_side(
        &self,
        num_syms: u32,
        entry_to_move: u32,
        dist_func: Option<EntryDistFunc>,
        dist_func_weight: f32,
    ) -> f32 {
        let mut which_side = 0f32;
        let (mut l_count, mut r_count) = (0i32, 0i32);
        let n = self.entries_picked.len() as i32;
        for (j, &p) in self.entries_picked.iter().enumerate() {
            let count = self.get_hist(entry_to_move, p, num_syms) as i32;
            let r = n + 1 - 2 * (j as i32 + 1);
            which_side += (r * count) as f32;
            if r >= 0 {
                l_count += r * count;
            } else {
                r_count += -r * count;
            }
        }
        if let Some(f) = dist_func {
            let front = self.entries_picked.first().copied().unwrap_or(0);
            let back = self.entries_picked.last().copied().unwrap_or(0);
            let w_left = lerp(
                1.0 - dist_func_weight,
                1.0 + dist_func_weight,
                f(entry_to_move, front),
            );
            let w_right = lerp(
                1.0 - dist_func_weight,
                1.0 + dist_func_weight,
                f(entry_to_move, back),
            );
            which_side = w_left * l_count as f32 - w_right * r_count as f32;
        }
        which_side
    }
}
