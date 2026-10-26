# ETC unpack (`lib/src/etcunpack.cxx`) -> `src/etcunpack.rs`

`ktx_unpack_etc` = `_ktxUnpackETC` (`LIBKTX_FEATURE_ETC_UNPACK`, used by the GL loader when
the context lacks ETC): same format table (ETC1, ETC2 RGB / sRGB, RGBA8 EAC / sRGB, punch-through
/ sRGB, R11 / RG11 signed and unsigned), `KTX_R16_FORMATS_*` / sRGB capability checks
(`KTX_UNSUPPORTED_TEXTURE_TYPE`), 4x4 padded decode then crop to the active area. Returns the
image with its GL format / internal format / type (`UnpackedEtc`); short input gives
`KTX_FILE_DATA_ERROR` (C reads past it).

The block decoder `external/etcdec/etcdec.cxx` is under the Ericsson SLA (source may only be
distributed unmodified), so it is not translated: individual / differential / T / H / planar,
punch-through (opaque bit; transparent texels RGBA 0) and EAC are written from the Khronos Data
Format Specification, keeping etcdec's output conventions: EAC alpha multiplier 0 gives the
base, 11-bit EAC extended to 16 bits as `(e << 5) + (e >> 6)` (unsigned) and
`±((|e| << 5) + (|e| >> 5))` (signed, base -128 read as -127, clamp ±1023), little-endian
16-bit channels, RG11 interleaved.

Tests: `tests/etcunpack.rs` vs etcdec (`scripts/golden/gen_etcunpack_golden.sh`): 528 random
images (11 formats x 6 sizes incl. cropping x 4 R16 x 2 sRGB capabilities) + 16 hand-built blocks
(H mode with equal colors, signed EAC base -128); byte exact. Every mode mutant tried is caught.
