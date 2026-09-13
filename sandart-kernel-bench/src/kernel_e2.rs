//! Kernel E2: answers the question E did not. E folded ALL its temporaries (avail, freecap, head,
//! pos/neg, totals, per-cell amounts...) into whole-grid `w*h` arrays touched at scattered per-span
//! offsets, AND fused props+colours into one per-cell loop with a closure -- both are confounds,
//! not the thing the task asked to measure. E2 isolates the ONE variable that matters: are the
//! per-pass AoS<->SoA STATE copies (heights/props/colours) worth removing, holding everything else
//! (temporary sizing/locality, loop structure) exactly as D has it?
//!
//! - **State**: `kernel_e::StateE` unchanged -- production-style SoA current/next buffers,
//!   O(1) `mem::swap`. `head_static` is `kernel_e::precompute_head_static_e`, unchanged.
//! - **Temporaries**: `Scratch` below is D's `Scratch` verbatim, MINUS the fields that only ever
//!   existed to hold a copy of raw state (`h`, `prop0..3`, `new_prop0..3` -- state is read/written
//!   directly instead) -- everything else (`avail`/`freecap`/`head`/`tau`/`c_sq`/`damping`/`gjs`/
//!   `f`/`pos`/`neg`/`jit`/.../`own_amount`/`safe_total`/`has_amount`/`left_amt`/`right_amt`) is
//!   the same padded-per-span / flat-per-span scheme, same sizes, same reuse-across-every-span
//!   buffers D uses -- small, L1-resident, nothing whole-grid.
//! - **Stage 1** (`stage1_e2`) computes exactly D's derived per-cell scalars into that scratch, but
//!   reads `wetness`/`threshold`/`grain_size`/`h` straight from `state`'s row-contiguous SoA slices
//!   instead of copying them into a padded buffer first -- there is no `scratch.h`/`scratch.prop*`
//!   to fill.
//! - **Stage 2/3** (`stage2_e2`/`stage3_e2`) are D's stage 2/3 unchanged, except the two places D
//!   read `scratch.h[p]`/`scratch.h[p+1]` now read `state.heights[idx]`/`state.heights[ridx]`
//!   directly (same values, same span-owned real indices, no bounds issue -- see `kernel_e.rs`'s
//!   doc comment point 1 for why an owned edge's `idx`/`idx+1` are always valid). `stage3_e2` also
//!   writes the final flux into `state.edge_vel_h[idx]` directly (D defers this to `copy_out`; E2
//!   has no copy_out, so there is nothing to defer it to).
//! - **Stage 4+5** is D's structure, unchanged, split the same way (`stage4_realized_e2` ->
//!   `stage4_gather_e2` -> `stage4_amounts_e2` -> one tight loop per prop channel -> colour
//!   unpack/mix/repack), with two differences forced by SoA state:
//!   1. `h_new` is written directly into `state.heights_b[idx]` (no `scratch.h_new` + copy-out).
//!   2. Each prop channel's mixing loop (`mix_prop_channel_e2`) reads a WINDOW of the REAL
//!      `state.prop[ch]` row (`[x_start-1, data_end]`, one real cell either side) instead of a
//!      padded scratch copy -- the "ghost" is a genuine casing cell (see `kernel_e.rs`'s doc
//!      comment point 1: `x_start >= 1` and `data_end <= w-1` always hold), so `left_amt`/
//!      `right_amt` (D's own scratch, correctly zero at a span boundary by construction) do the
//!      same job D's explicit padding did, at zero extra memory. Colour has no such per-channel
//!      real array to window into (one packed `u32` per cell, not 3 `Vec<f32>`), so it gets ONE
//!      small per-span unpack into padded scratch (`col_r`/`col_g`/`col_b`, D's own fields,
//!      unchanged sizing) -- this is not a whole-grid copy, just the span-sized conversion the
//!      packed layout makes unavoidable, then D's own `mix_color_channel` shape, then one repack
//!      loop into `state.colors_b`. No closures anywhere in the hot path.
//! - **E2-recip**: `stage4_amounts_e2::<true>` additionally computes `inv_total = 1/safe_total`
//!   once per cell (stored in `scratch.inv_total`, flat, same class as `safe_total`); the per-
//!   channel mix functions are generic over `RECIP` and multiply by it instead of dividing when
//!   `RECIP`. Monomorphised into `<false>`/`<true>` instantiations so both are independently
//!   nameable for the v128 count, exactly as `kernel_e`'s `stage45_e`/`stage45_e_recip` are.

#![allow(clippy::too_many_arguments)]

use crate::bf_math::*;
use crate::consts::*;
use crate::kernel_e::{self, StateE};
use crate::noise::{Noise, SALT_COLOR, SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use crate::row_span::{build_spans_raw, Span};
use crate::scalar_math::{cell_capacity_for, liquidity};
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// D's `Scratch` verbatim, minus `h`/`prop0..3`/`new_prop0..3` (state is read/written directly --
/// see module doc comment). Same `pad_off`/`flat_off` scheme, same sizes, same per-span reuse.
pub struct Scratch {
    pub(crate) spans: Vec<Span>,
    pub(crate) noise: Noise,
    pub(crate) head_static: Vec<f32>,

    pad_off: Vec<usize>,
    flat_off: Vec<usize>,

    avail: Vec<f32>,
    freecap: Vec<f32>,
    head: Vec<f32>,
    tau: Vec<f32>,
    c_sq: Vec<f32>,
    damping: Vec<f32>,
    gjs: Vec<f32>,
    active: Vec<f32>,
    granular_share: Vec<f32>,
    col_r: Vec<f32>,
    col_g: Vec<f32>,
    col_b: Vec<f32>,
    f: Vec<f32>,
    pos: Vec<f32>,
    neg: Vec<f32>,
    jit: Vec<f32>,
    pos_jit: Vec<f32>,
    neg_jit: Vec<f32>,
    pos2: Vec<f32>,
    neg2: Vec<f32>,
    out_total: Vec<f32>,
    in_total: Vec<f32>,
    out_total_jit: Vec<f32>,
    in_total_jit: Vec<f32>,

    cap: Vec<f32>,
    in_transit: Vec<f32>,
    out2: Vec<f32>,
    in2: Vec<f32>,
    left_amt: Vec<f32>,
    right_amt: Vec<f32>,
    own_amount: Vec<f32>,
    safe_total: Vec<f32>,
    inv_total: Vec<f32>,
    has_amount: Vec<f32>,
    new_col_r: Vec<u8>,
    new_col_g: Vec<u8>,
    new_col_b: Vec<u8>,
    noise_r: Vec<f32>,
    noise_g: Vec<f32>,
    noise_b: Vec<f32>,

    // ---- Hypothesis-3 experiment only (see `stage5_props_e2_copy_variant`): padded per-span
    // copies of the 4 prop channel windows, D-style, tested against E2's real-row-window read to
    // isolate whether locality (small reused buffer vs. a window into the whole-grid SoA row) is
    // what makes E2's stage4+5 cost more than D's despite removing the AoS<->SoA copies.
    prop_win0: Vec<f32>,
    prop_win1: Vec<f32>,
    prop_win2: Vec<f32>,
    prop_win3: Vec<f32>,
}

impl Scratch {
    pub fn new(state: &StateE) -> Self {
        let spans = build_spans_raw(state.cols, state.rows, state.block_size, state.w, state.h, &state.sim_blocks);
        let n_spans = spans.len();
        let mut pad_off = Vec::with_capacity(n_spans + 1);
        let mut flat_off = Vec::with_capacity(n_spans + 1);
        pad_off.push(0);
        flat_off.push(0);
        for s in &spans {
            let n_data = s.data_end() - s.x_start;
            pad_off.push(pad_off.last().unwrap() + n_data + 2);
            flat_off.push(flat_off.last().unwrap() + n_data);
        }
        let pad_len = *pad_off.last().unwrap();
        let flat_len = *flat_off.last().unwrap();

        let noise = Noise::new(state.time_seed);
        let head_static = kernel_e::precompute_head_static_e(state, &spans);
        Scratch {
            spans,
            noise,
            head_static,
            pad_off,
            flat_off,
            avail: vec![0.0; pad_len],
            freecap: vec![0.0; pad_len],
            head: vec![0.0; pad_len],
            tau: vec![0.0; pad_len],
            c_sq: vec![0.0; pad_len],
            damping: vec![0.0; pad_len],
            gjs: vec![0.0; pad_len],
            active: vec![0.0; pad_len],
            granular_share: vec![0.0; pad_len],
            col_r: vec![0.0; pad_len],
            col_g: vec![0.0; pad_len],
            col_b: vec![0.0; pad_len],
            f: vec![0.0; pad_len],
            pos: vec![0.0; pad_len],
            neg: vec![0.0; pad_len],
            jit: vec![0.0; pad_len],
            pos_jit: vec![0.0; pad_len],
            neg_jit: vec![0.0; pad_len],
            pos2: vec![0.0; pad_len],
            neg2: vec![0.0; pad_len],
            out_total: vec![0.0; pad_len],
            in_total: vec![0.0; pad_len],
            out_total_jit: vec![0.0; pad_len],
            in_total_jit: vec![0.0; pad_len],
            cap: vec![0.0; flat_len],
            in_transit: vec![0.0; flat_len],
            out2: vec![0.0; flat_len],
            in2: vec![0.0; flat_len],
            left_amt: vec![0.0; flat_len],
            right_amt: vec![0.0; flat_len],
            own_amount: vec![0.0; flat_len],
            safe_total: vec![0.0; flat_len],
            inv_total: vec![0.0; flat_len],
            has_amount: vec![0.0; flat_len],
            new_col_r: vec![0; flat_len],
            new_col_g: vec![0; flat_len],
            new_col_b: vec![0; flat_len],
            noise_r: vec![0.0; flat_len],
            noise_g: vec![0.0; flat_len],
            noise_b: vec![0.0; flat_len],
            prop_win0: vec![0.0; pad_len],
            prop_win1: vec![0.0; pad_len],
            prop_win2: vec![0.0; pad_len],
            prop_win3: vec![0.0; pad_len],
        }
    }
}

pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

/// Stage 1: D's `copy_in` minus the raw `h`/prop copy -- reads `wetness`/`threshold`/`grain_size`/
/// `h` straight from `state`'s row-contiguous SoA slices.
#[inline(never)]
pub(crate) fn stage1_e2(state: &StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize, bf: usize) {
    let last = bp + n_data + 1;
    scratch.tau[bp] = 0.0;
    scratch.tau[last] = 0.0;
    scratch.c_sq[bp] = 0.0;
    scratch.c_sq[last] = 0.0;
    scratch.damping[bp] = 0.0;
    scratch.damping[last] = 0.0;
    scratch.gjs[bp] = 0.05;
    scratch.gjs[last] = 0.05;
    scratch.active[bp] = 0.0;
    scratch.active[last] = 0.0;
    scratch.granular_share[bp] = 0.0;
    scratch.granular_share[last] = 0.0;
    scratch.head[bp] = 0.0;
    scratch.head[last] = 0.0;

    let row = span.y * w;
    let x0 = span.x_start;
    let mask_row = &state.shape_mask[row + x0..row + x0 + n_data];
    let h_row = &state.heights[row + x0..row + x0 + n_data];
    let wet_row = &state.prop[PROP_WETNESS][row + x0..row + x0 + n_data];
    let thr_row = &state.prop[PROP_THRESHOLD][row + x0..row + x0 + n_data];
    let grain_row = &state.prop[PROP_GRAIN_SIZE][row + x0..row + x0 + n_data];
    let head_static_row = &scratch.head_static[row + x0..row + x0 + n_data];

    for i in 0..n_data {
        let p = bp + i + 1;
        let inside = mask_ne(mask_row[i] as f32, MASK_OUTSIDE as f32);
        let hh = h_row[i];
        let wetness = wet_row[i];
        let threshold = thr_row[i];
        let grain_size = grain_row[i];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        let granular_share = 1.0 - liq;
        let (c_sq, damping) = wave_params_bf(wetness);

        scratch.active[p] = inside;
        scratch.granular_share[p] = granular_share;
        scratch.tau[p] = GRANULAR_TAU_SCALE * threshold * granular_share;
        scratch.c_sq[p] = c_sq;
        scratch.damping[p] = damping;
        scratch.gjs[p] = grain_jitter_strength_bf(grain_size, granular_share);
        scratch.head[p] = hh + head_static_row[i];
        scratch.cap[bf + i] = cap;
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
        &mut scratch.in_transit[bf..bf + n_data],
    );

    for i in 0..n_data {
        let p = bp + i + 1;
        let avail_raw = (h_row[i] - scratch.in_transit[bf + i]).max(0.0);
        scratch.avail[p] = avail_raw * scratch.active[p];
        scratch.freecap[p] = (scratch.cap[bf + i] - h_row[i]).max(0.0) * scratch.active[p];
    }
    scratch.avail[bp] = 0.0;
    scratch.avail[last] = 0.0;
    scratch.freecap[bp] = 0.0;
    scratch.freecap[last] = 0.0;
}

/// Stage 2: D's stage 2 unchanged, except `scratch.h[p]`/`scratch.h[p+1]` (removed) become direct
/// `state.heights[idx]`/`state.heights[ridx]` reads.
#[inline(never)]
pub(crate) fn stage2_e2(state: &StateE, scratch: &mut Scratch, span: Span, w: usize, n_edges: usize, bp: usize) {
    let elast = bp + n_edges + 1;
    scratch.f[bp] = 0.0;
    scratch.f[elast] = 0.0;

    let off_disp = scratch.noise.row_offset(span.y, SALT_DISPERSION, n_edges.max(1));
    let off_lock = scratch.noise.row_offset(span.y, SALT_LOCK, n_edges.max(1));
    let disp_row = scratch.noise.slice(off_disp, n_edges);
    let lock_row = scratch.noise.slice(off_lock, n_edges);
    let row = span.y * w;
    let x0 = span.x_start;

    for e in 0..n_edges {
        let idx = row + x0 + e;
        let ridx = idx + 1;
        let p = bp + e + 1;

        let active_e = scratch.active[p] * scratch.active[p + 1];
        let tau = scratch.tau[p];

        let q8 = (disp_row[e] * 256.0).floor().min(255.0) / 255.0;
        let dispersion = (q8 - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;
        let head_a = scratch.head[p] + dispersion;
        let head_b = scratch.head[p + 1];
        let driving = head_a - head_b;

        let q16 = (lock_row[e] * 65536.0).floor().min(65535.0) / 65535.0;
        let lock_mask = mask_lt(q16, GRAVITY_LOCK_CHANCE * scratch.granular_share[p]);

        let v_prev = state.edge_vel_h[idx];
        let sleep_mask = edge_sleeps_bf(driving, tau, v_prev, state.heights[idx], state.heights[ridx], scratch.freecap[p], scratch.freecap[p + 1]);
        let inactive = lock_mask.max(sleep_mask);

        let raw_candidate = flux_edge_candidate_bf(
            head_a,
            head_b,
            scratch.c_sq[p],
            scratch.damping[p],
            tau,
            scratch.avail[p],
            scratch.avail[p + 1],
            scratch.freecap[p + 1],
            scratch.freecap[p],
            v_prev,
        );
        scratch.f[p] = raw_candidate * (1.0 - inactive) * active_e;
    }
}

/// Stage 3: D's stage 3 unchanged, plus writing the final flux into `state.edge_vel_h[idx]`
/// directly (D defers this write to `copy_out`; E2 has no copy_out).
#[inline(never)]
pub(crate) fn stage3_e2(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize) -> f64 {
    let dlast = bp + n_data + 1;
    let elast = bp + n_edges + 1;
    scratch.pos[bp] = 0.0;
    scratch.pos[elast] = 0.0;
    scratch.neg[bp] = 0.0;
    scratch.neg[elast] = 0.0;
    scratch.jit[bp] = 0.0;
    scratch.jit[elast] = 0.0;
    scratch.pos_jit[bp] = 0.0;
    scratch.pos_jit[elast] = 0.0;
    scratch.neg_jit[bp] = 0.0;
    scratch.neg_jit[elast] = 0.0;
    scratch.out_total[bp] = 0.0;
    scratch.out_total[dlast] = 0.0;
    scratch.in_total[bp] = 0.0;
    scratch.in_total[dlast] = 0.0;
    scratch.out_total_jit[bp] = 0.0;
    scratch.out_total_jit[dlast] = 0.0;
    scratch.in_total_jit[bp] = 0.0;
    scratch.in_total_jit[dlast] = 0.0;

    let off_jit = scratch.noise.row_offset(span.y, SALT_JITTER, n_edges.max(1));
    let jit_row = scratch.noise.slice(off_jit, n_edges);

    for e in 0..n_edges {
        let p = bp + e + 1;
        let f = scratch.f[p];
        let mag = f.abs();
        let donor_is_e = mask_ge(f, 0.0);
        let s_donor = select(donor_is_e, scratch.gjs[p], scratch.gjs[p + 1]);
        let jit = edge_share_jitter_bf(s_donor, jit_row[e]);
        let pos = mag * donor_is_e;
        let neg = mag * (1.0 - donor_is_e);
        scratch.pos[p] = pos;
        scratch.neg[p] = neg;
        scratch.jit[p] = jit;
        scratch.pos_jit[p] = pos * jit;
        scratch.neg_jit[p] = neg * jit;
    }

    for i in 0..n_data {
        let p = bp + i + 1;
        scratch.out_total[p] = scratch.pos[p] + scratch.neg[p - 1];
        scratch.in_total[p] = scratch.neg[p] + scratch.pos[p - 1];
        scratch.out_total_jit[p] = scratch.pos_jit[p] + scratch.neg_jit[p - 1];
        scratch.in_total_jit[p] = scratch.neg_jit[p] + scratch.pos_jit[p - 1];
    }

    let row = span.y * w;
    let x0 = span.x_start;
    let mut total_flow = 0.0f64;
    for e in 0..n_edges {
        let p = bp + e + 1;
        let f = scratch.f[p];
        let donor_is_e = mask_ge(f, 0.0);
        let donor_avail = select(donor_is_e, scratch.avail[p], scratch.avail[p + 1]);
        let donor_out = select(donor_is_e, scratch.out_total[p], scratch.out_total[p + 1]);
        let donor_out_jit = select(donor_is_e, scratch.out_total_jit[p], scratch.out_total_jit[p + 1]);
        let acc_in = select(donor_is_e, scratch.in_total[p + 1], scratch.in_total[p]);
        let acc_in_jit = select(donor_is_e, scratch.in_total_jit[p + 1], scratch.in_total_jit[p]);
        let acc_free = select(donor_is_e, scratch.freecap[p + 1], scratch.freecap[p]);
        let jitter = scratch.jit[p];

        let scale = edge_arbitration_scale_bf(donor_out, donor_out_jit, donor_avail, acc_in, acc_in_jit, acc_free, jitter);
        let final_flux = f * scale;
        scratch.f[p] = final_flux;

        let idx = row + x0 + e;
        state.edge_vel_h[idx] = final_flux;

        let realized_mask = mask_gt(final_flux.abs(), MIN_FLUX);
        total_flow += (final_flux.abs() * realized_mask) as f64;
    }
    total_flow
}

/// Stage 4, part 1: `pos2`/`neg2` from the final flux (D's `f`, ghost-zeroed exactly as D leaves it).
#[inline(never)]
pub(crate) fn stage4_realized_e2(scratch: &mut Scratch, n_edges: usize, bp: usize) {
    let elast = bp + n_edges + 1;
    scratch.pos2[bp] = 0.0;
    scratch.pos2[elast] = 0.0;
    scratch.neg2[bp] = 0.0;
    scratch.neg2[elast] = 0.0;
    for e in 0..n_edges {
        let p = bp + e + 1;
        let f = scratch.f[p];
        let mask = mask_gt(f.abs(), MIN_FLUX);
        let fm = f * mask;
        scratch.pos2[p] = fm.max(0.0);
        scratch.neg2[p] = (-fm).max(0.0);
    }
}

/// Stage 4, part 2: gather realized out/in per cell (D's exact gather form).
#[inline(never)]
pub(crate) fn stage4_gather_e2(scratch: &mut Scratch, n_data: usize, bp: usize, bf: usize) {
    for i in 0..n_data {
        let p = bp + i + 1;
        scratch.out2[bf + i] = scratch.pos2[p] + scratch.neg2[p - 1];
        scratch.in2[bf + i] = scratch.neg2[p] + scratch.pos2[p - 1];
    }
}

/// Stage 4, part 3: `h_new` (written straight into `state.heights_b`, no scratch/copy-out) plus
/// `own_amount`/`safe_total`/`has_amount`/`left_amt`/`right_amt` (and, for E2-recip, `inv_total`),
/// D's exact formulas.
#[inline(never)]
pub(crate) fn stage4_amounts_e2<const RECIP: bool>(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    for i in 0..n_data {
        let p = bp + i + 1;
        let idx = row + x0 + i;
        let h_old = state.heights[idx];
        let out_flow = scratch.out2[bf + i];
        let in_flow = scratch.in2[bf + i];
        let h_new = (h_old - out_flow + in_flow).max(0.0);
        state.heights_b[idx] = h_new;

        let kept = (h_old - out_flow).max(0.0);
        let left_flux = scratch.f[p - 1];
        let right_flux = scratch.f[p];
        scratch.left_amt[bf + i] = left_flux * mask_gt(left_flux, MIN_FLUX);
        scratch.right_amt[bf + i] = (-right_flux) * mask_lt(right_flux, -MIN_FLUX);

        let own_amount = kept * mask_gt(h_new, 1e-6);
        let total_amount = own_amount + in_flow;
        scratch.own_amount[bf + i] = own_amount;
        let safe_total = total_amount.max(1e-12);
        scratch.safe_total[bf + i] = safe_total;
        scratch.has_amount[bf + i] = mask_gt(total_amount, 1e-6);
        if RECIP {
            scratch.inv_total[bf + i] = 1.0 / safe_total;
        }
    }
}

/// D's `mix_prop_channel`, generic over `RECIP`, fed a WINDOW of the real `state.prop[ch]` row
/// (one real cell of margin either side) instead of a padded scratch copy.
fn mix_prop_channel_e2<const RECIP: bool>(prop_window: &[f32], own_amount: &[f32], left_amt: &[f32], right_amt: &[f32], safe_total: &[f32], inv_total: &[f32], has_amount: &[f32], n_data: usize, out: &mut [f32]) {
    for i in 0..n_data {
        let p = i + 1;
        let own_val = prop_window[p];
        let mixed = prop_window[p - 1] * left_amt[i] + prop_window[p + 1] * right_amt[i];
        let computed = if RECIP { (own_val * own_amount[i] + mixed) * inv_total[i] } else { (own_val * own_amount[i] + mixed) / safe_total[i] };
        out[i] = select(has_amount[i], computed, own_val);
    }
}

/// Stage 5, props: one tight loop per channel, real-row window in, `state.prop_b` out directly.
#[inline(never)]
pub(crate) fn stage5_props_e2<const RECIP: bool>(state: &mut StateE, scratch: &Scratch, span: Span, w: usize, n_data: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let lo = row + x0 - 1;
    let hi = row + x0 + n_data + 1;
    let ff = bf..bf + n_data;
    for ch in 0..4 {
        let window = &state.prop[ch][lo..hi];
        let out = &mut state.prop_b[ch][row + x0..row + x0 + n_data];
        mix_prop_channel_e2::<RECIP>(window, &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], n_data, out);
    }
}

/// Hypothesis-3 experiment: same math as `stage5_props_e2`, but each channel's window is first
/// copied into a small, per-span-reused padded buffer (`scratch.prop_win{0..3}`, D's own scheme)
/// before mixing, instead of mixing directly off a window into the real, whole-grid `state.prop[ch]`
/// row. Isolates whether reading a small L1-resident buffer beats reading a window of a large
/// array touched by many other spans this same pass -- never wired into `run_pass`; only reachable
/// via `run_stage45_copy_variant`, a timing-only alternate path.
#[inline(never)]
pub(crate) fn stage5_props_e2_copy_variant<const RECIP: bool>(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let lo = row + x0 - 1;
    let hi = row + x0 + n_data + 1;
    let n_win = hi - lo;
    for (ch, dst) in [&mut scratch.prop_win0, &mut scratch.prop_win1, &mut scratch.prop_win2, &mut scratch.prop_win3].into_iter().enumerate() {
        let src = &state.prop[ch][lo..hi];
        dst[bp..bp + n_win].copy_from_slice(src);
    }
    let ff = bf..bf + n_data;
    let pp = bp..bp + n_win;
    for ch in 0..4 {
        let window: &[f32] = match ch {
            0 => &scratch.prop_win0[pp.clone()],
            1 => &scratch.prop_win1[pp.clone()],
            2 => &scratch.prop_win2[pp.clone()],
            _ => &scratch.prop_win3[pp.clone()],
        };
        let out = &mut state.prop_b[ch][row + x0..row + x0 + n_data];
        mix_prop_channel_e2::<RECIP>(window, &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], n_data, out);
    }
}

/// Kernel F (hypothesis-1 hybrid): the fixed chunk width tested for a cheap chunk-level "any flow
/// in this chunk?" test before paying for the branch-free mixing math.
/// Swept 8/32/64 (see the report): 8 was noise-dominated and sometimes SLOWER than E2 (per-chunk
/// overhead -- the `.any()` scan, the small stack-array colour unpack, breaking one span-wide
/// vectorisable loop into many tiny ones -- ate most of the skip's benefit, especially on the
/// gradient scene where only ~7% of 8-cell chunks are entirely flow-free). 32 was the best of the
/// three, a small (~1-5%) but consistent win on both scenes; 64 was no better than 32.
const F_CHUNK: usize = 32;

/// Kernel F, props: identical to `stage5_props_e2`'s math, restructured into `F_CHUNK`-cell
/// chunks. When `has_amount` is `0.0` for EVERY cell in a chunk, `mix_prop_channel_e2` would write
/// exactly `own_val` for every channel and cell in it (see `select`'s definition: the `computed`
/// branch is never selected) -- so that case is replaced with a straight `copy_from_slice` of the
/// frozen `state.prop[ch]` row into `state.prop_b[ch]`, skipping the mixed-value arithmetic
/// entirely. A chunk with ANY flowing cell still runs the exact same branch-free math as E2, on a
/// `len`-wide (<= `F_CHUNK`) window instead of the whole span -- same formula, smaller slice.
#[inline(never)]
pub(crate) fn stage5_props_e2_chunked<const RECIP: bool>(state: &mut StateE, scratch: &Scratch, span: Span, w: usize, n_data: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let mut i = 0;
    while i < n_data {
        let len = F_CHUNK.min(n_data - i);
        let ff = bf + i..bf + i + len;
        let any_flow = scratch.has_amount[ff.clone()].iter().any(|&a| a != 0.0);
        let base = row + x0 + i;
        if !any_flow {
            for ch in 0..4 {
                state.prop_b[ch][base..base + len].copy_from_slice(&state.prop[ch][base..base + len]);
            }
            i += len;
            continue;
        }
        let lo = base - 1;
        let hi = base + len + 1;
        for ch in 0..4 {
            let window = &state.prop[ch][lo..hi];
            let out_vals = {
                let mut tmp = [0.0f32; F_CHUNK];
                mix_prop_channel_e2::<RECIP>(window, &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], len, &mut tmp[..len]);
                tmp
            };
            state.prop_b[ch][base..base + len].copy_from_slice(&out_vals[..len]);
        }
        i += len;
    }
}

/// Kernel F, colours: same chunk-skip idea as `stage5_props_e2_chunked`, but colour needs its own
/// small stack-allocated unpack (the packed-`u32` layout has no per-channel real array to window
/// into at chunk granularity the way props does). Draws noise from the SAME whole-span
/// `row_offset`/`slice` E2 uses (computed once, outside the chunk loop) so a flowing chunk
/// consumes EXACTLY the same table entries at the same (cell, channel) position as E2 -- required
/// for bit-identical output, not just "close".
#[inline(never)]
pub(crate) fn stage5_colours_e2_chunked<const RECIP: bool>(state: &mut StateE, scratch: &Scratch, span: Span, w: usize, n_data: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let off_col = scratch.noise.row_offset(span.y, SALT_COLOR, (n_data * 4).max(1));
    let color_noise = scratch.noise.slice(off_col, n_data * 4);

    let mut i = 0;
    while i < n_data {
        let len = F_CHUNK.min(n_data - i);
        let ff = bf + i..bf + i + len;
        let any_flow = scratch.has_amount[ff.clone()].iter().any(|&a| a != 0.0);
        let base = row + x0 + i;
        if !any_flow {
            state.colors_b[base..base + len].copy_from_slice(&state.colors[base..base + len]);
            i += len;
            continue;
        }
        let lo = base - 1;
        let win_len = len + 2;
        let mut col_r = [0.0f32; F_CHUNK + 2];
        let mut col_g = [0.0f32; F_CHUNK + 2];
        let mut col_b = [0.0f32; F_CHUNK + 2];
        for k in 0..win_len {
            let c = state.colors[lo + k];
            col_r[k] = (c & 0xFF) as f32;
            col_g[k] = ((c >> 8) & 0xFF) as f32;
            col_b[k] = ((c >> 16) & 0xFF) as f32;
        }
        let mut noise_r = [0.0f32; F_CHUNK];
        let mut noise_g = [0.0f32; F_CHUNK];
        let mut noise_b = [0.0f32; F_CHUNK];
        let noise_slice = &color_noise[i * 4..(i + len) * 4];
        for k in 0..len {
            noise_r[k] = noise_slice[k * 4];
            noise_g[k] = noise_slice[k * 4 + 1];
            noise_b[k] = noise_slice[k * 4 + 2];
        }
        let mut new_r = [0u8; F_CHUNK];
        let mut new_g = [0u8; F_CHUNK];
        let mut new_b = [0u8; F_CHUNK];
        mix_color_channel_e2::<RECIP>(&col_r[..win_len], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &noise_r[..len], len, &mut new_r[..len]);
        mix_color_channel_e2::<RECIP>(&col_g[..win_len], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &noise_g[..len], len, &mut new_g[..len]);
        mix_color_channel_e2::<RECIP>(&col_b[..win_len], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &noise_b[..len], len, &mut new_b[..len]);
        for k in 0..len {
            let idx = base + k;
            state.colors_b[idx] = (new_r[k] as u32) | ((new_g[k] as u32) << 8) | ((new_b[k] as u32) << 16) | (255u32 << 24);
        }
        i += len;
    }
}

/// Kernel F's stage4+5: E2's own stage4 (realize/gather/amounts, unchanged -- every cell's
/// `has_amount` flag must be computed before any chunk can be skipped, so there is nothing to
/// skip there) followed by the two chunked mixing stages above.
#[inline(never)]
pub(crate) fn stage45_f(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
    stage4_realized_e2(scratch, n_edges, bp);
    stage4_gather_e2(scratch, n_data, bp, bf);
    stage4_amounts_e2::<false>(state, scratch, span, w, n_data, bp, bf);
    stage5_props_e2_chunked::<false>(state, scratch, span, w, n_data, bf);
    stage5_colours_e2_chunked::<false>(state, scratch, span, w, n_data, bf);
}

pub fn run_stage45_f(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage45_f(state, scratch, span, w, n_data, n_edges, bp, bf);
    }
}

/// Kernel F, full pass: E2's stage1/2/3 (unchanged -- hypothesis 1's sparsity in the ARBITRATION
/// stages is real too, but restructuring those safely was out of scope for this experiment; see
/// the report) plus the chunked stage4+5 above.
pub fn run_pass_f(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_stage1(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45_f(state, scratch);
    kernel_e::run_swap(state);
    total_flow
}

/// Colour, part 1: unpack r/g/b lane-wise from the frozen packed `u32` row into padded scratch
/// (D's `col_r`/`col_g`/`col_b` fields) -- the one small, span-sized conversion the packed layout
/// makes unavoidable (not a whole-grid copy). Ghost slots get real casing-cell bytes, same
/// zero-by-construction argument as `kernel_e.rs`'s doc comment (left_amt/right_amt are already
/// zero there, so whatever real byte lands in the ghost is inert).
#[inline(never)]
pub(crate) fn unpack_colours_e2(state: &StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    let lo = row + x0 - 1;
    for i in 0..n_data + 2 {
        let c = state.colors[lo + i];
        let p = bp + i;
        scratch.col_r[p] = (c & 0xFF) as f32;
        scratch.col_g[p] = ((c >> 8) & 0xFF) as f32;
        scratch.col_b[p] = ((c >> 16) & 0xFF) as f32;
    }
}

/// D's `mix_color_channel`, generic over `RECIP`.
fn mix_color_channel_e2<const RECIP: bool>(col_pad: &[f32], own_amount: &[f32], left_amt: &[f32], right_amt: &[f32], safe_total: &[f32], inv_total: &[f32], has_amount: &[f32], noise: &[f32], n_data: usize, out: &mut [u8]) {
    for i in 0..n_data {
        let p = i + 1;
        let own_val = col_pad[p];
        let mixed = col_pad[p - 1] * left_amt[i] + col_pad[p + 1] * right_amt[i];
        let raw = if RECIP { (own_val * own_amount[i] + mixed) * inv_total[i] } else { (own_val * own_amount[i] + mixed) / safe_total[i] };
        let computed = raw.clamp(0.0, 255.0);
        let new_c = select(has_amount[i], computed, own_val);
        let rounded = stochastic_round_bf(new_c, noise[i]).clamp(0.0, 255.0);
        out[i] = rounded as u8;
    }
}

/// Colour, part 3: repack the 3 per-channel `u8` results into `state.colors_b`, alpha 255.
#[inline(never)]
pub(crate) fn repack_colours_e2(state: &mut StateE, scratch: &Scratch, span: Span, w: usize, n_data: usize, bf: usize) {
    let row = span.y * w;
    let x0 = span.x_start;
    for i in 0..n_data {
        let idx = row + x0 + i;
        let r = scratch.new_col_r[bf + i] as u32;
        let g = scratch.new_col_g[bf + i] as u32;
        let b = scratch.new_col_b[bf + i] as u32;
        state.colors_b[idx] = r | (g << 8) | (b << 16) | (255u32 << 24);
    }
}

/// Colour, part 2 (orchestrator): unpack -> per-channel mix loops -> repack.
#[inline(never)]
pub(crate) fn stage5_colours_e2<const RECIP: bool>(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize, bf: usize) {
    unpack_colours_e2(state, scratch, span, w, n_data, bp);

    let off_col = scratch.noise.row_offset(span.y, SALT_COLOR, (n_data * 4).max(1));
    let color_noise = scratch.noise.slice(off_col, n_data * 4);
    for i in 0..n_data {
        scratch.noise_r[bf + i] = color_noise[i * 4];
        scratch.noise_g[bf + i] = color_noise[i * 4 + 1];
        scratch.noise_b[bf + i] = color_noise[i * 4 + 2];
    }

    let pp = bp..bp + n_data + 2;
    let ff = bf..bf + n_data;
    mix_color_channel_e2::<RECIP>(&scratch.col_r[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_r[ff.clone()], n_data, &mut scratch.new_col_r[ff.clone()]);
    mix_color_channel_e2::<RECIP>(&scratch.col_g[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_g[ff.clone()], n_data, &mut scratch.new_col_g[ff.clone()]);
    mix_color_channel_e2::<RECIP>(&scratch.col_b[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.inv_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_b[ff.clone()], n_data, &mut scratch.new_col_b[ff.clone()]);

    repack_colours_e2(state, scratch, span, w, n_data, bf);
}

#[inline(never)]
pub(crate) fn stage45_e2(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
    stage4_realized_e2(scratch, n_edges, bp);
    stage4_gather_e2(scratch, n_data, bp, bf);
    stage4_amounts_e2::<false>(state, scratch, span, w, n_data, bp, bf);
    stage5_props_e2::<false>(state, scratch, span, w, n_data, bf);
    stage5_colours_e2::<false>(state, scratch, span, w, n_data, bp, bf);
}

#[inline(never)]
pub(crate) fn stage45_e2_recip(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
    stage4_realized_e2(scratch, n_edges, bp);
    stage4_gather_e2(scratch, n_data, bp, bf);
    stage4_amounts_e2::<true>(state, scratch, span, w, n_data, bp, bf);
    stage5_props_e2::<true>(state, scratch, span, w, n_data, bf);
    stage5_colours_e2::<true>(state, scratch, span, w, n_data, bp, bf);
}

/// Hypothesis-3 experiment: `stage45_e2` with `stage5_props_e2_copy_variant` in place of
/// `stage5_props_e2` -- everything else (stage4, colours) identical. Timing-only; never called by
/// `run_pass`.
#[inline(never)]
pub(crate) fn stage45_e2_copy_variant(state: &mut StateE, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
    stage4_realized_e2(scratch, n_edges, bp);
    stage4_gather_e2(scratch, n_data, bp, bf);
    stage4_amounts_e2::<false>(state, scratch, span, w, n_data, bp, bf);
    stage5_props_e2_copy_variant::<false>(state, scratch, span, w, n_data, bp, bf);
    stage5_colours_e2::<false>(state, scratch, span, w, n_data, bp, bf);
}

pub fn run_stage45_copy_variant(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage45_e2_copy_variant(state, scratch, span, w, n_data, n_edges, bp, bf);
    }
}

/// Full pass using the copy-variant stage4+5, for equivalence checking against `run_pass` (must
/// be bit-identical -- the copy is a pure data-movement change, same values, same math).
pub fn run_pass_copy_variant(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_stage1(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45_copy_variant(state, scratch);
    kernel_e::run_swap(state);
    total_flow
}

pub fn run_stage1(state: &StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage1_e2(state, scratch, span, w, n_data, bp, bf);
    }
}

pub fn run_stage2(state: &StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        stage2_e2(state, scratch, span, w, n_edges, bp);
    }
}

pub fn run_stage3(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let mut total = 0.0f64;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        total += stage3_e2(state, scratch, span, w, n_data, n_edges, bp);
    }
    total
}

/// Hypothesis-3/4 diagnostic: `run_stage45`'s first three sub-steps only (realize/gather/amounts,
/// no prop or colour mixing) -- timing-only, isolates how much of stage4+5's cost is the
/// arithmetic BEFORE any per-channel mixing loop runs.
pub fn run_stage4_only(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage4_realized_e2(scratch, n_edges, bp);
        stage4_gather_e2(scratch, n_data, bp, bf);
        stage4_amounts_e2::<false>(state, scratch, span, w, n_data, bp, bf);
    }
}

/// Hypothesis-3/4 diagnostic: the 4-prop-channel mixing loops alone (needs `run_stage4_only` to
/// have already populated `own_amount`/`left_amt`/`right_amt`/`safe_total`/`has_amount`).
pub fn run_stage5_props_only(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bf = scratch.flat_off[si];
        stage5_props_e2::<false>(state, scratch, span, w, n_data, bf);
    }
}

/// Hypothesis-3/4 diagnostic: colour unpack + mix + repack alone (same precondition as above).
pub fn run_stage5_colours_only(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage5_colours_e2::<false>(state, scratch, span, w, n_data, bp, bf);
    }
}

pub fn run_stage45(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage45_e2(state, scratch, span, w, n_data, n_edges, bp, bf);
    }
}

pub fn run_stage45_recip(state: &mut StateE, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage45_e2_recip(state, scratch, span, w, n_data, n_edges, bp, bf);
    }
}

pub fn run_pass(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_stage1(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45(state, scratch);
    kernel_e::run_swap(state);
    total_flow
}

/// Hypothesis-1/2 diagnostic for E2: runs the real stage functions (so the counted values are
/// exactly what `run_pass` computes), then reads the post-stage scratch/state to count sparsity
/// and subnormals. Safe to call once on a fresh `Scratch`/`StateE` the way `run_pass` is; does not
/// call `run_swap`, so it leaves `state`'s "next" buffers populated but does not commit them --
/// call this INSTEAD of `run_pass` for a census, never in addition to it on the same state.
pub fn census_pass(state: &mut StateE, scratch: &mut Scratch) -> crate::census::Census {
    use crate::census::{is_subnormal, Census};
    let mut census = Census::default();
    let w = state.w;

    run_stage1(state, scratch);

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bp = scratch.pad_off[si];
        for i in 0..n_data {
            let p = bp + i + 1;
            if is_subnormal(scratch.head[p]) || is_subnormal(scratch.avail[p]) || is_subnormal(scratch.freecap[p]) {
                census.subnormal_head_avail_freecap += 1;
            }
        }
    }

    run_stage2(state, scratch);

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        for e in 0..n_edges {
            let p = bp + e + 1;
            let c = scratch.f[p];
            census.edges_total += 1;
            if c != 0.0 {
                census.edges_nonzero_candidate += 1;
                if is_subnormal(c) {
                    census.subnormal_candidate += 1;
                }
            }
        }
    }

    let _ = run_stage3(state, scratch);

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_edges = span.x_owned_end - span.x_start;
        let row = span.y * w;
        let x0 = span.x_start;
        let bp = scratch.pad_off[si];
        for e in 0..n_edges {
            let idx = row + x0 + e;
            let f = state.edge_vel_h[idx];
            if f.abs() > MIN_FLUX {
                census.edges_nonzero_final += 1;
            }
            if is_subnormal(f) {
                census.subnormal_final += 1;
            }
            let p = bp + e + 1;
            if is_subnormal(scratch.pos[p]) || is_subnormal(scratch.neg[p]) || is_subnormal(scratch.pos_jit[p]) || is_subnormal(scratch.neg_jit[p]) {
                census.subnormal_pos_neg += 1;
            }
        }
    }

    run_stage45(state, scratch);

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bf = scratch.flat_off[si];
        for i in 0..n_data {
            census.cells_total += 1;
            let o = scratch.out2[bf + i];
            let inn = scratch.in2[bf + i];
            if o != 0.0 || inn != 0.0 {
                census.cells_with_flow += 1;
            }
            if is_subnormal(o) || is_subnormal(inn) || is_subnormal(scratch.own_amount[bf + i]) || is_subnormal(scratch.left_amt[bf + i]) || is_subnormal(scratch.right_amt[bf + i]) {
                census.subnormal_mix_amounts += 1;
            }
        }
    }

    census
}

pub fn run_pass_recip(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    run_stage1(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(state, scratch);
    run_stage45_recip(state, scratch);
    kernel_e::run_swap(state);
    total_flow
}
