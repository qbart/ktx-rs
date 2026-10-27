//! Compressor side of `astcenc_symbolic_physical.cpp` and `astcenc_integer_sequence.cpp`:
//! `symbolic_to_physical` and `encode_ise`.

#![allow(clippy::needless_range_loop)]

use super::internal::*;
use super::tables::{
    COLOR_UQUANT_TO_SCRAMBLED_PQUANT, INTEGER_OF_QUINTS, INTEGER_OF_TRITS, QUANT_AND_XFER_TABLES,
};
use ktx_macros::isa;

/// `btq_counts`: (bits, trits, quints) per quant level.
const BTQ_COUNTS: [(u32, bool, bool); 21] = [
    (1, false, false),
    (0, true, false),
    (2, false, false),
    (0, false, true),
    (1, true, false),
    (3, false, false),
    (1, false, true),
    (2, true, false),
    (4, false, false),
    (2, false, true),
    (3, true, false),
    (5, false, false),
    (3, false, true),
    (4, true, false),
    (6, false, false),
    (4, false, true),
    (5, true, false),
    (7, false, false),
    (5, false, true),
    (6, true, false),
    (8, false, false),
];

#[isa]
/// `write_bits` (bytes past the end of `ptr` are not written).
fn write_bits(value: u32, bitcount: u32, bitoffset: u32, ptr: &mut [u8]) {
    let mut mask: u32 = (1u32 << bitcount) - 1;
    let mut value = value & mask;
    let p = (bitoffset >> 3) as usize;
    let bitoffset = bitoffset & 7;
    value <<= bitoffset;
    mask <<= bitoffset;
    mask = !mask;
    if let Some(b) = ptr.get_mut(p) {
        *b &= mask as u8;
        *b |= value as u8;
    }
    if let Some(b) = ptr.get_mut(p + 1) {
        *b &= (mask >> 8) as u8;
        *b |= (value >> 8) as u8;
    }
}

#[isa]
/// `encode_ise`: packs `character_count` values of `input_data` at `bit_offset`.
pub fn encode_ise(
    quant_level: u32,
    character_count: u32,
    input_data: &[u8],
    output_data: &mut [u8],
    mut bit_offset: u32,
) {
    let (bits, trits, quints) = BTQ_COUNTS[quant_level as usize];
    let mask: u32 = (1u32 << bits) - 1;
    let n = character_count as usize;
    let inp = |i: usize| u32::from(input_data.get(i).copied().unwrap_or(0));

    if trits {
        let mut i = 0usize;
        let full_trit_blocks = n / 5;
        let mut put = |pack: u32, count: u32, bit_offset: &mut u32| {
            write_bits(pack & 0xFF, count, *bit_offset, output_data);
            *bit_offset += count;
        };
        for _ in 0..full_trit_blocks {
            let t = |k: usize| (inp(i + k) >> bits) as usize;
            let tv =
                u32::from(INTEGER_OF_TRITS[t(4) * 81 + t(3) * 27 + t(2) * 9 + t(1) * 3 + t(0)]);
            // Element 0 + T0 + T1
            put(
                (inp(i) & mask) | ((tv & 0x3) << bits),
                bits + 2,
                &mut bit_offset,
            );
            // Element 1 + T2 + T3
            put(
                (inp(i + 1) & mask) | (((tv >> 2) & 0x3) << bits),
                bits + 2,
                &mut bit_offset,
            );
            // Element 2 + T4
            put(
                (inp(i + 2) & mask) | (((tv >> 4) & 0x1) << bits),
                bits + 1,
                &mut bit_offset,
            );
            // Element 3 + T5 + T6
            put(
                (inp(i + 3) & mask) | (((tv >> 5) & 0x3) << bits),
                bits + 2,
                &mut bit_offset,
            );
            // Element 4 + T7
            put(
                (inp(i + 4) & mask) | (((tv >> 7) & 0x1) << bits),
                bits + 1,
                &mut bit_offset,
            );
            i += 5;
        }

        // Loop tail for a partial block
        if i != n {
            let t = |k: usize| {
                if i + k >= n {
                    0
                } else {
                    (inp(i + k) >> bits) as usize
                }
            };
            let tv = u32::from(INTEGER_OF_TRITS[t(3) * 27 + t(2) * 9 + t(1) * 3 + t(0)]);
            const TBITS: [u32; 4] = [2, 2, 1, 2];
            const TSHIFT: [u32; 4] = [0, 2, 4, 5];
            let mut j = 0;
            while i < n {
                let pack = (inp(i) & mask) | (((tv >> TSHIFT[j]) & ((1 << TBITS[j]) - 1)) << bits);
                put(pack, bits + TBITS[j], &mut bit_offset);
                i += 1;
                j += 1;
            }
        }
    } else if quints {
        let mut i = 0usize;
        let full_quint_blocks = n / 3;
        let mut put = |pack: u32, count: u32, bit_offset: &mut u32| {
            write_bits(pack & 0xFF, count, *bit_offset, output_data);
            *bit_offset += count;
        };
        for _ in 0..full_quint_blocks {
            let q = |k: usize| (inp(i + k) >> bits) as usize;
            let tv = u32::from(INTEGER_OF_QUINTS[(q(2) * 5 + q(1)) * 5 + q(0)]);
            // Element 0
            put(
                (inp(i) & mask) | ((tv & 0x7) << bits),
                bits + 3,
                &mut bit_offset,
            );
            // Element 1
            put(
                (inp(i + 1) & mask) | (((tv >> 3) & 0x3) << bits),
                bits + 2,
                &mut bit_offset,
            );
            // Element 2
            put(
                (inp(i + 2) & mask) | (((tv >> 5) & 0x3) << bits),
                bits + 2,
                &mut bit_offset,
            );
            i += 3;
        }

        // Loop tail for a partial block
        if i != n {
            let q1 = if i + 1 >= n {
                0
            } else {
                (inp(i + 1) >> bits) as usize
            };
            let q0 = (inp(i) >> bits) as usize;
            let tv = u32::from(INTEGER_OF_QUINTS[q1 * 5 + q0]);
            const TBITS: [u32; 2] = [3, 2];
            const TSHIFT: [u32; 2] = [0, 3];
            let mut j = 0;
            while i < n {
                let pack = (inp(i) & mask) | (((tv >> TSHIFT[j]) & ((1 << TBITS[j]) - 1)) << bits);
                put(pack, bits + TBITS[j], &mut bit_offset);
                i += 1;
                j += 1;
            }
        }
    } else {
        for i in 0..n {
            write_bits(inp(i), bits, bit_offset, output_data);
            bit_offset += bits;
        }
    }
}

#[isa]
/// `bitrev8`.
fn bitrev8(p: u8) -> u8 {
    p.reverse_bits()
}

#[isa]
/// `symbolic_to_physical`.
pub fn symbolic_to_physical(
    bsd: &BlockSizeDescriptor,
    scb: &SymbolicCompressedBlock,
    pcb: &mut [u8; 16],
) {
    debug_assert!(scb.block_type != SYM_BTYPE_ERROR);

    // Constant color blocks; there is currently no attempt to coalesce larger void-extents
    let const_bytes: Option<[u8; 8]> = match scb.block_type {
        SYM_BTYPE_CONST_U16 => Some([0xFC, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        SYM_BTYPE_CONST_F16 => Some([0xFC, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        _ => None,
    };
    if let Some(cbytes) = const_bytes {
        pcb[..8].copy_from_slice(&cbytes);
        for i in 0..BLOCK_MAX_COMPONENTS {
            pcb[2 * i + 8] = (scb.constant_color[i] & 0xFF) as u8;
            pcb[2 * i + 9] = ((scb.constant_color[i] >> 8) & 0xFF) as u8;
        }
        return;
    }

    let partition_count = u32::from(scb.partition_count);

    // Compress the weights. They are encoded as an ordinary integer-sequence, then bit-reversed
    let mut weightbuf = [0u8; 16];

    let bm = bsd.get_block_mode(scb.block_mode as usize);
    let di = bsd.get_decimation_info(bm.decimation_mode as usize);
    let weight_count = di.weight_count as usize;
    let weight_quant_method = bm.get_weight_quant_mode();
    let weight_quant_levels = get_quant_level(weight_quant_method) as f32;
    let is_dual_plane = bm.is_dual_plane;

    let qat = &QUANT_AND_XFER_TABLES[weight_quant_method as usize];

    let real_weight_count = if is_dual_plane {
        2 * weight_count
    } else {
        weight_count
    };

    let bits_for_weights =
        super::get_ise_sequence_bitcount(real_weight_count as u32, weight_quant_method as usize);

    let quantize = |uqw: u8| {
        let qw = (f32::from(uqw) / 64.0) * (weight_quant_levels - 1.0);
        let qwi = (qw + 0.5) as usize;
        qat.scramble_map[qwi.min(31)]
    };
    let mut weights = [0u8; 64];
    if is_dual_plane {
        for i in 0..weight_count {
            weights[2 * i] = quantize(scb.weights[i]);
            weights[2 * i + 1] = quantize(scb.weights[i + WEIGHTS_PLANE2_OFFSET]);
        }
    } else {
        for i in 0..weight_count {
            weights[i] = quantize(scb.weights[i]);
        }
    }

    encode_ise(
        weight_quant_method,
        real_weight_count as u32,
        &weights,
        &mut weightbuf,
        0,
    );

    for i in 0..16 {
        pcb[i] = bitrev8(weightbuf[15 - i]);
    }

    write_bits(u32::from(scb.block_mode), 11, 0, pcb);
    write_bits(partition_count - 1, 2, 11, pcb);

    let mut below_weights_pos = 128 - bits_for_weights as i32;

    // Encode partition index and color endpoint types for blocks with 2+ partitions
    if partition_count > 1 {
        let pidx = u32::from(scb.partition_index);
        write_bits(pidx, 6, 13, pcb);
        write_bits(pidx >> 6, PARTITION_INDEX_BITS - 6, 19, pcb);

        if scb.color_formats_matched != 0 {
            write_bits(
                u32::from(scb.color_formats[0]) << 2,
                6,
                13 + PARTITION_INDEX_BITS,
                pcb,
            );
        } else {
            // Check endpoint types for each partition to determine the lowest class present
            let mut low_class = 4u32;
            for i in 0..partition_count as usize {
                let class_of_format = u32::from(scb.color_formats[i] >> 2);
                low_class = low_class.min(class_of_format);
            }
            if low_class == 3 {
                low_class = 2;
            }

            let mut encoded_type = low_class + 1;
            let mut bitpos = 2;
            for i in 0..partition_count as usize {
                let classbit_of_format =
                    (u32::from(scb.color_formats[i] >> 2)).wrapping_sub(low_class);
                encoded_type |= classbit_of_format << bitpos;
                bitpos += 1;
            }

            for i in 0..partition_count as usize {
                let lowbits_of_format = u32::from(scb.color_formats[i] & 3);
                encoded_type |= lowbits_of_format << bitpos;
                bitpos += 2;
            }

            let encoded_type_lowpart = encoded_type & 0x3F;
            let encoded_type_highpart = encoded_type >> 6;
            let encoded_type_highpart_size = (3 * partition_count) - 4;
            let encoded_type_highpart_pos = 128 - bits_for_weights - encoded_type_highpart_size;
            write_bits(encoded_type_lowpart, 6, 13 + PARTITION_INDEX_BITS, pcb);
            write_bits(
                encoded_type_highpart,
                encoded_type_highpart_size,
                encoded_type_highpart_pos,
                pcb,
            );
            below_weights_pos -= encoded_type_highpart_size as i32;
        }
    } else {
        write_bits(u32::from(scb.color_formats[0]), 4, 13, pcb);
    }

    // In dual-plane mode, encode the color component of the second plane of weights
    if is_dual_plane {
        write_bits(
            scb.plane2_component as u32,
            2,
            (below_weights_pos - 2) as u32,
            pcb,
        );
    }

    // Encode the color components
    let mut values_to_encode = [0u8; 32];
    let mut valuecount_to_encode = 0usize;

    let pack_table = &COLOR_UQUANT_TO_SCRAMBLED_PQUANT[(scb.quant_mode - QUANT_6) as usize];
    for i in 0..partition_count as usize {
        let vals = 2 * (scb.color_formats[i] as usize >> 2) + 2;
        for j in 0..vals {
            values_to_encode[j + valuecount_to_encode] =
                pack_table[scb.color_values[i][j] as usize];
        }
        valuecount_to_encode += vals;
    }

    encode_ise(
        scb.get_color_quant_mode(),
        valuecount_to_encode as u32,
        &values_to_encode,
        pcb,
        if partition_count == 1 {
            17
        } else {
            19 + PARTITION_INDEX_BITS
        },
    );
}
