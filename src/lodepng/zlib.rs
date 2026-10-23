//! Port of the lodepng zlib encoder (external/lodepng/lodepng.cpp,
//! version 20250506): boundary package-merge Huffman code lengths, hash
//! chain LZ77 with lazy matching, stored / fixed / dynamic deflate blocks and
//! the zlib wrapper. Byte exact with the C encoder (`tests/lodepng.rs`).

const FIRST_LENGTH_CODE_INDEX: u32 = 257;
const NUM_DEFLATE_CODE_SYMBOLS: usize = 288;
const NUM_DISTANCE_SYMBOLS: usize = 32;
const NUM_CODE_LENGTH_CODES: usize = 19;

/// The base lengths represented by codes 257-285.
const LENGTHBASE: [u32; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// The extra bits used by codes 257-285 (added to base length).
const LENGTHEXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// The base backwards distances.
const DISTANCEBASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// The extra bits of backwards distances.
const DISTANCEEXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order of the "code length alphabet code lengths".
const CLCL_ORDER: [usize; NUM_CODE_LENGTH_CODES] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Port of `LodePNGCompressSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressSettings {
    /// The block type for LZ (0, 1 or 2).
    pub btype: u32,
    /// Whether or not to use LZ77.
    pub use_lz77: bool,
    /// Must be a power of two <= 32768.
    pub windowsize: u32,
    /// Minimum LZ77 length. 3 is normally best, 6 can be better for some PNGs.
    pub minmatch: u32,
    /// Stop searching if >= this length found. Set to 258 for best compression.
    pub nicematch: u32,
    /// Use lazy matching: better compression but a bit slower.
    pub lazymatching: bool,
}

/// `lodepng_compress_settings_init` / `lodepng_default_compress_settings`.
impl Default for CompressSettings {
    fn default() -> CompressSettings {
        CompressSettings {
            btype: 2,
            use_lz77: true,
            windowsize: DEFAULT_WINDOWSIZE,
            minmatch: 3,
            nicematch: 128,
            lazymatching: true,
        }
    }
}

/// A good tradeoff between speed and compression ratio.
const DEFAULT_WINDOWSIZE: u32 = 2048;

/// `LodePNGBitWriter`: LSB of value written first, LSB of bytes used first.
struct BitWriter<'a> {
    data: &'a mut Vec<u8>,
    bp: usize,
}

impl BitWriter<'_> {
    fn write_bit(&mut self, bit: u32) {
        if self.bp & 7 == 0 {
            self.data.push(0);
        }
        if let Some(last) = self.data.last_mut() {
            *last |= ((bit & 1) << (self.bp & 7)) as u8;
        }
        self.bp += 1;
    }

    /// `writeBits`.
    fn write_bits(&mut self, value: u32, nbits: u32) {
        for i in 0..nbits {
            self.write_bit(value >> i);
        }
    }

    /// `writeBitsReversed`: for Huffman symbols, MSB first.
    fn write_bits_reversed(&mut self, value: u32, nbits: u32) {
        for i in 0..nbits {
            self.write_bit(value >> (nbits - 1 - i));
        }
    }
}

/// The encoding part of `HuffmanTree`.
#[derive(Debug, Default)]
struct HuffmanTree {
    codes: Vec<u32>,
    lengths: Vec<u32>,
    maxbitlen: u32,
    numcodes: usize,
}

impl HuffmanTree {
    /// `HuffmanTree_makeFromLengths2` (the decoding table is not needed).
    fn make_from_lengths2(&mut self) {
        let maxbitlen = self.maxbitlen as usize;
        let mut blcount = vec![0u32; maxbitlen + 1];
        let mut nextcode = vec![0u32; maxbitlen + 1];
        self.codes = vec![0; self.numcodes];
        // step 1: count number of instances of each code length
        for &l in &self.lengths {
            blcount[l as usize] += 1;
        }
        // step 2: generate the nextcode values
        for bits in 1..=maxbitlen {
            nextcode[bits] = (nextcode[bits - 1].wrapping_add(blcount[bits - 1])) << 1;
        }
        // step 3: generate all the codes
        for n in 0..self.numcodes {
            let l = self.lengths[n] as usize;
            if l != 0 {
                self.codes[n] = nextcode[l];
                nextcode[l] = nextcode[l].wrapping_add(1);
                // remove superfluous bits from the code
                self.codes[n] &= (1u32 << l) - 1;
            }
        }
    }

    /// `HuffmanTree_makeFromLengths`.
    fn make_from_lengths(bitlen: &[u32], maxbitlen: u32) -> HuffmanTree {
        let mut tree = HuffmanTree {
            codes: Vec::new(),
            lengths: bitlen.to_vec(),
            maxbitlen,
            numcodes: bitlen.len(),
        };
        tree.make_from_lengths2();
        tree
    }

    /// `HuffmanTree_makeFromFrequencies`.
    fn make_from_frequencies(
        frequencies: &[u32],
        mincodes: usize,
        mut numcodes: usize,
        maxbitlen: u32,
    ) -> Result<HuffmanTree, u32> {
        // trim zeroes
        while frequencies[numcodes - 1] == 0 && numcodes > mincodes {
            numcodes -= 1;
        }
        let mut tree = HuffmanTree {
            codes: Vec::new(),
            lengths: vec![0; numcodes],
            maxbitlen,
            numcodes,
        };
        lodepng_huffman_code_lengths(&mut tree.lengths, &frequencies[..numcodes], maxbitlen)?;
        tree.make_from_lengths2();
        Ok(tree)
    }
}

/// Chain node for boundary package merge (`BPMNode`).
#[derive(Debug, Clone, Copy, Default)]
struct BpmNode {
    /// The sum of all weights in this chain.
    weight: i32,
    /// Index of this leaf node (called "count" in the paper).
    index: u32,
    /// The next nodes in this chain (None if last).
    tail: Option<usize>,
    in_use: bool,
}

/// Lists of chains (`BPMLists`), nodes addressed by index into `memory`.
struct BpmLists {
    memory: Vec<BpmNode>,
    numfree: usize,
    nextfree: usize,
    freelist: Vec<usize>,
    chains0: Vec<usize>,
    chains1: Vec<usize>,
}

impl BpmLists {
    /// `bpmnode_create`: a new chain node from the memory pool.
    fn create(&mut self, weight: i32, index: u32, tail: Option<usize>) -> usize {
        // memory full, so garbage collect
        if self.nextfree >= self.numfree {
            // mark only those that are in use
            for node in &mut self.memory {
                node.in_use = false;
            }
            for i in 0..self.chains0.len() {
                for start in [self.chains0[i], self.chains1[i]] {
                    let mut node = Some(start);
                    while let Some(n) = node {
                        self.memory[n].in_use = true;
                        node = self.memory[n].tail;
                    }
                }
            }
            // collect those that are free
            self.numfree = 0;
            for i in 0..self.memory.len() {
                if !self.memory[i].in_use {
                    self.freelist[self.numfree] = i;
                    self.numfree += 1;
                }
            }
            self.nextfree = 0;
        }
        let result = self.freelist[self.nextfree];
        self.nextfree += 1;
        let node = &mut self.memory[result];
        node.weight = weight;
        node.index = index;
        node.tail = tail;
        result
    }

    /// `boundaryPM`: numpresent is the amount of leaves, c the current chain.
    fn boundary_pm(&mut self, leaves: &[BpmNode], c: usize, num: i32) {
        let numpresent = leaves.len();
        let lastindex = self.memory[self.chains1[c]].index as usize;
        if c == 0 {
            if lastindex >= numpresent {
                return;
            }
            self.chains0[c] = self.chains1[c];
            self.chains1[c] = self.create(leaves[lastindex].weight, lastindex as u32 + 1, None);
        } else {
            // sum of the weights of the head nodes of the previous lookahead chains.
            let sum = self.memory[self.chains0[c - 1]]
                .weight
                .wrapping_add(self.memory[self.chains1[c - 1]].weight);
            self.chains0[c] = self.chains1[c];
            if lastindex < numpresent && sum > leaves[lastindex].weight {
                let tail = self.memory[self.chains1[c]].tail;
                self.chains1[c] = self.create(leaves[lastindex].weight, lastindex as u32 + 1, tail);
                return;
            }
            self.chains1[c] = self.create(sum, lastindex as u32, Some(self.chains1[c - 1]));
            // in the end we are only interested in the chain of the last list, so no
            // need to recurse if we're at the last one (this gives measurable speedup)
            if num + 1 < (2 * numpresent as i32 - 2) {
                self.boundary_pm(leaves, c - 1, num);
                self.boundary_pm(leaves, c - 1, num);
            }
        }
    }
}

/// Port of `lodepng_huffman_code_lengths`: length-limited code lengths for
/// `frequencies` with the boundary package-merge algorithm.
pub fn lodepng_huffman_code_lengths(
    lengths: &mut [u32],
    frequencies: &[u32],
    maxbitlen: u32,
) -> Result<(), u32> {
    let numcodes = frequencies.len();
    if numcodes == 0 {
        return Err(80); // a tree of 0 symbols is not supposed to be made
    }
    if (1usize << maxbitlen) < numcodes {
        return Err(80); // represent all symbols
    }
    // the symbols, only those with > 0 frequency
    let mut leaves: Vec<BpmNode> = frequencies
        .iter()
        .enumerate()
        .filter(|(_, f)| **f > 0)
        .map(|(i, &f)| BpmNode {
            weight: f as i32,
            index: i as u32,
            ..BpmNode::default()
        })
        .collect();
    let numpresent = leaves.len();
    lengths.fill(0);

    // ensure at least two present symbols. There should be at least one symbol
    // according to RFC 1951 section 3.2.7. Some decoders incorrectly require two.
    if numpresent == 0 {
        lengths[0] = 1;
        lengths[1] = 1;
    } else if numpresent == 1 {
        let index = leaves[0].index as usize;
        lengths[index] = 1;
        lengths[if index == 0 { 1 } else { 0 }] = 1;
    } else {
        // bpmnode_sort: stable mergesort by weight
        leaves.sort_by_key(|l| l.weight);

        let listsize = maxbitlen as usize;
        let memsize = 2 * listsize * (listsize + 1);
        let mut lists = BpmLists {
            memory: vec![BpmNode::default(); memsize],
            numfree: memsize,
            nextfree: 0,
            freelist: (0..memsize).collect(),
            chains0: vec![0; listsize],
            chains1: vec![1; listsize],
        };
        lists.create(leaves[0].weight, 1, None);
        lists.create(leaves[1].weight, 2, None);

        // each boundaryPM call adds one chain to the last list, and we need 2 * numpresent - 2 chains.
        for i in 2..(2 * numpresent - 2) {
            lists.boundary_pm(&leaves, listsize - 1, i as i32);
        }

        let mut node = Some(lists.chains1[listsize - 1]);
        while let Some(n) = node {
            for leaf in leaves.iter().take(lists.memory[n].index as usize) {
                lengths[leaf.index as usize] += 1;
            }
            node = lists.memory[n].tail;
        }
    }
    Ok(())
}

/// `searchCodeIndex`: the index of the largest value <= `value` in sorted `array`.
fn search_code_index(array: &[u32], value: u32) -> usize {
    let mut left: usize = 1;
    let mut right: usize = array.len() - 1;
    while left <= right {
        let mid = (left + right) >> 1;
        if array[mid] >= value {
            right = mid - 1;
        } else {
            left = mid + 1;
        }
    }
    if left >= array.len() || array[left] > value {
        left -= 1;
    }
    left
}

/// `addLengthDistance`: length code, extra length bits, distance code, extra distance bits.
fn add_length_distance(values: &mut Vec<u32>, length: u32, distance: u32) {
    let length_code = search_code_index(&LENGTHBASE, length);
    let extra_length = length - LENGTHBASE[length_code];
    let dist_code = search_code_index(&DISTANCEBASE, distance);
    let extra_distance = distance - DISTANCEBASE[dist_code];
    values.extend_from_slice(&[
        length_code as u32 + FIRST_LENGTH_CODE_INDEX,
        extra_length,
        dist_code as u32,
        extra_distance,
    ]);
}

const MAX_SUPPORTED_DEFLATE_LENGTH: usize = 258;
const HASH_NUM_VALUES: usize = 65536;
const HASH_BIT_MASK: u32 = 65535;

/// Port of `Hash`.
struct Hash {
    /// hash value to head circular pos - can be outdated if went around window
    head: Vec<i32>,
    /// circular pos to prev circular pos
    chain: Vec<u16>,
    /// circular pos to hash value
    val: Vec<i32>,
    /// similar to head, but for chainz
    headz: Vec<i32>,
    /// those with same amount of zeros
    chainz: Vec<u16>,
    /// length of zeros streak, used as a second hash chain
    zeros: Vec<u16>,
}

impl Hash {
    /// `hash_init`.
    fn new(windowsize: usize) -> Hash {
        Hash {
            head: vec![-1; HASH_NUM_VALUES],
            val: vec![-1; windowsize],
            // same value as index indicates uninitialized
            chain: (0..windowsize).map(|i| i as u16).collect(),
            zeros: vec![0; windowsize],
            headz: vec![-1; MAX_SUPPORTED_DEFLATE_LENGTH + 1],
            chainz: (0..windowsize).map(|i| i as u16).collect(),
        }
    }

    /// `updateHashChain`; wpos = pos & (windowsize - 1).
    fn update(&mut self, wpos: usize, hashval: u32, numzeros: u16) {
        self.val[wpos] = hashval as i32;
        if self.head[hashval as usize] != -1 {
            self.chain[wpos] = self.head[hashval as usize] as u16;
        }
        self.head[hashval as usize] = wpos as i32;

        self.zeros[wpos] = numzeros;
        if self.headz[numzeros as usize] != -1 {
            self.chainz[wpos] = self.headz[numzeros as usize] as u16;
        }
        self.headz[numzeros as usize] = wpos as i32;
    }
}

/// `getHash`.
fn get_hash(data: &[u8], size: usize, pos: usize) -> u32 {
    let mut result: u32 = 0;
    if pos + 2 < size {
        // A simple shift and xor hash is used.
        result ^= u32::from(data[pos]);
        result ^= u32::from(data[pos + 1]) << 4;
        result ^= u32::from(data[pos + 2]) << 8;
    } else {
        if pos >= size {
            return 0;
        }
        for i in 0..size - pos {
            result ^= u32::from(data[pos + i]) << (i * 8);
        }
    }
    result & HASH_BIT_MASK
}

/// `countZeros`.
fn count_zeros(data: &[u8], size: usize, pos: usize) -> u32 {
    let end = (pos + MAX_SUPPORTED_DEFLATE_LENGTH).min(size);
    data[pos..end].iter().take_while(|&&b| b == 0).count() as u32
}

/// `encodeLZ77`: literal bytes and length/distance pairs of `input[inpos..insize]`.
#[allow(clippy::too_many_arguments)]
fn encode_lz77(
    out: &mut Vec<u32>,
    hash: &mut Hash,
    input: &[u8],
    inpos: usize,
    insize: usize,
    windowsize: u32,
    minmatch: u32,
    mut nicematch: u32,
    lazymatching: bool,
) -> Result<(), u32> {
    // for large window lengths, assume the user wants no compression loss.
    // Otherwise, max hash chain length speedup.
    let maxchainlength = if windowsize >= 8192 {
        windowsize
    } else {
        windowsize / 8
    };
    let maxlazymatch = if windowsize >= 8192 {
        MAX_SUPPORTED_DEFLATE_LENGTH as u32
    } else {
        64
    };
    let usezeros = true;
    let mut numzeros: u32 = 0;
    let mut lazy = false;
    let (mut lazylength, mut lazyoffset) = (0u32, 0u32);

    if windowsize == 0 || windowsize > 32768 {
        return Err(60); // windowsize smaller/larger than allowed
    }
    if windowsize & (windowsize - 1) != 0 {
        return Err(90); // must be power of two
    }
    if nicematch > MAX_SUPPORTED_DEFLATE_LENGTH as u32 {
        nicematch = MAX_SUPPORTED_DEFLATE_LENGTH as u32;
    }
    let wmask = windowsize as usize - 1;

    // Updates numzeros and the hash chains for `pos`.
    let advance = |hash: &mut Hash, numzeros: &mut u32, pos: usize| -> u32 {
        let hashval = get_hash(input, insize, pos);
        if usezeros && hashval == 0 {
            if *numzeros == 0 {
                *numzeros = count_zeros(input, insize, pos);
            } else if pos + *numzeros as usize > insize || input[pos + *numzeros as usize - 1] != 0
            {
                *numzeros -= 1;
            }
        } else {
            *numzeros = 0;
        }
        hash.update(pos & wmask, hashval, *numzeros as u16);
        hashval
    };

    let mut pos = inpos;
    while pos < insize {
        let wpos = pos & wmask; // position for in 'circular' hash buffers
        let mut chainlength: u32 = 0;

        let hashval = advance(hash, &mut numzeros, pos);

        // the length and offset found for the current position
        let mut length: u32 = 0;
        let mut offset: u32 = 0;

        let mut hashpos = hash.chain[wpos] as usize;
        let lastptr = insize.min(pos + MAX_SUPPORTED_DEFLATE_LENGTH);

        // search for the longest string
        let mut prev_offset: u32 = 0;
        loop {
            let c = chainlength;
            chainlength += 1;
            if c >= maxchainlength {
                break;
            }
            let current_offset = if hashpos <= wpos {
                (wpos - hashpos) as u32
            } else {
                (wpos + windowsize as usize - hashpos) as u32
            };

            if current_offset < prev_offset {
                break; // stop when went completely around the circular buffer
            }
            prev_offset = current_offset;
            if current_offset > 0 {
                // test the next characters
                let mut foreptr = pos;
                let mut backptr = pos - current_offset as usize;

                // common case in PNGs is lots of zeros. Quickly skip over them as a speedup
                if numzeros >= 3 {
                    let skip = u32::from(hash.zeros[hashpos]).min(numzeros) as usize;
                    backptr += skip;
                    foreptr += skip;
                }

                while foreptr < lastptr && input[backptr] == input[foreptr] {
                    backptr += 1;
                    foreptr += 1;
                }
                let current_length = (foreptr - pos) as u32;

                if current_length > length {
                    length = current_length; // the longest length
                    offset = current_offset; // the offset that is related to this longest length
                    // jump out once a length of max length is found (speed gain).
                    if current_length >= nicematch {
                        break;
                    }
                }
            }

            if hashpos == hash.chain[hashpos] as usize {
                break;
            }

            if numzeros >= 3 && length > numzeros {
                hashpos = hash.chainz[hashpos] as usize;
                if u32::from(hash.zeros[hashpos]) != numzeros {
                    break;
                }
            } else {
                hashpos = hash.chain[hashpos] as usize;
                // outdated hash value, happens if particular value was not encountered in whole last window
                if hash.val[hashpos] != hashval as i32 {
                    break;
                }
            }
        }

        if lazymatching {
            if !lazy
                && length >= 3
                && length <= maxlazymatch
                && length < MAX_SUPPORTED_DEFLATE_LENGTH as u32
            {
                lazy = true;
                lazylength = length;
                lazyoffset = offset;
                pos += 1;
                continue; // try the next byte
            }
            if lazy {
                lazy = false;
                if pos == 0 {
                    return Err(81);
                }
                if length > lazylength + 1 {
                    // push the previous character as literal
                    out.push(u32::from(input[pos - 1]));
                } else {
                    length = lazylength;
                    offset = lazyoffset;
                    // the same hashchain update will be done, this ensures no wrong alteration
                    hash.head[hashval as usize] = -1;
                    hash.headz[numzeros as usize] = -1;
                    pos -= 1;
                }
            }
        }
        if length >= 3 && offset > windowsize {
            return Err(86); // too big (or overflown negative) offset
        }

        // encode it as length/distance pair or literal value
        if length < 3 || length < minmatch || (length == 3 && offset > 4096) {
            // only lengths of 3 or higher are supported as length/distance pair;
            // a length of only 3 with a long offset may be not worth it
            out.push(u32::from(input[pos]));
        } else {
            add_length_distance(out, length, offset);
            for _ in 1..length {
                pos += 1;
                advance(hash, &mut numzeros, pos);
            }
        }
        pos += 1;
    }
    Ok(())
}

/// `deflateNoCompression`: stored blocks.
fn deflate_no_compression(out: &mut Vec<u8>, data: &[u8]) {
    let datasize = data.len();
    let numdeflateblocks = datasize.div_ceil(65535);
    let mut datapos = 0;
    for i in 0..numdeflateblocks {
        let bfinal = u8::from(i == numdeflateblocks - 1);
        let len = (datasize - datapos).min(65535);
        let nlen = 65535 - len;
        out.extend_from_slice(&[
            bfinal,
            (len & 255) as u8,
            (len >> 8) as u8,
            (nlen & 255) as u8,
            (nlen >> 8) as u8,
        ]);
        out.extend_from_slice(&data[datapos..datapos + len]);
        datapos += len;
    }
}

/// `writeLZ77data`.
fn write_lz77_data(
    writer: &mut BitWriter,
    lz77_encoded: &[u32],
    tree_ll: &HuffmanTree,
    tree_d: &HuffmanTree,
) {
    let mut i = 0;
    while i < lz77_encoded.len() {
        let val = lz77_encoded[i] as usize;
        writer.write_bits_reversed(tree_ll.codes[val], tree_ll.lengths[val]);
        if val > 256 {
            // for a length code, 3 more things have to be added
            let length_index = val - FIRST_LENGTH_CODE_INDEX as usize;
            let n_length_extra_bits = LENGTHEXTRA[length_index];
            let length_extra_bits = lz77_encoded[i + 1];
            let distance_code = lz77_encoded[i + 2] as usize;
            let n_distance_extra_bits = DISTANCEEXTRA[distance_code];
            let distance_extra_bits = lz77_encoded[i + 3];
            i += 3;

            writer.write_bits(length_extra_bits, n_length_extra_bits);
            writer.write_bits_reversed(tree_d.codes[distance_code], tree_d.lengths[distance_code]);
            writer.write_bits(distance_extra_bits, n_distance_extra_bits);
        }
        i += 1;
    }
}

/// `deflateDynamic`: a block with dynamic Huffman trees.
fn deflate_dynamic(
    writer: &mut BitWriter,
    hash: &mut Hash,
    data: &[u8],
    datapos: usize,
    dataend: usize,
    settings: &CompressSettings,
    final_block: bool,
) -> Result<(), u32> {
    let mut frequencies_ll = [0u32; 286];
    let mut frequencies_d = [0u32; 30];
    let mut frequencies_cl = [0u32; NUM_CODE_LENGTH_CODES];

    let mut lz77_encoded: Vec<u32> = Vec::new();
    if settings.use_lz77 {
        encode_lz77(
            &mut lz77_encoded,
            hash,
            data,
            datapos,
            dataend,
            settings.windowsize,
            settings.minmatch,
            settings.nicematch,
            settings.lazymatching,
        )?;
    } else {
        // no LZ77, but still will be Huffman compressed
        lz77_encoded.extend(data[datapos..dataend].iter().map(|&b| u32::from(b)));
    }

    // Count the frequencies of lit, len and dist codes
    let mut i = 0;
    while i < lz77_encoded.len() {
        let symbol = lz77_encoded[i] as usize;
        frequencies_ll[symbol] += 1;
        if symbol > 256 {
            let dist = lz77_encoded[i + 2] as usize;
            frequencies_d[dist] += 1;
            i += 3;
        }
        i += 1;
    }
    frequencies_ll[256] = 1; // there will be exactly 1 end code, at the end of the block

    // Make both huffman trees, one for the lit and len codes, one for the dist codes
    let tree_ll = HuffmanTree::make_from_frequencies(&frequencies_ll, 257, 286, 15)?;
    // 2, not 1, is chosen for mincodes: some buggy PNG decoders require at least 2 symbols in the dist tree
    let tree_d = HuffmanTree::make_from_frequencies(&frequencies_d, 2, 30, 15)?;

    let numcodes_ll = tree_ll.numcodes.min(286);
    let numcodes_d = tree_d.numcodes.min(30);
    // store the code lengths of both generated trees in bitlen_lld
    let mut bitlen_lld: Vec<u32> = Vec::with_capacity(numcodes_ll + numcodes_d);
    bitlen_lld.extend_from_slice(&tree_ll.lengths[..numcodes_ll]);
    bitlen_lld.extend_from_slice(&tree_d.lengths[..numcodes_d]);
    let numcodes_lld = bitlen_lld.len();
    let mut bitlen_lld_e: Vec<u32> = Vec::with_capacity(numcodes_lld);

    // run-length compress bitlen_ldd into bitlen_lld_e by using repeat codes 16 (copy length 3-6 times),
    // 17 (3-10 zeroes), 18 (11-138 zeroes)
    let mut i = 0;
    while i < numcodes_lld {
        let mut j: usize = 0; // amount of repetitions
        while i + j + 1 < numcodes_lld && bitlen_lld[i + j + 1] == bitlen_lld[i] {
            j += 1;
        }
        if bitlen_lld[i] == 0 && j >= 2 {
            // repeat code for zeroes
            j += 1; // include the first zero
            if j <= 10 {
                // repeat code 17 supports max 10 zeroes
                bitlen_lld_e.extend_from_slice(&[17, j as u32 - 3]);
            } else {
                // repeat code 18 supports max 138 zeroes
                j = j.min(138);
                bitlen_lld_e.extend_from_slice(&[18, j as u32 - 11]);
            }
            i += j - 1;
        } else if j >= 3 {
            // repeat code for value other than zero
            let num = j / 6;
            let rest = j % 6;
            bitlen_lld_e.push(bitlen_lld[i]);
            for _ in 0..num {
                bitlen_lld_e.extend_from_slice(&[16, 6 - 3]);
            }
            if rest >= 3 {
                bitlen_lld_e.extend_from_slice(&[16, rest as u32 - 3]);
            } else {
                j -= rest;
            }
            i += j;
        } else {
            // too short to benefit from repeat code
            bitlen_lld_e.push(bitlen_lld[i]);
        }
        i += 1;
    }

    // generate tree_cl, the huffmantree of huffmantrees
    let mut i = 0;
    while i < bitlen_lld_e.len() {
        frequencies_cl[bitlen_lld_e[i] as usize] += 1;
        // after a repeat code come the bits that specify the number of repetitions,
        // those don't need to be in the frequencies_cl calculation
        if bitlen_lld_e[i] >= 16 {
            i += 1;
        }
        i += 1;
    }

    let tree_cl = HuffmanTree::make_from_frequencies(
        &frequencies_cl,
        NUM_CODE_LENGTH_CODES,
        NUM_CODE_LENGTH_CODES,
        7,
    )?;

    // compute amount of code-length-code-lengths to output
    let mut numcodes_cl = NUM_CODE_LENGTH_CODES;
    // trim zeros at the end (using CLCL_ORDER), but minimum size must be 4 (see HCLEN below)
    while numcodes_cl > 4 && tree_cl.lengths[CLCL_ORDER[numcodes_cl - 1]] == 0 {
        numcodes_cl -= 1;
    }

    // Write block type
    writer.write_bits(u32::from(final_block), 1);
    writer.write_bits(0, 1); // first bit of BTYPE "dynamic"
    writer.write_bits(1, 1); // second bit of BTYPE "dynamic"

    // write the HLIT, HDIST and HCLEN values
    writer.write_bits((numcodes_ll - 257) as u32, 5);
    writer.write_bits((numcodes_d - 1) as u32, 5);
    writer.write_bits((numcodes_cl - 4) as u32, 4);

    // write the code lengths of the code length alphabet ("bitlen_cl")
    for &o in CLCL_ORDER.iter().take(numcodes_cl) {
        writer.write_bits(tree_cl.lengths[o], 3);
    }

    // write the lengths of the lit/len AND the dist alphabet
    let mut i = 0;
    while i < bitlen_lld_e.len() {
        let v = bitlen_lld_e[i] as usize;
        writer.write_bits_reversed(tree_cl.codes[v], tree_cl.lengths[v]);
        // extra bits of repeat codes
        let extra = match v {
            16 => 2,
            17 => 3,
            18 => 7,
            _ => 0,
        };
        if extra != 0 {
            i += 1;
            writer.write_bits(bitlen_lld_e[i], extra);
        }
        i += 1;
    }

    // write the compressed data symbols
    write_lz77_data(writer, &lz77_encoded, &tree_ll, &tree_d);
    // error: the length of the end code 256 must be larger than 0
    if tree_ll.lengths[256] == 0 {
        return Err(64);
    }
    // write the end code
    writer.write_bits_reversed(tree_ll.codes[256], tree_ll.lengths[256]);
    Ok(())
}

/// `generateFixedLitLenTree`.
fn generate_fixed_lit_len_tree() -> HuffmanTree {
    let mut bitlen = [0u32; NUM_DEFLATE_CODE_SYMBOLS];
    // 288 possible codes: 0-255=literals, 256=endcode, 257-285=lengthcodes, 286-287=unused
    for (i, b) in bitlen.iter_mut().enumerate() {
        *b = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    HuffmanTree::make_from_lengths(&bitlen, 15)
}

/// `generateFixedDistanceTree`.
fn generate_fixed_distance_tree() -> HuffmanTree {
    // there are 32 distance codes, but 30-31 are unused
    HuffmanTree::make_from_lengths(&[5u32; NUM_DISTANCE_SYMBOLS], 15)
}

/// `deflateFixed`: a block with the fixed Huffman trees.
fn deflate_fixed(
    writer: &mut BitWriter,
    hash: &mut Hash,
    data: &[u8],
    datapos: usize,
    dataend: usize,
    settings: &CompressSettings,
    final_block: bool,
) -> Result<(), u32> {
    let tree_ll = generate_fixed_lit_len_tree();
    let tree_d = generate_fixed_distance_tree();

    writer.write_bits(u32::from(final_block), 1);
    writer.write_bits(1, 1); // first bit of BTYPE
    writer.write_bits(0, 1); // second bit of BTYPE

    if settings.use_lz77 {
        let mut lz77_encoded = Vec::new();
        encode_lz77(
            &mut lz77_encoded,
            hash,
            data,
            datapos,
            dataend,
            settings.windowsize,
            settings.minmatch,
            settings.nicematch,
            settings.lazymatching,
        )?;
        write_lz77_data(writer, &lz77_encoded, &tree_ll, &tree_d);
    } else {
        // no LZ77, but still will be Huffman compressed
        for &b in &data[datapos..dataend] {
            writer.write_bits_reversed(tree_ll.codes[b as usize], tree_ll.lengths[b as usize]);
        }
    }
    // add END code
    writer.write_bits_reversed(tree_ll.codes[256], tree_ll.lengths[256]);
    Ok(())
}

/// Port of `lodepng_deflate`: appends the deflate stream of `input` to `out`.
pub fn lodepng_deflate(
    out: &mut Vec<u8>,
    input: &[u8],
    settings: &CompressSettings,
) -> Result<(), u32> {
    let insize = input.len();
    let blocksize = match settings.btype {
        0 => {
            deflate_no_compression(out, input);
            return Ok(());
        }
        1 => insize,
        // on PNGs, deflate blocks of 65-262k seem to give most dense encoding
        2 => (insize / 8 + 8).clamp(65536, 262144),
        _ => return Err(61),
    };

    let numdeflateblocks = if blocksize == 0 {
        1
    } else {
        insize.div_ceil(blocksize).max(1)
    };

    // hash_init
    let ws = settings.windowsize;
    if ws == 0 || ws > 32768 {
        return Err(60);
    }
    let mut hash = Hash::new(ws as usize);
    let mut writer = BitWriter { data: out, bp: 0 };

    for i in 0..numdeflateblocks {
        let final_block = i == numdeflateblocks - 1;
        let start = i * blocksize;
        let end = (start + blocksize).min(insize);
        if settings.btype == 1 {
            deflate_fixed(
                &mut writer,
                &mut hash,
                input,
                start,
                end,
                settings,
                final_block,
            )?;
        } else {
            deflate_dynamic(
                &mut writer,
                &mut hash,
                input,
                start,
                end,
                settings,
                final_block,
            )?;
        }
    }
    Ok(())
}

/// `update_adler32` / `adler32`.
pub fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    // at least 5552 sums can be done before the sums overflow, saving a lot of module divisions
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

/// Port of `lodepng_zlib_compress`.
pub fn lodepng_zlib_compress(input: &[u8], settings: &CompressSettings) -> Result<Vec<u8>, u32> {
    let mut deflated = Vec::new();
    lodepng_deflate(&mut deflated, input, settings)?;
    // zlib data: 1 byte CMF (CM+CINFO), 1 byte FLG, deflate data, 4 byte ADLER32 checksum
    let cmf: u32 = 120; // 0b01111000: CM 8, CINFO 7. With CINFO 7, any window size up to 32768 can be used.
    let flevel: u32 = 0;
    let fdict: u32 = 0;
    let mut cmfflg = 256 * cmf + fdict * 32 + flevel * 64;
    let fcheck = 31 - cmfflg % 31;
    cmfflg += fcheck;

    let mut out = Vec::with_capacity(deflated.len() + 6);
    out.push((cmfflg >> 8) as u8);
    out.push((cmfflg & 255) as u8);
    out.extend_from_slice(&deflated);
    out.extend_from_slice(&adler32(input).to_be_bytes());
    Ok(out)
}

// ---------------------------------------------------------------------------
// Inflator (decompressor)

/// Port of `LodePNGDecompressSettings` (no custom zlib / inflate callbacks).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecompressSettings {
    /// If true, continue and don't give an error message if the Adler32 checksum is corrupted.
    pub ignore_adler32: bool,
    /// Ignore complement of len checksum in uncompressed blocks.
    pub ignore_nlen: bool,
    /// Maximum decompressed size, 0 for no limit.
    pub max_output_size: usize,
}

/// Port of `LodePNGBitReader`.
struct BitReader<'a> {
    data: &'a [u8],
    size: usize,
    bitsize: usize,
    bp: usize,
    buffer: u32,
}

impl<'a> BitReader<'a> {
    /// `LodePNGBitReader_init`.
    fn new(data: &'a [u8]) -> Result<BitReader<'a>, u32> {
        let size = data.len();
        let bitsize = size.checked_mul(8).ok_or(105u32)?;
        bitsize.checked_add(64).ok_or(105u32)?;
        Ok(BitReader {
            data,
            size,
            bitsize,
            bp: 0,
            buffer: 0,
        })
    }

    fn byte(&self, i: usize) -> u32 {
        u32::from(self.data.get(i).copied().unwrap_or(0))
    }

    /// `ensureBits9` / `17` / `25`: reads up to `n` (2, 3 or 4) bytes.
    fn ensure_bytes(&mut self, n: usize) {
        let start = self.bp >> 3;
        let mut buffer = 0u32;
        for k in 0..n {
            if start + k < self.size {
                buffer |= self.byte(start + k) << (8 * k);
            }
        }
        self.buffer = buffer >> (self.bp & 7);
    }

    fn ensure_bits9(&mut self) {
        self.ensure_bytes(2);
    }

    fn ensure_bits17(&mut self) {
        self.ensure_bytes(3);
    }

    fn ensure_bits25(&mut self) {
        self.ensure_bytes(4);
    }

    /// `ensureBits32`.
    fn ensure_bits32(&mut self) {
        let start = self.bp >> 3;
        if start + 4 < self.size {
            let shift = self.bp & 7;
            let word = self.byte(start)
                | (self.byte(start + 1) << 8)
                | (self.byte(start + 2) << 16)
                | (self.byte(start + 3) << 24);
            // (data[start + 4] << 24) << (8 - shift): 32-bit shifts drop the high bits
            let extra = ((self.byte(start + 4) << 24) as u64) << (8 - shift);
            self.buffer = (word >> shift) | (extra as u32);
        } else {
            self.ensure_bytes(4);
        }
    }

    fn peek_bits(&self, nbits: u32) -> u32 {
        self.buffer & ((1u32 << nbits) - 1)
    }

    fn advance_bits(&mut self, nbits: u32) {
        self.buffer = self.buffer.checked_shr(nbits).unwrap_or(0);
        self.bp += nbits as usize;
    }

    fn read_bits(&mut self, nbits: u32) -> u32 {
        let result = self.peek_bits(nbits);
        self.advance_bits(nbits);
        result
    }
}

/// Amount of bits for the first huffman table lookup.
const FIRSTBITS: u32 = 9;
/// A symbol value too big to represent any valid symbol.
const INVALIDSYMBOL: u32 = 65535;
const LAST_LENGTH_CODE_INDEX: u32 = 285;

/// `reverseBits`.
fn reverse_bits(bits: u32, num: u32) -> u32 {
    (0..num).fold(0, |r, i| r | (((bits >> (num - i - 1)) & 1) << i))
}

/// The decoding tables of a `HuffmanTree`.
#[derive(Debug, Default)]
struct DecodeTree {
    table_len: Vec<u8>,
    table_value: Vec<u16>,
}

impl DecodeTree {
    /// `HuffmanTree_makeFromLengths` followed by `HuffmanTree_makeTable`.
    fn from_lengths(bitlen: &[u32], maxbitlen: u32) -> Result<DecodeTree, u32> {
        let tree = HuffmanTree::make_from_lengths(bitlen, maxbitlen);
        let headsize = 1usize << FIRSTBITS;
        let mask = (1u32 << FIRSTBITS) - 1;

        // compute maxlens: max total bit length of symbols sharing prefix in the first table
        let mut maxlens = vec![0u32; headsize];
        for (&symbol, &l) in tree.codes.iter().zip(&tree.lengths) {
            if l <= FIRSTBITS {
                continue;
            }
            let index = reverse_bits(symbol >> (l - FIRSTBITS), FIRSTBITS) as usize;
            maxlens[index] = maxlens[index].max(l);
        }
        // compute total table size
        let mut size = headsize;
        for &l in &maxlens {
            if l > FIRSTBITS {
                size += 1usize << (l - FIRSTBITS);
            }
        }
        let mut t = DecodeTree {
            table_len: vec![16; size],
            table_value: vec![0; size],
        };

        // fill in the first table for long symbols: max prefix size and pointer to secondary tables
        let mut pointer = headsize;
        for (i, &l) in maxlens.iter().enumerate() {
            if l <= FIRSTBITS {
                continue;
            }
            t.table_len[i] = l as u8;
            t.table_value[i] = pointer as u16;
            pointer += 1usize << (l - FIRSTBITS);
        }

        // fill in the first table for short symbols, or secondary table for long symbols
        let mut numpresent = 0;
        for (i, (&symbol, &l)) in tree.codes.iter().zip(&tree.lengths).enumerate() {
            if l == 0 {
                continue;
            }
            let reverse = reverse_bits(symbol, l);
            numpresent += 1;
            if l <= FIRSTBITS {
                // short symbol, fully in first table, replicated num times if l < FIRSTBITS
                for j in 0..(1u32 << (FIRSTBITS - l)) {
                    let index = (reverse | (j << l)) as usize;
                    if t.table_len[index] != 16 {
                        return Err(55); // invalid tree: long symbol shares prefix with short symbol
                    }
                    t.table_len[index] = l as u8;
                    t.table_value[index] = i as u16;
                }
            } else {
                // long symbol, shares prefix with other long symbols in first lookup table
                let index = (reverse & mask) as usize;
                let maxlen = u32::from(t.table_len[index]);
                if maxlen < l {
                    return Err(55);
                }
                let tablelen = maxlen - FIRSTBITS;
                let start = u32::from(t.table_value[index]);
                let num = 1u32 << (tablelen - (l - FIRSTBITS));
                for j in 0..num {
                    let reverse2 = reverse >> FIRSTBITS;
                    let index2 = (start + (reverse2 | (j << (l - FIRSTBITS)))) as usize;
                    let (Some(len), Some(value)) =
                        (t.table_len.get_mut(index2), t.table_value.get_mut(index2))
                    else {
                        return Err(55);
                    };
                    *len = l as u8;
                    *value = i as u16;
                }
            }
        }

        if numpresent < 2 {
            // Exactly 1 or 0 symbols: fill unused entries with an invalid symbol
            for i in 0..size {
                if t.table_len[i] == 16 {
                    t.table_len[i] = if i < headsize {
                        1
                    } else {
                        (FIRSTBITS + 1) as u8
                    };
                    t.table_value[i] = INVALIDSYMBOL as u16;
                }
            }
        } else if t.table_len.contains(&16) {
            // oversubscribed huffman tree
            return Err(55);
        }
        Ok(t)
    }

    /// `huffmanDecodeSymbol`.
    fn decode_symbol(&self, reader: &mut BitReader) -> u32 {
        let code = reader.peek_bits(FIRSTBITS) as usize;
        let l = u32::from(self.table_len[code]);
        let value = u32::from(self.table_value[code]);
        if l <= FIRSTBITS {
            reader.advance_bits(l);
            value
        } else {
            reader.advance_bits(FIRSTBITS);
            let value = (value + reader.peek_bits(l - FIRSTBITS)) as usize;
            let len = self
                .table_len
                .get(value)
                .map_or(FIRSTBITS + 1, |&x| u32::from(x));
            reader.advance_bits(len - FIRSTBITS);
            self.table_value
                .get(value)
                .map_or(INVALIDSYMBOL, |&x| u32::from(x))
        }
    }
}

/// `generateFixedLitLenTree` and `generateFixedDistanceTree`.
fn fixed_trees() -> Result<(DecodeTree, DecodeTree), u32> {
    let mut bitlen = [0u32; NUM_DEFLATE_CODE_SYMBOLS];
    for (i, b) in bitlen.iter_mut().enumerate() {
        *b = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let ll = DecodeTree::from_lengths(&bitlen, 15)?;
    let d = DecodeTree::from_lengths(&[5; NUM_DISTANCE_SYMBOLS], 15)?;
    Ok((ll, d))
}

/// `getTreeInflateDynamic`.
fn dynamic_trees(reader: &mut BitReader) -> Result<(DecodeTree, DecodeTree), u32> {
    if reader.bitsize - reader.bp < 14 {
        return Err(49);
    }
    reader.ensure_bits17();
    let hlit = reader.read_bits(5) as usize + 257;
    let hdist = reader.read_bits(5) as usize + 1;
    let hclen = reader.read_bits(4) as usize + 4;

    // read the code length codes out of 3 * (amount of code length codes) bits
    if reader
        .bp
        .checked_add(hclen * 3)
        .is_none_or(|e| e > reader.bitsize)
    {
        return Err(50);
    }
    let mut bitlen_cl = [0u32; NUM_CODE_LENGTH_CODES];
    for &o in CLCL_ORDER.iter().take(hclen) {
        reader.ensure_bits9();
        bitlen_cl[o] = reader.read_bits(3);
    }
    let tree_cl = DecodeTree::from_lengths(&bitlen_cl, 7)?;

    let mut bitlen_ll = [0u32; NUM_DEFLATE_CODE_SYMBOLS];
    let mut bitlen_d = [0u32; NUM_DISTANCE_SYMBOLS];
    let set = |i: usize, v: u32, ll: &mut [u32], d: &mut [u32]| {
        if i < hlit {
            if let Some(x) = ll.get_mut(i) {
                *x = v;
            }
        } else if let Some(x) = d.get_mut(i - hlit) {
            *x = v;
        }
    };
    let get = |i: usize, ll: &[u32], d: &[u32]| {
        if i < hlit {
            ll.get(i).copied().unwrap_or(0)
        } else {
            d.get(i - hlit).copied().unwrap_or(0)
        }
    };
    let mut i = 0usize;
    while i < hlit + hdist {
        reader.ensure_bits25();
        let code = tree_cl.decode_symbol(reader);
        match code {
            0..=15 => {
                set(i, code, &mut bitlen_ll, &mut bitlen_d);
                i += 1;
            }
            16 => {
                // repeat previous
                if i == 0 {
                    return Err(54);
                }
                let replength = 3 + reader.read_bits(2);
                let value = get(i - 1, &bitlen_ll, &bitlen_d);
                for _ in 0..replength {
                    if i >= hlit + hdist {
                        return Err(13);
                    }
                    set(i, value, &mut bitlen_ll, &mut bitlen_d);
                    i += 1;
                }
            }
            17 | 18 => {
                // repeat "0" 3-10 or 11-138 times
                let replength = if code == 17 {
                    3 + reader.read_bits(3)
                } else {
                    11 + reader.read_bits(7)
                };
                for _ in 0..replength {
                    if i >= hlit + hdist {
                        return Err(if code == 17 { 14 } else { 15 });
                    }
                    set(i, 0, &mut bitlen_ll, &mut bitlen_d);
                    i += 1;
                }
            }
            _ => return Err(16),
        }
        if reader.bp > reader.bitsize {
            return Err(50);
        }
    }
    if bitlen_ll[256] == 0 {
        return Err(64); // the length of the end code 256 must be larger than 0
    }
    let ll = DecodeTree::from_lengths(&bitlen_ll, 15)?;
    let d = DecodeTree::from_lengths(&bitlen_d, 15)?;
    Ok((ll, d))
}

/// `inflateHuffmanBlock`.
fn inflate_huffman_block(
    out: &mut Vec<u8>,
    reader: &mut BitReader,
    btype: u32,
    max_output_size: usize,
) -> Result<(), u32> {
    let (tree_ll, tree_d) = if btype == 1 {
        fixed_trees()?
    } else {
        dynamic_trees(reader)?
    };
    loop {
        reader.ensure_bits32();
        let mut code_ll = tree_ll.decode_symbol(reader);
        if code_ll <= 255 {
            // slightly faster code path if multiple literals in a row
            out.push(code_ll as u8);
            code_ll = tree_ll.decode_symbol(reader);
        }
        if code_ll <= 255 {
            out.push(code_ll as u8);
        } else if (FIRST_LENGTH_CODE_INDEX..=LAST_LENGTH_CODE_INDEX).contains(&code_ll) {
            let index = (code_ll - FIRST_LENGTH_CODE_INDEX) as usize;
            let mut length = LENGTHBASE[index] as usize;
            let numextrabits_l = LENGTHEXTRA[index];
            if numextrabits_l != 0 {
                reader.ensure_bits25();
                length += reader.read_bits(numextrabits_l) as usize;
            }
            reader.ensure_bits32();
            let code_d = tree_d.decode_symbol(reader);
            if code_d > 29 {
                return Err(if code_d <= 31 { 18 } else { 16 });
            }
            let mut distance = DISTANCEBASE[code_d as usize] as usize;
            let numextrabits_d = DISTANCEEXTRA[code_d as usize];
            if numextrabits_d != 0 {
                distance += reader.read_bits(numextrabits_d) as usize;
            }
            let start = out.len();
            if distance > start {
                return Err(52); // too long backward distance
            }
            let backward = start - distance;
            for k in 0..length {
                let b = out[backward + k];
                out.push(b);
            }
        } else if code_ll == 256 {
            break; // end code
        } else {
            return Err(16);
        }
        if reader.bp > reader.bitsize {
            return Err(51);
        }
        if max_output_size != 0 && out.len() > max_output_size {
            return Err(109);
        }
    }
    // The C++ checks the bit pointer once more after the end code
    if reader.bp > reader.bitsize {
        return Err(51);
    }
    if max_output_size != 0 && out.len() > max_output_size {
        return Err(109);
    }
    Ok(())
}

/// `inflateNoCompression`.
fn inflate_no_compression(
    out: &mut Vec<u8>,
    reader: &mut BitReader,
    settings: &DecompressSettings,
) -> Result<(), u32> {
    let size = reader.size;
    // go to first boundary of byte
    let mut bytepos = (reader.bp + 7) >> 3;
    if bytepos + 4 >= size {
        return Err(52);
    }
    let len = reader.byte(bytepos) + (reader.byte(bytepos + 1) << 8);
    let nlen = reader.byte(bytepos + 2) + (reader.byte(bytepos + 3) << 8);
    bytepos += 4;
    if !settings.ignore_nlen && len + nlen != 65535 {
        return Err(21);
    }
    let len = len as usize;
    if bytepos + len > size {
        return Err(23);
    }
    out.extend_from_slice(&reader.data[bytepos..bytepos + len]);
    reader.bp = (bytepos + len) << 3;
    Ok(())
}

/// `lodepng_inflatev`: appends to `out`.
fn inflatev(out: &mut Vec<u8>, input: &[u8], settings: &DecompressSettings) -> Result<(), u32> {
    let mut reader = BitReader::new(input)?;
    loop {
        if reader.bitsize - reader.bp < 3 {
            return Err(52);
        }
        reader.ensure_bits9();
        let bfinal = reader.read_bits(1);
        let btype = reader.read_bits(2);
        match btype {
            3 => return Err(20),
            0 => inflate_no_compression(out, &mut reader, settings)?,
            _ => inflate_huffman_block(out, &mut reader, btype, settings.max_output_size)?,
        }
        if settings.max_output_size != 0 && out.len() > settings.max_output_size {
            return Err(109);
        }
        if bfinal != 0 {
            return Ok(());
        }
    }
}

/// Port of `lodepng_inflate`: on error returns the code and the partial output.
pub fn lodepng_inflate(
    input: &[u8],
    settings: &DecompressSettings,
) -> Result<Vec<u8>, (u32, Vec<u8>)> {
    let mut out = Vec::new();
    match inflatev(&mut out, input, settings) {
        Ok(()) => Ok(out),
        Err(e) => Err((e, out)),
    }
}

/// Port of `lodepng_zlib_decompressv`: appends to `out`.
pub fn zlib_decompressv(
    out: &mut Vec<u8>,
    input: &[u8],
    settings: &DecompressSettings,
) -> Result<(), u32> {
    if input.len() < 2 {
        return Err(53);
    }
    let (in0, in1) = (u32::from(input[0]), u32::from(input[1]));
    if (in0 * 256 + in1) % 31 != 0 {
        return Err(24);
    }
    let cm = in0 & 15;
    let cinfo = (in0 >> 4) & 15;
    let fdict = (in1 >> 5) & 1;
    if cm != 8 || cinfo > 7 {
        return Err(25);
    }
    if fdict != 0 {
        return Err(26);
    }
    inflatev(out, &input[2..], settings)?;
    if !settings.ignore_adler32 {
        // The C reads the last 4 bytes of the input (before the input for tiny inputs)
        let b = |k: usize| {
            input
                .len()
                .checked_sub(4 - k)
                .and_then(|i| input.get(i))
                .copied()
                .unwrap_or(0)
        };
        let adler = u32::from_be_bytes([b(0), b(1), b(2), b(3)]);
        if adler32(out) != adler {
            return Err(58);
        }
    }
    Ok(())
}

/// Port of `lodepng_zlib_decompress`.
pub fn lodepng_zlib_decompress(
    input: &[u8],
    settings: &DecompressSettings,
) -> Result<Vec<u8>, u32> {
    let mut out = Vec::new();
    zlib_decompressv(&mut out, input, settings)?;
    Ok(out)
}
