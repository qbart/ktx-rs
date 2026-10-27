// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `vk2gl.h` (`vkFormat2glInternalFormat`, `vkFormat2glFormat`, `vkFormat2glType`) vs. the C
//! header compiled by `scripts/golden/gen_vk2gl_golden.sh` for every VkFormat value.
#![allow(clippy::panic)]

use ktx::VkFormat;
use ktx::vk2gl::{vk_format2gl_format, vk_format2gl_internal_format, vk_format2gl_type};

#[test]
fn vk2gl_matches_c() {
    let golden = include_str!("resources/golden/vk2gl.txt");
    let mut n = 0;
    for line in golden.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let Ok(v) = f[0].parse::<u32>() else {
            panic!("bad line {line}")
        };
        let hex = |x: u32| {
            if x == 0 {
                "0".to_string()
            } else {
                format!("{x:#x}")
            }
        };
        let fmt = VkFormat(v);
        let got = format!(
            "{} {} {} {}",
            v,
            hex(vk_format2gl_internal_format(fmt)),
            hex(vk_format2gl_format(fmt)),
            hex(vk_format2gl_type(fmt))
        );
        assert_eq!(got, line);
        n += 1;
    }
    assert_eq!(n, 296);
}
