// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Port of KTX-Software `interface/js_binding` on wasm-bindgen. Each cargo feature is one
//! upstream Emscripten module (docs/spec/js_binding.md); the embind-compatible JS API is built
//! by `js/embind.js` and `js/<module>.js` on top of these exports.

#[cfg(feature = "msc")]
pub mod msc;

/// `{name: value, ...}` for one enum, turned into an embind-style enum by `js/embind.js`.
#[cfg(feature = "msc")]
fn enum_object(values: &[(&str, u32)]) -> wasm_bindgen::JsValue {
    use js_sys::{Object, Reflect};
    use wasm_bindgen::JsValue;
    let obj = Object::new();
    for (name, value) in values {
        // Setting a property on a fresh plain object cannot fail.
        let _ = Reflect::set(&obj, &JsValue::from_str(name), &JsValue::from(*value));
    }
    obj.into()
}
