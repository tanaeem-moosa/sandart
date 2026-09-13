//! Kernel C: a genuinely branch-free lateral pass. Same span/stage structure as `kernel_a`/
//! `kernel_b` (see `row_span.rs`), but every rule the task lays out is applied:
//!
//! 1. **No `if`/`continue`/`match` on data inside a per-cell or per-edge loop.** Every branch
//!    `scalar_math` takes on a live value has a `bf_math` twin that returns a `0.0`/`1.0` mask or
//!    a `select()` instead -- see `bf_math.rs`'s module doc comment for the full list and why each
//!    one is safe. Arbitration is ALWAYS computed (no `if oversubscribed` shortcut): stage 3 runs
//!    the jitter + budget-term math for every edge on every span, whether or not anything is
//!    actually oversubscribed. The only `if`s left in this file are on `span.has_extra`/`i > 0`/
//!    `i < n_edges` -- loop-bounds/span-setup facts the task explicitly allows.
//!
//! 2. **The per-tick precompute.** `precompute_head_static` computes
//!    `k_of_liquidity(liq) * LATERAL_PRESSURE_SCALE * janssen_effective_depth(depth, liq)` once
//!    per cell, from the snapshot's props/`column_depth` at LOAD time, into `Scratch::head_static`
//!    (whole grid). `run_pass` never calls `janssen_effective_depth` (so never pays `exp()`) --
//!    every pass just adds the frozen `head_static[idx]` to that pass's live height. This is the
//!    literal reading of the task's rule 2: the fix for `exp()`'s cost is amortising it out of the
//!    hot loop entirely, not vectorising `exp()` itself. Timed separately in the bench harness
//!    (`precompute_c`) and reported as a one-time, per-snapshot-load cost, never folded into
//!    ns/cell/pass. `wetness`/`threshold`/`granular_share`/`wave_params` are NOT frozen -- they
//!    still come from each pass's live (evolving, via stage 5's mixing) props, exactly like
//!    kernel A/B/R, since only `column_depth` and the depth/k head term are the task's "frozen
//!    within a tick" claim.
//!
//! 3. **Randomness from a table, not a hash.** See `noise.rs`. Four per-row offsets (dispersion,
//!    lock, `edge_share_jitter`, colour entropy) are computed once per span and read as
//!    CONTIGUOUS slices, never a per-edge gather.
//!
//! 4. **No full-grid clone.** Verified in `bf_math.rs`'s `in_transit_row_bf` doc comment: the
//!    lateral pass only ever WRITES row `y`'s heights/props/colours, and only ever READS the row
//!    BELOW's heights/props (never row `y-1`'s) plus `edge_vel_v` (never written by this pass, so
//!    reading it from row `y-1` needs no freshness guarantee at all). `row_span::build_spans`
//!    already emits spans in non-decreasing `y` order block-row by block-row (`by` is the outer
//!    loop; two spans can share a `y` only within the same `by`, covering disjoint `x`-ranges) --
//!    so by the time this function reaches a span at row `y`, row `y+1` is guaranteed still to
//!    hold its pre-pass values. Stage 1 therefore reads `state.heights`/`state.cell_props`/
//!    `state.cell_colors` DIRECTLY (no clone) into small per-span scratch buffers sized to the
//!    widest span (`<= w` cells) -- exactly the "row-local frozen copies of only what mixing
//!    needs" the task offers as the alternative to relying on ordering. Both apply here: the
//!    per-span copy is what stage 5's mixing reads from (so an already-mutated cell 0 doesn't
//!    corrupt cell 1's mixing later in the same span), and the ordering argument is what makes
//!    reading row `y+1` directly out of `state` (uncloned) safe.
//!
//! 5. **`in_transit` branch-free, per row.** `bf_math::in_transit_row_bf`.
//!
//! 6. **Jacobi mixing**, identical law to kernel A/B (see their module doc comments), rewritten
//!    branch-free: `own_amount`/`has_amount`/the left/right inflow gates are all masks instead of
//!    `if`s, and an empty cell's `/safe_total` divisor is floored so the `select` between "own
//!    value unchanged" and "computed blend" never sees a `NaN` from the unused branch.
//!
//! 7. **Cross-block-boundary edges** via `row_span`'s spans, same as kernel A. See `kernel_c8.rs`
//!    for the `chunks_exact(8)` variant.

#![allow(clippy::too_many_arguments)]

use crate::bf_math::*;
use crate::consts::*;
use crate::noise::{Noise, SALT_COLOR, SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use crate::row_span::{build_spans, Span};
use crate::scalar_math::{cell_capacity_for, k_of_liquidity, liquidity};
use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// `k_of_liquidity(liq) * LATERAL_PRESSURE_SCALE * janssen_effective_depth(depth, liq)`, once per
/// cell, from `state`'s CURRENT `column_depth`/wetness -- see this module's doc comment point 2.
/// Exposed standalone (not just inside `Scratch::new`) so the bench harness can time it alone.
#[inline(never)]
pub fn precompute_head_static(state: &State) -> Vec<f32> {
    let n = state.w * state.h;
    let mut out = vec![0.0f32; n];
    for i in 0..n {
        let wetness = state.cell_props[i * 4 + PROP_WETNESS];
        let liq = liquidity(wetness);
        let k = k_of_liquidity(liq);
        let depth = crate::scalar_math::janssen_effective_depth(state.column_depth[i], liq);
        out[i] = k * LATERAL_PRESSURE_SCALE * depth;
    }
    out
}

pub struct Scratch {
    pub(crate) spans: Vec<Span>,
    pub(crate) noise: Noise,
    pub(crate) head_static: Vec<f32>,
    // Per-span scratch, sized to the widest span's data length + 1 sentinel slot (see
    // `run_pass`'s stage 1 for why the sentinel is needed).
    pub(crate) h: Vec<f32>,
    pub(crate) wetness: Vec<f32>,
    pub(crate) threshold: Vec<f32>,
    pub(crate) cap: Vec<f32>,
    pub(crate) avail: Vec<f32>,
    pub(crate) in_transit: Vec<f32>,
    pub(crate) freecap: Vec<f32>,
    pub(crate) head_base: Vec<f32>,
    pub(crate) granular_share: Vec<f32>,
    pub(crate) active: Vec<f32>,
    pub(crate) props: Vec<f32>,  // flat [i*4+ch], frozen per-span copy
    pub(crate) colors: Vec<f32>, // flat [i*4+ch], frozen per-span copy (as f32)
    pub(crate) candidate: Vec<f32>,
    pub(crate) out_total: Vec<f32>,
    pub(crate) in_total: Vec<f32>,
    pub(crate) out_total_jit: Vec<f32>,
    pub(crate) in_total_jit: Vec<f32>,
    pub(crate) total_out_flow: Vec<f32>,
    pub(crate) total_in_flow: Vec<f32>,
}

impl Scratch {
    pub fn new(state: &State) -> Self {
        let spans = build_spans(state);
        let max_len = spans.iter().map(|s| s.data_end() - s.x_start).max().unwrap_or(0) + 1;
        let noise = Noise::new(state.time_seed);
        let head_static = precompute_head_static(state);
        Scratch {
            spans,
            noise,
            head_static,
            h: vec![0.0; max_len],
            wetness: vec![0.0; max_len],
            threshold: vec![0.0; max_len],
            cap: vec![0.0; max_len],
            avail: vec![0.0; max_len],
            in_transit: vec![0.0; max_len],
            freecap: vec![0.0; max_len],
            head_base: vec![0.0; max_len],
            granular_share: vec![0.0; max_len],
            active: vec![0.0; max_len],
            props: vec![0.0; max_len * 4],
            colors: vec![0.0; max_len * 4],
            candidate: vec![0.0; max_len],
            out_total: vec![0.0; max_len],
            in_total: vec![0.0; max_len],
            out_total_jit: vec![0.0; max_len],
            in_total_jit: vec![0.0; max_len],
            total_out_flow: vec![0.0; max_len],
            total_in_flow: vec![0.0; max_len],
        }
    }
}

pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

/// Stage 1: per-cell frozen scalars, branch-free. Split into its own `#[inline(never)]` function
/// so the wasm SIMD-op count can be attributed per stage (task: "Report the per-stage v128 counts
/// if you split them").
#[inline(never)]
pub(crate) fn stage1(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_data: usize) {
    for i in 0..n_data {
        let x = span.x_start + i;
        let idx = span.y * w + x;
        let inside = mask_ne(state.shape_mask[idx] as f32, MASK_OUTSIDE as f32);
        let hh = state.heights[idx];
        let wetness = state.cell_props[idx * 4 + PROP_WETNESS];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        scratch.h[i] = hh;
        scratch.wetness[i] = wetness;
        scratch.threshold[i] = state.cell_props[idx * 4 + PROP_THRESHOLD];
        scratch.cap[i] = cap;
        scratch.granular_share[i] = 1.0 - liq;
        scratch.head_base[i] = hh + scratch.head_static[idx];
        scratch.active[i] = inside;
        for ch in 0..4 {
            scratch.props[i * 4 + ch] = state.cell_props[idx * 4 + ch];
            scratch.colors[i * 4 + ch] = state.cell_colors[idx * 4 + ch] as f32;
        }
    }
    // Sentinel: when this span has no readable acceptor column past its owned range (!has_extra),
    // index `n_data` (== n_edges) stands in for "the neighbour past the edge of the shape" in the
    // edge-active mask below and must read as OUTSIDE, not whatever an earlier span/pass left in
    // this scratch slot. A single per-span write, not a per-edge branch.
    if !span.has_extra {
        scratch.active[n_data] = 0.0;
    }

    crate::bf_math::in_transit_row_bf(
        span.x_start,
        n_data,
        span.y,
        w,
        state.h,
        &state.heights,
        &state.cell_props,
        &state.edge_vel_v,
        &state.shape_mask,
        &mut scratch.in_transit[..n_data],
    );
    for i in 0..n_data {
        let avail_raw = (scratch.h[i] - scratch.in_transit[i]).max(0.0);
        scratch.avail[i] = avail_raw * scratch.active[i];
        scratch.freecap[i] = (scratch.cap[i] - scratch.h[i]).max(0.0) * scratch.active[i];
    }
}

/// Stage 2: per-edge candidate flux, branch-free, RNG from the noise table.
#[inline(never)]
pub(crate) fn stage2(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_edges: usize) {
    let off_disp = scratch.noise.row_offset(span.y, SALT_DISPERSION, n_edges.max(1));
    let off_lock = scratch.noise.row_offset(span.y, SALT_LOCK, n_edges.max(1));
    let off_jit = scratch.noise.row_offset(span.y, SALT_JITTER, n_edges.max(1));

    for e in 0..n_edges {
        let x = span.x_start + e;
        let idx = span.y * w + x;
        let disp01 = scratch.noise.slice(off_disp, n_edges)[e];
        let lock01 = scratch.noise.slice(off_lock, n_edges)[e];
        let jit01 = scratch.noise.slice(off_jit, n_edges)[e];

        let active_e = scratch.active[e] * scratch.active[e + 1];
        let tau = GRANULAR_TAU_SCALE * scratch.threshold[e] * scratch.granular_share[e];

        // 8-bit quantised uniform draw, matching `dispersion_roll`'s `& 0xFF`.
        let q8 = (disp01 * 256.0).floor().min(255.0) / 255.0;
        let dispersion = (q8 - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;
        let head_a = scratch.head_base[e] + dispersion;
        let head_b = scratch.head_base[e + 1];
        let driving = head_a - head_b;

        // 16-bit quantised uniform draw, matching `lock_roll`'s `& 0xFFFF`.
        let q16 = (lock01 * 65536.0).floor().min(65535.0) / 65535.0;
        let lock_mask = mask_lt(q16, GRAVITY_LOCK_CHANCE * scratch.granular_share[e]);

        let v_prev = state.edge_vel_h[idx];
        let sleep_mask = edge_sleeps_bf(driving, tau, v_prev, scratch.h[e], scratch.h[e + 1], scratch.freecap[e], scratch.freecap[e + 1]);
        let inactive = lock_mask.max(sleep_mask);

        let (c_sq, damping) = wave_params_bf(scratch.wetness[e]);
        let raw_candidate = flux_edge_candidate_bf(
            head_a,
            head_b,
            c_sq,
            damping,
            tau,
            scratch.avail[e],
            scratch.avail[e + 1],
            scratch.freecap[e + 1],
            scratch.freecap[e],
            v_prev,
        );
        // `edge_share_jitter`'s draw is read here too (not just at arbitration time) so its
        // per-edge `u01` is fixed once per edge -- stage 3 recomputes it a second time
        // (cheap: one more contiguous read) rather than storing it, same trade kernel A/B make
        // for `out_total_jit`.
        let _ = jit01;
        scratch.candidate[e] = raw_candidate * (1.0 - inactive) * active_e;
    }
}

/// Stage 3: per-cell out/in totals AND their jitter-weighted totals, in one branch-free sweep
/// (arbitration is ALWAYS computed -- task rule 1, no `oversubscribed` shortcut), then the final
/// arbitration scale + realised flux.
#[inline(never)]
pub(crate) fn stage3(state: &mut State, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) -> f64 {
    for i in 0..n_data {
        scratch.out_total[i] = 0.0;
        scratch.in_total[i] = 0.0;
        scratch.out_total_jit[i] = 0.0;
        scratch.in_total_jit[i] = 0.0;
    }
    let off_jit = scratch.noise.row_offset(span.y, SALT_JITTER, n_edges.max(1));

    // Pass 1: accumulate raw and jitter-weighted totals. Donor/acceptor is chosen by the SIGN of
    // the candidate, branch-free via `donor_is_e` (1.0 when edge `e`'s LEFT cell donates) applied
    // as a multiplier to BOTH cells' accumulators every iteration, rather than indexing into
    // whichever one is the "real" donor.
    for e in 0..n_edges {
        let c = scratch.candidate[e];
        let mag = c.abs();
        let donor_is_e = mask_ge(c, 0.0);
        let gjs_e = grain_jitter_strength_bf(scratch.props[e * 4 + PROP_GRAIN_SIZE], scratch.granular_share[e]);
        let gjs_e1 = grain_jitter_strength_bf(scratch.props[(e + 1) * 4 + PROP_GRAIN_SIZE], scratch.granular_share[e + 1]);
        let s_donor = select(donor_is_e, gjs_e, gjs_e1);
        let u01 = scratch.noise.slice(off_jit, n_edges)[e];
        let jit = edge_share_jitter_bf(s_donor, u01);

        scratch.out_total[e] += mag * donor_is_e;
        scratch.out_total[e + 1] += mag * (1.0 - donor_is_e);
        scratch.in_total[e + 1] += mag * donor_is_e;
        scratch.in_total[e] += mag * (1.0 - donor_is_e);
        scratch.out_total_jit[e] += mag * jit * donor_is_e;
        scratch.out_total_jit[e + 1] += mag * jit * (1.0 - donor_is_e);
        scratch.in_total_jit[e + 1] += mag * jit * donor_is_e;
        scratch.in_total_jit[e] += mag * jit * (1.0 - donor_is_e);
    }

    for i in 0..n_data {
        scratch.total_out_flow[i] = 0.0;
        scratch.total_in_flow[i] = 0.0;
    }
    let mut total_flow = 0.0f64;
    for e in 0..n_edges {
        let x = span.x_start + e;
        let idx = span.y * w + x;
        let c = scratch.candidate[e];
        let mag = c.abs();
        let donor_is_e = mask_ge(c, 0.0);
        let gjs_e = grain_jitter_strength_bf(scratch.props[e * 4 + PROP_GRAIN_SIZE], scratch.granular_share[e]);
        let gjs_e1 = grain_jitter_strength_bf(scratch.props[(e + 1) * 4 + PROP_GRAIN_SIZE], scratch.granular_share[e + 1]);
        let s_donor = select(donor_is_e, gjs_e, gjs_e1);
        let u01 = scratch.noise.slice(off_jit, n_edges)[e];
        let jit = edge_share_jitter_bf(s_donor, u01);

        let donor_out = select(donor_is_e, scratch.out_total[e], scratch.out_total[e + 1]);
        let donor_out_jit = select(donor_is_e, scratch.out_total_jit[e], scratch.out_total_jit[e + 1]);
        let donor_avail = select(donor_is_e, scratch.avail[e], scratch.avail[e + 1]);
        let acc_in = select(donor_is_e, scratch.in_total[e + 1], scratch.in_total[e]);
        let acc_in_jit = select(donor_is_e, scratch.in_total_jit[e + 1], scratch.in_total_jit[e]);
        let acc_free = select(donor_is_e, scratch.freecap[e + 1], scratch.freecap[e]);

        let scale = edge_arbitration_scale_bf(donor_out, donor_out_jit, donor_avail, acc_in, acc_in_jit, acc_free, jit);
        let final_flux = c * scale;
        scratch.candidate[e] = final_flux;
        state.edge_vel_h[idx] = final_flux;

        let realized_mask = mask_gt(final_flux.abs(), MIN_FLUX);
        let realized = final_flux.abs() * realized_mask;
        scratch.total_out_flow[e] += realized * donor_is_e;
        scratch.total_out_flow[e + 1] += realized * (1.0 - donor_is_e);
        scratch.total_in_flow[e + 1] += realized * donor_is_e;
        scratch.total_in_flow[e] += realized * (1.0 - donor_is_e);
        let _ = mag;
        total_flow += realized as f64;
    }
    total_flow
}

/// Stages 4+5: apply heights, Jacobi-mix props/colours with stochastic-rounded colour channels --
/// branch-free (see this module's doc comment point 6). `i > 0`/`i < n_edges` are loop-bounds
/// guards on the span's own local index range (task rule 1's carve-out), not data branches.
#[inline(never)]
pub(crate) fn stage45(state: &mut State, scratch: &Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) {
    let off_col = scratch.noise.row_offset(span.y, SALT_COLOR, (n_data * 4).max(1));
    let color_noise = scratch.noise.slice(off_col, n_data * 4);

    for i in 0..n_data {
        let x = span.x_start + i;
        let idx = span.y * w + x;
        let out_flow = scratch.total_out_flow[i];
        let in_flow = scratch.total_in_flow[i];
        let h_old = scratch.h[i];
        let h_new = (h_old - out_flow + in_flow).max(0.0);
        state.heights[idx] = h_new;

        let kept = (h_old - out_flow).max(0.0);
        let left_flux = if i > 0 { scratch.candidate[i - 1] } else { 0.0 };
        let right_flux = if i < n_edges { scratch.candidate[i] } else { 0.0 };
        let left_amt = left_flux * mask_gt(left_flux, MIN_FLUX);
        let right_amt = (-right_flux) * mask_lt(right_flux, -MIN_FLUX);

        let own_amount = kept * mask_gt(h_new, 1e-6);
        let total_amount = own_amount + in_flow;
        let safe_total = total_amount.max(1e-12);
        let has_amount = mask_gt(total_amount, 1e-6);

        for ch in 0..4 {
            let own_val = scratch.props[i * 4 + ch];
            let left_val = if i > 0 { scratch.props[(i - 1) * 4 + ch] } else { 0.0 };
            let right_val = if i < n_edges { scratch.props[(i + 1) * 4 + ch] } else { 0.0 };
            let mixed = left_val * left_amt + right_val * right_amt;
            let computed = (own_val * own_amount + mixed) / safe_total;
            let new_prop = select(has_amount, computed, own_val);
            state.cell_props[idx * 4 + ch] = new_prop;

            let own_c = scratch.colors[i * 4 + ch];
            let left_c = if i > 0 { scratch.colors[(i - 1) * 4 + ch] } else { 0.0 };
            let right_c = if i < n_edges { scratch.colors[(i + 1) * 4 + ch] } else { 0.0 };
            let mixed_c = left_c * left_amt + right_c * right_amt;
            let computed_c = ((own_c * own_amount + mixed_c) / safe_total).clamp(0.0, 255.0);
            let new_c = select(has_amount, computed_c, own_c);
            let r01 = color_noise[i * 4 + ch];
            let rounded = stochastic_round_bf(new_c, r01).clamp(0.0, 255.0);
            state.cell_colors[idx * 4 + ch] = rounded as u8;
        }
        state.cell_colors[idx * 4 + 3] = 255;
    }
}

pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let mut total_flow = 0.0f64;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;

        stage1(state, scratch, span, w, n_data);
        stage2(state, scratch, span, w, n_edges);
        total_flow += stage3(state, scratch, span, w, n_data, n_edges);
        stage45(state, scratch, span, w, n_data, n_edges);
    }
    total_flow
}
