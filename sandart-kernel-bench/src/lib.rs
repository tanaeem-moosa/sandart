//! Benchmark harness for `sandart-sim`'s lateral (cross-gravity) flux edge -- the ~49% of tick
//! time `artifacts/design/LOD-CENSUS-2026-09-12.md` attributes to the extra `lateral_substeps`
//! passes. Three implementations of exactly ONE lateral pass, all restricted to the
//! production-default configuration (`head_field_active=false`, `multiplicative_lateral_gate`
//! off, `pressure_sensitive_flow=false`) and to `PHASE == 1` (the base pass; never a weighted
//! `lateral_substeps` extra pass -- see `consts::PHASE`):
//!
//! - `kernel_r`: the scalar reference, a faithful structural port of `settle_tick`'s red-black
//!   collect/arbitrate/apply.
//! - `kernel_a`: the same math expressed as whole-array stages over contiguous spans
//!   (`row_span`), branch-free where the compiler can auto-vectorise.
//! - `kernel_b`: kernel A's structure with stages 1-2's smooth arithmetic replaced by explicit
//!   `wide::f32x8` SIMD.
//!
//! **Why this crate does not depend on `sandart-sim`.** The task this crate exists for is
//! explicit: "the only allowed change inside sandart-sim/src is an `#[ignore]` test that dumps a
//! state snapshot" -- everything else is a NEW workspace member. Linking `sandart-sim` here would
//! pull `physics.rs`'s `settle_tick` and its private helpers into this crate's dependency graph
//! for no benefit (they are `pub(crate)`/private to `sandart-sim` and mostly not `pub` at all, so
//! calling the real functions from outside that crate is not even possible without changing their
//! visibility -- itself a change to production code this task forbids). Every constant and
//! function this crate needs is instead transcribed by hand into `consts.rs`/`scalar_math.rs`,
//! with a doc comment pointing at its `physics.rs` original. This is a real, accepted risk (see
//! `consts.rs`'s module doc comment): if `physics.rs` changes one of these values, this crate
//! goes stale silently. Re-running the validation in this module's next paragraph after any
//! `physics.rs` lateral-edge change is what would catch that.
//!
//! **Validating kernel R against a real lateral pass, without touching `settle_tick`.** The task
//! allows an instrumented `cfg(test)` capture of `cand_h` as a fallback if a clean extraction
//! isn't possible -- but that capture would have to live INSIDE `settle_tick`'s body (there is
//! nowhere else `cand_h` exists), which the harness's hard rule forbids regardless of `cfg(test)`
//! gating ("the only allowed change inside sandart-sim/src is an `#[ignore]` test"). Instead, R
//! was validated by SCENARIO CONSTRUCTION: a 64-wide, 3-row box (`shape_mask` INSIDE only on the
//! middle row, OUTSIDE above and below) with `gravity_dir = (0, 0.04)` and `lateral_substeps =
//! 1.0`, filled with a DrySand-under-Water-style two-material split so both the granular and
//! liquid branches of the lateral edge are exercised. With no INSIDE cell above or below the one
//! live row, `in_transit_at`'s own guard (`cy > 0 && cy + 1 < h && shape_mask[(cy+1)*w+cx] !=
//! OUTSIDE`) is false everywhere, phase 0's vertical edge condition (`y > 0 && y + 1 < h &&
//! is_inside(x, y+1)`) is false everywhere, and the granular CA's own per-cell body is gated on
//! the identical vertical-neighbour test -- so a REAL, UNMODIFIED `DrawingSimulation::update`
//! call changes `heightmap.data` on that row by EXACTLY the lateral pass's own edges, nothing
//! else. Comparing that one real tick's before/after heights against `kernel_r::run_pass` fed the
//! identical pre-tick snapshot (heights, props, colours, `edge_vel_h`/`edge_vel_v` = 0 initially,
//! `column_depth` = 0 initially, `time_seed` = the real sim's `seed` after its own tick-1
//! xorshift advance, block list = the row's one block) reproduced the real tick's height field
//! within float noise on every case tried. See the report for the exact numbers; this validation
//! is a standalone check, not a `#[test]` in this crate (nothing here runs under `cargo test`
//! without the snapshot files present).
//!
//! **What "heights" means for A and B's Jacobi mix.** `settle_tick`'s red-black colouring exists
//! ONLY to make its own per-edge, in-place `cell_avail`/`cell_freecap` writes order-independent
//! (see the "2b. RED-BLACK EDGE COLOURING" comment in `physics.rs`). Kernels A and B compute
//! those two arrays ONCE per cell, before any edge candidate exists (stage 1) -- there is no
//! shared-mutable-write ordering problem left to solve, so both process every edge in a span in
//! one un-coloured sweep (still exactly the edges `block(x)` being simulated would select; see
//! `row_span.rs`). Stage 5's mixing (a cell keeps `h_old - total_out` of its OWN frozen props and
//! takes each inflow at the DONOR's frozen props, weight-averaged) is the user's specified design,
//! not a port of R's sequential per-edge `advect_properties` -- the two are expected to diverge on
//! any cell with two live inflows in the same pass, which is exactly the "floating-point order and
//! simultaneous mixing" divergence the task calls out as acceptable.

pub mod bf_math;
pub mod census;
pub mod consts;
pub mod kernel_a;
pub mod kernel_b;
pub mod kernel_c;
pub mod kernel_c8;
pub mod kernel_d;
pub mod kernel_e;
pub mod kernel_e2;
pub mod kernel_r;
pub mod metrics;
pub mod noise;
pub mod row_span;
pub mod scalar_math;
pub mod snapshot;

#[cfg(target_arch = "wasm32")]
pub mod wasm_api;
