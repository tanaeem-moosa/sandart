//! Step 0 of "option 4" (SESSION-HANDOVER-2026-09-13.md §6): section timers for one
//! `DrawingSimulation::update()` tick, gated behind the `phase-timing` Cargo feature (see the
//! feature's own doc comment in `sandart-sim/Cargo.toml`).
//!
//! MEASUREMENT ONLY. Nothing in this module ever changes a physics value, a schedule decision,
//! or control flow -- every call site just brackets an existing, unmodified region of code with
//! a start/stop pair. With the feature OFF (always true for `sandart-wasm`, the shipped crate --
//! see the Cargo.toml comment on why `std::time::Instant` cannot be linked there), every function
//! below is a zero-argument-effect no-op that the compiler removes entirely: `now_ns()` returns a
//! constant `0`, `add()` discards both arguments, so a call site's `let t0 = now_ns(); ...;
//! add(SEC, now_ns() - t0);` has no observable effect and no live value crosses the dead code, so
//! LTO/inlining removes it. Verify with `cargo check -p sandart-wasm --target
//! wasm32-unknown-unknown --release` (this feature must never be enabled there) and by comparing
//! `diag_state_checksum` with the feature on vs. off (see the session's report for the numbers).
//!
//! ## Sections
//! Indices double as the reporting order. `fresh_active` and `settle_tick_total` are measured by
//! the call sites in `lib.rs::update()`, wrapping `physics::compute_fresh_active` and
//! `physics::settle_tick` from OUTSIDE (so neither function's internal control flow, including
//! its early returns, needs to change). Everything else is measured inside `settle_tick` itself,
//! at existing sequential boundaries in its body -- see the call sites for exactly which region
//! each index covers. `settle_tick_total` minus the sum of the internal sections is the residual
//! `settle_tick` spends on work this pass did not itemise (pre-phase-loop setup: lateral span
//! construction, the persistent head-field relaxation, per-tick scratch buffer setup; and the
//! phase loop's own non-phase-0 arbitrate+apply work, oversubscription bookkeeping, and tail
//! bookkeeping after the phase loop) -- report it, don't hide it.
//!
//! A caller-side "remainder of update()" (the marble-displacement handling, the "perfect
//! simulation" bypass, and the `has_active` gate -- everything in `update()` outside
//! `compute_fresh_active`/`settle_tick`) is NOT its own section here: it is derived by the bench
//! harness as `whole_tick_wall_ns - fresh_active_ns - settle_tick_total_ns`, using the harness's
//! own wall-clock bracket around the whole `sim.update(...)` call plus this module's `snapshot()`.

pub const SEC_FRESH_ACTIVE: usize = 0;
pub const SEC_CLASSIFICATION: usize = 1;
pub const SEC_TEMP_HEIGHTS_COPY: usize = 2;
pub const SEC_PHASE0_COLLECT: usize = 3;
pub const SEC_PHASE0_APPLY: usize = 4;
pub const SEC_PHASE1_TRAVERSAL: usize = 5;
pub const SEC_LATERAL_EDGE_PASS: usize = 6;
pub const SEC_COPY_BACK: usize = 7;
pub const SEC_SETTLE_TICK_TOTAL: usize = 8;
pub const N_SECTIONS: usize = 9;

pub const SECTION_NAMES: [&str; N_SECTIONS] = [
    "fresh_active",
    "classification",
    "temp_heights_copy",
    "phase0_collect",
    "phase0_apply",
    "phase1_traversal",
    "lateral_edge_pass",
    "copy_back",
    "settle_tick_total",
];

#[cfg(feature = "phase-timing")]
mod imp {
    use super::N_SECTIONS;
    use std::cell::Cell;

    // On wasm32, `std::time::Instant::now()` panics ("time not implemented on this platform")
    // unless a platform shim is linked in -- there is none in a bare `wasm32-unknown-unknown`
    // cdylib built without wasm-bindgen. This feature must never reach that target inside
    // `sandart-wasm` (Cargo.toml's own comment; verified by `cargo check -p sandart-wasm --target
    // wasm32-unknown-unknown --release` with the feature left off there), but the STANDALONE
    // bench crate that exercises this module under node DOES build for wasm32 with the feature
    // on, so this module needs a clock that works on both targets rather than assuming native.
    #[cfg(target_arch = "wasm32")]
    unsafe extern "C" {
        // Provided by the bench harness's JS driver as `performance.now()` (milliseconds, a
        // monotonic `f64`) -- see `sandart-phase-bench/bench_wasm.mjs`. Never imported into the
        // shipped `sandart-wasm` module: that crate never enables `phase-timing`, so this
        // `extern` block is never compiled as part of it.
        fn phase_timing_now_ms() -> f64;
    }

    #[cfg(target_arch = "wasm32")]
    #[inline]
    fn now_ns() -> u64 {
        (unsafe { phase_timing_now_ms() } * 1_000_000.0) as u64
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    fn now_ns() -> u64 {
        // A fixed process-lifetime epoch rather than calling `Instant::now()` twice per section
        // and subtracting `Duration`s: this way `now_ns()` has the same `() -> u64` shape on both
        // targets, so call sites in `physics.rs`/`lib.rs` don't need target-specific code.
        thread_local! {
            static EPOCH: std::time::Instant = std::time::Instant::now();
        }
        EPOCH.with(|e| e.elapsed().as_nanos() as u64)
    }

    thread_local! {
        static TOTALS: Cell<[u64; N_SECTIONS]> = const { Cell::new([0u64; N_SECTIONS]) };
    }

    #[inline]
    pub fn start() -> u64 {
        now_ns()
    }

    #[inline]
    pub fn add(section: usize, t0: u64) {
        let dt = now_ns().saturating_sub(t0);
        TOTALS.with(|c| {
            let mut t = c.get();
            t[section] += dt;
            c.set(t);
        });
    }

    pub fn reset_tick() {
        TOTALS.with(|c| c.set([0u64; N_SECTIONS]));
    }

    pub fn snapshot() -> [u64; N_SECTIONS] {
        TOTALS.with(|c| c.get())
    }
}

#[cfg(not(feature = "phase-timing"))]
mod imp {
    use super::N_SECTIONS;

    #[inline(always)]
    pub fn start() -> u64 {
        0
    }

    #[inline(always)]
    pub fn add(_section: usize, _t0: u64) {}

    #[inline(always)]
    pub fn reset_tick() {}

    #[inline(always)]
    pub fn snapshot() -> [u64; N_SECTIONS] {
        [0u64; N_SECTIONS]
    }
}

pub use imp::{add, reset_tick, snapshot, start};
