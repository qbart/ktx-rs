// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! `std::thread::scope` for the encoders' worker threads. wasm has no threads, so there every
//! "spawned" job runs to completion on the calling thread, in spawn order. No job waits on
//! another (they only share work queues and write disjoint outputs), so the results match a
//! threaded run with the same thread count.

#[cfg(not(target_family = "wasm"))]
pub(crate) use std::thread::scope;

#[cfg(target_family = "wasm")]
pub(crate) struct Scope;

#[cfg(target_family = "wasm")]
pub(crate) struct ScopedJoinHandle<T>(T);

#[cfg(target_family = "wasm")]
impl Scope {
    pub(crate) fn spawn<T>(&self, f: impl FnOnce() -> T) -> ScopedJoinHandle<T> {
        ScopedJoinHandle(f())
    }
}

#[cfg(target_family = "wasm")]
impl<T> ScopedJoinHandle<T> {
    pub(crate) fn join(self) -> std::thread::Result<T> {
        Ok(self.0)
    }
}

#[cfg(target_family = "wasm")]
pub(crate) fn scope<T>(f: impl FnOnce(&Scope) -> T) -> T {
    f(&Scope)
}
