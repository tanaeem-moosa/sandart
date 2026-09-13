//! Kernel D: kernel C's exact math (RNG table, offsets, formulas, Jacobi mixing law -- see
//! `kernel_c.rs`'s module doc comment for the law itself, unchanged here), with ONLY the data
//! layout changed so the loop vectoriser can actually act on it. The four blockers the task
//! identified in C, and how D removes each:
//!
//! 1. **Stage 3 scatters** (`out_total[e] += ...; out_total[e+1] += ...`, a read-modify-write into
//!    overlapping slots on consecutive iterations). D never scatters: every per-cell total is
//!    computed by GATHERING from a padded, ghost-terminated per-edge flux array --
//!    `out[i] = pos[i+1] + neg[i]` (cell `i`'s right edge's positive share, plus its left edge's
//!    negative share), a pure two-slice shifted add.
//! 2. **Stage 3 recomputes `grain_jitter_strength` per edge, twice, in two loops.** D computes it
//!    ONCE PER CELL in copy-in (`gjs`), and the per-edge jitter `jit[e]` (a `select` on the donor's
//!    precomputed `gjs`) ONCE, in stage 3's first loop, then reuses the stored value in the second
//!    (arbitration) loop instead of recomputing it.
//! 3. **Stage 4/5 used interleaved AoS** (`props[i*4+ch]`, `if i > 0`/`if i < n_edges` inside a
//!    `for ch in 0..4` loop). D copies each of the 4 props and each of 3 colour channels into its
//!    OWN contiguous `Vec<f32>`, padded with one ghost cell at each end so every neighbour read
//!    (`prop[i-1]`, `prop[i+1]`) is unconditional -- no bounds check, no interleave, one tight loop
//!    per channel over plain slices.
//! 4. **The precompute ran over all grid cells.** `precompute_head_static_d` only visits the cells
//!    that appear in some span's data range (i.e. the simulated cells, +1 acceptor column per
//!    span) -- see `precompute_cell_count`.
//!
//! **Padding convention, and why every span gets its OWN slice of one big buffer.** Every per-cell
//! scratch array is one flat `Vec<f32>` covering EVERY span at once: span `k`'s data starts at
//! `pad_off[k]`, real cell `i` (`0..n_data`) of that span lives at `pad_off[k] + i + 1`, and indices
//! `pad_off[k]` and `pad_off[k] + n_data + 1` are that span's own ghost cells (`h = 0`, `avail = 0`,
//! `freecap = 0`, everything else `0` too -- never read for a value that matters, same reasoning as
//! a single-span buffer would need). Per-edge arrays (the flux and its derived `pos`/`neg`/jitter)
//! share the SAME flat buffer and the SAME per-span base offset, since edge `e`'s padded index
//! (`pad_off[k] + e + 1`) coincides with its left cell's. An earlier version of this file reused
//! ONE small buffer (sized to the widest span) across every span, overwriting it span by span; that
//! is correct ONLY if every stage for a span runs before the next span's copy-in starts (as kernel
//! C does, interleaved). This version instead gives every span a permanent slice so the FIVE
//! STAGES can each run across every span independently (`run_copy_in`, `run_stage2`, `run_stage3`,
//! `run_stage45`, `run_copy_out`) -- needed so the bench harness can time each stage on its own
//! from wasm (no clock import there, so each stage needs its own `extern "C"` export bracketed by
//! `performance.now()` from JS) -- without one span's data being clobbered before a later stage
//! reads it. `run_pass` chains all five, in order, for the ns/cell/pass headline number.
//!
//! This is safe with respect to `state` for the same reason the single-buffer version was:
//! `copy_in`/`stage2`/`stage3`/`stage45` never write `state` (only read it), and `copy_out` -- the
//! only writer -- runs last, after every read has already happened for every span.

#![allow(clippy::too_many_arguments)]

use crate::bf_math::*;
use crate::consts::*;
use crate::noise::{Noise, SALT_COLOR, SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use crate::row_span::{build_spans, Span};
use crate::scalar_math::{cell_capacity_for, janssen_effective_depth, k_of_liquidity, liquidity};
use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// Same value as `kernel_c::precompute_head_static`, but visiting only the cells that appear in
/// some span's data range (task rule 4: "the precompute only needs the simulated cells"). The
/// output is still whole-grid-sized so `copy_in` can index it directly by `idx`; every slot this
/// loop does not visit stays `0.0` and is never read (no span ever reads `head_static[idx]` for an
/// `idx` outside its own data range).
#[inline(never)]
pub fn precompute_head_static_d(state: &State, spans: &[Span]) -> Vec<f32> {
    let w = state.w;
    let mut out = vec![0.0f32; state.w * state.h];
    for span in spans {
        let n_data = span.data_end() - span.x_start;
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let wetness = state.cell_props[idx * 4 + PROP_WETNESS];
            let liq = liquidity(wetness);
            let k = k_of_liquidity(liq);
            let depth = janssen_effective_depth(state.column_depth[idx], liq);
            out[idx] = k * LATERAL_PRESSURE_SCALE * depth;
        }
    }
    out
}

/// Number of cells the precompute above actually visits -- the denominator for its ns/cell cost,
/// distinct from `simulated_cell_count`'s "owned edges only" count (this includes each span's `+1`
/// acceptor column, since the precompute must cover it too: stage 1 reads `head_static` for the
/// acceptor exactly like any other cell in the span).
pub fn precompute_cell_count(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.data_end() - s.x_start).sum()
}

pub struct Scratch {
    pub(crate) spans: Vec<Span>,
    pub(crate) noise: Noise,
    pub(crate) head_static: Vec<f32>,

    /// Span `k`'s padded data starts at `pad_off[k]` (length `spans.len() + 1`, cumulative, so
    /// `pad_off[k+1] - pad_off[k] == n_data_k + 2`).
    pad_off: Vec<usize>,
    /// Span `k`'s unpadded (real-cell-only) data starts at `flat_off[k]` (cumulative, `flat_off[k+1]
    /// - flat_off[k] == n_data_k`).
    flat_off: Vec<usize>,

    // ---- Padded per-cell/per-edge scratch, ONE flat buffer for every span (see module doc
    // comment). Real item `i` of span `k` at index `pad_off[k] + i + 1`; that span's ghosts at
    // `pad_off[k]` and `pad_off[k] + n_data_k + 1`.
    h: Vec<f32>,
    avail: Vec<f32>,
    freecap: Vec<f32>,
    head: Vec<f32>,
    tau: Vec<f32>,
    c_sq: Vec<f32>,
    damping: Vec<f32>,
    gjs: Vec<f32>,
    active: Vec<f32>,
    granular_share: Vec<f32>,
    prop0: Vec<f32>,
    prop1: Vec<f32>,
    prop2: Vec<f32>,
    prop3: Vec<f32>,
    col_r: Vec<f32>,
    col_g: Vec<f32>,
    col_b: Vec<f32>,
    f: Vec<f32>,       // per-edge flux: stage2's candidate, then stage3 overwrites with the final
    pos: Vec<f32>,     // max(f, 0)
    neg: Vec<f32>,     // max(-f, 0)
    jit: Vec<f32>,     // edge_share_jitter_bf, computed once per edge
    pos_jit: Vec<f32>, // pos * jit
    neg_jit: Vec<f32>, // neg * jit
    pos2: Vec<f32>,    // stage 4: max(f_final_masked, 0)
    neg2: Vec<f32>,    // stage 4: max(-f_final_masked, 0)
    out_total: Vec<f32>,
    in_total: Vec<f32>,
    out_total_jit: Vec<f32>,
    in_total_jit: Vec<f32>,

    // ---- Unpadded per-cell scratch/output, one flat buffer for every span. Real cell `i` of span
    // `k` at index `flat_off[k] + i`.
    cap: Vec<f32>,
    in_transit: Vec<f32>,
    out2: Vec<f32>,
    in2: Vec<f32>,
    h_new: Vec<f32>,
    left_amt: Vec<f32>,
    right_amt: Vec<f32>,
    own_amount: Vec<f32>,
    safe_total: Vec<f32>,
    has_amount: Vec<f32>,
    new_prop0: Vec<f32>,
    new_prop1: Vec<f32>,
    new_prop2: Vec<f32>,
    new_prop3: Vec<f32>,
    new_col_r: Vec<u8>,
    new_col_g: Vec<u8>,
    new_col_b: Vec<u8>,
    noise_r: Vec<f32>,
    noise_g: Vec<f32>,
    noise_b: Vec<f32>,
}

impl Scratch {
    pub fn new(state: &State) -> Self {
        let spans = build_spans(state);
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
        let head_static = precompute_head_static_d(state, &spans);
        Scratch {
            spans,
            noise,
            head_static,
            pad_off,
            flat_off,
            h: vec![0.0; pad_len],
            avail: vec![0.0; pad_len],
            freecap: vec![0.0; pad_len],
            head: vec![0.0; pad_len],
            tau: vec![0.0; pad_len],
            c_sq: vec![0.0; pad_len],
            damping: vec![0.0; pad_len],
            gjs: vec![0.0; pad_len],
            active: vec![0.0; pad_len],
            granular_share: vec![0.0; pad_len],
            prop0: vec![0.0; pad_len],
            prop1: vec![0.0; pad_len],
            prop2: vec![0.0; pad_len],
            prop3: vec![0.0; pad_len],
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
            h_new: vec![0.0; flat_len],
            left_amt: vec![0.0; flat_len],
            right_amt: vec![0.0; flat_len],
            own_amount: vec![0.0; flat_len],
            safe_total: vec![0.0; flat_len],
            has_amount: vec![0.0; flat_len],
            new_prop0: vec![0.0; flat_len],
            new_prop1: vec![0.0; flat_len],
            new_prop2: vec![0.0; flat_len],
            new_prop3: vec![0.0; flat_len],
            new_col_r: vec![0; flat_len],
            new_col_g: vec![0; flat_len],
            new_col_b: vec![0; flat_len],
            noise_r: vec![0.0; flat_len],
            noise_g: vec![0.0; flat_len],
            noise_b: vec![0.0; flat_len],
        }
    }
}

pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

/// Copy-in: the only AoS -> SoA conversion (task: "time it separately... it tells us whether
/// switching the production layout to SoA would be worth it"). Reads `state` directly (no whole-
/// grid clone -- same ordering argument as `kernel_c.rs`'s module doc comment point 4, strengthened
/// here since NOTHING writes `state` until `copy_out`, run last across every span). `bp`/`bf` are
/// this span's base offsets into the padded/flat scratch buffers (see module doc comment).
#[inline(never)]
pub(crate) fn copy_in(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_data: usize, bp: usize, bf: usize) {
    let last = bp + n_data + 1;
    scratch.h[bp] = 0.0;
    scratch.h[last] = 0.0;
    scratch.head[bp] = 0.0;
    scratch.head[last] = 0.0;
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
    scratch.prop0[bp] = 0.0;
    scratch.prop0[last] = 0.0;
    scratch.prop1[bp] = 0.0;
    scratch.prop1[last] = 0.0;
    scratch.prop2[bp] = 0.0;
    scratch.prop2[last] = 0.0;
    scratch.prop3[bp] = 0.0;
    scratch.prop3[last] = 0.0;
    scratch.col_r[bp] = 0.0;
    scratch.col_r[last] = 0.0;
    scratch.col_g[bp] = 0.0;
    scratch.col_g[last] = 0.0;
    scratch.col_b[bp] = 0.0;
    scratch.col_b[last] = 0.0;

    // Row is contiguous (row-major grid), so `state.heights`/`shape_mask`/`head_static` slice
    // directly, and `cell_props`/`cell_colors` (interleaved 4-per-cell) slice as one `n_data*4`-
    // wide AoS chunk -- same hoist-the-row-slice move as `kernel_c8`'s stage1, tried here to see
    // whether removing the per-field `idx*4+ch` bounds check unlocks more of copy-in, or whether
    // the interleave itself (not the bounds check) is what blocks the vectoriser. See the report:
    // it is the interleave -- this hoist does not change copy-in's v128 op count.
    let row = span.y * w;
    let mask_row = &state.shape_mask[row + span.x_start..row + span.x_start + n_data];
    let h_row = &state.heights[row + span.x_start..row + span.x_start + n_data];
    let head_static_row = &scratch.head_static[row + span.x_start..row + span.x_start + n_data];
    let props_row = &state.cell_props[(row + span.x_start) * 4..(row + span.x_start + n_data) * 4];
    let colors_row = &state.cell_colors[(row + span.x_start) * 4..(row + span.x_start + n_data) * 4];

    for i in 0..n_data {
        let p = bp + i + 1;
        let inside = mask_ne(mask_row[i] as f32, MASK_OUTSIDE as f32);
        let hh = h_row[i];
        let wetness = props_row[i * 4 + PROP_WETNESS];
        let threshold = props_row[i * 4 + PROP_THRESHOLD];
        let grain_size = props_row[i * 4 + PROP_GRAIN_SIZE];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        let granular_share = 1.0 - liq;
        let (c_sq, damping) = wave_params_bf(wetness);

        scratch.h[p] = hh;
        scratch.active[p] = inside;
        scratch.granular_share[p] = granular_share;
        scratch.tau[p] = GRANULAR_TAU_SCALE * threshold * granular_share;
        scratch.c_sq[p] = c_sq;
        scratch.damping[p] = damping;
        scratch.gjs[p] = grain_jitter_strength_bf(grain_size, granular_share);
        scratch.head[p] = hh + head_static_row[i];
        scratch.cap[bf + i] = cap;

        scratch.prop0[p] = props_row[i * 4];
        scratch.prop1[p] = props_row[i * 4 + 1];
        scratch.prop2[p] = props_row[i * 4 + 2];
        scratch.prop3[p] = props_row[i * 4 + 3];
        scratch.col_r[p] = colors_row[i * 4] as f32;
        scratch.col_g[p] = colors_row[i * 4 + 1] as f32;
        scratch.col_b[p] = colors_row[i * 4 + 2] as f32;
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
        &mut scratch.in_transit[bf..bf + n_data],
    );

    for i in 0..n_data {
        let p = bp + i + 1;
        let avail_raw = (scratch.h[p] - scratch.in_transit[bf + i]).max(0.0);
        scratch.avail[p] = avail_raw * scratch.active[p];
        scratch.freecap[p] = (scratch.cap[bf + i] - scratch.h[p]).max(0.0) * scratch.active[p];
    }
    scratch.avail[bp] = 0.0;
    scratch.avail[last] = 0.0;
    scratch.freecap[bp] = 0.0;
    scratch.freecap[last] = 0.0;
}

/// Stage 2, as C, with the dispersion/lock noise slices hoisted out of the loop (no per-edge
/// `noise.slice(...)[e]` re-slice) and `tau`/`c_sq`/`damping` read from copy-in's per-cell arrays
/// instead of recomputed.
#[inline(never)]
pub(crate) fn stage2(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_edges: usize, bp: usize) {
    let elast = bp + n_edges + 1;
    scratch.f[bp] = 0.0;
    scratch.f[elast] = 0.0;

    let off_disp = scratch.noise.row_offset(span.y, SALT_DISPERSION, n_edges.max(1));
    let off_lock = scratch.noise.row_offset(span.y, SALT_LOCK, n_edges.max(1));
    let disp_row = scratch.noise.slice(off_disp, n_edges);
    let lock_row = scratch.noise.slice(off_lock, n_edges);

    for e in 0..n_edges {
        let x = span.x_start + e;
        let idx = span.y * w + x;
        let p = bp + e + 1; // shared edge index / left-cell index

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
        let sleep_mask = edge_sleeps_bf(driving, tau, v_prev, scratch.h[p], scratch.h[p + 1], scratch.freecap[p], scratch.freecap[p + 1]);
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

/// Stage 3, in GATHER form: no scatter, `grain_jitter_strength`/`edge_share_jitter` computed once
/// per edge (not twice). Never touches `state` -- the final flux is written to `scratch.f` and read
/// back by `copy_out`.
#[inline(never)]
pub(crate) fn stage3(scratch: &mut Scratch, span: Span, n_data: usize, n_edges: usize, bp: usize) -> f64 {
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

    // Pass 1: per-edge pos/neg + jitter, computed once (never recomputed in pass 2).
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

    // Per-cell totals, gathered from the two adjacent edges -- no scatter, no read-modify-write.
    for i in 0..n_data {
        let p = bp + i + 1;
        scratch.out_total[p] = scratch.pos[p] + scratch.neg[p - 1];
        scratch.in_total[p] = scratch.neg[p] + scratch.pos[p - 1];
        scratch.out_total_jit[p] = scratch.pos_jit[p] + scratch.neg_jit[p - 1];
        scratch.in_total_jit[p] = scratch.neg_jit[p] + scratch.pos_jit[p - 1];
    }

    // Pass 2: arbitration scale + final flux, reusing pass 1's stored jitter.
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

        let realized_mask = mask_gt(final_flux.abs(), MIN_FLUX);
        total_flow += (final_flux.abs() * realized_mask) as f64;
    }
    total_flow
}

fn mix_prop_channel(prop_pad: &[f32], own_amount: &[f32], left_amt: &[f32], right_amt: &[f32], safe_total: &[f32], has_amount: &[f32], n_data: usize, out: &mut [f32]) {
    for i in 0..n_data {
        let p = i + 1;
        let own_val = prop_pad[p];
        let mixed = prop_pad[p - 1] * left_amt[i] + prop_pad[p + 1] * right_amt[i];
        let computed = (own_val * own_amount[i] + mixed) / safe_total[i];
        out[i] = select(has_amount[i], computed, own_val);
    }
}

fn mix_color_channel(col_pad: &[f32], own_amount: &[f32], left_amt: &[f32], right_amt: &[f32], safe_total: &[f32], has_amount: &[f32], noise: &[f32], n_data: usize, out: &mut [u8]) {
    for i in 0..n_data {
        let p = i + 1;
        let own_val = col_pad[p];
        let mixed = col_pad[p - 1] * left_amt[i] + col_pad[p + 1] * right_amt[i];
        let computed = ((own_val * own_amount[i] + mixed) / safe_total[i]).clamp(0.0, 255.0);
        let new_c = select(has_amount[i], computed, own_val);
        let rounded = stochastic_round_bf(new_c, noise[i]).clamp(0.0, 255.0);
        out[i] = rounded as u8;
    }
}

/// Stages 4+5: recompute realized out/in from the FINAL flux (gather form again, MIN_FLUX-masked
/// exactly like C's `realized`/`total_out_flow`/`total_in_flow`), then one tight loop per property
/// and colour channel over padded slices (task: "Stage 5, one tight loop per channel"). Touches
/// only `scratch` -- no `state` access, so this stage's cost is pure math, no AoS traffic.
#[inline(never)]
pub(crate) fn stage45(scratch: &mut Scratch, span: Span, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
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

    for i in 0..n_data {
        let p = bp + i + 1;
        scratch.out2[bf + i] = scratch.pos2[p] + scratch.neg2[p - 1];
        scratch.in2[bf + i] = scratch.neg2[p] + scratch.pos2[p - 1];
    }

    for i in 0..n_data {
        let p = bp + i + 1;
        let h_old = scratch.h[p];
        let out_flow = scratch.out2[bf + i];
        let in_flow = scratch.in2[bf + i];
        let h_new = (h_old - out_flow + in_flow).max(0.0);
        scratch.h_new[bf + i] = h_new;

        let kept = (h_old - out_flow).max(0.0);
        let left_flux = scratch.f[p - 1];
        let right_flux = scratch.f[p];
        scratch.left_amt[bf + i] = left_flux * mask_gt(left_flux, MIN_FLUX);
        scratch.right_amt[bf + i] = (-right_flux) * mask_lt(right_flux, -MIN_FLUX);

        let own_amount = kept * mask_gt(h_new, 1e-6);
        let total_amount = own_amount + in_flow;
        scratch.own_amount[bf + i] = own_amount;
        scratch.safe_total[bf + i] = total_amount.max(1e-12);
        scratch.has_amount[bf + i] = mask_gt(total_amount, 1e-6);
    }

    let n = n_data;
    let pp = bp..bp + n_data + 2;
    let ff = bf..bf + n;
    mix_prop_channel(&scratch.prop0[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], n, &mut scratch.new_prop0[ff.clone()]);
    mix_prop_channel(&scratch.prop1[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], n, &mut scratch.new_prop1[ff.clone()]);
    mix_prop_channel(&scratch.prop2[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], n, &mut scratch.new_prop2[ff.clone()]);
    mix_prop_channel(&scratch.prop3[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], n, &mut scratch.new_prop3[ff.clone()]);

    // Colour entropy: one interleaved read from the noise table (same offset/width as C, so the
    // SAME table bytes feed the same (cell, channel) pair), de-interleaved ONCE into 3 contiguous
    // per-channel slices so each channel's tight loop stays a plain slice read.
    let off_col = scratch.noise.row_offset(span.y, SALT_COLOR, (n_data * 4).max(1));
    let color_noise = scratch.noise.slice(off_col, n_data * 4);
    for i in 0..n_data {
        scratch.noise_r[bf + i] = color_noise[i * 4];
        scratch.noise_g[bf + i] = color_noise[i * 4 + 1];
        scratch.noise_b[bf + i] = color_noise[i * 4 + 2];
    }
    mix_color_channel(&scratch.col_r[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_r[ff.clone()], n, &mut scratch.new_col_r[ff.clone()]);
    mix_color_channel(&scratch.col_g[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_g[ff.clone()], n, &mut scratch.new_col_g[ff.clone()]);
    mix_color_channel(&scratch.col_b[pp.clone()], &scratch.own_amount[ff.clone()], &scratch.left_amt[ff.clone()], &scratch.right_amt[ff.clone()], &scratch.safe_total[ff.clone()], &scratch.has_amount[ff.clone()], &scratch.noise_b[ff.clone()], n, &mut scratch.new_col_b[ff.clone()]);
}

/// Copy-out: the only place this kernel writes `state` (heights, props, colours, `edge_vel_h`).
/// Timed separately from the math stages, same rationale as copy-in.
#[inline(never)]
pub(crate) fn copy_out(state: &mut State, scratch: &Scratch, span: Span, w: usize, n_data: usize, n_edges: usize, bp: usize, bf: usize) {
    for i in 0..n_data {
        let x = span.x_start + i;
        let idx = span.y * w + x;
        state.heights[idx] = scratch.h_new[bf + i];
        state.cell_props[idx * 4] = scratch.new_prop0[bf + i];
        state.cell_props[idx * 4 + 1] = scratch.new_prop1[bf + i];
        state.cell_props[idx * 4 + 2] = scratch.new_prop2[bf + i];
        state.cell_props[idx * 4 + 3] = scratch.new_prop3[bf + i];
        state.cell_colors[idx * 4] = scratch.new_col_r[bf + i];
        state.cell_colors[idx * 4 + 1] = scratch.new_col_g[bf + i];
        state.cell_colors[idx * 4 + 2] = scratch.new_col_b[bf + i];
        state.cell_colors[idx * 4 + 3] = 255;
    }
    for e in 0..n_edges {
        let x = span.x_start + e;
        let idx = span.y * w + x;
        state.edge_vel_h[idx] = scratch.f[bp + e + 1];
    }
}

/// Runs `copy_in` across every span. Exposed standalone so the bench harness can time this stage
/// in isolation (native: `Instant` around this call; wasm: its own `extern "C"` export). Safe to
/// call independently of the other stages because every span owns a permanent slice of the
/// scratch buffers (see module doc comment) -- unlike a single reused per-span buffer, one span's
/// copy-in can never be overwritten by another span's before a later stage reads it.
pub fn run_copy_in(state: &State, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        copy_in(state, scratch, span, w, n_data, bp, bf);
    }
}

pub fn run_stage2(state: &State, scratch: &mut Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        stage2(state, scratch, span, w, n_edges, bp);
    }
}

pub fn run_stage3(scratch: &mut Scratch) -> f64 {
    let mut total = 0.0f64;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        total += stage3(scratch, span, n_data, n_edges, bp);
    }
    total
}

pub fn run_stage45(scratch: &mut Scratch) {
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        stage45(scratch, span, n_data, n_edges, bp, bf);
    }
}

pub fn run_copy_out(state: &mut State, scratch: &Scratch) {
    let w = state.w;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;
        let bp = scratch.pad_off[si];
        let bf = scratch.flat_off[si];
        copy_out(state, scratch, span, w, n_data, n_edges, bp, bf);
    }
}

pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    run_copy_in(state, scratch);
    run_stage2(state, scratch);
    let total_flow = run_stage3(scratch);
    run_stage45(scratch);
    run_copy_out(state, scratch);
    total_flow
}
