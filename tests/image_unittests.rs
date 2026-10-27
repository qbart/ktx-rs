// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of tests/unittests/image_unittests.cc (`NormaliseColorTest`, `ImagePackingTest`).

use ktx::cli::imageio::image::{Component, color_normalize, pack_f3x9_e1x5};

/// `test_color`: sets the first N components of `input`, normalizes and compares with `output`.
fn test_color<T: Component + std::fmt::Debug, const N: usize>(input: [u32; 4], output: [u32; 4]) {
    let mut color = [T::default(); N];
    for i in 0..N {
        color[i] = T::from_u32(input[i]);
    }
    for i in 0..N {
        assert_eq!(color[i], T::from_u32(input[i]));
    }
    color_normalize(&mut color);
    for i in 0..N {
        assert_eq!(
            color[i],
            T::from_u32(output[i]),
            "N={N} input {input:?} comp {i}"
        );
    }
}

const MIN_VAL: u32 = 0;
const MAX_VAL: [u32; 3] = [255, 65535, 4294967295];
const SOME_VAL: [u32; 3] = [31, 191, 178];

/// `test_by_channel<componentCount>`; `(uint8_t)` / `(uint16_t)` casts become masks.
fn test_by_channel<const N: usize>(
    min_res: &[u32; 3],
    max_res: &[u32; 3],
    some_res8: &[u32; 3],
    some_res16: &[u32; 3],
    some_res32: &[u32; 3],
) {
    let (m, x, s) = (MIN_VAL, MAX_VAL, SOME_VAL);
    let some = |mask: u32| [s[0] & mask, s[1] & mask, s[2] & mask, 0];
    let res = |r: &[u32; 3], mask: u32| [r[0] & mask, r[1] & mask, r[2] & mask, 0];
    let splat = |v: u32, a: u32| [v, v, v, a];

    test_color::<u8, N>([m; 4], splat(min_res[0] & 0xff, 0));
    test_color::<u8, N>(
        splat(x[0] & 0xff, x[0] & 0xff),
        splat(max_res[0] & 0xff, x[0] & 0xff),
    );
    test_color::<u8, N>(some(0xff), res(some_res8, 0xff));

    test_color::<u16, N>([m; 4], splat(min_res[1] & 0xffff, 0));
    test_color::<u16, N>(
        splat(x[1] & 0xffff, x[1] & 0xffff),
        splat(max_res[1] & 0xffff, x[1] & 0xffff),
    );
    test_color::<u16, N>(some(0xffff), res(some_res16, 0xffff));

    test_color::<u32, N>([m; 4], splat(min_res[2], 0));
    test_color::<u32, N>(splat(x[2], x[2]), splat(max_res[2], x[2]));
    test_color::<u32, N>(some(u32::MAX), res(some_res32, u32::MAX));
}

// Hand tested vectors and their normalised results in GeoGebra
// These are -0.577350 float in 8bit-UNORM, 16bit-UNORM and 32bit-UNORM values
const MIN_RES: [u32; 3] = [54, 13849, 907633408];
const MAX_RES: [u32; 3] = [201, 51686, 3387333888];

const SOME_RES8: [u32; 3] = [30, 192, 179];
const SOME_RES16: [u32; 3] = [13790, 13883, 13875];
const SOME_RES32: [u32; 3] = [907633408, 907633408, 907633408];

const MIN_RES2: [u32; 3] = [37, 9597, 628983424];
const MAX_RES2: [u32; 3] = [218, 55938, 3665984000];
const SOME_RES8_2: [u32; 3] = [21, 198, 128];
const SOME_RES16_2: [u32; 3] = [9541, 9654, 32768];
const SOME_RES32_2: [u32; 3] = [628983424, 628983424, 2147483648];

const MIN_RES1: [u32; 3] = [0, 0, 0];
const MAX_RES1: [u32; 3] = [255, 65535, 4294967295];
// Second and third values are unused because its single channel image result
const SOME_RES8_1: [u32; 3] = [255, 0, 0];
const SOME_RES16_1: [u32; 3] = [65535, 0, 0];
const SOME_RES32_1: [u32; 3] = [4294967295, 0, 0];

#[test]
fn normalise_color_test_multi_channel_test() {
    test_by_channel::<4>(&MIN_RES, &MAX_RES, &SOME_RES8, &SOME_RES16, &SOME_RES32);
    test_by_channel::<3>(&MIN_RES, &MAX_RES, &SOME_RES8, &SOME_RES16, &SOME_RES32);
    test_by_channel::<2>(
        &MIN_RES2,
        &MAX_RES2,
        &SOME_RES8_2,
        &SOME_RES16_2,
        &SOME_RES32_2,
    );
    test_by_channel::<1>(
        &MIN_RES1,
        &MAX_RES1,
        &SOME_RES8_1,
        &SOME_RES16_1,
        &SOME_RES32_1,
    );
}

#[test]
fn image_packing_test_e5b9g9r9_full_range() {
    let values = [0.0f32, 1.0, 32768.0, 49152.0, 65408.0, 65536.0];
    let expected = [
        0x00000000u32,
        0x80000100,
        0xf8000100,
        0xf8000180,
        0xf80001ff,
        0xf80001ff,
    ];
    for channel in 0..3 {
        for x in 0..6 {
            let mut pixel = [0.0f32; 3];
            pixel[channel] = values[x];
            let expected_word =
                (expected[x] & 0xf8000000) | ((expected[x] & 0x1ff) << (channel * 9));
            assert_eq!(
                pack_f3x9_e1x5(pixel),
                expected_word,
                "value {}, channel {channel}",
                values[x]
            );
        }
    }
}
