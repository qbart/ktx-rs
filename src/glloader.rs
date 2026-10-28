// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of `lib/src/glloader.c`, `lib/src/gl_funcs.c` and `gl_funclist.inl`: creates OpenGL
//! {,ES} texture objects from KTX textures (`ktxTexture_GLUpload`).
//!
//! Behind the `opengl` feature (upstream `LIBKTX_FEATURE_GL_UPLOAD`). Together with
//! `vkloader` this is the only module that may use `unsafe`: every GL command is called through
//! a function pointer looked up in the application's GL library, on the GL context current on
//! the calling thread. The upload functions are therefore `unsafe fn`.
//!
//! As in C, libktx never loads GL itself: [`ktx_load_opengl`] takes the application's
//! `GetProcAddress` (e.g. `eglGetProcAddress`) or, without one, searches the running process
//! (`dlopen(NULL)`; on Windows an already loaded `libGLESv2.dll`, `libGLES_CM.dll` or
//! `opengl32.dll` + `wglGetProcAddress`). The command table and the discovered context
//! capabilities are process globals, as in C.
//!
//! The upload itself is the GL independent `glupload` module, run here on the loaded commands
//! ([`NativeGl`]) and the process global capabilities.
//!
//! Deviations (no runtime exceptions): an optional command that is missing for a texture that
//! needs it returns `KTX_UNSUPPORTED_TEXTURE_TYPE` instead of the C assert; a null
//! `glGetString` result is treated as an empty string.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::{Mutex, MutexGuard};

use crate::error::{ErrorCode, KtxResult};
pub use crate::glupload::{GLboolean, GLenum, GLint, GLsizei, GLuint};
use crate::glupload::{GlCommands, GlContextCaps, GlOptional};
use crate::texture_any::AnyTexture;
use crate::texture1::Texture1;
use crate::texture2::Texture2;

type PfnGlBindTexture = unsafe extern "system" fn(GLenum, GLuint);
type PfnGlCompressedTexImage1D =
    unsafe extern "system" fn(GLenum, GLint, GLenum, GLsizei, GLint, GLsizei, *const c_void);
type PfnGlCompressedTexImage2D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLenum,
    GLsizei,
    GLsizei,
    GLint,
    GLsizei,
    *const c_void,
);
type PfnGlCompressedTexImage3D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLenum,
    GLsizei,
    GLsizei,
    GLsizei,
    GLint,
    GLsizei,
    *const c_void,
);
type PfnGlCompressedTexSubImage1D =
    unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLenum, GLsizei, *const c_void);
type PfnGlCompressedTexSubImage2D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLenum,
    GLsizei,
    *const c_void,
);
type PfnGlCompressedTexSubImage3D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLsizei,
    GLenum,
    GLsizei,
    *const c_void,
);
type PfnGlDeleteTextures = unsafe extern "system" fn(GLsizei, *const GLuint);
type PfnGlGenerateMipmap = unsafe extern "system" fn(GLenum);
type PfnGlGenTextures = unsafe extern "system" fn(GLsizei, *mut GLuint);
type PfnGlGetError = unsafe extern "system" fn() -> GLenum;
type PfnGlGetIntegerv = unsafe extern "system" fn(GLenum, *mut GLint);
type PfnGlGetString = unsafe extern "system" fn(GLenum) -> *const u8;
type PfnGlGetStringi = unsafe extern "system" fn(GLenum, GLuint) -> *const u8;
type PfnGlPixelStorei = unsafe extern "system" fn(GLenum, GLint);
type PfnGlTexImage1D =
    unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLint, GLenum, GLenum, *const c_void);
type PfnGlTexImage2D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLint,
    GLenum,
    GLenum,
    *const c_void,
);
type PfnGlTexImage3D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLsizei,
    GLint,
    GLenum,
    GLenum,
    *const c_void,
);
type PfnGlTexParameteri = unsafe extern "system" fn(GLenum, GLenum, GLint);
type PfnGlTexParameteriv = unsafe extern "system" fn(GLenum, GLenum, *const GLint);
type PfnGlTexStorage1D = unsafe extern "system" fn(GLenum, GLsizei, GLenum, GLsizei);
type PfnGlTexStorage2D = unsafe extern "system" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei);
type PfnGlTexStorage3D =
    unsafe extern "system" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei, GLsizei);
type PfnGlTexSubImage1D =
    unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLenum, GLenum, *const c_void);
type PfnGlTexSubImage2D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLenum,
    GLenum,
    *const c_void,
);
type PfnGlTexSubImage3D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLsizei,
    GLenum,
    GLenum,
    *const c_void,
);

/// Port of `struct glFuncPtrs` (`gl_funclist.inl`): the GL {,ES} commands used by libktx.
/// Some are loaded (and zeroed for ES 2) like in C without being called by the upload.
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct GlFuncs {
    gl_bind_texture: Option<PfnGlBindTexture>,
    gl_compressed_tex_image_1d: Option<PfnGlCompressedTexImage1D>,
    gl_compressed_tex_image_2d: Option<PfnGlCompressedTexImage2D>,
    gl_compressed_tex_image_3d: Option<PfnGlCompressedTexImage3D>,
    gl_compressed_tex_sub_image_1d: Option<PfnGlCompressedTexSubImage1D>,
    gl_compressed_tex_sub_image_2d: Option<PfnGlCompressedTexSubImage2D>,
    gl_compressed_tex_sub_image_3d: Option<PfnGlCompressedTexSubImage3D>,
    gl_delete_textures: Option<PfnGlDeleteTextures>,
    gl_generate_mipmap: Option<PfnGlGenerateMipmap>,
    gl_gen_textures: Option<PfnGlGenTextures>,
    gl_get_error: Option<PfnGlGetError>,
    gl_get_integerv: Option<PfnGlGetIntegerv>,
    gl_get_string: Option<PfnGlGetString>,
    gl_get_stringi: Option<PfnGlGetStringi>,
    gl_pixel_storei: Option<PfnGlPixelStorei>,
    gl_tex_image_1d: Option<PfnGlTexImage1D>,
    gl_tex_image_2d: Option<PfnGlTexImage2D>,
    gl_tex_image_3d: Option<PfnGlTexImage3D>,
    gl_tex_parameteri: Option<PfnGlTexParameteri>,
    gl_tex_parameteriv: Option<PfnGlTexParameteriv>,
    gl_tex_storage_1d: Option<PfnGlTexStorage1D>,
    gl_tex_storage_2d: Option<PfnGlTexStorage2D>,
    gl_tex_storage_3d: Option<PfnGlTexStorage3D>,
    gl_tex_sub_image_1d: Option<PfnGlTexSubImage1D>,
    gl_tex_sub_image_2d: Option<PfnGlTexSubImage2D>,
    gl_tex_sub_image_3d: Option<PfnGlTexSubImage3D>,
}

/// The required commands of [`GlFuncs`] ("Present in all GL versions").
#[derive(Clone, Copy)]
struct Required {
    bind_texture: PfnGlBindTexture,
    compressed_tex_image_2d: PfnGlCompressedTexImage2D,
    delete_textures: PfnGlDeleteTextures,
    gen_textures: PfnGlGenTextures,
    get_error: PfnGlGetError,
    get_integerv: PfnGlGetIntegerv,
    get_string: PfnGlGetString,
    pixel_storei: PfnGlPixelStorei,
    tex_image_2d: PfnGlTexImage2D,
    tex_parameteri: PfnGlTexParameteri,
}

/// The loaded commands. Only constructed by [`ktx_load_opengl`] and only used inside the
/// `unsafe` upload functions, whose contract makes the GL context current.
#[derive(Clone, Copy)]
struct NativeGl {
    gl: GlFuncs,
    req: Required,
}

/// The loaded commands plus the `discoverContextCapabilities` statics of `glloader.c`.
struct GlState {
    cmds: NativeGl,
    caps: GlContextCaps,
}

/// `gl` + `openGLLoaded` + the context capability statics.
static GL_STATE: Mutex<Option<GlState>> = Mutex::new(None);

fn gl_state() -> MutexGuard<'static, Option<GlState>> {
    GL_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The process (or GL dll) searched by the default `GetProcAddress`.
#[cfg(unix)]
fn get_opengl_module_handle() -> Option<(libloading::Library, Option<WglGetProcAddress>)> {
    // Using NULL returns a handle that can be used to search the process that loaded us and any
    // other libraries it has loaded. That's all we need to search as the app is responsible for
    // creating the GL context so it must be there.
    Some((libloading::os::unix::Library::this().into(), None))
}

type WglGetProcAddress = unsafe extern "system" fn(*const c_char) -> *const c_void;

/// `ktxFindOpenGL`: an OpenGL {,ES} dll the process has already loaded. The C version first
/// checks whether libktx is statically linked to GL, which does not apply to Rust.
#[cfg(windows)]
fn get_opengl_module_handle() -> Option<(libloading::Library, Option<WglGetProcAddress>)> {
    use libloading::os::windows::Library;
    // Emulators probably also have opengl32.lib loaded so check that last.
    for name in ["libGLESv2.dll", "libGLES_CM.dll"] {
        if let Ok(m) = Library::open_already_loaded(name) {
            return Some((m.into(), None));
        }
    }
    let module: libloading::Library = Library::open_already_loaded("opengl32.dll").ok()?.into();
    // Need wglGetProcAddr for non-OpenGL-2 functions.
    // SAFETY: wglGetProcAddress has this signature.
    let wgl = unsafe { module.get::<WglGetProcAddress>(b"wglGetProcAddress\0") }
        .ok()
        .map(|s| *s)?;
    Some((module, Some(wgl)))
}

/// Port of `ktxLoadOpenGLLibrary`: finds the GL library the application loaded.
fn ktx_load_opengl_library() -> KtxResult<(libloading::Library, Option<WglGetProcAddress>)> {
    match get_opengl_module_handle() {
        Some(m) => Ok(m),
        None => {
            eprintln!("OpenGL lib not linked or loaded by application.");
            // Normal use is for this constructor to be called by an application that has
            // completed OpenGL initialization.
            Err(ErrorCode::LibraryNotLinked) // So release version doesn't crash.
        }
    }
}

/// `PFNGLGETPROCADDRESS`: returns the address of a GL command (null if unavailable).
pub type GlGetProcAddress<'a> = &'a mut dyn FnMut(&CStr) -> *const c_void;

/// Port of `ktxLoadOpenGL`: loads the GL commands used by the upload functions through
/// `pfn_gl_get_proc_address`, or by searching the process when `None`. Loads only once.
///
/// # Safety
/// The returned addresses must be the named GL commands of the context the application will
/// make current when uploading.
pub unsafe fn ktx_load_opengl(pfn_gl_get_proc_address: Option<GlGetProcAddress>) -> KtxResult<()> {
    let mut state = gl_state();
    if state.is_some() {
        return Ok(());
    }

    let module = match pfn_gl_get_proc_address {
        Some(_) => None,
        None => Some(ktx_load_opengl_library()?),
    };
    let mut default_get;
    let get: GlGetProcAddress = match pfn_gl_get_proc_address {
        Some(f) => f,
        None => {
            default_get = |name: &CStr| -> *const c_void {
                let Some((m, wgl)) = module.as_ref() else {
                    return std::ptr::null();
                };
                if let Some(wgl) = wgl {
                    // SAFETY: wglGetProcAddress on a NUL terminated name.
                    let p = unsafe { wgl(name.as_ptr()) };
                    if !p.is_null() {
                        return p;
                    }
                }
                // SAFETY: only the address is read.
                unsafe { m.get::<*const c_void>(name.to_bytes_with_nul()) }
                    .ok()
                    .map_or(std::ptr::null(), |s| *s)
            };
            &mut default_get
        }
    };

    macro_rules! gl_function {
        ($field:ident, $t:ty, $name:literal, $required:expr) => {{
            let p = get($name);
            let f = if p.is_null() {
                None
            } else {
                // SAFETY: the address of the command named like `$t` (caller contract).
                Some(unsafe { std::mem::transmute::<*const c_void, $t>(p) })
            };
            if f.is_none() && $required {
                eprintln!(
                    "Could not load OpenGL command: {}!",
                    $name.to_string_lossy()
                );
                return Err(ErrorCode::NotFound);
            }
            f
        }};
    }
    let gl = GlFuncs {
        gl_bind_texture: gl_function!(gl_bind_texture, PfnGlBindTexture, c"glBindTexture", true),
        gl_compressed_tex_image_1d: gl_function!(
            gl_compressed_tex_image_1d,
            PfnGlCompressedTexImage1D,
            c"glCompressedTexImage1D",
            false
        ),
        gl_compressed_tex_image_2d: gl_function!(
            gl_compressed_tex_image_2d,
            PfnGlCompressedTexImage2D,
            c"glCompressedTexImage2D",
            true
        ),
        gl_compressed_tex_image_3d: gl_function!(
            gl_compressed_tex_image_3d,
            PfnGlCompressedTexImage3D,
            c"glCompressedTexImage3D",
            false
        ),
        gl_compressed_tex_sub_image_1d: gl_function!(
            gl_compressed_tex_sub_image_1d,
            PfnGlCompressedTexSubImage1D,
            c"glCompressedTexSubImage1D",
            false
        ),
        gl_compressed_tex_sub_image_2d: gl_function!(
            gl_compressed_tex_sub_image_2d,
            PfnGlCompressedTexSubImage2D,
            c"glCompressedTexSubImage2D",
            true
        ),
        gl_compressed_tex_sub_image_3d: gl_function!(
            gl_compressed_tex_sub_image_3d,
            PfnGlCompressedTexSubImage3D,
            c"glCompressedTexSubImage3D",
            false
        ),
        gl_delete_textures: gl_function!(
            gl_delete_textures,
            PfnGlDeleteTextures,
            c"glDeleteTextures",
            true
        ),
        gl_generate_mipmap: gl_function!(
            gl_generate_mipmap,
            PfnGlGenerateMipmap,
            c"glGenerateMipmap",
            false
        ),
        gl_gen_textures: gl_function!(gl_gen_textures, PfnGlGenTextures, c"glGenTextures", true),
        gl_get_error: gl_function!(gl_get_error, PfnGlGetError, c"glGetError", true),
        gl_get_integerv: gl_function!(gl_get_integerv, PfnGlGetIntegerv, c"glGetIntegerv", true),
        gl_get_string: gl_function!(gl_get_string, PfnGlGetString, c"glGetString", true),
        gl_get_stringi: gl_function!(gl_get_stringi, PfnGlGetStringi, c"glGetStringi", false),
        gl_pixel_storei: gl_function!(gl_pixel_storei, PfnGlPixelStorei, c"glPixelStorei", true),
        gl_tex_image_1d: gl_function!(gl_tex_image_1d, PfnGlTexImage1D, c"glTexImage1D", false),
        gl_tex_image_2d: gl_function!(gl_tex_image_2d, PfnGlTexImage2D, c"glTexImage2D", true),
        gl_tex_image_3d: gl_function!(gl_tex_image_3d, PfnGlTexImage3D, c"glTexImage3D", false),
        gl_tex_parameteri: gl_function!(
            gl_tex_parameteri,
            PfnGlTexParameteri,
            c"glTexParameteri",
            true
        ),
        gl_tex_parameteriv: gl_function!(
            gl_tex_parameteriv,
            PfnGlTexParameteriv,
            c"glTexParameteriv",
            true
        ),
        gl_tex_storage_1d: gl_function!(
            gl_tex_storage_1d,
            PfnGlTexStorage1D,
            c"glTexStorage1D",
            false
        ),
        gl_tex_storage_2d: gl_function!(
            gl_tex_storage_2d,
            PfnGlTexStorage2D,
            c"glTexStorage2D",
            false
        ),
        gl_tex_storage_3d: gl_function!(
            gl_tex_storage_3d,
            PfnGlTexStorage3D,
            c"glTexStorage3D",
            false
        ),
        gl_tex_sub_image_1d: gl_function!(
            gl_tex_sub_image_1d,
            PfnGlTexSubImage1D,
            c"glTexSubImage1D",
            false
        ),
        gl_tex_sub_image_2d: gl_function!(
            gl_tex_sub_image_2d,
            PfnGlTexSubImage2D,
            c"glTexSubImage2D",
            true
        ),
        gl_tex_sub_image_3d: gl_function!(
            gl_tex_sub_image_3d,
            PfnGlTexSubImage3D,
            c"glTexSubImage3D",
            false
        ),
    };
    // The required commands were checked above.
    let (
        Some(bind_texture),
        Some(compressed_tex_image_2d),
        Some(delete_textures),
        Some(gen_textures),
        Some(get_error),
        Some(get_integerv),
        Some(get_string),
        Some(pixel_storei),
        Some(tex_image_2d),
        Some(tex_parameteri),
    ) = (
        gl.gl_bind_texture,
        gl.gl_compressed_tex_image_2d,
        gl.gl_delete_textures,
        gl.gl_gen_textures,
        gl.gl_get_error,
        gl.gl_get_integerv,
        gl.gl_get_string,
        gl.gl_pixel_storei,
        gl.gl_tex_image_2d,
        gl.gl_tex_parameteri,
    )
    else {
        return Err(ErrorCode::NotFound);
    };
    *state = Some(GlState {
        cmds: NativeGl {
            gl,
            req: Required {
            bind_texture,
            compressed_tex_image_2d,
            delete_textures,
            gen_textures,
            get_error,
            get_integerv,
            get_string,
            pixel_storei,
            tex_image_2d,
            tex_parameteri,
        },
        },
        caps: GlContextCaps::default(),
    });
    Ok(())
}

impl GlCommands for NativeGl {
    fn has(&self, cmd: GlOptional) -> bool {
        let g = &self.gl;
        match cmd {
            GlOptional::GetStringi => g.gl_get_stringi.is_some(),
            GlOptional::TexImage1D => g.gl_tex_image_1d.is_some(),
            GlOptional::CompressedTexImage1D => g.gl_compressed_tex_image_1d.is_some(),
            GlOptional::TexImage3D => g.gl_tex_image_3d.is_some(),
            GlOptional::CompressedTexImage3D => g.gl_compressed_tex_image_3d.is_some(),
            GlOptional::CompressedTexSubImage3D => g.gl_compressed_tex_sub_image_3d.is_some(),
            GlOptional::TexSubImage3D => g.gl_tex_sub_image_3d.is_some(),
            GlOptional::GenerateMipmap => g.gl_generate_mipmap.is_some(),
            GlOptional::TexStorage1D => g.gl_tex_storage_1d.is_some(),
            GlOptional::TexStorage2D => g.gl_tex_storage_2d.is_some(),
            GlOptional::TexStorage3D => g.gl_tex_storage_3d.is_some(),
        }
    }

    // SAFETY (every command below): the commands were loaded for the context the caller of the
    // `unsafe` upload made current; slices are the image data GL reads, `size` bytes of it for
    // compressed images.
    fn bind_texture(&self, target: GLenum, texture: GLuint) {
        unsafe { (self.req.bind_texture)(target, texture) }
    }

    fn compressed_tex_image_2d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLenum,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: &[u8],
    ) {
        unsafe {
            (self.req.compressed_tex_image_2d)(
                target,
                level,
                internalformat,
                width,
                height,
                border,
                image_size,
                data.as_ptr().cast::<c_void>(),
            )
        }
    }

    fn delete_texture(&self, texture: GLuint) {
        unsafe { (self.req.delete_textures)(1, &texture) }
    }

    fn gen_texture(&self) -> GLuint {
        let mut t = 0;
        unsafe { (self.req.gen_textures)(1, &mut t) };
        t
    }

    fn get_error(&self) -> GLenum {
        unsafe { (self.req.get_error)() }
    }

    fn get_integerv(&self, pname: GLenum, data: &mut GLint) {
        unsafe { (self.req.get_integerv)(pname, data) }
    }

    fn get_string(&self, name: GLenum) -> String {
        // GL returns a static NUL terminated string or null.
        unsafe {
            let p = (self.req.get_string)(name);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p.cast::<c_char>())
                    .to_string_lossy()
                    .into_owned()
            }
        }
    }

    fn pixel_storei(&self, pname: GLenum, param: GLint) {
        unsafe { (self.req.pixel_storei)(pname, param) }
    }

    fn tex_image_2d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLint,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        format: GLenum,
        ty: GLenum,
        pixels: &[u8],
    ) {
        unsafe {
            (self.req.tex_image_2d)(
                target,
                level,
                internalformat,
                width,
                height,
                border,
                format,
                ty,
                pixels.as_ptr().cast::<c_void>(),
            )
        }
    }

    fn tex_parameteri(&self, target: GLenum, pname: GLenum, param: GLint) {
        unsafe { (self.req.tex_parameteri)(target, pname, param) }
    }

    fn get_stringi(&self, name: GLenum, index: GLuint) -> Option<String> {
        let f = self.gl.gl_get_stringi?;
        unsafe {
            let p = f(name, index);
            (!p.is_null()).then(|| {
                CStr::from_ptr(p.cast::<c_char>())
                    .to_string_lossy()
                    .into_owned()
            })
        }
    }

    fn generate_mipmap(&self, target: GLenum) {
        if let Some(f) = self.gl.gl_generate_mipmap {
            unsafe { f(target) }
        }
    }

    fn tex_image_1d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLint,
        width: GLsizei,
        border: GLint,
        format: GLenum,
        ty: GLenum,
        pixels: &[u8],
    ) {
        if let Some(f) = self.gl.gl_tex_image_1d {
            unsafe {
                f(
                    target,
                    level,
                    internalformat,
                    width,
                    border,
                    format,
                    ty,
                    pixels.as_ptr().cast::<c_void>(),
                )
            }
        }
    }

    fn compressed_tex_image_1d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLenum,
        width: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: &[u8],
    ) {
        if let Some(f) = self.gl.gl_compressed_tex_image_1d {
            unsafe {
                f(
                    target,
                    level,
                    internalformat,
                    width,
                    border,
                    image_size,
                    data.as_ptr().cast::<c_void>(),
                )
            }
        }
    }

    fn tex_image_3d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLint,
        width: GLsizei,
        height: GLsizei,
        depth: GLsizei,
        border: GLint,
        format: GLenum,
        ty: GLenum,
        pixels: &[u8],
    ) {
        if let Some(f) = self.gl.gl_tex_image_3d {
            unsafe {
                f(
                    target,
                    level,
                    internalformat,
                    width,
                    height,
                    depth,
                    border,
                    format,
                    ty,
                    pixels.as_ptr().cast::<c_void>(),
                )
            }
        }
    }

    fn compressed_tex_image_3d(
        &self,
        target: GLenum,
        level: GLint,
        internalformat: GLenum,
        width: GLsizei,
        height: GLsizei,
        depth: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: &[u8],
    ) {
        if let Some(f) = self.gl.gl_compressed_tex_image_3d {
            unsafe {
                f(
                    target,
                    level,
                    internalformat,
                    width,
                    height,
                    depth,
                    border,
                    image_size,
                    data.as_ptr().cast::<c_void>(),
                )
            }
        }
    }
}

/// Loads the commands (`ktxLoadOpenGL(NULL)`) and runs `f` on them and the capabilities.
///
/// # Safety
/// See [`Texture1::gl_upload`].
unsafe fn with_loaded_state<R>(
    f: impl FnOnce(&NativeGl, &mut GlContextCaps) -> KtxResult<R>,
) -> KtxResult<R> {
    // SAFETY: forwarded contract.
    unsafe { ktx_load_opengl(None)? };
    let mut guard = gl_state();
    let st = guard.as_mut().ok_or(ErrorCode::LibraryNotLinked)?;
    let cmds = st.cmds;
    f(&cmds, &mut st.caps)
}

impl Texture1 {
    /// Port of `ktxTexture1_GLUpload`: creates a GL texture object (named `*p_texture` when it
    /// is non-zero, otherwise generated and returned there) holding the images, and returns its
    /// target. On a GL error `p_glerror` receives it and `KTX_GL_ERROR` is returned.
    ///
    /// # Safety
    /// A GL context must be current on this thread and belong to the commands loaded by
    /// [`ktx_load_opengl`].
    pub unsafe fn gl_upload(
        &mut self,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
        // SAFETY: forwarded contract.
        unsafe {
            with_loaded_state(|gl, caps| {
                self.gl_upload_with(gl, caps, p_texture, p_target, p_glerror)
            })
        }
    }
}

impl Texture2 {
    /// Port of `ktxTexture2_GLUpload`. Basis Universal textures must be transcoded first
    /// (`KTX_INVALID_OPERATION`).
    ///
    /// # Safety
    /// See [`Texture1::gl_upload`].
    pub unsafe fn gl_upload(
        &mut self,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
        // SAFETY: forwarded contract.
        unsafe {
            with_loaded_state(|gl, caps| {
                self.gl_upload_with(gl, caps, p_texture, p_target, p_glerror)
            })
        }
    }
}

impl AnyTexture {
    /// Port of `ktxTexture_GLUpload`.
    ///
    /// # Safety
    /// See [`Texture1::gl_upload`].
    pub unsafe fn gl_upload(
        &mut self,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
        // SAFETY: forwarded contract.
        unsafe {
            with_loaded_state(|gl, caps| {
                self.gl_upload_with(gl, caps, p_texture, p_target, p_glerror)
            })
        }
    }
}

/// `R16Formats` discovered for the current context (`KTX_R16_FORMATS_*`); all formats before
/// the first upload. Exposed for tests of the capability discovery.
pub fn context_r16_formats() -> u32 {
    gl_state().as_ref().map_or_else(
        || GlContextCaps::default().r16_formats(),
        |s| s.caps.r16_formats(),
    )
}

/// Whether the loaded GL commands treat the context as GL ES (`_CONTEXT_ES_PROFILE_BIT`).
pub fn context_is_es() -> bool {
    gl_state().as_ref().is_some_and(|s| s.caps.is_es())
}
