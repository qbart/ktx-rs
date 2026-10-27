// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! OpenGL loader (`glloader.c`) on a GL ES 3.0 context (Mesa llvmpipe through surfaceless EGL),
//! with the default command lookup of `ktxLoadOpenGL(NULL)`: the process symbols
//! (`dlopen(NULL)`), so libGLESv2 is loaded globally first as when an application links it.
//! Uploads are read back through a framebuffer (`glReadPixels`), as GL ES has no
//! `glGetTexImage`. A separate test binary: libktx keeps the loaded commands per process.
#![cfg(all(feature = "opengl", unix))]
#![allow(unsafe_code, clippy::panic)]

mod gl_common;

use std::ffi::c_void;

use ktx::glloader::{GLenum, GLuint, context_is_es, ktx_load_opengl};
use ktx::texture::{KtxTexture, TextureCreateInfo, TextureCreateStorage};
use ktx::{Texture2, VkFormat};

const GL_TEXTURE_2D: GLenum = 0x0DE1;
const GL_TEXTURE_CUBE_MAP: GLenum = 0x8513;
const GL_TEXTURE_CUBE_MAP_POSITIVE_X: GLenum = 0x8515;
const GL_FRAMEBUFFER: GLenum = 0x8D40;
const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
const GL_RGBA: GLenum = 0x1908;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;

type GenFramebuffers = unsafe extern "system" fn(i32, *mut GLuint);
type BindFramebuffer = unsafe extern "system" fn(GLenum, GLuint);
type FramebufferTexture2D = unsafe extern "system" fn(GLenum, GLenum, GLenum, GLuint, i32);
type CheckFramebufferStatus = unsafe extern "system" fn(GLenum) -> GLenum;
type ReadPixels = unsafe extern "system" fn(i32, i32, i32, i32, GLenum, GLenum, *mut c_void);
type PixelStorei = unsafe extern "system" fn(GLenum, i32);

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn rgba8(w: u32, h: u32, levels: u32, faces: u32) -> Texture2 {
    let ci = TextureCreateInfo {
        vk_format: VkFormat::R8G8B8A8_UNORM,
        base_width: w,
        base_height: h,
        base_depth: 1,
        num_dimensions: 2,
        num_levels: levels,
        num_layers: 1,
        num_faces: faces,
        ..Default::default()
    };
    let mut t = Texture2::create(&ci, TextureCreateStorage::AllocStorage)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let n = t.data_size;
    if let Some(d) = t.get_data_mut() {
        d.copy_from_slice(&pattern(n, w + faces));
    }
    t
}

#[test]
fn gl_upload_es3_process_symbols() {
    let gl = gl_common::Gl::new(true);
    // As if the application linked libGLESv2: its symbols are in the global namespace.
    let _gles = unsafe {
        libloading::os::unix::Library::open(
            Some("libGLESv2.so.2"),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_GLOBAL,
        )
    }
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(unsafe { ktx_load_opengl(None) }, Ok(()));

    let gen_framebuffers: GenFramebuffers = gl.func("glGenFramebuffers");
    let bind_framebuffer: BindFramebuffer = gl.func("glBindFramebuffer");
    let framebuffer_texture_2d: FramebufferTexture2D = gl.func("glFramebufferTexture2D");
    let check_status: CheckFramebufferStatus = gl.func("glCheckFramebufferStatus");
    let read_pixels: ReadPixels = gl.func("glReadPixels");
    let pixel_storei: PixelStorei = gl.func("glPixelStorei");
    let mut fbo = 0;
    unsafe {
        gen_framebuffers(1, &mut fbo);
        bind_framebuffer(GL_FRAMEBUFFER, fbo);
        pixel_storei(GL_PACK_ALIGNMENT, 1);
    }
    let read = |tex: GLuint, target: GLenum, level: i32, w: i32, h: i32| -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        unsafe {
            framebuffer_texture_2d(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, target, tex, level);
            assert_eq!(check_status(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            read_pixels(
                0,
                0,
                w,
                h,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                out.as_mut_ptr().cast(),
            );
        }
        out
    };

    let mut t = rgba8(8, 4, 3, 1);
    let (mut name, mut target) = (0, 0);
    let r = unsafe { t.gl_upload(Some(&mut name), &mut target, None) };
    assert_eq!((r, target), (Ok(()), GL_TEXTURE_2D));
    assert!(context_is_es());
    for level in 0..3 {
        let (w, h) = ((8 >> level).max(1), (4 >> level).max(1));
        let off = t.get_image_offset(level, 0, 0).unwrap_or(0);
        let want = &t.get_data().unwrap_or_default()[off..off + t.get_level_size(level)];
        assert!(
            read(name, GL_TEXTURE_2D, level as i32, w, h) == want,
            "level {level}"
        );
    }

    let mut cube = rgba8(4, 4, 1, 6);
    let mut cname = 0;
    let r = unsafe { cube.gl_upload(Some(&mut cname), &mut target, None) };
    assert_eq!((r, target), (Ok(()), GL_TEXTURE_CUBE_MAP));
    for face in 0..6 {
        let off = cube.get_image_offset(0, 0, face).unwrap_or(0);
        let want = &cube.get_data().unwrap_or_default()[off..off + 64];
        assert!(
            read(cname, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, 4, 4) == want,
            "face {face}"
        );
    }
}
