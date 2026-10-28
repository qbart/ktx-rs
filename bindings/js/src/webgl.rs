// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `texture.glUpload` of `ktx_wrapper.cpp`: `ktxTexture_GLUpload` (the `glupload` core) on
//! the GL commands of `js/gl.js`, which emulates the part of Emscripten's WebGL backed GL
//! (`library_webgl.js`) that libktx uses, on the context made current through `Module.GL`.

use js_sys::Array;
use ktx::glupload::{GLenum, GLint, GLsizei, GLuint, GlCommands, GlContextCaps, GlOptional};
use ktx::ktx_error_string;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

use super::libktx::{Texture, log};

#[wasm_bindgen]
extern "C" {
    /// The `glX` functions of `js/gl.js` for the current context.
    pub type JsGl;

    #[wasm_bindgen(method, js_name = hasCommand)]
    fn has_command(this: &JsGl, name: &str) -> bool;
    #[wasm_bindgen(method, js_name = glBindTexture)]
    fn gl_bind_texture(this: &JsGl, target: GLenum, texture: GLuint);
    #[wasm_bindgen(method, js_name = glCompressedTexImage2D)]
    fn gl_compressed_tex_image_2d(
        this: &JsGl,
        target: GLenum,
        level: GLint,
        internalformat: GLenum,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: &[u8],
    );
    #[wasm_bindgen(method, js_name = glDeleteTexture)]
    fn gl_delete_texture(this: &JsGl, texture: GLuint);
    #[wasm_bindgen(method, js_name = glGenTexture)]
    fn gl_gen_texture(this: &JsGl) -> GLuint;
    #[wasm_bindgen(method, js_name = glGetError)]
    fn gl_get_error(this: &JsGl) -> GLenum;
    /// The value, or `undefined` when GL records an error (nothing is written).
    #[wasm_bindgen(method, js_name = glGetIntegerv)]
    fn gl_get_integerv(this: &JsGl, pname: GLenum) -> Option<GLint>;
    #[wasm_bindgen(method, js_name = glGetString)]
    fn gl_get_string(this: &JsGl, name: GLenum) -> Option<String>;
    #[wasm_bindgen(method, js_name = glGetStringi)]
    fn gl_get_stringi(this: &JsGl, name: GLenum, index: GLuint) -> Option<String>;
    #[wasm_bindgen(method, js_name = glPixelStorei)]
    fn gl_pixel_storei(this: &JsGl, pname: GLenum, param: GLint);
    #[wasm_bindgen(method, js_name = glTexImage2D)]
    fn gl_tex_image_2d(
        this: &JsGl,
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
    #[wasm_bindgen(method, js_name = glTexParameteri)]
    fn gl_tex_parameteri(this: &JsGl, target: GLenum, pname: GLenum, param: GLint);
    #[wasm_bindgen(method, js_name = glGenerateMipmap)]
    fn gl_generate_mipmap(this: &JsGl, target: GLenum);
    #[wasm_bindgen(method, js_name = glTexImage3D)]
    fn gl_tex_image_3d(
        this: &JsGl,
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
    );
    #[wasm_bindgen(method, js_name = glCompressedTexImage3D)]
    fn gl_compressed_tex_image_3d(
        this: &JsGl,
        target: GLenum,
        level: GLint,
        internalformat: GLenum,
        width: GLsizei,
        height: GLsizei,
        depth: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: &[u8],
    );
}

impl GlCommands for JsGl {
    fn has(&self, cmd: GlOptional) -> bool {
        let name = match cmd {
            GlOptional::GetStringi => "glGetStringi",
            GlOptional::TexImage1D => "glTexImage1D",
            GlOptional::CompressedTexImage1D => "glCompressedTexImage1D",
            GlOptional::TexImage3D => "glTexImage3D",
            GlOptional::CompressedTexImage3D => "glCompressedTexImage3D",
            GlOptional::CompressedTexSubImage3D => "glCompressedTexSubImage3D",
            GlOptional::TexSubImage3D => "glTexSubImage3D",
            GlOptional::GenerateMipmap => "glGenerateMipmap",
            GlOptional::TexStorage1D => "glTexStorage1D",
            GlOptional::TexStorage2D => "glTexStorage2D",
            GlOptional::TexStorage3D => "glTexStorage3D",
        };
        self.has_command(name)
    }
    fn bind_texture(&self, target: GLenum, texture: GLuint) {
        self.gl_bind_texture(target, texture);
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
        self.gl_compressed_tex_image_2d(
            target,
            level,
            internalformat,
            width,
            height,
            border,
            image_size,
            data,
        );
    }
    fn delete_texture(&self, texture: GLuint) {
        self.gl_delete_texture(texture);
    }
    fn gen_texture(&self) -> GLuint {
        self.gl_gen_texture()
    }
    fn get_error(&self) -> GLenum {
        self.gl_get_error()
    }
    fn get_integerv(&self, pname: GLenum, data: &mut GLint) {
        if let Some(v) = self.gl_get_integerv(pname) {
            *data = v;
        }
    }
    fn get_string(&self, name: GLenum) -> String {
        self.gl_get_string(name).unwrap_or_default()
    }
    fn pixel_storei(&self, pname: GLenum, param: GLint) {
        self.gl_pixel_storei(pname, param);
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
        self.gl_tex_image_2d(
            target,
            level,
            internalformat,
            width,
            height,
            border,
            format,
            ty,
            pixels,
        );
    }
    fn tex_parameteri(&self, target: GLenum, pname: GLenum, param: GLint) {
        self.gl_tex_parameteri(target, pname, param);
    }
    fn get_stringi(&self, name: GLenum, index: GLuint) -> Option<String> {
        self.gl_get_stringi(name, index)
    }
    fn generate_mipmap(&self, target: GLenum) {
        self.gl_generate_mipmap(target);
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
        self.gl_tex_image_3d(
            target,
            level,
            internalformat,
            width,
            height,
            depth,
            border,
            format,
            ty,
            pixels,
        );
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
        self.gl_compressed_tex_image_3d(
            target,
            level,
            internalformat,
            width,
            height,
            depth,
            border,
            image_size,
            data,
        );
    }
}

thread_local! {
    /// The `discoverContextCapabilities` statics (per module instance, as in the wasm C build).
    static CAPS: RefCell<GlContextCaps> = RefCell::new(GlContextCaps::default());
}

#[wasm_bindgen(js_class = texture)]
impl Texture {
    /// `ktxTexture_GLUpload(texture, &texname, &target, &error)` with `texname = 0`: returns
    /// `[texname, target, error]`; `js/libktx_common.js` turns the name into the WebGLTexture.
    pub fn _gl_upload(&mut self, gl: &JsGl) -> Array {
        let (mut texname, mut target, mut error) = (0u32, 0u32, 0u32);
        let result = match &mut self.ptr {
            Some(t) => CAPS.with(|caps| match caps.try_borrow_mut() {
                Ok(mut caps) => t.gl_upload_with(
                    gl,
                    &mut caps,
                    Some(&mut texname),
                    &mut target,
                    Some(&mut error),
                ),
                Err(_) => Err(ktx::ErrorCode::InvalidOperation),
            }),
            None => Err(ktx::ErrorCode::InvalidValue),
        };
        if let Err(e) = result {
            log(&format!(
                "ERROR: Failed to GL upload: {}",
                ktx_error_string(e)
            ));
        }
        [texname, target, error]
            .iter()
            .map(|&v| JsValue::from(v))
            .collect()
    }
}
