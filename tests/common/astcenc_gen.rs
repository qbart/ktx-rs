//! Shared helpers of the astcenc compressor tests: FNV hash (NaNs canonicalized), LCG and
//! the generated test images of `scripts/golden/astcenc_*_golden.cpp`.

use ktx::astcenc::internal::*;
use ktx::astcenc::vecmath::Vfloat4;
use ktx::astcenc::*;

pub struct H(pub u64);

impl H {
    pub fn new() -> H {
        H(0xcbf29ce484222325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    pub fn f(&mut self, x: f32) {
        let u = if x.is_nan() {
            0x7fc00000u32
        } else {
            x.to_bits()
        };
        self.bytes(&u.to_le_bytes());
    }
    pub fn v4(&mut self, a: Vfloat4) {
        for l in a.m {
            self.f(l);
        }
    }
    pub fn u(&mut self, x: u32) {
        self.bytes(&x.to_le_bytes());
    }
}

pub struct Lcg(pub u32);

impl Lcg {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
    pub fn unit(&mut self) -> f32 {
        (self.next() & 0xffff) as f32 / 65535.0
    }
}

/// The test image of size index `si`, data type `dt` (U8 / F16 / F32) and content `kind`.
pub fn make_image(si: usize, (sx, sy, sz): (u32, u32, u32), dt: usize, kind: u32) -> AstcencImage {
    let (dx, dy) = (sx * 2 - 1, sy * 2 - 1);
    let dz = if sz == 1 { 1 } else { sz * 2 - 1 };
    let mut l = Lcg((si * 100 + dt * 10 + kind as usize) as u32);
    let mut p8 = vec![Vec::new(); dz as usize];
    let mut p16 = vec![Vec::new(); dz as usize];
    let mut p32 = vec![Vec::new(); dz as usize];
    for z in 0..dz as usize {
        for i in 0..(dx * dy * 4) {
            let mut v = l.unit();
            if kind == 1 {
                v = if i % 4 == 3 { 1.0 } else { 0.5 };
            }
            if kind == 2 && i % 4 != 3 {
                v = ((i / 4) % 7) as f32 / 6.0;
            }
            if kind == 3 {
                v = v * 4.0 - 1.0;
            }
            let mut b8 = (l.next() & 0xff) as u8;
            if kind == 1 {
                b8 = if i % 4 == 3 { 255 } else { 128 };
            }
            if kind == 2 && i % 4 != 3 {
                b8 = ((i / 4) * 37) as u8;
            }
            p8[z].push(b8);
            let hb = l.next();
            let mut half = if kind == 3 {
                (hb & 0xffff) as u16
            } else {
                (hb % 0x3c01) as u16
            };
            if kind == 1 {
                half = if i % 4 == 3 { 0x3c00 } else { 0x3800 };
            }
            p16[z].push(half);
            p32[z].push(v);
        }
    }
    AstcencImage {
        dim_x: dx,
        dim_y: dy,
        dim_z: dz,
        data: match dt {
            0 => AstcencImageData::U8(p8),
            1 => AstcencImageData::F16(p16),
            _ => AstcencImageData::F32(p32),
        },
    }
}

/// A compression context for a block size (quality medium, LDR).
pub fn context(sx: u32, sy: u32, sz: u32) -> Box<AstcencContext> {
    let c = astcenc_config_init(AstcencProfile::Ldr, sx, sy, sz, 60.0, 0);
    let Ok(c) = c else {
        panic!("config_init failed")
    };
    let Ok(ctx) = astcenc_context_alloc(&c, 1) else {
        panic!("context_alloc failed")
    };
    ctx
}

/// The empty block of a block size with the harness channel weights.
pub fn new_block(texel_count: u32, kind: u32) -> ImageBlock {
    ImageBlock {
        texel_count: texel_count as u8,
        channel_weight: Vfloat4::new(1.0, 0.5 + kind as f32 * 0.25, 2.0, 0.75),
        ..ImageBlock::default()
    }
}

pub const SIZES: [(u32, u32, u32); 9] = [
    (4, 4, 1),
    (5, 5, 1),
    (6, 6, 1),
    (8, 8, 1),
    (10, 5, 1),
    (12, 12, 1),
    (3, 3, 3),
    (4, 4, 4),
    (6, 6, 6),
];
