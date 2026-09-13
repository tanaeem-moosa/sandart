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
use crate::{kernel_a, kernel_b, kernel_c, kernel_c8, kernel_d, kernel_e, kernel_e2, kernel_r};
use std::cell::RefCell;

struct Loaded<S> {
    bytes: Vec<u8>,
    state: State,
    scratch: S,
}

/// Same idea as `Loaded`, but for kernel E's own `StateE` (SoA, double-buffered) instead of the
/// snapshot's AoS `State`. Two independent instances (`E`/`E_RECIP`) so E-recip's separate
/// `stage45_e_recip` run doesn't disturb E's own timed state.
struct LoadedE {
    bytes: Vec<u8>,
    state: kernel_e::StateE,
    scratch: kernel_e::Scratch,
}

/// Same idea, for kernel E2's `Scratch` (D-sized/D-shaped temporaries over `kernel_e::StateE`).
/// These exports exist only so E2's stage functions survive dead-code elimination in the wasm
/// build for the v128 op count -- no wasm timing is taken for E2 (see native_bench's own timing,
/// which is authoritative for this round).
struct LoadedE2 {
    bytes: Vec<u8>,
    state: kernel_e::StateE,
    scratch: kernel_e2::Scratch,
}

thread_local! {
    static R: RefCell<Option<Loaded<kernel_r::Scratch>>> = RefCell::new(None);
    static A: RefCell<Option<Loaded<kernel_a::Scratch>>> = RefCell::new(None);
    static B: RefCell<Option<Loaded<kernel_b::Scratch>>> = RefCell::new(None);
    // kernel_c8 reuses kernel_c's Scratch type verbatim (see kernel_c8.rs's module doc comment) --
    // only run_pass differs.
    static C: RefCell<Option<Loaded<kernel_c::Scratch>>> = RefCell::new(None);
    static C8: RefCell<Option<Loaded<kernel_c::Scratch>>> = RefCell::new(None);
    static D: RefCell<Option<Loaded<kernel_d::Scratch>>> = RefCell::new(None);
    static E: RefCell<Option<LoadedE>> = RefCell::new(None);
    static E_RECIP: RefCell<Option<LoadedE>> = RefCell::new(None);
    static E2: RefCell<Option<LoadedE2>> = RefCell::new(None);
    // Kernel F (hypothesis-1 hybrid, `kernel_e2::run_pass_f`) -- same state/scratch shape as E2,
    // separate slot so timing F doesn't disturb E2's own loaded state.
    static F: RefCell<Option<LoadedE2>> = RefCell::new(None);
    // A bare `State` (no scratch/no full Scratch::new) so `run_precompute_c` can be timed
    // repeatedly from JS (bracketed with `performance.now()`, same as every `run_*` export) as
    // its own isolated cost -- see kernel_c.rs's module doc comment point 2 on why this is
    // reported separately from ns/cell/pass rather than folded into `init_c`.
    static PRECOMPUTE_STATE: RefCell<Option<State>> = RefCell::new(None);
    // Same idea for D's precompute, but it also needs the span list (it only visits span cells --
    // see `kernel_d::precompute_head_static_d`), built once here so repeated timed calls don't pay
    // `build_spans` too.
    static PRECOMPUTE_STATE_D: RefCell<Option<(State, Vec<crate::row_span::Span>)>> = RefCell::new(None);
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
pub unsafe extern "C" fn init_c(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_c::Scratch::new(&state);
    C.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_c8(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_c::Scratch::new(&state);
    C8.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_d(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let scratch = kernel_d::Scratch::new(&state);
    D.with(|c| *c.borrow_mut() = Some(Loaded { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_e(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let raw = snapshot::parse(&bytes);
    let state = kernel_e::StateE::from_state(&raw);
    let scratch = kernel_e::Scratch::new(&state);
    E.with(|c| *c.borrow_mut() = Some(LoadedE { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_e_recip(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let raw = snapshot::parse(&bytes);
    let state = kernel_e::StateE::from_state(&raw);
    let scratch = kernel_e::Scratch::new(&state);
    E_RECIP.with(|c| *c.borrow_mut() = Some(LoadedE { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_e2(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let raw = snapshot::parse(&bytes);
    let state = kernel_e::StateE::from_state(&raw);
    let scratch = kernel_e2::Scratch::new(&state);
    E2.with(|c| *c.borrow_mut() = Some(LoadedE2 { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_f(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let raw = snapshot::parse(&bytes);
    let state = kernel_e::StateE::from_state(&raw);
    let scratch = kernel_e2::Scratch::new(&state);
    F.with(|c| *c.borrow_mut() = Some(LoadedE2 { bytes, state, scratch }));
}

#[unsafe(no_mangle)]
pub extern "C" fn run_f() -> f64 {
    F.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_f not called");
        kernel_e2::run_pass_f(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_f() {
    F.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_f not called");
        let raw = snapshot::parse(&l.bytes);
        l.state = kernel_e::StateE::from_state(&raw);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_f() -> u32 {
    F.with(|c| kernel_e2::simulated_cell_count(&c.borrow().as_ref().expect("init_f not called").scratch) as u32)
}

/// Loads a bare `State` (no `Scratch`, so no precompute has happened yet) for
/// `run_precompute_c` to repeatedly precompute FROM, isolating that one-time cost from
/// `init_c`'s full `Scratch::new` (which also builds `row_span`'s spans and the noise table).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_precompute_c(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    PRECOMPUTE_STATE.with(|c| *c.borrow_mut() = Some(state));
}

/// Runs ONLY `kernel_c::precompute_head_static`, timed by bracketing this call with
/// `performance.now()` from JS exactly like `run_r`/`run_a`/`run_b`/`run_c`. Returns the first
/// output element so the whole computation cannot be dead-code-eliminated as an unused result.
#[unsafe(no_mangle)]
pub extern "C" fn run_precompute_c() -> f32 {
    PRECOMPUTE_STATE.with(|c| {
        let b = c.borrow();
        let state = b.as_ref().expect("init_precompute_c not called");
        let out = kernel_c::precompute_head_static(state);
        out[0]
    })
}

/// D's precompute analogue of `init_precompute_c`/`run_precompute_c`: loads a bare `State` plus
/// its span list (no full `Scratch::new`), so `run_precompute_d` can be timed repeatedly in
/// isolation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_precompute_d(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take_bytes(ptr, len) };
    let state = snapshot::parse(&bytes);
    let spans = crate::row_span::build_spans(&state);
    PRECOMPUTE_STATE_D.with(|c| *c.borrow_mut() = Some((state, spans)));
}

/// Runs ONLY `kernel_d::precompute_head_static_d`, timed by bracketing this call with
/// `performance.now()` from JS exactly like `run_precompute_c`. Returns the first output element
/// so the computation cannot be dead-code-eliminated as unused.
#[unsafe(no_mangle)]
pub extern "C" fn run_precompute_d() -> f32 {
    PRECOMPUTE_STATE_D.with(|c| {
        let b = c.borrow();
        let (state, spans) = b.as_ref().expect("init_precompute_d not called");
        let out = kernel_d::precompute_head_static_d(state, spans);
        out[0]
    })
}

/// Denominator for `run_precompute_d`'s ns/cell cost: the number of cells the precompute actually
/// visits (simulated cells + each span's acceptor column), NOT `w*h`.
#[unsafe(no_mangle)]
pub extern "C" fn precompute_cell_count_d() -> u32 {
    PRECOMPUTE_STATE_D.with(|c| {
        let b = c.borrow();
        let (_, spans) = b.as_ref().expect("init_precompute_d not called");
        kernel_d::precompute_cell_count(spans) as u32
    })
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
pub extern "C" fn run_c() -> f64 {
    C.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_c not called");
        kernel_c::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_c8() -> f64 {
    C8.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_c8 not called");
        kernel_c8::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d() -> f64 {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_pass(&mut l.state, &mut l.scratch)
    })
}

/// D's five stages, exposed individually so `bench_wasm.mjs` can bracket each with
/// `performance.now()` in sequence -- together they are exactly one `run_d()` (see
/// `kernel_d::run_pass`), so calling all five per iteration in order reproduces the same state
/// evolution as `run_d` while attributing time per stage (task: "For wasm, split stages into
/// `#[inline(never)]` functions and time them from JS").
#[unsafe(no_mangle)]
pub extern "C" fn run_e() -> f64 {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e_recip() -> f64 {
    E_RECIP.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e_recip not called");
        kernel_e::run_pass_recip(&mut l.state, &mut l.scratch)
    })
}

/// E's five stages, exposed individually so `bench_wasm.mjs` can bracket each with
/// `performance.now()` in sequence -- same rationale as D's `run_d_*` exports.
#[unsafe(no_mangle)]
pub extern "C" fn run_e_precompute() {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_precompute(&l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e_stage2() {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_stage2(&mut l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e_stage3() -> f64 {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_stage3(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e_stage45() {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_stage45(&mut l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e_swap() {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        kernel_e::run_swap(&mut l.state);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2() -> f64 {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_pass(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_recip() -> f64 {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_pass_recip(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_stage1() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_stage1(&l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_stage2() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_stage2(&l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_stage3() -> f64 {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_stage3(&mut l.state, &mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_stage45() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_stage45(&mut l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_stage45_recip() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e2::run_stage45_recip(&mut l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_e2_swap() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        kernel_e::run_swap(&mut l.state);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_e2() {
    E2.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e2 not called");
        let raw = snapshot::parse(&l.bytes);
        l.state = kernel_e::StateE::from_state(&raw);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_e2() -> u32 {
    E2.with(|c| kernel_e2::simulated_cell_count(&c.borrow().as_ref().expect("init_e2 not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d_copy_in() {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_copy_in(&l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d_stage2() {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_stage2(&l.state, &mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d_stage3() -> f64 {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_stage3(&mut l.scratch)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d_stage45() {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_stage45(&mut l.scratch);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn run_d_copy_out() {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        kernel_d::run_copy_out(&mut l.state, &l.scratch);
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
pub extern "C" fn reset_c() {
    C.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_c not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_c8() {
    C8.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_c8 not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_d() {
    D.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_d not called");
        l.state = snapshot::parse(&l.bytes);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_e() {
    E.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e not called");
        let raw = snapshot::parse(&l.bytes);
        l.state = kernel_e::StateE::from_state(&raw);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_e_recip() {
    E_RECIP.with(|c| {
        let mut b = c.borrow_mut();
        let l = b.as_mut().expect("init_e_recip not called");
        let raw = snapshot::parse(&l.bytes);
        l.state = kernel_e::StateE::from_state(&raw);
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

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_c() -> u32 {
    C.with(|c| kernel_c::simulated_cell_count(&c.borrow().as_ref().expect("init_c not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_c8() -> u32 {
    C8.with(|c| kernel_c8::simulated_cell_count(&c.borrow().as_ref().expect("init_c8 not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_d() -> u32 {
    D.with(|c| kernel_d::simulated_cell_count(&c.borrow().as_ref().expect("init_d not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_e() -> u32 {
    E.with(|c| kernel_e::simulated_cell_count(&c.borrow().as_ref().expect("init_e not called").scratch) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cell_count_e_recip() -> u32 {
    E_RECIP.with(|c| kernel_e::simulated_cell_count(&c.borrow().as_ref().expect("init_e_recip not called").scratch) as u32)
}
