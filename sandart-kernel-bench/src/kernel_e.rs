//! Kernel E: kernel D's exact math (see `kernel_d.rs`'s module doc comment for the law itself,
//! unchanged here), on a storage layout that mirrors production's post-`0253caa` change --
//! `CellProps` as four separate `Vec<f32>` and one packed `Vec<u32>` per cell for colour -- with
//! double-buffered heights/props/colours so the pass reads a FROZEN "current" set and writes a
//! "next" set, then swaps (`std::mem::swap`, O(1)), instead of D's per-span copy-in/copy-out.
//!
//! **Why this removes copies D still pays.** D's `copy_in` converts AoS (`cell_props[idx*4+ch]`,
//! `cell_colors[idx*4+ch]`) into per-channel padded scratch once per span, and `copy_out` converts
//! back -- together ~50-80 ns/cell/pass of the measured cost (see the report). `StateE` is already
//! SoA, so there is nothing to convert: stage 4+5 reads `state.prop[ch][idx]` directly (the frozen
//! "current" array) and writes `state.prop_b[ch][idx]` directly (the "next" array) -- no scratch
//! copy of the raw value in either direction. Colours are one `u32` per cell (`pack_e`/`unpack`
//! below, same bit layout as production's `pack_rgba`), unpacked/repacked lane-wise once per cell.
//!
//! **Double buffering and the "untouched cells" invariant.** `sim_blocks` (and so every span) is
//! FIXED across all repeated passes in this benchmark (see `row_span.rs`'s doc comment on
//! `snapshot::State::sim_blocks`), so the SET of cells any span ever writes never changes pass over
//! pass. `StateE::from_state` clones `current` into `next` ONCE, outside timing. From then on:
//! - a TOUCHED cell (inside some span's `[x_start, data_end)`) is fully recomputed from this pass's
//!   `current` values and written into `next` EVERY pass -- never incrementally updated, so whatever
//!   `next` held before this pass at that index is irrelevant, it is fully overwritten.
//! - an UNTOUCHED cell is written by NO span, ever, so `current[idx] == next[idx]` holds forever
//!   once it holds once (proved by induction: it holds after the initial clone, and the swap that
//!   follows each pass cannot break it because neither buffer's value at that index ever changes).
//!
//! So after `swap_buffers_e`'s `mem::swap`, the new "current" is fully valid at every cell with NO
//! per-pass resync beyond the swap itself -- the "cheapest correct scheme" the task asks for is
//! exactly this: one `clone()` at load (untimed) plus an O(1) `mem::swap` per pass. An alternative
//! that copied every untouched cell each pass would cost O(w*h) per pass for no benefit; measured
//! swap cost is reported alongside the stage timings.
//!
//! **No padding.** D pads every per-span scratch buffer with one ghost cell at each end so
//! `prop[i-1]`/`prop[i+1]` are always in-bounds and read as zero outside the span. E instead
//! indexes the REAL grid directly (`idx-1`/`idx+1`), relying on two facts:
//! 1. Every vessel's outermost ring of columns is `MASK_OUTSIDE` casing, so no span's owned range
//!    ever reaches column 0 or `w-1` -- `idx-1`/`idx+1` for an owned cell are always valid indices
//!    into the real grid (never a bounds problem), and a genuine grid-edge read only ever lands on
//!    a casing cell, which carries `h=0`/`active=0`, reproducing D's ghost `h=0` exactly.
//! 2. `state.edge_vel_h` is REAL, whole-grid production state (not scratch invented by this
//!    kernel), so at a span's boundary (`idx-1` left of the span's first owned edge, or `idx` at
//!    the span's own acceptor cell, which owns no outgoing edge) it can hold stale, unrelated data
//!    from whatever a non-simulated block's edge last carried -- reading it unmasked there would
//!    NOT reproduce D's explicit zero ghost. So `left_flux`/`right_flux` (stage 4+5, the only place
//!    boundary `edge_vel_h` values are read) are masked by `x > x_start` / `x < x_owned_end` --
//!    loop-bounds facts about the span, not per-cell data branches, exactly mirroring kernel C's
//!    `if i > 0`/`if i < n_edges` sentinels. Every OTHER scratch array in this file (`pos`/`neg`/
//!    `out_total`/... ) is invented by this kernel alone and zero-initialised once in `Scratch::new`
//!    -- since no span ever writes an unowned index, those stay exactly zero forever with no mask
//!    needed, by the same induction argument as the double-buffer invariant above.
//!
//! **E-recip.** Identical to E except `stage45`'s division-per-channel becomes one
//! `inv_total = 1.0 / safe_total` per cell, reused for all 4 props and 3 colour channels --
//! implemented as `stage45_e_impl::<RECIP>`, monomorphised into two distinct `#[inline(never)]`
//! functions (`stage45_e`, `stage45_e_recip`) so both are independently nameable in the wasm
//! disassembly for the v128 op count.

#![allow(clippy::too_many_arguments)]

use crate::bf_math::*;
use crate::consts::*;
use crate::noise::{Noise, SALT_COLOR, SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use crate::row_span::{build_spans_raw, Span};
use crate::scalar_math::{cell_capacity_for, janssen_effective_depth, k_of_liquidity, liquidity};
use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// Same bit layout as production's `pack_rgba`/`unpack_rgba` (`sandart-sim/src/lib.rs`): little-
/// endian `[r, g, b, a]` packed into one `u32`, `r` in the low byte.
#[inline]
fn pack_e(r: f32, g: f32, b: f32) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | (255u32 << 24)
}

/// SoA state mirroring production's post-`0253caa` layout: `CellProps` as four `Vec<f32>`, colour
/// as one packed `Vec<u32>` per cell. `heights`/`prop`/`colors` are the FROZEN "current" set this
/// pass reads; `heights_b`/`prop_b`/`colors_b` are "next", written this pass, then swapped in by
/// `swap_buffers_e`. `edge_vel_h`/`edge_vel_v`/`column_depth`/`shape_mask` are single-buffered,
/// "as they are" -- see this module's doc comment for why `edge_vel_h` in particular needs no
/// second buffer (read-then-overwrite within the same pass, exactly like production).
pub struct StateE {
    pub w: usize,
    pub h: usize,
    pub block_size: usize,
    pub cols: usize,
    pub rows: usize,
    pub time_seed: u32,
    pub sim_blocks: Vec<u32>,
    pub shape_mask: Vec<u8>,
    pub edge_vel_h: Vec<f32>,
    pub edge_vel_v: Vec<f32>,
    pub column_depth: Vec<f32>,
    pub heights: Vec<f32>,
    pub heights_b: Vec<f32>,
    pub prop: [Vec<f32>; 4],
    pub prop_b: [Vec<f32>; 4],
    pub colors: Vec<u32>,
    pub colors_b: Vec<u32>,
}

impl StateE {
    /// AoS -> SoA conversion, run ONCE at load, OUTSIDE timing (task requirement). Also performs
    /// the one-time `next = current.clone()` that makes the swap scheme above correct from pass 1.
    pub fn from_state(s: &State) -> Self {
        let n = s.w * s.h;
        let mut prop = [vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]];
        let mut colors = vec![0u32; n];
        for i in 0..n {
            for ch in 0..4 {
                prop[ch][i] = s.cell_props[i * 4 + ch];
            }
            let r = s.cell_colors[i * 4] as u32;
            let g = s.cell_colors[i * 4 + 1] as u32;
            let b = s.cell_colors[i * 4 + 2] as u32;
            let a = s.cell_colors[i * 4 + 3] as u32;
            colors[i] = r | (g << 8) | (b << 16) | (a << 24);
        }
        let heights_b = s.heights.clone();
        let prop_b = prop.clone();
        let colors_b = colors.clone();
        StateE {
            w: s.w,
            h: s.h,
            block_size: s.block_size,
            cols: s.cols,
            rows: s.rows,
            time_seed: s.time_seed,
            sim_blocks: s.sim_blocks.clone(),
            shape_mask: s.shape_mask.clone(),
            edge_vel_h: s.edge_vel_h.clone(),
            edge_vel_v: s.edge_vel_v.clone(),
            column_depth: s.column_depth.clone(),
            heights: s.heights.clone(),
            heights_b,
            prop,
            prop_b,
            colors,
            colors_b,
        }
    }

    /// SoA -> AoS, for feeding `metrics::compare`/`native_bench`'s equivalence tables, which are
    /// written once against `snapshot::State`. Not part of any timed path.
    pub fn to_state(&self) -> State {
        let n = self.w * self.h;
        let mut cell_props = vec![0.0f32; n * 4];
        let mut cell_colors = vec![0u8; n * 4];
        for i in 0..n {
            for ch in 0..4 {
                cell_props[i * 4 + ch] = self.prop[ch][i];
            }
            let c = self.colors[i];
            cell_colors[i * 4] = c as u8;
            cell_colors[i * 4 + 1] = (c >> 8) as u8;
            cell_colors[i * 4 + 2] = (c >> 16) as u8;
            cell_colors[i * 4 + 3] = (c >> 24) as u8;
        }
        State {
            w: self.w,
            h: self.h,
            block_size: self.block_size,
            cols: self.cols,
            rows: self.rows,
            time_seed: self.time_seed,
            tick_count: 0,
            sim_blocks: self.sim_blocks.clone(),
            shape_mask: self.shape_mask.clone(),
            heights: self.heights.clone(),
            cell_props,
            cell_colors,
            edge_vel_h: self.edge_vel_h.clone(),
            edge_vel_v: self.edge_vel_v.clone(),
            column_depth: self.column_depth.clone(),
        }
    }
}

/// `kernel_d::precompute_head_static_d`'s exact value, reading `StateE`'s SoA wetness/column_depth
/// instead of interleaved `cell_props`. Whole-grid output, only span-data cells ever populated
/// (task rule 4), computed ONCE per snapshot load -- never recomputed pass to pass (see
/// `kernel_c.rs`'s doc comment point 2, unchanged reasoning here).
#[inline(never)]
pub fn precompute_head_static_e(state: &StateE, spans: &[Span]) -> Vec<f32> {
    let w = state.w;
    let mut out = vec![0.0f32; state.w * state.h];
    for span in spans {
        let n_data = span.data_end() - span.x_start;
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let wetness = state.prop[PROP_WETNESS][idx];
            let liq = liquidity(wetness);
            let k = k_of_liquidity(liq);
            let depth = janssen_effective_depth(state.column_depth[idx], liq);
            out[idx] = k * LATERAL_PRESSURE_SCALE * depth;
        }
    }
    out
}

pub fn precompute_cell_count(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.data_end() - s.x_start).sum()
}

/// Every array here is WHOLE-GRID (`w*h`), zero-initialised once in `Scratch::new`, and only ever
/// written at indices some span owns (see module doc comment for why that makes per-pass re-zeroing
/// unnecessary). No per-span padding/offset bookkeeping (`pad_off`/`flat_off` in `kernel_d.rs`)
/// exists here at all -- every stage indexes the real grid `idx` directly.
pub struct Scratch {
    pub(crate) spans: Vec<Span>,
    pub(crate) noise: Noise,
    pub(crate) head_static: Vec<f32>,

    active: Vec<f32>,
    granular_share: Vec<f32>,
    tau: Vec<f32>,
    c_sq: Vec<f32>,
    damping: Vec<f32>,
    gjs: Vec<f32>,
    head: Vec<f32>,
    avail: Vec<f32>,
    freecap: Vec<f32>,
    in_transit: Vec<f32>,

    pos: Vec<f32>,
    neg: Vec<f32>,
    jit: Vec<f32>,
    pos_jit: Vec<f32>,
    neg_jit: Vec<f32>,
    out_total: Vec<f32>,
    in_total: Vec<f32>,
    out_total_jit: Vec<f32>,
    in_total_jit: Vec<f32>,

    pos2: Vec<f32>,
    neg2: Vec<f32>,
    own_amount: Vec<f32>,
    safe_total: Vec<f32>,
    has_amount: Vec<f32>,
    left_amt: Vec<f32>,
    right_amt: Vec<f32>,
}

impl Scratch {
    pub fn new(state: &StateE) -> Self {
        let spans = build_spans_raw(state.cols, state.rows, state.block_size, state.w, state.h, &state.sim_blocks);
        let noise = Noise::new(state.time_seed);
        let head_static = precompute_head_static_e(state, &spans);
        let n = state.w * state.h;
        Scratch {
            spans,
            noise,
            head_static,
            active: vec![0.0; n],
            granular_share: vec![0.0; n],
            tau: vec![0.0; n],
            c_sq: vec![0.0; n],
            damping: vec![0.0; n],
            gjs: vec![0.0; n],
            head: vec![0.0; n],
            avail: vec![0.0; n],
            freecap: vec![0.0; n],
            in_transit: vec![0.0; n],
            pos: vec![0.0; n],
            neg: vec![0.0; n],
            jit: vec![0.0; n],
            pos_jit: vec![0.0; n],
            neg_jit: vec![0.0; n],
            out_total: vec![0.0; n],
            in_total: vec![0.0; n],
            out_total_jit: vec![0.0; n],
            in_total_jit: vec![0.0; n],
            pos2: vec![0.0; n],
            neg2: vec![0.0; n],
            own_amount: vec![0.0; n],
            safe_total: vec![0.0; n],
            has_amount: vec![0.0; n],
            left_amt: vec![0.0; n],
            right_amt: vec![0.0; n],
        }
    }
}

pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

/// Precompute: D's `copy_in` minus the AoS->SoA conversion (nothing to convert -- `state.prop`/
/// `state.heights` are already the arrays this stage reads). Computes the same per-cell derived
/// scalars D's `copy_in` does, direct real-index writes, no padding.
#[inline(never)]
pub(crate) fn precompute_e(state: &StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize) {
    let row = span.y * w;
    let x0 = span.x_start;

    for i in 0..n_data {
        let idx = row + x0 + i;
        let wetness = state.prop[PROP_WETNESS][idx];
        let threshold = state.prop[PROP_THRESHOLD][idx];
        let grain_size = state.prop[PROP_GRAIN_SIZE][idx];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        let granular_share = 1.0 - liq;
        let (c_sq, damping) = wave_params_bf(wetness);
        let inside = mask_ne(state.shape_mask[idx] as f32, MASK_OUTSIDE as f32);
        let hh = state.heights[idx];

        scratch.active[idx] = inside;
        scratch.granular_share[idx] = granular_share;
        scratch.tau[idx] = GRANULAR_TAU_SCALE * threshold * granular_share;
        scratch.c_sq[idx] = c_sq;
        scratch.damping[idx] = damping;
        scratch.gjs[idx] = grain_jitter_strength_bf(grain_size, granular_share);
        scratch.head[idx] = hh + scratch.head_static[idx];
        scratch.freecap[idx] = (cap - hh).max(0.0) * inside;
    }

    crate::bf_math::in_transit_row_bf_soa(
        x0,
        n_data,
        span.y,
        w,
        state.h,
        &state.heights,
        &state.prop[PROP_WETNESS],
        &state.edge_vel_v,
        &state.shape_mask,
        &mut scratch.in_transit[row + x0..row + x0 + n_data],
    );

    for i in 0..n_data {
        let idx = row + x0 + i;
        let avail_raw = (state.heights[idx] - scratch.in_transit[idx]).max(0.0);
        scratch.avail[idx] = avail_raw * scratch.active[idx];
    }
}

/// Stage 2: same math as `kernel_d::stage2`, direct real-index reads/writes. Writes the raw
/// candidate flux directly into `state.edge_vel_h[idx]`, overwriting `v_prev` (already read this
/// same call) -- see module doc comment for why `edge_vel_h` needs no second buffer.
#[inline(never)]
pub(crate) fn stage2_e(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_edges: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let off_disp = scratch.noise.row_offset(span.y, SALT_DISPERSION, n_edges.max(1));
    let off_lock = scratch.noise.row_offset(span.y, SALT_LOCK, n_edges.max(1));
    let disp_row = scratch.noise.slice(off_disp, n_edges);
    let lock_row = scratch.noise.slice(off_lock, n_edges);

    for e in 0..n_edges {
        let idx = row + x0 + e;
        let ridx = idx + 1;

        let active_e = scratch.active[idx] * scratch.active[ridx];
        let tau = scratch.tau[idx];

        let q8 = (disp_row[e] * 256.0).floor().min(255.0) / 255.0;
        let dispersion = (q8 - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;
        let head_a = scratch.head[idx] + dispersion;
        let head_b = scratch.head[ridx];
        let driving = head_a - head_b;

        let q16 = (lock_row[e] * 65536.0).floor().min(65535.0) / 65535.0;
        let lock_mask = mask_lt(q16, GRAVITY_LOCK_CHANCE * scratch.granular_share[idx]);

        let v_prev = state.edge_vel_h[idx];
        let sleep_mask = edge_sleeps_bf(driving, tau, v_prev, state.heights[idx], state.heights[ridx], scratch.freecap[idx], scratch.freecap[ridx]);
        let inactive = lock_mask.max(sleep_mask);

        let raw_candidate = flux_edge_candidate_bf(
            head_a,
            head_b,
            scratch.c_sq[idx],
            scratch.damping[idx],
            tau,
            scratch.avail[idx],
            scratch.avail[ridx],
            scratch.freecap[ridx],
            scratch.freecap[idx],
            v_prev,
        );
        state.edge_vel_h[idx] = raw_candidate * (1.0 - inactive) * active_e;
    }
}

/// Stage 3: same gather-form math as `kernel_d::stage3`. `pos`/`neg`/`out_total`/... are this
/// kernel's own scratch (never real state), so boundary reads (`idx-1` at a span's first owned
/// cell) need no mask -- they are zero by construction (see module doc comment). Reads/writes the
/// candidate then final flux via `state.edge_vel_h` directly, same as stage 2.
#[inline(never)]
pub(crate) fn stage3_e(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) -> f64 {
    let row = span.y * w;
    let x0 = span.x_start;
    let off_jit = scratch.noise.row_offset(span.y, SALT_JITTER, n_edges.max(1));
    let jit_row = scratch.noise.slice(off_jit, n_edges);

    for e in 0..n_edges {
        let idx = row + x0 + e;
        let ridx = idx + 1;
        let f = state.edge_vel_h[idx];
        let mag = f.abs();
        let donor_is_e = mask_ge(f, 0.0);
        let s_donor = select(donor_is_e, scratch.gjs[idx], scratch.gjs[ridx]);
        let jit = edge_share_jitter_bf(s_donor, jit_row[e]);
        let pos = mag * donor_is_e;
        let neg = mag * (1.0 - donor_is_e);
        scratch.pos[idx] = pos;
        scratch.neg[idx] = neg;
        scratch.jit[idx] = jit;
        scratch.pos_jit[idx] = pos * jit;
        scratch.neg_jit[idx] = neg * jit;
    }

    for i in 0..n_data {
        let idx = row + x0 + i;
        let lidx = idx.saturating_sub(1);
        scratch.out_total[idx] = scratch.pos[idx] + scratch.neg[lidx];
        scratch.in_total[idx] = scratch.neg[idx] + scratch.pos[lidx];
        scratch.out_total_jit[idx] = scratch.pos_jit[idx] + scratch.neg_jit[lidx];
        scratch.in_total_jit[idx] = scratch.neg_jit[idx] + scratch.pos_jit[lidx];
    }

    let mut total_flow = 0.0f64;
    for e in 0..n_edges {
        let idx = row + x0 + e;
        let ridx = idx + 1;
        let f = state.edge_vel_h[idx];
        let donor_is_e = mask_ge(f, 0.0);
        let donor_avail = select(donor_is_e, scratch.avail[idx], scratch.avail[ridx]);
        let donor_out = select(donor_is_e, scratch.out_total[idx], scratch.out_total[ridx]);
        let donor_out_jit = select(donor_is_e, scratch.out_total_jit[idx], scratch.out_total_jit[ridx]);
        let acc_in = select(donor_is_e, scratch.in_total[ridx], scratch.in_total[idx]);
        let acc_in_jit = select(donor_is_e, scratch.in_total_jit[ridx], scratch.in_total_jit[idx]);
        let acc_free = select(donor_is_e, scratch.freecap[ridx], scratch.freecap[idx]);
        let jitter = scratch.jit[idx];

        let scale = edge_arbitration_scale_bf(donor_out, donor_out_jit, donor_avail, acc_in, acc_in_jit, acc_free, jitter);
        let final_flux = f * scale;
        state.edge_vel_h[idx] = final_flux;

        let realized_mask = mask_gt(final_flux.abs(), MIN_FLUX);
        total_flow += (final_flux.abs() * realized_mask) as f64;
    }
    total_flow
}

/// Stages 4+5, generic over `RECIP` (task: E-recip computes `inv_total` once per cell and
/// multiplies each channel by it instead of dividing per channel -- monomorphised below into two
/// distinct `#[inline(never)]` functions so both show up separately in the wasm disassembly).
/// Writes heights/props/colours DIRECTLY into the "next" buffers -- no scratch copy-out (D's
/// `copy_out` has no equivalent here: the value computed IS the final value).
#[inline(always)]
fn stage45_e_impl<const RECIP: bool>(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let x_owned_end = span.x_owned_end;

    for e in 0..n_edges {
        let idx = row + x0 + e;
        let f = state.edge_vel_h[idx];
        let mask = mask_gt(f.abs(), MIN_FLUX);
        let fm = f * mask;
        scratch.pos2[idx] = fm.max(0.0);
        scratch.neg2[idx] = (-fm).max(0.0);
    }

    for i in 0..n_data {
        let x = x0 + i;
        let idx = row + x;
        let lidx = idx.saturating_sub(1);

        let out2 = scratch.pos2[idx] + scratch.neg2[lidx];
        let in2 = scratch.neg2[idx] + scratch.pos2[lidx];
        let h_old = state.heights[idx];
        let h_new = (h_old - out2 + in2).max(0.0);
        state.heights_b[idx] = h_new;

        let kept = (h_old - out2).max(0.0);
        // `edge_vel_h` is real state, not this kernel's own scratch -- mask the two boundary
        // reads (see module doc comment point 2) rather than trusting them to be zero.
        let left_flux = state.edge_vel_h[lidx] * mask_gt(x as f32, x0 as f32);
        let right_flux = state.edge_vel_h[idx] * mask_lt(x as f32, x_owned_end as f32);
        scratch.left_amt[idx] = left_flux * mask_gt(left_flux, MIN_FLUX);
        scratch.right_amt[idx] = (-right_flux) * mask_lt(right_flux, -MIN_FLUX);

        let own_amount = kept * mask_gt(h_new, 1e-6);
        let total_amount = own_amount + in2;
        scratch.own_amount[idx] = own_amount;
        scratch.safe_total[idx] = total_amount.max(1e-12);
        scratch.has_amount[idx] = mask_gt(total_amount, 1e-6);
    }

    // One pass per cell mixes all 4 props + 3 colour channels together so `inv_total` (E-recip)
    // is genuinely computed once per cell, not once per channel -- see module doc comment.
    let off_col = scratch.noise.row_offset(span.y, SALT_COLOR, (n_data * 4).max(1));
    let color_noise = scratch.noise.slice(off_col, n_data * 4);

    for i in 0..n_data {
        let x = x0 + i;
        let idx = row + x;
        let lidx = idx.saturating_sub(1);
        let ridx = (idx + 1).min(row + w - 1);

        let left_amt = scratch.left_amt[idx];
        let right_amt = scratch.right_amt[idx];
        let own_amount = scratch.own_amount[idx];
        let has_amount = scratch.has_amount[idx];
        let safe_total = scratch.safe_total[idx];
        let inv_total = if RECIP { 1.0 / safe_total } else { 0.0 };

        let mix = |own_val: f32, left_val: f32, right_val: f32| -> f32 {
            let mixed = left_val * left_amt + right_val * right_amt;
            let computed = if RECIP { (own_val * own_amount + mixed) * inv_total } else { (own_val * own_amount + mixed) / safe_total };
            select(has_amount, computed, own_val)
        };

        for ch in 0..4 {
            let own_val = state.prop[ch][idx];
            let left_val = state.prop[ch][lidx];
            let right_val = state.prop[ch][ridx];
            state.prop_b[ch][idx] = mix(own_val, left_val, right_val);
        }

        // Colour: unpack r/g/b lane-wise from the frozen packed u32, mix, stochastic-round, repack
        // with alpha 255 -- same as D's `mix_color_channel`, just reading/writing one u32 lane
        // instead of a separate per-channel Vec<f32>.
        let own_c = state.colors[idx];
        let left_c = state.colors[lidx];
        let right_c = state.colors[ridx];
        let mut rgb = [0.0f32; 3];
        for ch in 0..3 {
            let shift = ch * 8;
            let own_v = ((own_c >> shift) & 0xFF) as f32;
            let left_v = ((left_c >> shift) & 0xFF) as f32;
            let right_v = ((right_c >> shift) & 0xFF) as f32;
            let computed = mix(own_v, left_v, right_v).clamp(0.0, 255.0);
            let r01 = color_noise[i * 4 + ch];
            rgb[ch] = stochastic_round_bf(computed, r01).clamp(0.0, 255.0);
        }
        state.colors_b[idx] = pack_e(rgb[0], rgb[1], rgb[2]);
    }
}

#[inline(never)]
pub(crate) fn stage45_e(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) {
    stage45_e_impl::<false>(state, scratch, span, w, n_data, n_edges);
}

#[inline(never)]
pub(crate) fn stage45_e_recip(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize) {
    stage45_e_impl::<true>(state, scratch, span, w, n_data, n_edges);
}

/// The swap scheme's entire per-pass cost: `std::mem::swap` on each double-buffered `Vec` (heights,
/// 4 props, colours) -- O(1) each (a pointer/len/cap swap, no element copy). Cells outside every
/// span stay correct automatically (see module doc comment); nothing else to do here.
#[inline(never)]
pub(crate) fn swap_buffers_e(state: &mut StateE) {
    core::mem::swap(&mut state.heights, &mut state.heights_b);
    for ch in 0..4 {
        core::mem::swap(&mut state.prop[ch], &mut state.prop_b[ch]);
    }
    core::mem::swap(&mut state.colors, &mut state.colors_b);
}

pub fn run_precompute(state: &StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        precompute_e(state, scratch, span, w, n_data);
    }
}

pub fn run_stage2(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_edges = span.x_owned_end - span.x_start;
        stage2_e(state, scratch, span, w, n_edges);
    }
}

pub fn run_stage3(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let mut total = 0.0f64;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        total += stage3_e(state, scratch, span, w, n_data, n_edges);
    }
    total
}

pub fn run_stage45(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        stage45_e(state, scratch, span, w, n_data, n_edges);
    }
}

pub fn run_stage45_recip(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        stage45_e_recip(state, scratch, span, w, n_data, n_edges);
    }
}

pub fn run_swap(state: &mut StateE) {
    swap_buffers_e(state);
}

pub fn run_pass(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_precompute(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45(state, scratch);
    run_swap(state);
    total_flow
}

pub fn run_pass_recip(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_precompute(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45_recip(state, scratch);
    run_swap(state);
    total_flow
}
