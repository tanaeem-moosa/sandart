//! Kernel B: kernel A's same span/stage structure (see `kernel_a.rs` and `row_span.rs`), with
//! the smooth, branch-light arithmetic of stages 1 and 2 replaced by explicit `wide::f32x8` SIMD
//! over 8-cell chunks of a span -- `DEFAULT_BLOCK_SIZE` is 8, so one lane maps to one lateral
//! solver block/CPU-vector width at a time, matching the task's "block-at-a-time" framing.
//!
//! **What is vectorised.** Stage 1: `liquidity` (smoothstep, pure arithmetic), `cell_capacity`,
//! `k_of_liquidity`, and the head-base multiply-add (`h + k*SCALE*depth`). Stage 2: `driving`,
//! the branch-free `yielded = max(driving-tau,0) - max(-driving-tau,0)` identity (see below),
//! `raw`/`v` (`wave_params`'s 5-branch piecewise ramp reimplemented as a `bitselect` cascade),
//! the `+/-1` clamp, and the forward/backward candidate selection (also `bitselect`).
//!
//! **What is NOT vectorised, and why (the task's own list, confirmed against this port):**
//! - `in_transit_at` (`avail_a`/`avail_b`): a boundary-checked neighbour-row read, inherently
//!   branchy and address-dependent per cell -- computed scalar into an array, same as kernel A.
//! - `janssen_effective_depth`'s `exp()` term: `wide::f32x8::exp` DOES exist and IS used here
//!   (see stage 1) -- this is the one place this kernel's "not vectorised" list is shorter than
//!   originally expected; see the report for the instruction-count evidence.
//! - Every hash-based draw (dispersion, lock, `edge_share_jitter`, colour entropy): a 5-round
//!   32-bit integer avalanche mix, computed scalar per edge/cell into small arrays that stage 2's
//!   vector code then loads. Multiplying 8 independent 32-bit hash pipelines side by side would
//!   need a SIMD integer avalanche (`u32x8`), which this kernel does not attempt -- flagged as
//!   the clearest remaining lever if this were taken further.
//! - Stage 3 (arbitration) and stage 5 (Jacobi mixing): unchanged from kernel A, scalar. Both are
//!   small-fanout (<=2 neighbours per cell) and dominated by hash calls and cache-scattered
//!   4-channel prop/colour reads, not the kind of uniform elementwise math SIMD helps with.

use crate::consts::*;
use crate::row_span::{build_spans, Span};
use crate::scalar_math::*;
use crate::snapshot::State;
use wide::f32x8;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

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
    pub fn new(state: &State) -> Self {
        let spans = build_spans(state);
        let max_len = spans.iter().map(|s| s.data_end() - s.x_start).max().unwrap_or(0) + 9;
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

#[inline]
fn load8(a: &[f32]) -> f32x8 {
    let mut buf = [0.0f32; 8];
    let n = a.len().min(8);
    buf[..n].copy_from_slice(&a[..n]);
    f32x8::from(buf)
}

/// `wave_params`, vectorised as a `bitselect` cascade over the same five wetness bands.
#[inline]
fn wave_params_simd(wetness: f32x8) -> (f32x8, f32x8) {
    let c0 = f32x8::splat(0.08);
    let d0 = f32x8::splat(0.76);
    let c1 = f32x8::splat(0.18);
    let d1 = f32x8::splat(0.92);
    let c2 = f32x8::splat(0.22);
    let d2 = f32x8::splat(0.88);
    let c3 = f32x8::splat(0.16);
    let d3 = f32x8::splat(0.86);
    let c4 = f32x8::splat(0.24);
    let d4 = f32x8::splat(0.98);

    let t01 = ((wetness - f32x8::splat(0.75)) / f32x8::splat(0.10)).max(f32x8::splat(0.0)).min(f32x8::splat(1.0));
    let t12 = ((wetness - f32x8::splat(0.85)) / f32x8::splat(0.05)).max(f32x8::splat(0.0)).min(f32x8::splat(1.0));
    let t23 = ((wetness - f32x8::splat(0.90)) / f32x8::splat(0.05)).max(f32x8::splat(0.0)).min(f32x8::splat(1.0));
    let t34 = ((wetness - f32x8::splat(0.95)) / f32x8::splat(0.05)).max(f32x8::splat(0.0)).min(f32x8::splat(1.0));

    let c_band01 = c0 + (c1 - c0) * t01;
    let d_band01 = d0 + (d1 - d0) * t01;
    let c_band12 = c1 + (c2 - c1) * t12;
    let d_band12 = d1 + (d2 - d1) * t12;
    let c_band23 = c2 + (c3 - c2) * t23;
    let d_band23 = d2 + (d3 - d2) * t23;
    let c_band34 = c3 + (c4 - c3) * t34;
    let d_band34 = d3 + (d4 - d3) * t34;

    let m75 = wetness.simd_le(f32x8::splat(0.75));
    let m85 = wetness.simd_le(f32x8::splat(0.85));
    let m90 = wetness.simd_le(f32x8::splat(0.90));
    let m95 = wetness.simd_le(f32x8::splat(0.95));

    // Cascade from the innermost (highest-wetness) band outward, exactly mirroring the scalar
    // if/else-if chain: `bitselect(mask, if_true, if_false)`.
    let c = m75.bitselect(c0, m85.bitselect(c_band01, m90.bitselect(c_band12, m95.bitselect(c_band23, c_band34))));
    let d = m75.bitselect(d0, m85.bitselect(d_band01, m90.bitselect(d_band12, m95.bitselect(d_band23, d_band34))));
    (c, d)
}

pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    let w = state.w;
    let h_grid = state.h;
    let time_seed = state.time_seed;
    let mut total_flow = 0.0f64;

    let frozen_heights = state.heights.clone();
    let frozen_props = state.cell_props.clone();
    let frozen_colors = state.cell_colors.clone();

    for si in 0..scratch.spans.len() {
        let span = scratch.spans[si];
        let n_data = span.data_end() - span.x_start;
        let n_edges = span.x_owned_end - span.x_start;

        // ---- Stage 1, vectorised in 8-wide chunks ----
        let mut i = 0usize;
        while i < n_data {
            let n = (n_data - i).min(8);
            let mut h_buf = [0.0f32; 8];
            let mut wet_buf = [0.0f32; 8];
            let mut thr_buf = [0.0f32; 8];
            let mut depth_buf = [0.0f32; 8];
            let mut inside_buf = [false; 8];
            for j in 0..n {
                let x = span.x_start + i + j;
                let idx = span.y * w + x;
                h_buf[j] = frozen_heights[idx];
                wet_buf[j] = frozen_props[idx * 4 + PROP_WETNESS];
                thr_buf[j] = frozen_props[idx * 4 + PROP_THRESHOLD];
                depth_buf[j] = state.column_depth[idx];
                inside_buf[j] = state.shape_mask[idx] != MASK_OUTSIDE;
            }
            let h_v = f32x8::from(h_buf);
            let wet_v = f32x8::from(wet_buf);
            let depth_raw_v = f32x8::from(depth_buf);

            // liquidity: smoothstep((wetness-0.65)/0.2)
            let t = ((wet_v - f32x8::splat(0.65)) / f32x8::splat(0.20)).max(f32x8::splat(0.0)).min(f32x8::splat(1.0));
            let liq_v = t * t * (f32x8::splat(3.0) - f32x8::splat(2.0) * t);
            let one = f32x8::splat(1.0);
            let cap_v = f32x8::splat(1.5) * (one - liq_v) + liq_v;
            let k_v = liq_v + (one - liq_v) * f32x8::splat(LATERAL_EARTH_PRESSURE_K);
            // Janssen saturating term needs `exp()`, which `wide` DOES provide for f32x8 -- see
            // the module doc comment. Still one call per chunk, not per cell: 8 exps become one.
            let saturating_v = f32x8::splat(JANSSEN_DEPTH_SCALE)
                * (one - (-depth_raw_v / f32x8::splat(JANSSEN_DEPTH_SCALE)).exp());
            let depth_v = liq_v * depth_raw_v + (one - liq_v) * saturating_v;
            let head_base_v = h_v + k_v * f32x8::splat(LATERAL_PRESSURE_SCALE) * depth_v;
            let granular_share_v = one - liq_v;

            let liq_arr: [f32; 8] = liq_v.into();
            let cap_arr: [f32; 8] = cap_v.into();
            let head_arr: [f32; 8] = head_base_v.into();
            let gshare_arr: [f32; 8] = granular_share_v.into();

            for j in 0..n {
                let x = span.x_start + i + j;
                let idx = span.y * w + x;
                scratch.h[i + j] = h_buf[j];
                scratch.wetness[i + j] = wet_buf[j];
                scratch.threshold[i + j] = thr_buf[j];
                scratch.liq[i + j] = liq_arr[j];
                scratch.cap[i + j] = cap_arr[j];
                scratch.head_base[i + j] = head_arr[j];
                scratch.granular_share[i + j] = gshare_arr[j];
                scratch.active[i + j] = inside_buf[j];
                // in_transit_at is irreducibly scalar/branchy -- see module doc comment.
                scratch.avail[i + j] = if inside_buf[j] {
                    (h_buf[j] - in_transit_at(idx, w, h_grid, &frozen_heights, &frozen_props, &state.edge_vel_v, &state.shape_mask)).max(0.0)
                } else {
                    0.0
                };
                scratch.freecap[i + j] = if inside_buf[j] { (cap_arr[j] - h_buf[j]).max(0.0) } else { 0.0 };
            }
            i += 8;
        }

        // ---- Stage 2, vectorised in 8-wide edge chunks ----
        let mut e = 0usize;
        while e < n_edges {
            let n = (n_edges - e).min(8);
            let head_a_v = load8(&scratch.head_base[e..(e + n).min(scratch.head_base.len())]);
            let head_b_v = load8(&scratch.head_base[(e + 1)..(e + 1 + n).min(scratch.head_base.len())]);
            let wet_v = load8(&scratch.wetness[e..(e + n).min(scratch.wetness.len())]);
            let thr_v = load8(&scratch.threshold[e..(e + n).min(scratch.threshold.len())]);
            let gshare_v = load8(&scratch.granular_share[e..(e + n).min(scratch.granular_share.len())]);
            let tau_v = f32x8::splat(GRANULAR_TAU_SCALE) * thr_v * gshare_v;

            // Per-edge scalar draws (dispersion, lock, v_prev, active/sleep mask): irreducibly
            // per-edge hash/branch work, computed here and loaded for the vector combine below.
            let mut disp_buf = [0.0f32; 8];
            let mut vprev_buf = [0.0f32; 8];
            let mut mask_buf = [0.0f32; 8]; // 1.0 = active edge, 0.0 = sleeping/locked/out-of-shape
            let mut avail_a_buf = [0.0f32; 8];
            let mut avail_b_buf = [0.0f32; 8];
            let mut fwd_buf = [0.0f32; 8];
            let mut bwd_buf = [0.0f32; 8];
            for j in 0..n {
                let ei = e + j;
                let x = span.x_start + ei;
                let idx = span.y * w + x;
                let nb_idx = idx + 1;
                let edge_ok = scratch.active[ei] && x + 1 < w && scratch.active[ei + 1];
                let tau = scratch.threshold[ei] * scratch.granular_share[ei] * GRANULAR_TAU_SCALE;
                if !edge_ok {
                    mask_buf[j] = 0.0;
                    continue;
                }
                let seed = cell_seed(x, span.y, time_seed);
                let dispersion = dispersion_roll(seed, nb_idx, tau);
                let driving = (scratch.head_base[ei] + dispersion) - scratch.head_base[ei + 1];
                let lock = lock_roll(seed, nb_idx) < GRAVITY_LOCK_CHANCE * scratch.granular_share[ei];
                let sleeps = edge_sleeps(
                    driving, tau, state.edge_vel_h[idx],
                    scratch.h[ei], scratch.h[ei + 1],
                    scratch.freecap[ei], scratch.freecap[ei + 1],
                );
                if lock || sleeps {
                    mask_buf[j] = 0.0;
                    state.edge_vel_h[idx] = 0.0;
                } else {
                    mask_buf[j] = 1.0;
                }
                disp_buf[j] = dispersion;
                vprev_buf[j] = state.edge_vel_h[idx];
                avail_a_buf[j] = scratch.avail[ei];
                avail_b_buf[j] = scratch.avail[ei + 1];
                fwd_buf[j] = scratch.freecap[ei + 1];
                bwd_buf[j] = scratch.freecap[ei];
            }
            let disp_v = f32x8::from(disp_buf);
            let vprev_v = f32x8::from(vprev_buf);
            let mask_v = f32x8::from(mask_buf);
            let avail_a_v = f32x8::from(avail_a_buf);
            let avail_b_v = f32x8::from(avail_b_buf);
            let fwd_v = f32x8::from(fwd_buf);
            let bwd_v = f32x8::from(bwd_buf);

            let driving_v = (head_a_v + disp_v) - head_b_v;
            // yielded = max(driving-tau,0) - max(-driving-tau,0); see module doc comment for the
            // branch-free derivation.
            let yielded_v = (driving_v - tau_v).max(f32x8::splat(0.0)) - ((-driving_v) - tau_v).max(f32x8::splat(0.0));
            let (c_sq_v, damping_v) = wave_params_simd(wet_v);
            let raw_v = (vprev_v + c_sq_v * yielded_v) * damping_v;
            let v_v = raw_v.max(f32x8::splat(-1.0)).min(f32x8::splat(1.0));

            let fwd_amt = v_v.min(avail_a_v).min(fwd_v);
            let bwd_amt = -((-v_v).min(avail_b_v).min(bwd_v));
            let is_pos = v_v.simd_gt(f32x8::splat(0.0));
            let is_neg = v_v.simd_lt(f32x8::splat(0.0));
            let signed = is_pos.bitselect(fwd_amt, is_neg.bitselect(bwd_amt, f32x8::splat(0.0)));
            let result_v = signed * mask_v;

            let result_arr: [f32; 8] = result_v.into();
            for j in 0..n {
                scratch.candidate[e + j] = result_arr[j];
            }
            e += 8;
        }

        // ---- Stage 3: per-cell out/in totals, Zalesak scale (scalar; see kernel_a) ----
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
            scratch.candidate[e] = final_flux;
            state.edge_vel_h[idx] = final_flux;
            if final_flux.abs() > MIN_FLUX {
                scratch.total_out_flow[donor_i] += final_flux.abs();
                scratch.total_in_flow[acceptor_i] += final_flux.abs();
                total_flow += final_flux.abs() as f64;
            }
        }

        // ---- Stages 4+5: apply heights, Jacobi-mix props/colours (scalar; see kernel_a) ----
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
                continue;
            }
            let kept = (h_old - out_flow).max(0.0);
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
                let left_flux = scratch.candidate[i - 1];
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
                let entropy = h_new.to_bits() ^ (idx as u32).wrapping_mul(2_654_435_761) ^ (ch as u32).wrapping_mul(97);
                state.cell_colors[idx * 4 + ch] = stochastic_round(new_color_f.clamp(0.0, 255.0), entropy);
            }
            state.cell_colors[idx * 4 + 3] = 255;
        }
    }

    total_flow
}
