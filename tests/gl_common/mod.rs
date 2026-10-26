//! Headless EGL (Mesa surfaceless platform) contexts for the GL loader tests.
#![allow(dead_code, unsafe_code, clippy::panic)]

use khronos_egl as egl;
use std::ffi::c_void;

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

pub struct Gl {
    pub egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
}

impl Gl {
    /// A current context: desktop core 3.3+ (`es == false`) or GL ES 3.0.
    pub fn new(es: bool) -> Gl {
        unsafe {
            let egl = egl::DynamicInstance::<egl::EGL1_5>::load_required()
                .unwrap_or_else(|e| panic!("libEGL: {e}"));
            let display = egl
                .get_platform_display(
                    PLATFORM_SURFACELESS_MESA,
                    egl::DEFAULT_DISPLAY,
                    &[egl::ATTRIB_NONE],
                )
                .unwrap_or_else(|e| panic!("eglGetPlatformDisplay: {e}"));
            egl.initialize(display)
                .unwrap_or_else(|e| panic!("eglInitialize: {e}"));
            egl.bind_api(if es {
                egl::OPENGL_ES_API
            } else {
                egl::OPENGL_API
            })
            .unwrap_or_else(|e| panic!("eglBindAPI: {e}"));
            let renderable = if es {
                egl::OPENGL_ES3_BIT
            } else {
                egl::OPENGL_BIT
            };
            let config = egl
                .choose_first_config(
                    display,
                    &[
                        egl::RENDERABLE_TYPE,
                        renderable,
                        egl::SURFACE_TYPE,
                        egl::PBUFFER_BIT,
                        egl::NONE,
                    ],
                )
                .unwrap_or_else(|e| panic!("eglChooseConfig: {e}"))
                .unwrap_or_else(|| panic!("no EGL config"));
            let attrs = if es {
                vec![egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE]
            } else {
                vec![
                    egl::CONTEXT_MAJOR_VERSION,
                    3,
                    egl::CONTEXT_MINOR_VERSION,
                    3,
                    egl::CONTEXT_OPENGL_PROFILE_MASK,
                    egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                    egl::NONE,
                ]
            };
            let context = egl
                .create_context(display, config, None, &attrs)
                .unwrap_or_else(|e| panic!("eglCreateContext: {e}"));
            egl.make_current(display, None, None, Some(context))
                .unwrap_or_else(|e| panic!("eglMakeCurrent: {e}"));
            Gl {
                egl,
                display,
                context,
            }
        }
    }

    pub fn proc_address(&self, name: &str) -> *const c_void {
        self.egl
            .get_proc_address(name)
            .map_or(std::ptr::null(), |f| f as *const c_void)
    }

    /// A GL command by name, as type `T`.
    pub fn func<T: Copy>(&self, name: &str) -> T {
        let p = self.proc_address(name);
        assert!(!p.is_null(), "{name} not found");
        unsafe { std::mem::transmute_copy::<*const c_void, T>(&p) }
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
