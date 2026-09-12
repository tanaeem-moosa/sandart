//! `extern "C"` exports for the node timing runner (`bench_wasm.mjs`). No `wasm-bindgen` --
//! see the task's own suggestion ("a cdylib with extern "C" exports ... no wasm-bindgen needed").
//!
//! Protocol per kernel (`r`, `a`, `b`):
//!   1. JS calls `alloc(len)` to get a pointer into wasm linear memory, writes the snapshot
//!      file's raw bytes there (`new Uint8Array(memory.buffer, ptr, len).set(data)`).
//!   2. JS calls `init_<k>(ptr, len)`, which parses those bytes (`snapshot::parse`) into an
//!      owned, correctly-aligned `State` plus that kernel's scratch buffers, and keeps a copy of
//!      the raw bytes for `reset_<k>`.
//!   3. JS calls `run_<k>()` repeatedly, bracketing each call with `performance.now()` for the
//!      timing samples. Each call mutates the loaded state in place (this is also how the
//!      "after 200 repeated passes" equivalence numbers were produced, natively -- see
//!      `native_bench.rs`).
//!   4. `reset_<k>()` re-parses the original bytes, undoing every mutation, for a fresh
//!      single-pass timing/equivalence run.
//!   5. `cell_count_<k>()` returns the denominator for ns/cell/pass.

use crate::snapshot::{self, State};
use crate::{kernel_a, kernel_b, kernel_r};
use std::cell::RefCell;

struct Loaded<S> {
    bytes: Vec<u8>,
    state: State,
    scratch: S,
}

thread_local! {
    static R: RefCell<Option<Loaded<kernel_r::Scratch>>> = RefCell::new(None);
    static A: RefCell<Option<Loaded<kernel_a::Scratch>>> = RefCell::new(None);
    static B: RefCell<Option<Loaded<kernel_b::Scratch>>> = RefCell::new(None);
}

/// Allocates `len` bytes inside wasm linear memory and leaks them to the caller; paired with
/// `init_*`, which reclaims exactly this allocation via `Vec::from_raw_parts`.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf: Vec<u8> = vec![0u8; len];
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr`/`len` must be exactly the pointer and length most recently returned by `alloc`, not yet
/// reclaimed.
unsafe fn take_bytes(ptr: *mut u8, len: usize) -> Vec<u8> {
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_r(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_r::Scratch::new(state.w, state.h);
    R.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_a(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_a::Scratch::new(&state);
    A.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_b(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_b::Scratch::new(&state);
    B.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub extern "C" fn run_r() -> f64 {
    R.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_r not called");
        kernel_r::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_a() -> f64 {
    A.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_a not called");
        kernel_a::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_b() -> f64 {
    B.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_b not called");
        kernel_b::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_r() {
    R.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_r not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_a() {
    A.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_a not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_b() {
    B.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_b not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_r() -> u32 {
    R.with(|c| kernel_r::simulated_cell_count(&c.borrow().as_ref().expect("init_r not called").state) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_a() -> u32 {
    A.with(|c| kernel_a::simulated_cell_count(&c.borrow().as_ref().expect("init_a not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_b() -> u32 {
    B.with(|c| kernel_b::simulated_cell_count(&c.borrow().as_ref().expect("init_b not called").scratch) as u32)
}
