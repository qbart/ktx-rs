// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/deflate_utils.h (`OptionsDeflate`).

use super::command::*;
use super::options::{OptionSpec, Options, ParseResult};

/// Port of `OptionsDeflate`.
#[derive(Debug, Clone, Default)]
pub struct OptionsDeflate {
    /// The given options, each with a leading space (`KTXwriterScParams`).
    pub compress_options: String,
    pub zstd: Option<u32>,
    pub zlib: Option<u32>,
}

impl OptionsDeflate {
    pub const K_ZSTD: &'static str = "zstd";
    pub const K_ZLIB: &'static str = "zlib";

    /// `captureCompressOption<uint32_t>`.
    fn capture_compress_option(&mut self, args: &ParseResult, name: &str) -> u32 {
        let value = args.as_u32(name);
        self.compress_options += &format!(" --{name} {value}");
        value
    }
}

impl OptionGroup for OptionsDeflate {
    fn init(&mut self, opts: &mut Options) {
        opts.add(
            OptionSpec::value(
                Self::K_ZSTD,
                "Supercompress the data with Zstandard. Cannot be used with ETC1S / BasisLZ format. Level range is [1,22]. Lower levels give faster but worse compression. Values above 20 should be used with caution as they require more memory.",
                "<level>",
            )
            .u32(),
        );
        opts.add(
            OptionSpec::value(
                Self::K_ZLIB,
                "Supercompress the data with ZLIB. Cannot be used with ETC1S / BasisLZ format. Level range is [1,9]. Lower levels give faster but worse compression.",
                "<level>",
            )
            .u32(),
        );
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count(Self::K_ZSTD) > 0 {
            let zstd = self.capture_compress_option(args, Self::K_ZSTD);
            self.zstd = Some(zstd);
            if !(1..=22).contains(&zstd) {
                return Err(report.fatal_usage(&format!(
                    "Invalid zstd level: \"{zstd}\". Value must be between 1 and 22 inclusive."
                )));
            }
        }
        if args.count(Self::K_ZLIB) > 0 {
            let zlib = self.capture_compress_option(args, Self::K_ZLIB);
            self.zlib = Some(zlib);
            if !(1..=9).contains(&zlib) {
                return Err(report.fatal_usage(&format!(
                    "Invalid zlib level: \"{zlib}\". Value must be between 1 and 9 inclusive."
                )));
            }
        }
        if self.zstd.is_some() && self.zlib.is_some() {
            return Err(report.fatal_usage(
                "Conflicting options: zstd and zlib cannot be used at the same time.",
            ));
        }
        Ok(())
    }
}
