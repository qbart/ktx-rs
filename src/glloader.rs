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
//! Deviations (no runtime exceptions): an optional command that is missing for a texture that
//! needs it returns `KTX_UNSUPPORTED_TEXTURE_TYPE` instead of the C assert; a null
//! `glGetString` result is treated as an empty string.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::{Mutex, MutexGuard};

use crate::error::{ErrorCode, KtxResult};
use crate::etcunpack::{KTX_ALL_R16_FORMATS, KTX_R16_FORMATS_SNORM, ktx_unpack_etc};
use crate::gl_format::{
    GL_ALPHA, GL_COMPRESSED_R11_EAC, GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, GL_ETC1_RGB8_OES, GL_RGB,
    GL_RGB8, GL_RGBA, GL_RGBA8,
};
use crate::texture::KtxTexture;
use crate::texture_any::AnyTexture;
use crate::texture1::Texture1;
use crate::texture2::Texture2;
use crate::vk2gl::{
    GL_INVALID_VALUE, vk_format2gl_format, vk_format2gl_internal_format, vk_format2gl_type,
};
use crate::vkformat::VkFormat;

pub type GLenum = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLboolean = u8;

const GL_NO_ERROR: GLenum = 0;
const GL_INVALID_ENUM: GLenum = 0x0500;
const GL_INVALID_VALUE_ERROR: GLenum = 0x0501;
const GL_TEXTURE_1D: GLenum = 0x0DE0;
const GL_TEXTURE_2D: GLenum = 0x0DE1;
const GL_TEXTURE_3D: GLenum = 0x806F;
const GL_TEXTURE_1D_ARRAY: GLenum = 0x8C18;
const GL_TEXTURE_2D_ARRAY: GLenum = 0x8C1A;
const GL_TEXTURE_CUBE_MAP: GLenum = 0x8513;
const GL_TEXTURE_CUBE_MAP_POSITIVE_X: GLenum = 0x8515;
const GL_TEXTURE_CUBE_MAP_ARRAY: GLenum = 0x9009;
const GL_TEXTURE_MAX_LEVEL: GLenum = 0x813D;
const GL_GENERATE_MIPMAP: GLenum = 0x8191;
const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
const GL_VERSION: GLenum = 0x1F02;
const GL_EXTENSIONS: GLenum = 0x1F03;
const GL_NUM_EXTENSIONS: GLenum = 0x821D;
const GL_MAJOR_VERSION: GLenum = 0x821B;
const GL_MINOR_VERSION: GLenum = 0x821C;
const GL_CONTEXT_PROFILE_MASK: GLenum = 0x9126;
const GL_CONTEXT_CORE_PROFILE_BIT: GLint = 0x1;
const GL_CONTEXT_COMPATIBILITY_PROFILE_BIT: GLint = 0x2;
const GL_LUMINANCE: GLenum = 0x1909; // deprecated
const GL_LUMINANCE_ALPHA: GLenum = 0x190A; // deprecated
const GL_INTENSITY: GLenum = 0x8049; // deprecated
const GL_TRUE: GLint = 1;

const CONTEXT_ES_PROFILE_BIT: GLint = 0x4;
/// Non-legacy sized formats are supported.
const NON_LEGACY_FORMATS: GLint = 0x1;
/// Legacy sized formats are supported.
const LEGACY_FORMATS: GLint = 0x2;
const ALL_SIZED_FORMATS: GLint = NON_LEGACY_FORMATS | LEGACY_FORMATS;
/// No sized formats are supported.
const NO_SIZED_FORMATS: GLint = 0;
const KTX_GL_UNPACK_ALIGNMENT: GLint = 4;

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

/// The loaded commands plus the `discoverContextCapabilities` statics of `glloader.c`.
struct GlState {
    gl: GlFuncs,
    req: Required,
    context_profile: GLint,
    sized_formats: GLint,
    supports_swizzle: bool,
    r16_formats: u32,
    supports_srgb: bool,
    supports_cube_map_arrays: bool,
    supports_max_level: bool,
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
        context_profile: 0,
        sized_formats: ALL_SIZED_FORMATS,
        supports_swizzle: true,
        r16_formats: KTX_ALL_R16_FORMATS,
        supports_srgb: true,
        supports_cube_map_arrays: false,
        supports_max_level: false,
    });
    Ok(())
}

impl GlState {
    /// `glGetString` as a string ("" for null).
    unsafe fn get_string(&self, name: GLenum) -> String {
        // SAFETY: current context (caller contract); GL returns a static NUL terminated string.
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

    unsafe fn get_integer(&self, name: GLenum, value: &mut GLint) {
        // SAFETY: current context; one integer is written.
        unsafe { (self.req.get_integerv)(name, value) };
    }

    /// `hasExtension`.
    unsafe fn has_extension(&self, extension: &str) -> bool {
        // SAFETY (whole function): current context.
        unsafe {
            match self.gl.gl_get_stringi {
                None => self.get_string(GL_EXTENSIONS).contains(extension),
                Some(get_stringi) => {
                    let mut n = 0;
                    self.get_integer(GL_NUM_EXTENSIONS, &mut n);
                    (0..n.max(0) as GLuint).any(|i| {
                        let p = get_stringi(GL_EXTENSIONS, i);
                        !p.is_null()
                            && CStr::from_ptr(p.cast::<c_char>()).to_bytes() == extension.as_bytes()
                    })
                }
            }
        }
    }

    /// `discoverContextCapabilities`.
    unsafe fn discover_context_capabilities(&mut self) {
        // SAFETY (whole function): current context.
        unsafe {
            let mut major_version: GLint = 1;
            let mut minor_version: GLint = 0;

            let version = self.get_string(GL_VERSION);
            if version.contains("GL ES") {
                self.context_profile = CONTEXT_ES_PROFILE_BIT;
            }
            // MAJOR & MINOR only introduced in GL {,ES} 3.0
            self.get_integer(GL_MAJOR_VERSION, &mut major_version);
            self.get_integer(GL_MINOR_VERSION, &mut minor_version);
            if (self.req.get_error)() != GL_NO_ERROR {
                // < v3.0; resort to the old-fashioned way.
                let prefix = if self.context_profile & CONTEXT_ES_PROFILE_BIT != 0 {
                    "OpenGL ES "
                } else {
                    "OpenGL "
                };
                scan_version(&version, prefix, &mut major_version, &mut minor_version);
            }
            if self.context_profile & CONTEXT_ES_PROFILE_BIT != 0 {
                if major_version < 3 {
                    self.supports_swizzle = false;
                    self.sized_formats = NO_SIZED_FORMATS;
                    self.r16_formats = 0;
                    self.supports_srgb = false;
                    // These things could be found by dlsym when, e.g. the same driver supports
                    // ES1, ES2 and ES3. For all but Tex*3D, there's no corresponding extension
                    // whose presence we could check. Just zero the pointers to prevent use.
                    self.gl.gl_get_stringi = None;
                    self.gl.gl_compressed_tex_image_1d = None;
                    self.gl.gl_tex_storage_1d = None;
                    self.gl.gl_tex_storage_2d = None;
                    self.gl.gl_tex_storage_3d = None;
                    if !self.has_extension("GL_OES_texture_3D") {
                        self.gl.gl_compressed_tex_image_3d = None;
                        self.gl.gl_compressed_tex_sub_image_3d = None;
                        self.gl.gl_tex_image_3d = None;
                        self.gl.gl_tex_sub_image_3d = None;
                    }
                    if major_version < 2 {
                        self.gl.gl_generate_mipmap = None;
                    }
                } else {
                    self.sized_formats = NON_LEGACY_FORMATS;
                    if self.has_extension("GL_EXT_texture_cube_map_array") {
                        self.supports_cube_map_arrays = true;
                    }
                    self.supports_max_level = true;
                }
                if self.has_extension("GL_OES_required_internalformat") {
                    self.sized_formats |= ALL_SIZED_FORMATS;
                }
                // There are no OES extensions for sRGB textures or R16 formats.
            } else {
                // PROFILE_MASK was introduced in OpenGL 3.2.
                // Profiles: CONTEXT_CORE_PROFILE_BIT 0x1,
                //           CONTEXT_COMPATIBILITY_PROFILE_BIT 0x2.
                let mut profile = self.context_profile;
                self.get_integer(GL_CONTEXT_PROFILE_MASK, &mut profile);
                self.context_profile = profile;
                if (self.req.get_error)() == GL_NO_ERROR {
                    // >= 3.2
                    if major_version == 3 && minor_version < 3 {
                        self.supports_swizzle = false;
                    }
                    if self.context_profile & GL_CONTEXT_CORE_PROFILE_BIT != 0 {
                        self.sized_formats &= !LEGACY_FORMATS;
                    }
                    if major_version >= 4 {
                        self.supports_cube_map_arrays = true;
                    }
                    self.supports_max_level = true;
                } else {
                    // < 3.2
                    self.context_profile = GL_CONTEXT_COMPATIBILITY_PROFILE_BIT;
                    self.supports_swizzle = false;
                    // sRGB textures introduced in 2.0
                    if major_version < 2 && !self.has_extension("GL_EXT_texture_sRGB") {
                        self.supports_srgb = false;
                    }
                    // R{,G]16 introduced in 3.0; R{,G}16_SNORM introduced in 3.1.
                    if major_version == 3 {
                        if minor_version == 0 {
                            self.r16_formats &= !KTX_R16_FORMATS_SNORM;
                        }
                        if minor_version < 1 {
                            if self.has_extension("GL_ARB_texture_query_levels") {
                                self.supports_max_level = true;
                            }
                        } else {
                            self.supports_max_level = true;
                        }
                    } else if self.has_extension("GL_ARB_texture_rg") {
                        self.r16_formats &= !KTX_R16_FORMATS_SNORM;
                    } else {
                        self.r16_formats = 0;
                    }
                }
                if !self.supports_cube_map_arrays
                    && self.has_extension("GL_ARB_texture_cube_map_array")
                {
                    self.supports_cube_map_arrays = true;
                }
            }
        }
    }
}

/// `sscanf(version, "<prefix>%d.%d ", &major, &minor)`: values are only set when matched.
fn scan_version(version: &str, prefix: &str, major: &mut GLint, minor: &mut GLint) {
    let Some(rest) = version.strip_prefix(prefix) else {
        return;
    };
    let int = |s: &str| -> Option<(GLint, usize)> {
        let t = s.trim_start();
        let skipped = s.len() - t.len();
        let n = t
            .char_indices()
            .take_while(|&(i, c)| c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))
            .count();
        t[..n].parse().ok().map(|v| (v, skipped + n))
    };
    let Some((maj, n)) = int(rest) else {
        return;
    };
    *major = maj;
    if let Some(after) = rest[n..].strip_prefix('.')
        && let Some((min, _)) = int(after)
    {
        *minor = min;
    }
}

/// Port of `ktx_glformatinfo`.
#[derive(Clone, Copy, Debug)]
struct GlFormatInfo {
    gl_format: u32,
    gl_internalformat: u32,
    gl_base_internalformat: u32,
    gl_type: u32,
}

/// Port of `ktx_cbdata`.
struct CbData {
    gl_target: GLenum,
    gl_format: GLenum,
    gl_internalformat: GLenum,
    gl_type: GLenum,
    gl_error: GLenum,
    num_layers: GLuint,
}

#[derive(Clone, Copy)]
enum IterCallback {
    TexImage1D,
    CompressedTexImage1D,
    TexImage2D,
    CompressedTexImage2D,
    TexImage3D,
    CompressedTexImage3D,
}

impl GlState {
    /// The image callbacks (`texImage1DCallback` ... `compressedTexImage3DCallback`).
    #[allow(clippy::too_many_arguments)]
    unsafe fn image_callback(
        &self,
        which: IterCallback,
        cb: &mut CbData,
        miplevel: u32,
        face: u32,
        width: u32,
        height: u32,
        depth: u32,
        face_lod_size: u64,
        pixels: &[u8],
    ) -> KtxResult<()> {
        let target = cb.gl_target + face;
        let level = miplevel as GLint;
        let pix = pixels.as_ptr().cast::<c_void>();
        let height_or_layers = if cb.num_layers == 0 {
            height
        } else {
            cb.num_layers
        } as GLsizei;
        let depth_or_layers = if cb.num_layers == 0 {
            depth
        } else {
            cb.num_layers
        } as GLsizei;
        let compressed = matches!(
            which,
            IterCallback::CompressedTexImage1D
                | IterCallback::CompressedTexImage2D
                | IterCallback::CompressedTexImage3D
        );
        if compressed && face_lod_size > u64::from(u32::MAX) {
            return Err(ErrorCode::InvalidOperation); // Too big for OpenGL {,ES}.
        }
        let size = face_lod_size as GLsizei;
        let missing = Err(ErrorCode::UnsupportedTextureType);
        // SAFETY (whole match): current context; `pixels` holds the image being specified.
        let glerror = unsafe {
            match which {
                IterCallback::TexImage1D => {
                    let Some(f) = self.gl.gl_tex_image_1d else {
                        return missing;
                    };
                    f(
                        target,
                        level,
                        cb.gl_internalformat as GLint,
                        width as GLsizei,
                        0,
                        cb.gl_format,
                        cb.gl_type,
                        pix,
                    );
                    (self.req.get_error)()
                }
                IterCallback::CompressedTexImage1D => {
                    let Some(f) = self.gl.gl_compressed_tex_image_1d else {
                        return missing;
                    };
                    f(
                        target,
                        level,
                        cb.gl_internalformat,
                        width as GLsizei,
                        0,
                        size,
                        pix,
                    );
                    (self.req.get_error)()
                }
                IterCallback::TexImage2D => {
                    (self.req.tex_image_2d)(
                        target,
                        level,
                        cb.gl_internalformat as GLint,
                        width as GLsizei,
                        height_or_layers,
                        0,
                        cb.gl_format,
                        cb.gl_type,
                        pix,
                    );
                    (self.req.get_error)()
                }
                IterCallback::CompressedTexImage2D => {
                    // It is simpler to just attempt to load the format, rather than divine
                    // which formats are supported by the implementation. In the event of an
                    // error, software unpacking can be attempted.
                    (self.req.compressed_tex_image_2d)(
                        target,
                        level,
                        cb.gl_internalformat,
                        width as GLsizei,
                        height_or_layers,
                        0,
                        size,
                        pix,
                    );
                    let mut glerror = (self.req.get_error)();
                    // Renderion is returning INVALID_VALUE. Oops!!
                    if (glerror == GL_INVALID_ENUM || glerror == GL_INVALID_VALUE_ERROR)
                        && (cb.gl_internalformat == GL_ETC1_RGB8_OES
                            || (GL_COMPRESSED_R11_EAC..=GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC)
                                .contains(&cb.gl_internalformat))
                    {
                        let unpacked = ktx_unpack_etc(
                            pixels,
                            cb.gl_internalformat,
                            width,
                            height,
                            self.r16_formats,
                            self.supports_srgb,
                        )?;
                        let mut internalformat = unpacked.internal_format;
                        if self.sized_formats & NON_LEGACY_FORMATS == 0 {
                            if internalformat == GL_RGB8 {
                                internalformat = GL_RGB;
                            } else if internalformat == GL_RGBA8 {
                                internalformat = GL_RGBA;
                            }
                        }
                        (self.req.tex_image_2d)(
                            target,
                            level,
                            internalformat as GLint,
                            width as GLsizei,
                            height_or_layers,
                            0,
                            unpacked.format,
                            unpacked.r#type,
                            unpacked.image.as_ptr().cast::<c_void>(),
                        );
                        glerror = (self.req.get_error)();
                    }
                    glerror
                }
                IterCallback::TexImage3D => {
                    let Some(f) = self.gl.gl_tex_image_3d else {
                        return missing;
                    };
                    f(
                        target,
                        level,
                        cb.gl_internalformat as GLint,
                        width as GLsizei,
                        height as GLsizei,
                        depth_or_layers,
                        0,
                        cb.gl_format,
                        cb.gl_type,
                        pix,
                    );
                    (self.req.get_error)()
                }
                IterCallback::CompressedTexImage3D => {
                    let Some(f) = self.gl.gl_compressed_tex_image_3d else {
                        return missing;
                    };
                    f(
                        target,
                        level,
                        cb.gl_internalformat,
                        width as GLsizei,
                        height as GLsizei,
                        depth_or_layers,
                        0,
                        size,
                        pix,
                    );
                    (self.req.get_error)()
                }
            }
        };
        cb.gl_error = glerror;
        if glerror == GL_NO_ERROR {
            Ok(())
        } else {
            Err(ErrorCode::GlError)
        }
    }
}

/// Port of `ktxTexture_GLUploadPrivate`.
///
/// # Safety
/// See [`Texture2::gl_upload`].
unsafe fn ktx_texture_gl_upload_private(
    st: &mut GlState,
    this: &mut dyn KtxTexture,
    format_info: &GlFormatInfo,
    p_texture: Option<&mut GLuint>,
    p_target: &mut GLenum,
    p_glerror: Option<&mut GLenum>,
) -> KtxResult<()> {
    let mut target = GL_TEXTURE_2D;
    let mut p_glerror = p_glerror;
    if let Some(e) = p_glerror.as_deref_mut() {
        *e = GL_NO_ERROR;
    }

    // SAFETY (whole function): current context (caller contract).
    unsafe {
        if st.context_profile == 0 {
            st.discover_context_capabilities();
        }

        let user_texname = p_texture.as_deref().copied().filter(|&t| t != 0);
        let texname = match user_texname {
            Some(t) => t,
            None => {
                let mut t = 0;
                (st.req.gen_textures)(1, &mut t);
                t
            }
        };

        let t = this.base();
        let mut cb_data = CbData {
            gl_target: 0,
            gl_format: format_info.gl_format,
            gl_internalformat: format_info.gl_internalformat,
            gl_type: format_info.gl_type,
            gl_error: GL_NO_ERROR,
            num_layers: 0,
        };

        let mut dimensions = t.num_dimensions;
        if t.is_array {
            dimensions += 1;
            if t.num_faces == 6 {
                target = GL_TEXTURE_CUBE_MAP_ARRAY;
            } else {
                match t.num_dimensions {
                    1 => target = GL_TEXTURE_1D_ARRAY,
                    2 => target = GL_TEXTURE_2D_ARRAY,
                    // _ktxCheckHeader should have caught this.
                    _ => {}
                }
            }
            cb_data.num_layers = t.num_layers;
        } else {
            if t.num_faces == 6 {
                target = GL_TEXTURE_CUBE_MAP;
            } else {
                match t.num_dimensions {
                    1 => target = GL_TEXTURE_1D,
                    2 => target = GL_TEXTURE_2D,
                    3 => target = GL_TEXTURE_3D,
                    // _ktxCheckHeader shold have caught this.
                    _ => {}
                }
            }
            cb_data.num_layers = 0;
        }

        if target == GL_TEXTURE_1D
            && ((t.is_compressed && st.gl.gl_compressed_tex_image_1d.is_none())
                || (!t.is_compressed && st.gl.gl_tex_image_1d.is_none()))
        {
            return Err(ErrorCode::UnsupportedTextureType);
        }

        // Reject 3D texture if unsupported.
        if target == GL_TEXTURE_3D
            && ((t.is_compressed && st.gl.gl_compressed_tex_image_3d.is_none())
                || (!t.is_compressed && st.gl.gl_tex_image_3d.is_none()))
        {
            return Err(ErrorCode::UnsupportedTextureType);
        }

        // Reject cube map arrays if not supported.
        if target == GL_TEXTURE_CUBE_MAP_ARRAY && !st.supports_cube_map_arrays {
            return Err(ErrorCode::UnsupportedTextureType);
        }

        // XXX Need to reject other array textures & cube maps if not supported.

        let iter_cb = match (dimensions, t.is_compressed) {
            (1, true) => IterCallback::CompressedTexImage1D,
            (1, false) => IterCallback::TexImage1D,
            (3, true) => IterCallback::CompressedTexImage3D,
            (3, false) => IterCallback::TexImage3D,
            (_, true) => IterCallback::CompressedTexImage2D,
            (_, false) => IterCallback::TexImage2D,
        };

        (st.req.bind_texture)(target, texname);

        // Prefer glGenerateMipmaps over GL_GENERATE_MIPMAP
        if t.generate_mipmaps && st.gl.gl_generate_mipmap.is_none() {
            (st.req.tex_parameteri)(target, GL_GENERATE_MIPMAP, GL_TRUE);
        }

        if !t.generate_mipmaps && st.supports_max_level {
            (st.req.tex_parameteri)(target, GL_TEXTURE_MAX_LEVEL, t.num_levels as GLint - 1);
        }

        cb_data.gl_target = if target == GL_TEXTURE_CUBE_MAP {
            GL_TEXTURE_CUBE_MAP_POSITIVE_X
        } else {
            target
        };

        if !t.is_compressed {
            // When no sized formats are supported, or legacy sized formats are not supported,
            // must change internal format.
            if st.sized_formats == NO_SIZED_FORMATS
                || (st.sized_formats & LEGACY_FORMATS == 0
                    && (format_info.gl_base_internalformat == GL_ALPHA
                        || format_info.gl_base_internalformat == GL_LUMINANCE
                        || format_info.gl_base_internalformat == GL_LUMINANCE_ALPHA
                        || format_info.gl_base_internalformat == GL_INTENSITY))
            {
                cb_data.gl_internalformat = format_info.gl_base_internalformat;
            }
        }

        let (generate_mipmaps, active_stream) = (t.generate_mipmaps, t.is_active_stream());
        let state: &GlState = st;
        let mut cb = |miplevel, face, w, h, d, size, pixels: &[u8]| {
            state.image_callback(iter_cb, &mut cb_data, miplevel, face, w, h, d, size, pixels)
        };
        let result = if active_stream {
            this.iterate_load_level_faces(&mut cb)
        } else {
            this.iterate_level_faces(&mut cb)
        };

        // GL errors are the only reason for failure.
        if result.is_err()
            && cb_data.gl_error != GL_NO_ERROR
            && let Some(e) = p_glerror
        {
            *e = cb_data.gl_error;
        }

        if result.is_ok() {
            // Prefer glGenerateMipmaps over GL_GENERATE_MIPMAP
            if generate_mipmaps && let Some(generate_mipmap) = st.gl.gl_generate_mipmap {
                generate_mipmap(target);
            }
            *p_target = target;
            if let Some(p) = p_texture {
                *p = texname;
            }
        } else if user_texname.is_none() {
            (st.req.delete_textures)(1, &texname);
        }
        result
    }
}

/// Sets `GL_UNPACK_ALIGNMENT` to `alignment` around `f`, restoring the previous value.
unsafe fn with_unpack_alignment<R>(st: &GlState, alignment: GLint, f: impl FnOnce() -> R) -> R {
    // SAFETY: current context.
    unsafe {
        let mut previous_unpack_alignment = 0;
        st.get_integer(GL_UNPACK_ALIGNMENT, &mut previous_unpack_alignment);
        if previous_unpack_alignment != alignment {
            (st.req.pixel_storei)(GL_UNPACK_ALIGNMENT, alignment);
        }
        let r = f();
        // restore previous GL state
        if previous_unpack_alignment != alignment {
            (st.req.pixel_storei)(GL_UNPACK_ALIGNMENT, previous_unpack_alignment);
        }
        r
    }
}

/// Loads the commands (`ktxLoadOpenGL(NULL)`) and returns the state.
unsafe fn loaded_state() -> KtxResult<MutexGuard<'static, Option<GlState>>> {
    // SAFETY: forwarded contract.
    unsafe { ktx_load_opengl(None)? };
    Ok(gl_state())
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
            let mut guard = loaded_state()?;
            let st = guard.as_mut().ok_or(ErrorCode::LibraryNotLinked)?;
            let format_info = GlFormatInfo {
                gl_format: self.gl_format,
                gl_internalformat: self.gl_internalformat,
                gl_base_internalformat: self.gl_base_internalformat,
                gl_type: self.gl_type,
            };
            // KTX 1 files require an unpack alignment of 4
            let st_ptr: *mut GlState = st;
            with_unpack_alignment(&*st_ptr, KTX_GL_UNPACK_ALIGNMENT, || {
                ktx_texture_gl_upload_private(
                    &mut *st_ptr,
                    self,
                    &format_info,
                    p_texture,
                    p_target,
                    p_glerror,
                )
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
            let mut guard = loaded_state()?;
            let st = guard.as_mut().ok_or(ErrorCode::LibraryNotLinked)?;

            let mut format_info = GlFormatInfo {
                gl_format: 0,
                gl_internalformat: 0,
                gl_base_internalformat: 0,
                gl_type: 0,
            };
            if self.vk_format != VkFormat::UNDEFINED {
                format_info.gl_internalformat = vk_format2gl_internal_format(self.vk_format);
                if format_info.gl_internalformat == GL_INVALID_VALUE {
                    // TODO Check for mapping metadata. If none
                    return Err(ErrorCode::InvalidOperation);
                }
            } else {
                // TODO: Check DFD for ASTC HDR or 3D or RGB[DEM] and figure out format.
                return Err(ErrorCode::InvalidOperation); // BasisU textures must be transcoded
                // before upload.
            }

            if self.is_compressed {
                // Unused.
                format_info.gl_format = GL_INVALID_VALUE;
                format_info.gl_type = GL_INVALID_VALUE;
                format_info.gl_base_internalformat = GL_INVALID_VALUE;
            } else {
                format_info.gl_format = vk_format2gl_format(self.vk_format);
                format_info.gl_type = vk_format2gl_type(self.vk_format);
                format_info.gl_base_internalformat = format_info.gl_internalformat;
                if format_info.gl_format == GL_INVALID_VALUE
                    || format_info.gl_type == GL_INVALID_VALUE
                {
                    return Err(ErrorCode::InvalidOperation);
                }
            }
            // KTX 2 files require an unpack alignment of 1. OGL default is 4.
            let st_ptr: *mut GlState = st;
            with_unpack_alignment(&*st_ptr, 1, || {
                ktx_texture_gl_upload_private(
                    &mut *st_ptr,
                    self,
                    &format_info,
                    p_texture,
                    p_target,
                    p_glerror,
                )
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
            match self {
                AnyTexture::Ktx2(t) => t.gl_upload(p_texture, p_target, p_glerror),
                AnyTexture::Ktx1(t) => t.gl_upload(p_texture, p_target, p_glerror),
            }
        }
    }
}

/// `R16Formats` discovered for the current context (`KTX_R16_FORMATS_*`); all formats before
/// the first upload. Exposed for tests of the capability discovery.
pub fn context_r16_formats() -> u32 {
    gl_state()
        .as_ref()
        .map_or(KTX_ALL_R16_FORMATS, |s| s.r16_formats)
}

/// Whether the loaded GL commands treat the context as GL ES (`_CONTEXT_ES_PROFILE_BIT`).
pub fn context_is_es() -> bool {
    gl_state()
        .as_ref()
        .is_some_and(|s| s.context_profile & CONTEXT_ES_PROFILE_BIT != 0)
}
