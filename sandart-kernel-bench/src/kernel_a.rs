//! Kernel A: array form, plain branch-free-where-possible slice loops, over the contiguous
//! active spans `row_span::build_spans` finds. See that module's doc comment for span semantics,
//! and `lib.rs`'s module doc comment for the six stages and the Jacobi-mixing design (deliberately
//! NOT bit-identical to R's sequential per-edge `advect_properties` -- see stage 5 below).
//!
//! Falls back to scalar, per-cell work for: the mask/`is_inside` test (baked into how spans are
//! built, so it costs nothing per-edge here), `in_transit_at` (a handful of neighbour-row reads
//! gated on a boundary check -- not vectorised), and every hash-based draw (dispersion, lock,
//! `edge_share_jitter`, and the colour stochastic-rounding entropy) -- all five are irreducibly
//! per-cell/per-edge scalar integer work.

use crate::consts::*;
use crate::row_span::{build_spans, Span};
use crate::scalar_math::*;
use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

pub struct Scratch {
    spans: Vec<Span>,
    // Per-span scratch buffers, sized to the widest span seen so far.
    h: Vec<f32>,
    wetness: Vec<f32>,
    threshold: Vec<f32>,
    liq: Vec<f32>,
    cap: Vec<f32>,
    avail: Vec<f32>,
    freecap: Vec<f32>,
    head_base: Vec<f32>,
    granular_share: Vec<f32>,
    candidate: Vec<f32>,  // len = number of edges in span (data_len - 1)
    active: Vec<bool>,    // per-edge: is this edge inside the shape on both ends
    out_total: Vec<f32>,
    in_total: Vec<f32>,
    out_total_jit: Vec<f32>,
    in_total_jit: Vec<f32>,
    total_out_flow: Vec<f32>, // per-cell realised outflow (post arbitration), for the mix
    total_in_flow: Vec<f32>,  // per-cell realised inflow
}

impl Scratch {
    pub fn new(state: &State) -> Self {
        let spans = build_spans(state);
        let max_len = spans.iter().map(|s| s.data_end() - s.x_start).max().unwrap_or(0) + 1;
        Scratch {
            spans,
            h: vec![0.0; max_len],
            wetness: vec![0.0; max_len],
            threshold: vec![0.0; max_len],
            liq: vec![0.0; max_len],
            cap: vec![0.0; max_len],
            avail: vec![0.0; max_len],
            freecap: vec![0.0; max_len],
            head_base: vec![0.0; max_len],
            granular_share: vec![0.0; max_len],
            candidate: vec![0.0; max_len],
            active: vec![false; max_len],
            out_total: vec![0.0; max_len],
            in_total: vec![0.0; max_len],
            out_total_jit: vec![0.0; max_len],
            in_total_jit: vec![0.0; max_len],
            total_out_flow: vec![0.0; max_len],
            total_in_flow: vec![0.0; max_len],
        }
    }
}

/// Number of cells covered by `scratch.spans` (denominator sanity check; should equal
/// `kernel_r::simulated_cell_count`'s "owned" count, not counting `+1` acceptor extensions).
pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let h_grid = state.h;
    let time_seed = state.time_seed;
    let mut total_flow = 0.0f64;

    // Frozen pre-pass heights/props/colours -- stage 5's mixing law explicitly reads the
    // FROZEN state, never a value another edge already mutated this pass (unlike R). Cloning
    // once per call is not free, but keeps the mixing law honest and simple; see the report for
    // what this costs relative to R's zero-copy in-place mutation.
    let frozen_heights = state.heights.clone();
    let frozen_props = state.cell_props.clone();
    let frozen_colors = state.cell_colors.clone();

    // Iterate spans by owning `scratch.spans` immutably via index to avoid borrow conflicts with
    // `state`'s mutation below.
    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start; // cells with stage-1 data
        let n_edges = span.x_owned_end - span.x_start; // edges this span owns

        // ---- Stage 1: per-cell frozen arrays ----
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let inside = state.shape_mask[idx] != MASK_OUTSIDE;
            let hh = frozen_heights[idx];
            let wetness = frozen_props[idx * 4 + PROP_WETNESS];
            let liq = liquidity(wetness);
            let cap = 1.5 * (1.0 - liq) + liq;
            let avail = if inside {
                (hh - in_transit_at(idx, w, h_grid, &frozen_heights, &frozen_props, &state.edge_vel_v, &state.shape_mask)).max(0.0)
            } else {
                0.0
            };
            scratch.h[i] = hh;
            scratch.wetness[i] = wetness;
            scratch.threshold[i] = frozen_props[idx * 4 + PROP_THRESHOLD];
            scratch.liq[i] = liq;
            scratch.cap[i] = cap;
            scratch.avail[i] = avail;
            scratch.freecap[i] = if inside { (cap - hh).max(0.0) } else { 0.0 };
            let k = k_of_liquidity(liq);
            let depth = janssen_effective_depth(state.column_depth[idx], liq);
            scratch.head_base[i] = hh + k * LATERAL_PRESSURE_SCALE * depth;
            scratch.granular_share[i] = 1.0 - liq;
            scratch.active[i] = inside; // reused as "cell i is inside" for the edge loop below
        }

        // ---- Stage 2: candidate flux per edge ----
        for e in 0..n_edges {
            let x = span.x_start + e;
            let idx = span.y * w + x;
            let nb_idx = idx + 1;
            let edge_ok = scratch.active[e] && x + 1 < w && scratch.active[e + 1];
            if !edge_ok {
                scratch.candidate[e] = 0.0;
                continue;
            }
            let tau = GRANULAR_TAU_SCALE * scratch.threshold[e] * scratch.granular_share[e];
            let seed = cell_seed(x, span.y, time_seed);
            let dispersion = dispersion_roll(seed, nb_idx, tau);
            let head_a = scratch.head_base[e] + dispersion;
            let head_b_full = scratch.head_base[e + 1];
            let driving = head_a - head_b_full;
            let lock = lock_roll(seed, nb_idx) < GRAVITY_LOCK_CHANCE * scratch.granular_share[e];
            let sleeps = edge_sleeps(
                driving, tau, state.edge_vel_h[idx],
                scratch.h[e], scratch.h[e + 1],
                scratch.freecap[e], scratch.freecap[e + 1],
            );
            if lock || sleeps {
                scratch.candidate[e] = 0.0;
                state.edge_vel_h[idx] = 0.0;
            } else {
                let (c_sq, damping) = wave_params(scratch.wetness[e]);
                scratch.candidate[e] = flux_edge_candidate(
                    head_a, head_b_full, c_sq, damping, tau,
                    scratch.avail[e], scratch.avail[e + 1],
                    scratch.freecap[e + 1], scratch.freecap[e],
                    1.0, state.edge_vel_h[idx],
                );
            }
        }

        // ---- Stage 3: per-cell out/in totals, then the Zalesak scale ----
        for i in 0..n_data {
            scratch.out_total[i] = 0.0;
            scratch.in_total[i] = 0.0;
            scratch.out_total_jit[i] = 0.0;
            scratch.in_total_jit[i] = 0.0;
        }
        let mut oversubscribed = false;
        for e in 0..n_edges {
            let c = scratch.candidate[e];
            if c == 0.0 {
                continue;
            }
            let (donor, acceptor, mag) = if c >= 0.0 { (e, e + 1, c) } else { (e + 1, e, -c) };
            scratch.out_total[donor] += mag;
            scratch.in_total[acceptor] += mag;
            oversubscribed |= scratch.out_total[donor] > scratch.avail[donor] || scratch.in_total[acceptor] > scratch.freecap[acceptor];
        }
        if oversubscribed {
            for e in 0..n_edges {
                let c = scratch.candidate[e];
                if c == 0.0 {
                    continue;
                }
                let x = span.x_start + e;
                let idx = span.y * w + x;
                let (donor_i, acceptor_i, mag) = if c >= 0.0 { (e, e + 1, c) } else { (e + 1, e, -c) };
                let donor_idx = span.y * w + span.x_start + donor_i;
                let jit = edge_share_jitter(&frozen_props, donor_idx, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
                scratch.out_total_jit[donor_i] += mag * jit;
                scratch.in_total_jit[acceptor_i] += mag * jit;
            }
        }

        for i in 0..n_data {
            scratch.total_out_flow[i] = 0.0;
            scratch.total_in_flow[i] = 0.0;
        }
        for e in 0..n_edges {
            let c = scratch.candidate[e];
            if c == 0.0 {
                continue;
            }
            let x = span.x_start + e;
            let idx = span.y * w + x;
            let (donor_i, acceptor_i, _mag) = if c >= 0.0 { (e, e + 1, c) } else { (e + 1, e, -c) };
            let scale = if oversubscribed {
                let donor_idx = span.y * w + span.x_start + donor_i;
                let jit = edge_share_jitter(&frozen_props, donor_idx, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
                edge_arbitration_scale(
                    scratch.out_total[donor_i], scratch.out_total_jit[donor_i], scratch.avail[donor_i],
                    scratch.in_total[acceptor_i], scratch.in_total_jit[acceptor_i], scratch.freecap[acceptor_i],
                    jit,
                )
            } else {
                1.0
            };
            let final_flux = c * scale;
            scratch.candidate[e] = final_flux; // reuse as the FINAL flux from here on
            state.edge_vel_h[idx] = final_flux;
            if final_flux.abs() > MIN_FLUX {
                scratch.total_out_flow[donor_i] += final_flux.abs();
                scratch.total_in_flow[acceptor_i] += final_flux.abs();
                total_flow += final_flux.abs() as f64;
            }
        }

        // ---- Stages 4+5: apply heights, Jacobi-mix props/colours, stochastic-round colours ----
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let out_flow = scratch.total_out_flow[i];
            let in_flow = scratch.total_in_flow[i];
            if out_flow == 0.0 && in_flow == 0.0 {
                continue;
            }
            let h_old = scratch.h[i];
            let h_new = (h_old - out_flow + in_flow).max(0.0);
            state.heights[idx] = h_new;
            if in_flow <= 0.0 {
                // Pure donor this pass: keeps its own (frozen, unchanged) props/colours.
                continue;
            }
            let kept = (h_old - out_flow).max(0.0);
            // Inflow sources: the left edge (i-1, if it donated rightward into i) and the right
            // edge (i, if it donated leftward into i).
            let mut mixed_props = [0.0f32; 4];
            let mut mixed_colors = [0.0f32; 4];
            let mut add_source = |src_i: usize, amount: f32| {
                let src_idx = span.y * w + span.x_start + src_i;
                for ch in 0..4 {
                    mixed_props[ch] += frozen_props[src_idx * 4 + ch] * amount;
                    mixed_colors[ch] += frozen_colors[src_idx * 4 + ch] as f32 * amount;
                }
            };
            if i > 0 {
                let left_edge = i - 1;
                let left_flux = scratch.candidate[left_edge];
                if left_flux > MIN_FLUX {
                    add_source(i - 1, left_flux);
                }
            }
            if i < n_edges {
                let right_flux = scratch.candidate[i];
                if right_flux < -MIN_FLUX {
                    add_source(i + 1, -right_flux);
                }
            }
            for ch in 0..4 {
                let own_amount = if h_new > 1e-6 { kept } else { 0.0 };
                let total_amount = own_amount + in_flow;
                let own_val = frozen_props[idx * 4 + ch];
                let new_prop = if total_amount > 1e-6 {
                    (own_val * own_amount + mixed_props[ch]) / total_amount
                } else {
                    own_val
                };
                state.cell_props[idx * 4 + ch] = new_prop;

                let own_color = frozen_colors[idx * 4 + ch] as f32;
                let new_color_f = if total_amount > 1e-6 {
                    (own_color * own_amount + mixed_colors[ch]) / total_amount
                } else {
                    own_color
                };
                let entropy = (h_new.to_bits()) ^ (idx as u32).wrapping_mul(2_654_435_761) ^ (ch as u32).wrapping_mul(97);
                state.cell_colors[idx * 4 + ch] = stochastic_round(new_color_f.clamp(0.0, 255.0), entropy);
            }
            state.cell_colors[idx * 4 + 3] = 255;
        }
    }

    total_flow
}
