//! Kernel C8: kernel C's exact math (see `kernel_c.rs`'s module doc comment; this file only
//! replaces stages 1 and 2), restructured around `chunks_exact(8)` over slices of the REAL grid
//! arrays -- `state.heights`/`state.shape_mask`/`state.column_depth` are row-major, so a run of
//! `n` cells starting at `(x_start, y)` IS a contiguous slice (`&state.heights[row+x_start ..
//! row+x_start+n]`), no per-span copy needed to iterate it in 8-wide chunks. `state.cell_props`/
//! `state.cell_colors` are interleaved 4-channel-per-cell but still contiguous per row, so an
//! 8-cell chunk of them is a 32-wide contiguous slice, chunked again by 4 inside the loop.
//!
//! `n_edges` is always a multiple of `block_size` (spans are unions of whole simulated blocks;
//! see `row_span.rs`), and the shipped snapshots use `block_size == 8`
//! (`BLOCK-GEOMETRY-2026-09-02.md`'s `DEFAULT_BLOCK_SIZE` at grid 512), so `chunks_exact(8)`
//! consumes every edge with no remainder; `n_data` is `n_edges` or `n_edges + 1` (the `has_extra`
//! acceptor column), so the cell loop has AT MOST one leftover cell, handled by
//! `chunks_exact(8)`'s own `.remainder()`.
//!
//! Stages 3 (arbitration) and 4+5 (Jacobi mix) are IDENTICAL to kernel C -- both are already
//! scalar there (small-fanout, scattered 4-channel reads; see `kernel_c.rs`'s doc comment) and
//! chunking them would not change that, so this file reuses `kernel_c::stage3`/`kernel_c::stage45`
//! rather than duplicating them.

use crate::bf_math::*;
use crate::consts::*;
use crate::kernel_c::{stage3, stage45, Scratch};
use crate::noise::{SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use crate::row_span::Span;
use crate::scalar_math::{cell_capacity_for, liquidity};
use crate::snapshot::State;

pub use crate::kernel_c::{precompute_head_static, simulated_cell_count};

/// Stage 1, chunked. Reads 8 cells at a time directly from `state`'s row-contiguous slices;
/// writes the same per-span scratch arrays kernel C's stage 2/3/4+5 already consume.
#[inline(never)]
fn stage1_c8(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_data: usize) {
    let row = span.y * w;
    let mask_row = &state.shape_mask[row + span.x_start..row + span.x_start + n_data];
    let h_row = &state.heights[row + span.x_start..row + span.x_start + n_data];
    let props_row = &state.cell_props[(row + span.x_start) * 4..(row + span.x_start + n_data) * 4];
    let head_static_row = &scratch.head_static[row + span.x_start..row + span.x_start + n_data];
    let colors_row = &state.cell_colors[(row + span.x_start) * 4..(row + span.x_start + n_data) * 4];

    let mut i = 0usize;
    let mask_chunks = mask_row.chunks_exact(8);
    let h_chunks = h_row.chunks_exact(8);
    let props_chunks = props_row.chunks_exact(32);
    let head_static_chunks = head_static_row.chunks_exact(8);
    let colors_chunks = colors_row.chunks_exact(32);

    for ((((mc, hc), pc), hsc), cc) in mask_chunks.zip(h_chunks).zip(props_chunks).zip(head_static_chunks).zip(colors_chunks) {
        for j in 0..8 {
            let hh = hc[j];
            let wetness = pc[j * 4 + PROP_WETNESS];
            let liq = liquidity(wetness);
            let cap = cell_capacity_for(wetness);
            let inside = mask_ne(mc[j] as f32, MASK_OUTSIDE as f32);
            scratch.h[i + j] = hh;
            scratch.wetness[i + j] = wetness;
            scratch.threshold[i + j] = pc[j * 4 + PROP_THRESHOLD];
            scratch.cap[i + j] = cap;
            scratch.granular_share[i + j] = 1.0 - liq;
            scratch.head_base[i + j] = hh + hsc[j];
            scratch.active[i + j] = inside;
            for ch in 0..4 {
                scratch.props[(i + j) * 4 + ch] = pc[j * 4 + ch];
                scratch.colors[(i + j) * 4 + ch] = cc[j * 4 + ch] as f32;
            }
        }
        i += 8;
    }
    // Remainder: at most one cell (the `has_extra` acceptor column), when `n_data` is not itself
    // a multiple of 8. A loop-bounds tail, not a per-cell data branch -- see `row_span.rs`'s doc
    // comment for why `n_data` is `n_edges` or `n_edges + 1`.
    for j in i..n_data {
        let idx = row + span.x_start + j;
        let hh = state.heights[idx];
        let wetness = state.cell_props[idx * 4 + PROP_WETNESS];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        let inside = mask_ne(state.shape_mask[idx] as f32, MASK_OUTSIDE as f32);
        scratch.h[j] = hh;
        scratch.wetness[j] = wetness;
        scratch.threshold[j] = state.cell_props[idx * 4 + PROP_THRESHOLD];
        scratch.cap[j] = cap;
        scratch.granular_share[j] = 1.0 - liq;
        scratch.head_base[j] = hh + scratch.head_static[idx];
        scratch.active[j] = inside;
        for ch in 0..4 {
            scratch.props[j * 4 + ch] = state.cell_props[idx * 4 + ch];
            scratch.colors[j * 4 + ch] = state.cell_colors[idx * 4 + ch] as f32;
        }
    }

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

/// Stage 2, chunked in blocks of 8 edges (== `block_size` at the shipped grid; see module doc
/// comment). Reads scratch (already per-span, `<= w` cells) 8 at a time via `chunks_exact`.
#[inline(never)]
fn stage2_c8(state: &State, scratch: &mut Scratch, span: Span, w: usize, n_edges: usize) {
    let off_disp = scratch.noise.row_offset(span.y, SALT_DISPERSION, n_edges.max(1));
    let off_lock = scratch.noise.row_offset(span.y, SALT_LOCK, n_edges.max(1));
    let off_jit = scratch.noise.row_offset(span.y, SALT_JITTER, n_edges.max(1));

    let disp_row = scratch.noise.slice(off_disp, n_edges);
    let lock_row = scratch.noise.slice(off_lock, n_edges);
    let mut e0 = 0usize;
    while e0 + 8 <= n_edges {
        let disp_c = &disp_row[e0..e0 + 8];
        let lock_c = &lock_row[e0..e0 + 8];
        for j in 0..8 {
            let e = e0 + j;
            let x = span.x_start + e;
            let idx = span.y * w + x;
            let active_e = scratch.active[e] * scratch.active[e + 1];
            let tau = GRANULAR_TAU_SCALE * scratch.threshold[e] * scratch.granular_share[e];

            let q8 = (disp_c[j] * 256.0).floor().min(255.0) / 255.0;
            let dispersion = (q8 - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;
            let head_a = scratch.head_base[e] + dispersion;
            let head_b = scratch.head_base[e + 1];
            let driving = head_a - head_b;

            let q16 = (lock_c[j] * 65536.0).floor().min(65535.0) / 65535.0;
            let lock_mask = mask_lt(q16, GRAVITY_LOCK_CHANCE * scratch.granular_share[e]);

            let v_prev = state.edge_vel_h[idx];
            let sleep_mask =
                edge_sleeps_bf(driving, tau, v_prev, scratch.h[e], scratch.h[e + 1], scratch.freecap[e], scratch.freecap[e + 1]);
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
            scratch.candidate[e] = raw_candidate * (1.0 - inactive) * active_e;
        }
        e0 += 8;
    }
    for e in e0..n_edges {
        let x = span.x_start + e;
        let idx = span.y * w + x;
        let active_e = scratch.active[e] * scratch.active[e + 1];
        let tau = GRANULAR_TAU_SCALE * scratch.threshold[e] * scratch.granular_share[e];
        let disp01 = scratch.noise.slice(off_disp, n_edges)[e];
        let lock01 = scratch.noise.slice(off_lock, n_edges)[e];
        let q8 = (disp01 * 256.0).floor().min(255.0) / 255.0;
        let dispersion = (q8 - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;
        let head_a = scratch.head_base[e] + dispersion;
        let head_b = scratch.head_base[e + 1];
        let driving = head_a - head_b;
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
        scratch.candidate[e] = raw_candidate * (1.0 - inactive) * active_e;
    }
    let _ = off_jit; // consumed inside kernel_c::stage3, not here
}

pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let mut total_flow = 0.0f64;
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;

        stage1_c8(state, scratch, span, w, n_data);
        stage2_c8(state, scratch, span, w, n_edges);
        total_flow += stage3(state, scratch, span, w, n_data, n_edges);
        stage45(state, scratch, span, w, n_data, n_edges);
    }
    total_flow
}
