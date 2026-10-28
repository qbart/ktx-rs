// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! The GL independent part of `lib/src/glloader.c`: context capability discovery and
//! `ktxTexture*_GLUpload` over a [`GlCommands`] implementation. The native OpenGL {,ES}
//! commands live in `glloader` (feature `opengl`); the JS binding provides WebGL through
//! Emscripten-like GL emulation. The capability statics of C are a [`GlContextCaps`] owned by
//! the caller (a process global in both users, as in C).

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

pub const GL_NO_ERROR: GLenum = 0;
pub const GL_INVALID_ENUM: GLenum = 0x0500;
pub const GL_INVALID_VALUE_ERROR: GLenum = 0x0501;
pub const GL_INVALID_OPERATION: GLenum = 0x0502;
pub const GL_TEXTURE_1D: GLenum = 0x0DE0;
pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
pub const GL_TEXTURE_3D: GLenum = 0x806F;
pub const GL_TEXTURE_1D_ARRAY: GLenum = 0x8C18;
pub const GL_TEXTURE_2D_ARRAY: GLenum = 0x8C1A;
pub const GL_TEXTURE_CUBE_MAP: GLenum = 0x8513;
pub const GL_TEXTURE_CUBE_MAP_POSITIVE_X: GLenum = 0x8515;
pub const GL_TEXTURE_CUBE_MAP_ARRAY: GLenum = 0x9009;
pub const GL_TEXTURE_MAX_LEVEL: GLenum = 0x813D;
pub const GL_GENERATE_MIPMAP: GLenum = 0x8191;
pub const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub const GL_VERSION: GLenum = 0x1F02;
pub const GL_EXTENSIONS: GLenum = 0x1F03;
pub const GL_NUM_EXTENSIONS: GLenum = 0x821D;
pub const GL_MAJOR_VERSION: GLenum = 0x821B;
pub const GL_MINOR_VERSION: GLenum = 0x821C;
pub const GL_CONTEXT_PROFILE_MASK: GLenum = 0x9126;
const GL_CONTEXT_CORE_PROFILE_BIT: GLint = 0x1;
const GL_CONTEXT_COMPATIBILITY_PROFILE_BIT: GLint = 0x2;
const GL_LUMINANCE: GLenum = 0x1909; // deprecated
const GL_LUMINANCE_ALPHA: GLenum = 0x190A; // deprecated
const GL_INTENSITY: GLenum = 0x8049; // deprecated
const GL_TRUE: GLint = 1;

pub(crate) const CONTEXT_ES_PROFILE_BIT: GLint = 0x4;
/// Non-legacy sized formats are supported.
const NON_LEGACY_FORMATS: GLint = 0x1;
/// Legacy sized formats are supported.
const LEGACY_FORMATS: GLint = 0x2;
const ALL_SIZED_FORMATS: GLint = NON_LEGACY_FORMATS | LEGACY_FORMATS;
/// No sized formats are supported.
const NO_SIZED_FORMATS: GLint = 0;
const KTX_GL_UNPACK_ALIGNMENT: GLint = 4;

/// The optional commands of `gl_funclist.inl` the upload checks for (`gl.glX != NULL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlOptional {
    GetStringi,
    TexImage1D,
    CompressedTexImage1D,
    TexImage3D,
    CompressedTexImage3D,
    CompressedTexSubImage3D,
    TexSubImage3D,
    GenerateMipmap,
    TexStorage1D,
    TexStorage2D,
    TexStorage3D,
}

/// The GL {,ES} commands used by `ktxTexture_GLUpload`, on the context the caller made
/// current. Optional commands are only called when [`GlCommands::has`] reports them. Image
/// data is passed as the slice C passes as a pointer.
pub trait GlCommands {
    /// Whether an optional command was loaded.
    fn has(&self, cmd: GlOptional) -> bool;
    fn bind_texture(&self, target: GLenum, texture: GLuint);
    #[allow(clippy::too_many_arguments)]
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
    );
    /// `glDeleteTextures(1, &texture)`.
    fn delete_texture(&self, texture: GLuint);
    /// `glGenTextures(1, &texture)`.
    fn gen_texture(&self) -> GLuint;
    fn get_error(&self) -> GLenum;
    fn get_integerv(&self, pname: GLenum, data: &mut GLint);
    /// `glGetString`; "" for a null result.
    fn get_string(&self, name: GLenum) -> String;
    fn pixel_storei(&self, pname: GLenum, param: GLint);
    #[allow(clippy::too_many_arguments)]
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
    );
    fn tex_parameteri(&self, target: GLenum, pname: GLenum, param: GLint);

    /// `glGetStringi`; `None` for a null result.
    fn get_stringi(&self, _name: GLenum, _index: GLuint) -> Option<String> {
        None
    }
    fn generate_mipmap(&self, _target: GLenum) {}
    #[allow(clippy::too_many_arguments)]
    fn tex_image_1d(
        &self,
        _target: GLenum,
        _level: GLint,
        _internalformat: GLint,
        _width: GLsizei,
        _border: GLint,
        _format: GLenum,
        _ty: GLenum,
        _pixels: &[u8],
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    fn compressed_tex_image_1d(
        &self,
        _target: GLenum,
        _level: GLint,
        _internalformat: GLenum,
        _width: GLsizei,
        _border: GLint,
        _image_size: GLsizei,
        _data: &[u8],
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    fn tex_image_3d(
        &self,
        _target: GLenum,
        _level: GLint,
        _internalformat: GLint,
        _width: GLsizei,
        _height: GLsizei,
        _depth: GLsizei,
        _border: GLint,
        _format: GLenum,
        _ty: GLenum,
        _pixels: &[u8],
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    fn compressed_tex_image_3d(
        &self,
        _target: GLenum,
        _level: GLint,
        _internalformat: GLenum,
        _width: GLsizei,
        _height: GLsizei,
        _depth: GLsizei,
        _border: GLint,
        _image_size: GLsizei,
        _data: &[u8],
    ) {
    }
}

/// The `discoverContextCapabilities` statics of `glloader.c`, plus the optional commands it
/// zeroes in `gl` for old ES contexts.
#[derive(Debug, Clone)]
pub struct GlContextCaps {
    pub(crate) context_profile: GLint,
    pub(crate) sized_formats: GLint,
    pub(crate) supports_swizzle: bool,
    pub(crate) r16_formats: u32,
    pub(crate) supports_srgb: bool,
    pub(crate) supports_cube_map_arrays: bool,
    pub(crate) supports_max_level: bool,
    zeroed: Vec<GlOptional>,
}

impl Default for GlContextCaps {
    fn default() -> Self {
        GlContextCaps {
            context_profile: 0,
            sized_formats: ALL_SIZED_FORMATS,
            supports_swizzle: true,
            r16_formats: KTX_ALL_R16_FORMATS,
            supports_srgb: true,
            supports_cube_map_arrays: false,
            supports_max_level: false,
            zeroed: Vec::new(),
        }
    }
}

impl GlContextCaps {
    /// `R16Formats` (`KTX_R16_FORMATS_*`).
    pub fn r16_formats(&self) -> u32 {
        self.r16_formats
    }

    /// Whether the context is GL ES (`_CONTEXT_ES_PROFILE_BIT`).
    pub fn is_es(&self) -> bool {
        self.context_profile & CONTEXT_ES_PROFILE_BIT != 0
    }

    /// `gl.<cmd> != NULL`.
    fn has(&self, gl: &dyn GlCommands, cmd: GlOptional) -> bool {
        !self.zeroed.contains(&cmd) && gl.has(cmd)
    }

    fn zero(&mut self, cmd: GlOptional) {
        self.zeroed.push(cmd);
    }

    /// `hasExtension`.
    fn has_extension(&self, gl: &dyn GlCommands, extension: &str) -> bool {
        if !self.has(gl, GlOptional::GetStringi) {
            gl.get_string(GL_EXTENSIONS).contains(extension)
        } else {
            let mut n = 0;
            gl.get_integerv(GL_NUM_EXTENSIONS, &mut n);
            (0..n.max(0) as GLuint)
                .any(|i| gl.get_stringi(GL_EXTENSIONS, i).as_deref() == Some(extension))
        }
    }

    /// `discoverContextCapabilities`.
    fn discover_context_capabilities(&mut self, gl: &dyn GlCommands) {
        let mut major_version: GLint = 1;
        let mut minor_version: GLint = 0;

        let version = gl.get_string(GL_VERSION);
        if version.contains("GL ES") {
            self.context_profile = CONTEXT_ES_PROFILE_BIT;
        }
        // MAJOR & MINOR only introduced in GL {,ES} 3.0
        gl.get_integerv(GL_MAJOR_VERSION, &mut major_version);
        gl.get_integerv(GL_MINOR_VERSION, &mut minor_version);
        if gl.get_error() != GL_NO_ERROR {
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
                self.zero(GlOptional::GetStringi);
                self.zero(GlOptional::CompressedTexImage1D);
                self.zero(GlOptional::TexStorage1D);
                self.zero(GlOptional::TexStorage2D);
                self.zero(GlOptional::TexStorage3D);
                if !self.has_extension(gl, "GL_OES_texture_3D") {
                    self.zero(GlOptional::CompressedTexImage3D);
                    self.zero(GlOptional::CompressedTexSubImage3D);
                    self.zero(GlOptional::TexImage3D);
                    self.zero(GlOptional::TexSubImage3D);
                }
                if major_version < 2 {
                    self.zero(GlOptional::GenerateMipmap);
                }
            } else {
                self.sized_formats = NON_LEGACY_FORMATS;
                if self.has_extension(gl, "GL_EXT_texture_cube_map_array") {
                    self.supports_cube_map_arrays = true;
                }
                self.supports_max_level = true;
            }
            if self.has_extension(gl, "GL_OES_required_internalformat") {
                self.sized_formats |= ALL_SIZED_FORMATS;
            }
            // There are no OES extensions for sRGB textures or R16 formats.
        } else {
            // PROFILE_MASK was introduced in OpenGL 3.2.
            // Profiles: CONTEXT_CORE_PROFILE_BIT 0x1,
            //           CONTEXT_COMPATIBILITY_PROFILE_BIT 0x2.
            let mut profile = self.context_profile;
            gl.get_integerv(GL_CONTEXT_PROFILE_MASK, &mut profile);
            self.context_profile = profile;
            if gl.get_error() == GL_NO_ERROR {
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
                if major_version < 2 && !self.has_extension(gl, "GL_EXT_texture_sRGB") {
                    self.supports_srgb = false;
                }
                // R{,G]16 introduced in 3.0; R{,G}16_SNORM introduced in 3.1.
                if major_version == 3 {
                    if minor_version == 0 {
                        self.r16_formats &= !KTX_R16_FORMATS_SNORM;
                    }
                    if minor_version < 1 {
                        if self.has_extension(gl, "GL_ARB_texture_query_levels") {
                            self.supports_max_level = true;
                        }
                    } else {
                        self.supports_max_level = true;
                    }
                } else if self.has_extension(gl, "GL_ARB_texture_rg") {
                    self.r16_formats &= !KTX_R16_FORMATS_SNORM;
                } else {
                    self.r16_formats = 0;
                }
            }
            if !self.supports_cube_map_arrays
                && self.has_extension(gl, "GL_ARB_texture_cube_map_array")
            {
                self.supports_cube_map_arrays = true;
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

/// The image callbacks (`texImage1DCallback` ... `compressedTexImage3DCallback`).
#[allow(clippy::too_many_arguments)]
fn image_callback(
    gl: &dyn GlCommands,
    caps: &GlContextCaps,
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
    let glerror = match which {
        IterCallback::TexImage1D => {
            if !caps.has(gl, GlOptional::TexImage1D) {
                return missing;
            }
            gl.tex_image_1d(
                target,
                level,
                cb.gl_internalformat as GLint,
                width as GLsizei,
                0,
                cb.gl_format,
                cb.gl_type,
                pixels,
            );
            gl.get_error()
        }
        IterCallback::CompressedTexImage1D => {
            if !caps.has(gl, GlOptional::CompressedTexImage1D) {
                return missing;
            }
            gl.compressed_tex_image_1d(
                target,
                level,
                cb.gl_internalformat,
                width as GLsizei,
                0,
                size,
                pixels,
            );
            gl.get_error()
        }
        IterCallback::TexImage2D => {
            gl.tex_image_2d(
                target,
                level,
                cb.gl_internalformat as GLint,
                width as GLsizei,
                height_or_layers,
                0,
                cb.gl_format,
                cb.gl_type,
                pixels,
            );
            gl.get_error()
        }
        IterCallback::CompressedTexImage2D => {
            // It is simpler to just attempt to load the format, rather than divine which
            // formats are supported by the implementation. In the event of an error, software
            // unpacking can be attempted.
            gl.compressed_tex_image_2d(
                target,
                level,
                cb.gl_internalformat,
                width as GLsizei,
                height_or_layers,
                0,
                size,
                pixels,
            );
            let mut glerror = gl.get_error();
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
                    caps.r16_formats,
                    caps.supports_srgb,
                )?;
                let mut internalformat = unpacked.internal_format;
                if caps.sized_formats & NON_LEGACY_FORMATS == 0 {
                    if internalformat == GL_RGB8 {
                        internalformat = GL_RGB;
                    } else if internalformat == GL_RGBA8 {
                        internalformat = GL_RGBA;
                    }
                }
                gl.tex_image_2d(
                    target,
                    level,
                    internalformat as GLint,
                    width as GLsizei,
                    height_or_layers,
                    0,
                    unpacked.format,
                    unpacked.r#type,
                    &unpacked.image,
                );
                glerror = gl.get_error();
            }
            glerror
        }
        IterCallback::TexImage3D => {
            if !caps.has(gl, GlOptional::TexImage3D) {
                return missing;
            }
            gl.tex_image_3d(
                target,
                level,
                cb.gl_internalformat as GLint,
                width as GLsizei,
                height as GLsizei,
                depth_or_layers,
                0,
                cb.gl_format,
                cb.gl_type,
                pixels,
            );
            gl.get_error()
        }
        IterCallback::CompressedTexImage3D => {
            if !caps.has(gl, GlOptional::CompressedTexImage3D) {
                return missing;
            }
            gl.compressed_tex_image_3d(
                target,
                level,
                cb.gl_internalformat,
                width as GLsizei,
                height as GLsizei,
                depth_or_layers,
                0,
                size,
                pixels,
            );
            gl.get_error()
        }
    };
    cb.gl_error = glerror;
    if glerror == GL_NO_ERROR {
        Ok(())
    } else {
        Err(ErrorCode::GlError)
    }
}

/// Port of `ktxTexture_GLUploadPrivate`.
fn ktx_texture_gl_upload_private(
    gl: &dyn GlCommands,
    caps: &mut GlContextCaps,
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

    if caps.context_profile == 0 {
        caps.discover_context_capabilities(gl);
    }

    let user_texname = p_texture.as_deref().copied().filter(|&t| t != 0);
    let texname = match user_texname {
        Some(t) => t,
        None => gl.gen_texture(),
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
        && ((t.is_compressed && !caps.has(gl, GlOptional::CompressedTexImage1D))
            || (!t.is_compressed && !caps.has(gl, GlOptional::TexImage1D)))
    {
        return Err(ErrorCode::UnsupportedTextureType);
    }

    // Reject 3D texture if unsupported.
    if target == GL_TEXTURE_3D
        && ((t.is_compressed && !caps.has(gl, GlOptional::CompressedTexImage3D))
            || (!t.is_compressed && !caps.has(gl, GlOptional::TexImage3D)))
    {
        return Err(ErrorCode::UnsupportedTextureType);
    }

    // Reject cube map arrays if not supported.
    if target == GL_TEXTURE_CUBE_MAP_ARRAY && !caps.supports_cube_map_arrays {
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

    gl.bind_texture(target, texname);

    // Prefer glGenerateMipmaps over GL_GENERATE_MIPMAP
    let has_generate_mipmap = caps.has(gl, GlOptional::GenerateMipmap);
    if t.generate_mipmaps && !has_generate_mipmap {
        gl.tex_parameteri(target, GL_GENERATE_MIPMAP, GL_TRUE);
    }

    if !t.generate_mipmaps && caps.supports_max_level {
        gl.tex_parameteri(target, GL_TEXTURE_MAX_LEVEL, t.num_levels as GLint - 1);
    }

    cb_data.gl_target = if target == GL_TEXTURE_CUBE_MAP {
        GL_TEXTURE_CUBE_MAP_POSITIVE_X
    } else {
        target
    };

    if !t.is_compressed {
        // When no sized formats are supported, or legacy sized formats are not supported,
        // must change internal format.
        if caps.sized_formats == NO_SIZED_FORMATS
            || (caps.sized_formats & LEGACY_FORMATS == 0
                && (format_info.gl_base_internalformat == GL_ALPHA
                    || format_info.gl_base_internalformat == GL_LUMINANCE
                    || format_info.gl_base_internalformat == GL_LUMINANCE_ALPHA
                    || format_info.gl_base_internalformat == GL_INTENSITY))
        {
            cb_data.gl_internalformat = format_info.gl_base_internalformat;
        }
    }

    let (generate_mipmaps, active_stream) = (t.generate_mipmaps, t.is_active_stream());
    let caps_ref: &GlContextCaps = caps;
    let mut cb = |miplevel, face, w, h, d, size, pixels: &[u8]| {
        image_callback(
            gl,
            caps_ref,
            iter_cb,
            &mut cb_data,
            miplevel,
            face,
            w,
            h,
            d,
            size,
            pixels,
        )
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
        if generate_mipmaps && has_generate_mipmap {
            gl.generate_mipmap(target);
        }
        *p_target = target;
        if let Some(p) = p_texture {
            *p = texname;
        }
    } else if user_texname.is_none() {
        gl.delete_texture(texname);
    }
    result
}

/// Sets `GL_UNPACK_ALIGNMENT` to `alignment` around `f`, restoring the previous value.
fn with_unpack_alignment<R>(gl: &dyn GlCommands, alignment: GLint, f: impl FnOnce() -> R) -> R {
    let mut previous_unpack_alignment = 0;
    gl.get_integerv(GL_UNPACK_ALIGNMENT, &mut previous_unpack_alignment);
    if previous_unpack_alignment != alignment {
        gl.pixel_storei(GL_UNPACK_ALIGNMENT, alignment);
    }
    let r = f();
    // restore previous GL state
    if previous_unpack_alignment != alignment {
        gl.pixel_storei(GL_UNPACK_ALIGNMENT, previous_unpack_alignment);
    }
    r
}

impl Texture1 {
    /// `ktxTexture1_GLUpload` on `gl`: creates a GL texture object (named `*p_texture` when
    /// it is non-zero, otherwise generated and returned there) holding the images, and returns
    /// its target. On a GL error `p_glerror` receives it and `KTX_GL_ERROR` is returned.
    pub fn gl_upload_with(
        &mut self,
        gl: &dyn GlCommands,
        caps: &mut GlContextCaps,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
        let format_info = GlFormatInfo {
            gl_format: self.gl_format,
            gl_internalformat: self.gl_internalformat,
            gl_base_internalformat: self.gl_base_internalformat,
            gl_type: self.gl_type,
        };
        // KTX 1 files require an unpack alignment of 4
        with_unpack_alignment(gl, KTX_GL_UNPACK_ALIGNMENT, || {
            ktx_texture_gl_upload_private(
                gl,
                caps,
                self,
                &format_info,
                p_texture,
                p_target,
                p_glerror,
            )
        })
    }
}

impl Texture2 {
    /// `ktxTexture2_GLUpload` on `gl`. Basis Universal textures must be transcoded first
    /// (`KTX_INVALID_OPERATION`).
    pub fn gl_upload_with(
        &mut self,
        gl: &dyn GlCommands,
        caps: &mut GlContextCaps,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
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
            if format_info.gl_format == GL_INVALID_VALUE || format_info.gl_type == GL_INVALID_VALUE
            {
                return Err(ErrorCode::InvalidOperation);
            }
        }
        // KTX 2 files require an unpack alignment of 1. OGL default is 4.
        with_unpack_alignment(gl, 1, || {
            ktx_texture_gl_upload_private(
                gl,
                caps,
                self,
                &format_info,
                p_texture,
                p_target,
                p_glerror,
            )
        })
    }
}

impl AnyTexture {
    /// `ktxTexture_GLUpload` on `gl`.
    pub fn gl_upload_with(
        &mut self,
        gl: &dyn GlCommands,
        caps: &mut GlContextCaps,
        p_texture: Option<&mut GLuint>,
        p_target: &mut GLenum,
        p_glerror: Option<&mut GLenum>,
    ) -> KtxResult<()> {
        match self {
            AnyTexture::Ktx2(t) => t.gl_upload_with(gl, caps, p_texture, p_target, p_glerror),
            AnyTexture::Ktx1(t) => t.gl_upload_with(gl, caps, p_texture, p_target, p_glerror),
        }
    }
}
