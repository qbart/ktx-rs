// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! OpenGL loader (`glloader.c`) on a desktop GL core profile context (Mesa llvmpipe through
//! surfaceless EGL), commands loaded with `eglGetProcAddress`. Not in the C suite (only the
//! interactive `glloadtests`): every upload is read back with `glGetTexImage` and compared.
//! One test function: libktx keeps the loaded commands and context capabilities per process.
#![cfg(all(feature = "opengl", not(target_family = "wasm")))]
#![allow(unsafe_code, clippy::panic)]

mod common;
mod gl_common;

use std::ffi::c_void;

use ktx::etcunpack::ktx_unpack_etc;
use ktx::glloader::{GLenum, GLuint, context_is_es, ktx_load_opengl};
use ktx::texture::{KtxTexture, TextureCreateInfo, TextureCreateStorage};
use ktx::{ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture1, Texture2, VkFormat};

const GL_TEXTURE_2D: GLenum = 0x0DE1;
const GL_TEXTURE_3D: GLenum = 0x806F;
const GL_TEXTURE_1D: GLenum = 0x0DE0;
const GL_TEXTURE_1D_ARRAY: GLenum = 0x8C18;
const GL_TEXTURE_2D_ARRAY: GLenum = 0x8C1A;
const GL_TEXTURE_CUBE_MAP: GLenum = 0x8513;
const GL_TEXTURE_CUBE_MAP_POSITIVE_X: GLenum = 0x8515;
const GL_TEXTURE_WIDTH: GLenum = 0x1000;
const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
const GL_RED: GLenum = 0x1903;
const GL_RGB: GLenum = 0x1907;
const GL_RGBA: GLenum = 0x1908;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;
const GL_INVALID_ENUM: GLenum = 0x0500;
const GL_R8: u32 = 0x8229;
const GL_ETC1_RGB8_OES: u32 = 0x8D64;

type GetTexImage = unsafe extern "system" fn(GLenum, i32, GLenum, GLenum, *mut c_void);
type PixelStorei = unsafe extern "system" fn(GLenum, i32);
type GetIntegerv = unsafe extern "system" fn(GLenum, *mut i32);
type GetTexLevelParameteriv = unsafe extern "system" fn(GLenum, i32, GLenum, *mut i32);
type BindTexture = unsafe extern "system" fn(GLenum, GLuint);
type GenTextures = unsafe extern "system" fn(i32, *mut GLuint);
type IsTexture = unsafe extern "system" fn(GLuint) -> u8;
type GetCompressedTexImage = unsafe extern "system" fn(GLenum, i32, *mut c_void);

struct Fns {
    get_tex_image: GetTexImage,
    get_integerv: GetIntegerv,
    get_tex_level_parameteriv: GetTexLevelParameteriv,
    bind_texture: BindTexture,
    gen_textures: GenTextures,
    is_texture: IsTexture,
}

impl Fns {
    fn read(&self, target: GLenum, level: i32, format: GLenum, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        unsafe {
            (self.get_tex_image)(
                target,
                level,
                format,
                GL_UNSIGNED_BYTE,
                out.as_mut_ptr().cast(),
            )
        };
        out
    }

    fn unpack_alignment(&self) -> i32 {
        let mut v = 0;
        unsafe { (self.get_integerv)(GL_UNPACK_ALIGNMENT, &mut v) };
        v
    }
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn ktx2_rgba8(w: u32, h: u32, d: u32, dims: u32, levels: u32, layers: u32, faces: u32) -> Texture2 {
    let ci = TextureCreateInfo {
        vk_format: VkFormat::R8G8B8A8_UNORM,
        base_width: w,
        base_height: h,
        base_depth: d,
        num_dimensions: dims,
        num_levels: levels,
        num_layers: layers,
        num_faces: faces,
        is_array: layers > 1,
        ..Default::default()
    };
    let mut t = Texture2::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let n = t.data_size;
    if let Some(data) = t.get_data_mut() {
        data.copy_from_slice(&pattern(n, w * 7 + levels));
    }
    t
}

/// Image `(level, layer 0, face)` of a texture as stored (all layers / slices of the level).
fn image(t: &dyn KtxTexture, level: u32, face: u32, len: usize) -> Vec<u8> {
    let off = t.get_image_offset(level, 0, face).unwrap_or(0);
    t.base().get_data().unwrap_or_default()[off..off + len].to_vec()
}

#[test]
fn gl_upload_desktop_core() {
    let gl = gl_common::Gl::new(false);
    let fns = Fns {
        get_tex_image: gl.func("glGetTexImage"),
        get_integerv: gl.func("glGetIntegerv"),
        get_tex_level_parameteriv: gl.func("glGetTexLevelParameteriv"),
        bind_texture: gl.func("glBindTexture"),
        gen_textures: gl.func("glGenTextures"),
        is_texture: gl.func("glIsTexture"),
    };
    let pixel_storei: PixelStorei = gl.func("glPixelStorei");
    unsafe { pixel_storei(GL_PACK_ALIGNMENT, 1) };

    let mut get = |name: &std::ffi::CStr| gl.proc_address(&name.to_string_lossy());
    assert_eq!(unsafe { ktx_load_opengl(Some(&mut get)) }, Ok(()));
    // Loads once: a second call keeps the commands.
    assert_eq!(unsafe { ktx_load_opengl(None) }, Ok(()));

    let alignment = fns.unpack_alignment();

    // 2D with mip levels.
    let mut t = ktx2_rgba8(16, 8, 1, 2, 4, 1, 1);
    let (mut name, mut target, mut glerror) = (0, 0, 0xFFFF);
    let r = unsafe { t.gl_upload(Some(&mut name), &mut target, Some(&mut glerror)) };
    assert_eq!((r, target, glerror), (Ok(()), GL_TEXTURE_2D, 0));
    assert!(!context_is_es());
    assert_ne!(name, 0);
    // KTX 2 uploads use unpack alignment 1 and restore the previous value.
    assert_eq!(fns.unpack_alignment(), alignment);
    for level in 0..4 {
        let len = t.get_level_size(level);
        let got = fns.read(GL_TEXTURE_2D, level as i32, GL_RGBA, len);
        assert!(got == image(&t, level, 0, len), "2D level {level}");
    }

    // Cube map: one image per face target.
    let mut cube = ktx2_rgba8(8, 8, 1, 2, 2, 1, 6);
    let r = unsafe { cube.gl_upload(None, &mut target, None) };
    assert_eq!((r, target), (Ok(()), GL_TEXTURE_CUBE_MAP));
    for level in 0..2 {
        for face in 0..6 {
            let len = cube.get_image_size(level);
            let got = fns.read(
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                level as i32,
                GL_RGBA,
                len,
            );
            assert!(
                got == image(&cube, level, face, len),
                "cube level {level} face {face}"
            );
        }
    }

    // 2D array and 3D: all layers / slices of a level at once.
    for (mut tex, want_target) in [
        (ktx2_rgba8(8, 4, 1, 2, 2, 3, 1), GL_TEXTURE_2D_ARRAY),
        (ktx2_rgba8(4, 4, 4, 3, 3, 1, 1), GL_TEXTURE_3D),
    ] {
        let r = unsafe { tex.gl_upload(None, &mut target, None) };
        assert_eq!((r, target), (Ok(()), want_target));
        for level in 0..tex.num_levels {
            let len = tex.get_level_size(level);
            let got = fns.read(target, level as i32, GL_RGBA, len);
            assert!(
                got == image(&tex, level, 0, len),
                "{target:#x} level {level}"
            );
        }
    }

    // 1D (glTexImage1D) and 1D array (glTexImage2D with the layers as height).
    for (mut tex, want_target) in [
        (ktx2_rgba8(16, 1, 1, 1, 2, 1, 1), GL_TEXTURE_1D),
        (ktx2_rgba8(16, 1, 1, 1, 2, 3, 1), GL_TEXTURE_1D_ARRAY),
    ] {
        let r = unsafe { tex.gl_upload(None, &mut target, None) };
        assert_eq!((r, target), (Ok(()), want_target));
        for level in 0..tex.num_levels {
            let len = tex.get_level_size(level);
            let got = fns.read(target, level as i32, GL_RGBA, len);
            assert!(
                got == image(&tex, level, 0, len),
                "{target:#x} level {level}"
            );
        }
    }

    // KTX 1 R8 5x3: rows padded to 4 bytes, uploaded with unpack alignment 4.
    let ci = TextureCreateInfo {
        gl_internalformat: GL_R8,
        base_width: 5,
        base_height: 3,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    let mut t1 = Texture1::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let tight = pattern(15, 5);
    assert_eq!(t1.set_image_from_memory(0, 0, 0, &tight), Ok(()));
    let r = unsafe { t1.gl_upload(None, &mut target, None) };
    assert_eq!((r, target), (Ok(()), GL_TEXTURE_2D));
    assert_eq!(fns.read(GL_TEXTURE_2D, 0, GL_RED, 15), tight);
    assert_eq!(fns.unpack_alignment(), alignment);

    // ETC1 is not a desktop GL format: glCompressedTexImage2D fails with INVALID_ENUM and the
    // image is unpacked in software (_ktxUnpackETC) and uploaded as RGB8.
    let ci = TextureCreateInfo {
        gl_internalformat: GL_ETC1_RGB8_OES,
        base_width: 8,
        base_height: 8,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: 1,
        num_layers: 1,
        num_faces: 1,
        ..Default::default()
    };
    let mut etc = Texture1::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let blocks = pattern(32, 9);
    assert_eq!(etc.set_image_from_memory(0, 0, 0, &blocks), Ok(()));
    let r = unsafe { etc.gl_upload(None, &mut target, None) };
    assert_eq!(r, Ok(()));
    let want = ktx_unpack_etc(&blocks, GL_ETC1_RGB8_OES, 8, 8, 3, true)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(fns.read(GL_TEXTURE_2D, 0, GL_RGB, 192), want.image);

    // Compressed formats the context supports (ETC2 RGB8 is core in GL 4.3) are uploaded as is.
    let mut etc2 = {
        let ci = TextureCreateInfo {
            vk_format: VkFormat::ETC2_R8G8B8_UNORM_BLOCK,
            base_width: 8,
            base_height: 8,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            ..Default::default()
        };
        Texture2::create(&ci, TextureCreateStorage::AllocStorage)
            .unwrap_or_else(|e| panic!("{e:?}"))
    };
    let blocks2 = pattern(32, 11);
    if let Some(d) = etc2.get_data_mut() {
        d.copy_from_slice(&blocks2);
    }
    let r = unsafe { etc2.gl_upload(None, &mut target, None) };
    assert_eq!(r, Ok(()));
    let get_compressed: GetCompressedTexImage = gl.func("glGetCompressedTexImage");
    let mut got = vec![0u8; 32];
    unsafe { get_compressed(GL_TEXTURE_2D, 0, got.as_mut_ptr().cast()) };
    assert_eq!(got, blocks2);

    // Generated mip levels (glGenerateMipmap).
    let mut gen_tex = {
        let ci = TextureCreateInfo {
            vk_format: VkFormat::R8G8B8A8_UNORM,
            base_width: 16,
            base_height: 16,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            generate_mipmaps: true,
            ..Default::default()
        };
        Texture2::create(&ci, TextureCreateStorage::AllocStorage)
            .unwrap_or_else(|e| panic!("{e:?}"))
    };
    let r = unsafe { gen_tex.gl_upload(None, &mut target, None) };
    assert_eq!(r, Ok(()));
    let mut w = 0;
    unsafe { (fns.get_tex_level_parameteriv)(GL_TEXTURE_2D, 4, GL_TEXTURE_WIDTH, &mut w) };
    assert_eq!(w, 1);

    // A texture name supplied by the application is used.
    let mut user = 0;
    unsafe { (fns.gen_textures)(1, &mut user) };
    let mut named = user;
    let mut small = ktx2_rgba8(4, 4, 1, 2, 1, 1, 1);
    let r = unsafe { small.gl_upload(Some(&mut named), &mut target, None) };
    assert_eq!((r, named), (Ok(()), user));
    unsafe { (fns.bind_texture)(GL_TEXTURE_2D, user) };
    assert_eq!(
        fns.read(GL_TEXTURE_2D, 0, GL_RGBA, 64),
        image(&small, 0, 0, 64)
    );

    // A format the context rejects: GL error returned, generated name deleted.
    let mut pvrtc = {
        let ci = TextureCreateInfo {
            vk_format: VkFormat::PVRTC1_4BPP_UNORM_BLOCK_IMG,
            base_width: 8,
            base_height: 8,
            base_depth: 1,
            num_dimensions: 2,
            num_levels: 1,
            num_layers: 1,
            num_faces: 1,
            ..Default::default()
        };
        Texture2::create(&ci, TextureCreateStorage::AllocStorage)
            .unwrap_or_else(|e| panic!("{e:?}"))
    };
    let (mut name, mut glerror) = (0, 0);
    let r = unsafe { pvrtc.gl_upload(Some(&mut name), &mut target, Some(&mut glerror)) };
    assert_eq!(
        (r, glerror, name),
        (Err(ErrorCode::GlError), GL_INVALID_ENUM, 0)
    );

    // Basis Universal textures must be transcoded first.
    let data = std::fs::read(common::ktx2_path().join("color_grid_blze.ktx2")).unwrap_or_default();
    let mut basis = Texture2::create_from_memory(&data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        unsafe { basis.gl_upload(None, &mut target, None) },
        Err(ErrorCode::InvalidOperation)
    );
    assert_eq!(unsafe { (fns.is_texture)(user) }, 1);
}
