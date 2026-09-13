//! Task item 4, "the correct bar": kernel A's exact scalar structure and early-outs (see
//! `kernel_a.rs`'s module doc comment -- unchanged here), reading and writing the SAME
//! production-style SoA state `kernel_e`/`kernel_e2` use (`kernel_e::StateE`: 4 prop `Vec<f32>`,
//! one packed `Vec<u32>` per cell for colour, double-buffered current/next with an O(1)
//! `mem::swap`), instead of kernel A's own `snapshot::State` (interleaved `cell_props`/
//! `cell_colors`, single-buffered with a per-pass whole-grid `clone()` for the "frozen" read).
//!
//! **Why double-buffering, not a clone.** Kernel A's own `frozen_heights`/`frozen_props`/
//! `frozen_colors` clones exist ONLY to give stage 4+5 a same-pass-untouched read of every other
//! cell (see `kernel_a.rs`'s doc comment on why that's needed instead of R's in-place mutation).
//! `StateE`'s current/next scheme (see `kernel_e.rs`'s module doc comment) gives the identical
//! guarantee -- reads come from `current`, writes go to `next`, so `current` is untouched by this
//! pass's own writes -- at O(1) swap cost per pass instead of an O(w*h) clone. This is not a
//! deviation from "port kernel A's structure": it is the SAME frozen-read requirement, satisfied
//! the way `kernel_e2` already satisfies it, which is exactly what makes this the "production-
//! style" bar the task asks for -- comparing A's math against E2's layout-and-copy strategy is the
//! whole point of this kernel.
//!
//! **What's kept from kernel A, unchanged:** the per-edge `if !edge_ok { candidate = 0; continue
//! }` skip, the per-edge `if c == 0.0 { continue }` skip in the out/in-total and arbitration
//! loops, and the per-cell `if out_flow == 0.0 && in_flow == 0.0 { continue }` skip in stage 4+5 --
//! every early-out `kernel_a.rs`'s module doc comment calls out survives verbatim. Per-span
//! scratch buffers are kernel A's own (`h`/`wetness`/`threshold`/.../`candidate`/`active`), sized
//! to the widest span, never whole-grid.
//!
//! **Colour:** kernel A's channel loop mixes all 4 "colour" channels (r,g,b,a) even though the
//! 4th (alpha) is unconditionally overwritten with `255` immediately after -- see `kernel_a.rs`'s
//! stage 4+5. That 4th-channel arithmetic is dead by construction (its result is never read), so
//! this port only mixes r/g/b (3 channels unpacked from/repacked into the packed `u32`) -- a
//! behaviour-PRESERVING simplification, not a math change: verified bit-identical to A in
//! `a_soa_bench`.
//!
//! **`in_transit_at`:** kernel A calls `scalar_math::in_transit_at`, which reads interleaved
//! `cell_props`. This port uses `in_transit_at_soa` below -- the identical scalar law, reading
//! `StateE`'s own `prop[PROP_WETNESS]` slice directly instead of `cell_props[idx*4+PROP_WETNESS]`.
//! No behaviour difference; same values.

use crate::consts::*;
use crate::kernel_e::StateE;
use crate::row_span::{build_spans_raw, Span};
use crate::scalar_math::*;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// `scalar_math::in_transit_at`, reading `StateE`'s SoA wetness slice instead of interleaved
/// `cell_props`. See module doc comment.
#[inline]
#[allow(clippy::too_many_arguments)]
fn in_transit_at_soa(c: usize, w: usize, h: usize, heights: &[f32], wetness: &[f32], edge_vel_v: &[f32], shape_mask: &[u8]) -> f32 {
    let cx = c % w;
    let cy = c / w;
    if !(cx > 0 && cx + 1 < w && cy > 0 && cy + 1 < h && shape_mask[(cy + 1) * w + cx] != MASK_OUTSIDE) {
        return 0.0;
    }
    let below = c + w;
    let h_below = heights[below];
    let cap_below = cell_capacity_for(wetness[below]);
    let downstream_route = edge_vel_v[c].max(0.0) + (cap_below - h_below).max(0.0);
    edge_vel_v[c - w].max(0.0).min(downstream_route)
}

/// Kernel A's own `Scratch`, unchanged field-for-field (see `kernel_a.rs`) minus the fields that
/// only ever held a raw copy of state A had to clone (`wetness` is kept -- it's a genuinely
/// derived-and-reused per-cell scratch value here too, cheap either way).
pub struct Scratch {
    spans: Vec<Span>,
    h: Vec<f32>,
    wetness: Vec<f32>,
    threshold: Vec<f32>,
    liq: Vec<f32>,
    cap: Vec<f32>,
    avail: Vec<f32>,
    freecap: Vec<f32>,
    head_base: Vec<f32>,
    granular_share: Vec<f32>,
    candidate: Vec<f32>,
    active: Vec<bool>,
    out_total: Vec<f32>,
    in_total: Vec<f32>,
    out_total_jit: Vec<f32>,
    in_total_jit: Vec<f32>,
    total_out_flow: Vec<f32>,
    total_in_flow: Vec<f32>,
}

impl Scratch {
    pub fn new(state: &StateE) -> Self {
        let spans = build_spans_raw(state.cols, state.rows, state.block_size, state.w, state.h, &state.sim_blocks);
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

pub fn simulated_cell_count(scratch: &Scratch) -> usize {
    scratch.spans.iter().map(|s| s.x_owned_end - s.x_start).sum()
}

pub fn run_pass(state: &mut StateE, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let h_grid = state.h;
    let time_seed = state.time_seed;
    let mut total_flow = 0.0f64;

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;

        // ---- Stage 1 ---- (reads `state`'s CURRENT/frozen arrays directly; no clone)
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let inside = state.shape_mask[idx] != MASK_OUTSIDE;
            let hh = state.heights[idx];
            let wetness = state.prop[PROP_WETNESS][idx];
            let liq = liquidity(wetness);
            let cap = 1.5 * (1.0 - liq) + liq;
            let avail = if inside {
                (hh - in_transit_at_soa(idx, w, h_grid, &state.heights, &state.prop[PROP_WETNESS], &state.edge_vel_v, &state.shape_mask)).max(0.0)
            } else {
                0.0
            };
            scratch.h[i] = hh;
            scratch.wetness[i] = wetness;
            scratch.threshold[i] = state.prop[PROP_THRESHOLD][idx];
            scratch.liq[i] = liq;
            scratch.cap[i] = cap;
            scratch.avail[i] = avail;
            scratch.freecap[i] = if inside { (cap - hh).max(0.0) } else { 0.0 };
            let k = k_of_liquidity(liq);
            let depth = janssen_effective_depth(state.column_depth[idx], liq);
            scratch.head_base[i] = hh + k * LATERAL_PRESSURE_SCALE * depth;
            scratch.granular_share[i] = 1.0 - liq;
            scratch.active[i] = inside;
        }

        // ---- Stage 2 ---- (edge_vel_h is single-buffered real state, same as kernel A/E/E2)
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

        // ---- Stage 3 ---- (Zalesak scale, exactly kernel A's)
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
                let jit = edge_share_jitter_soa(state, donor_idx, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
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
                let jit = edge_share_jitter_soa(state, donor_idx, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
                edge_arbitration_scale(
                    scratch.out_total[donor_i], scratch.out_total_jit[donor_i], scratch.avail[donor_i],
                    scratch.in_total[acceptor_i], scratch.in_total_jit[acceptor_i], scratch.freecap[acceptor_i],
                    jit,
                )
            } else {
                1.0
            };
            let final_flux = c * scale;
            scratch.candidate[e] = final_flux;
            state.edge_vel_h[idx] = final_flux;
            if final_flux.abs() > MIN_FLUX {
                scratch.total_out_flow[donor_i] += final_flux.abs();
                scratch.total_in_flow[acceptor_i] += final_flux.abs();
                total_flow += final_flux.abs() as f64;
            }
        }

        // ---- Stages 4+5 ---- reads CURRENT (`state.heights`/`state.prop`/`state.colors`),
        // writes NEXT (`state.heights_b`/`state.prop_b`/`state.colors_b`) -- the double-buffer
        // scheme stands in for kernel A's `frozen_*` clones, see module doc comment.
        for i in 0..n_data {
            let x = span.x_start + i;
            let idx = span.y * w + x;
            let out_flow = scratch.total_out_flow[i];
            let in_flow = scratch.total_in_flow[i];
            state.heights_b[idx] = state.heights[idx]; // default: unchanged (overwritten below if touched)
            for ch in 0..4 {
                state.prop_b[ch][idx] = state.prop[ch][idx];
            }
            state.colors_b[idx] = state.colors[idx];
            if out_flow == 0.0 && in_flow == 0.0 {
                continue;
            }
            let h_old = scratch.h[i];
            let h_new = (h_old - out_flow + in_flow).max(0.0);
            state.heights_b[idx] = h_new;
            if in_flow <= 0.0 {
                // Pure donor: keeps its own (frozen, unchanged) props/colours -- already copied
                // above.
                continue;
            }
            let kept = (h_old - out_flow).max(0.0);
            let mut mixed_props = [0.0f32; 4];
            let mut mixed_rgb = [0.0f32; 3];
            let mut add_source = |src_i: usize, amount: f32| {
                let src_idx = span.y * w + span.x_start + src_i;
                for ch in 0..4 {
                    mixed_props[ch] += state.prop[ch][src_idx] * amount;
                }
                let c = state.colors[src_idx];
                for ch in 0..3 {
                    mixed_rgb[ch] += ((c >> (ch * 8)) & 0xFF) as f32 * amount;
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
            let own_amount = if h_new > 1e-6 { kept } else { 0.0 };
            let total_amount = own_amount + in_flow;
            for ch in 0..4 {
                let own_val = state.prop[ch][idx];
                let new_prop = if total_amount > 1e-6 {
                    (own_val * own_amount + mixed_props[ch]) / total_amount
                } else {
                    own_val
                };
                state.prop_b[ch][idx] = new_prop;
            }

            let own_c = state.colors[idx];
            let mut rgb = [0u8; 3];
            for ch in 0..3 {
                let own_color = ((own_c >> (ch * 8)) & 0xFF) as f32;
                let new_color_f = if total_amount > 1e-6 {
                    (own_color * own_amount + mixed_rgb[ch]) / total_amount
                } else {
                    own_color
                };
                let entropy = (h_new.to_bits()) ^ (idx as u32).wrapping_mul(2_654_435_761) ^ (ch as u32).wrapping_mul(97);
                rgb[ch] = stochastic_round(new_color_f.clamp(0.0, 255.0), entropy);
            }
            state.colors_b[idx] = (rgb[0] as u32) | ((rgb[1] as u32) << 8) | ((rgb[2] as u32) << 16) | (255u32 << 24);
        }
    }

    crate::kernel_e::run_swap(state);
    total_flow
}

/// `scalar_math::edge_share_jitter`, reading `StateE`'s SoA grain-size/wetness slices for
/// `grain_jitter_strength` instead of interleaved `cell_props`.
#[inline]
fn edge_share_jitter_soa(state: &StateE, donor: usize, edge_key: usize, salt: u32, time_seed: u32) -> f32 {
    let wetness = state.prop[PROP_WETNESS][donor];
    let grain_size = state.prop[PROP_GRAIN_SIZE][donor];
    let granular_share = (1.0 - liquidity(wetness)).clamp(0.0, 1.0);
    let gran_s = (GRAIN_JITTER_SCALE * grain_size).clamp(0.0, GRAIN_JITTER_MAX) * granular_share;
    let s = gran_s.max(0.05);
    if s <= 0.0 {
        return 1.0;
    }
    edge_share_jitter_hash(edge_key, salt, time_seed, s)
}

#[inline]
fn edge_share_jitter_hash(edge_key: usize, salt: u32, time_seed: u32, s: f32) -> f32 {
    let h = time_seed ^ (edge_key as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x2545_F491);
    let u = hash_u01_soa(h);
    1.0 + s * (2.0 * u - 1.0)
}

#[inline]
fn hash_u01_soa(mut h: u32) -> f32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

pub fn run_swap(state: &mut StateE) {
    crate::kernel_e::run_swap(state);
}
