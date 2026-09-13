//! Branch-free reformulations of the handful of `scalar_math` functions that contain `if`/`else`
//! on data. Kernel C (and C8) call ONLY these versions inside per-cell/per-edge loops -- never
//! `scalar_math`'s `wave_params`, `edge_sleeps`, `flux_edge_candidate`, `in_transit_at`,
//! `budget_term` or `edge_arbitration_scale`, all of which branch on a live value (wetness band,
//! driving vs. tau, starvation, budget headroom). `scalar_math`'s OTHER functions --
//! `liquidity`, `cell_capacity_for`, `k_of_liquidity`, `janssen_effective_depth`,
//! `grain_jitter_strength`'s core (see `grain_jitter_strength_bf` below, which only exists
//! because the original takes a `&[f32]` + index instead of the two floats kernel C already has
//! unpacked) -- are already pure clamp/min/max arithmetic with zero branches, so kernel C calls
//! those directly.
//!
//! Every mask below is a `1.0`/`0.0` float, combined with `.max()` (OR), `.min()`/multiply (AND)
//! and `select()` (`a` when the mask is 1, `b` when it is 0) instead of `if`/`else`. Divisions
//! that would only be safe on one branch of the original `if` are floored with `.max(tiny)` so
//! the unused side of a `select` is always finite, never `NaN`/`Inf` (multiplying a 0 mask by an
//! `Inf` is `NaN`, which `select` would then leak into the result).

use crate::consts::*;

#[inline]
pub fn mask_gt(a: f32, b: f32) -> f32 {
    (a > b) as i32 as f32
}
#[inline]
pub fn mask_lt(a: f32, b: f32) -> f32 {
    (a < b) as i32 as f32
}
#[inline]
pub fn mask_le(a: f32, b: f32) -> f32 {
    (a <= b) as i32 as f32
}
#[inline]
pub fn mask_ge(a: f32, b: f32) -> f32 {
    (a >= b) as i32 as f32
}
#[inline]
pub fn mask_eq(a: f32, b: f32) -> f32 {
    (a == b) as i32 as f32
}
#[inline]
pub fn mask_ne(a: f32, b: f32) -> f32 {
    (a != b) as i32 as f32
}
/// `a` where `mask == 1.0`, `b` where `mask == 0.0`. Undefined (but not unsound -- just a wrong
/// float) for a mask outside `{0.0, 1.0}`; every mask this module produces is one of those two.
#[inline]
pub fn select(mask: f32, a: f32, b: f32) -> f32 {
    b + mask * (a - b)
}

/// `scalar_math::wave_params`, as the same `bitselect` cascade `kernel_b::wave_params_simd` uses
/// for `f32x8`, scalarised. Innermost (highest-wetness) band evaluated first, then folded outward
/// with `select`, mirroring the scalar `if/else if` chain exactly.
#[inline]
pub fn wave_params_bf(wetness: f32) -> (f32, f32) {
    let t01 = ((wetness - 0.75) / 0.10).max(0.0).min(1.0);
    let t12 = ((wetness - 0.85) / 0.05).max(0.0).min(1.0);
    let t23 = ((wetness - 0.90) / 0.05).max(0.0).min(1.0);
    let t34 = ((wetness - 0.95) / 0.05).max(0.0).min(1.0);

    let c_band01 = 0.08 + (0.18 - 0.08) * t01;
    let d_band01 = 0.76 + (0.92 - 0.76) * t01;
    let c_band12 = 0.18 + (0.22 - 0.18) * t12;
    let d_band12 = 0.92 + (0.88 - 0.92) * t12;
    let c_band23 = 0.22 + (0.16 - 0.22) * t23;
    let d_band23 = 0.88 + (0.86 - 0.88) * t23;
    let c_band34 = 0.16 + (0.24 - 0.16) * t34;
    let d_band34 = 0.86 + (0.98 - 0.86) * t34;

    let m75 = mask_le(wetness, 0.75);
    let m85 = mask_le(wetness, 0.85);
    let m90 = mask_le(wetness, 0.90);
    let m95 = mask_le(wetness, 0.95);

    let c1 = select(m95, c_band23, c_band34);
    let c2 = select(m90, c_band12, c1);
    let c3 = select(m85, c_band01, c2);
    let c = select(m75, 0.08, c3);

    let d1 = select(m95, d_band23, d_band34);
    let d2 = select(m90, d_band12, d1);
    let d3 = select(m85, d_band01, d2);
    let d = select(m75, 0.76, d3);

    (c, d)
}

/// `scalar_math::edge_sleeps`, returned as a `1.0` (sleeps/starved) / `0.0` (flows) mask instead
/// of `bool`.
#[inline]
pub fn edge_sleeps_bf(driving: f32, tau: f32, v_e: f32, avail_a: f32, avail_b: f32, room_a: f32, room_b: f32) -> f32 {
    let starved_a = mask_le(avail_a, 0.0).max(mask_le(room_b, 0.0));
    let starved_b = mask_le(avail_b, 0.0).max(mask_le(room_a, 0.0));
    let starved = starved_a.min(starved_b);
    let idle = mask_eq(v_e, 0.0) * mask_le(driving.abs(), tau);
    starved.max(idle)
}

/// `scalar_math::flux_edge_candidate`, minus the always-`1.0` `weight` argument this crate never
/// varies (`PHASE == 1`; see `consts::PHASE`) -- multiplying by a compile-time `1.0` is not a
/// branch, but it is also not doing anything, so it is dropped rather than kept as decoration.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn flux_edge_candidate_bf(
    head_a: f32,
    head_b: f32,
    c_sq: f32,
    damping: f32,
    tau: f32,
    avail_a: f32,
    avail_b: f32,
    max_accept_fwd: f32,
    max_accept_bwd: f32,
    v_e_prev: f32,
) -> f32 {
    let driving = head_a - head_b;
    // `yielded = clamp(driving, -tau, tau)`'s complement, as the branch-free identity
    // `max(driving-tau,0) - max(-driving-tau,0)` kernel_b's module doc comment derives.
    let yielded = (driving - tau).max(0.0) - ((-driving) - tau).max(0.0);
    let raw = (v_e_prev + c_sq * yielded) * damping;
    let v = raw.max(-1.0).min(1.0);
    let fwd = v.min(avail_a).min(max_accept_fwd);
    let bwd = -((-v).min(avail_b).min(max_accept_bwd));
    fwd * mask_gt(v, 0.0) + bwd * mask_lt(v, 0.0)
}

/// `scalar_math::budget_term`. The `jit_total.max(1e-30)` divisor keeps `scaled` finite even when
/// `need == 0.0` (jit_total could otherwise be exactly `0.0`, e.g. an edge nobody arbitrates
/// against, which would make `scaled` a `0/0` `NaN` that `select` would leak through even though
/// its multiplier is zero).
#[inline]
pub fn budget_term_bf(raw_total: f32, jit_total: f32, budget: f32, jitter: f32) -> f32 {
    let need = mask_gt(raw_total, budget) * mask_gt(jit_total, 0.0);
    let scaled = (budget * jitter / jit_total.max(1e-30)).max(0.0);
    select(need, scaled, 1.0)
}

/// `scalar_math::edge_arbitration_scale`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn edge_arbitration_scale_bf(
    donor_out_total: f32,
    donor_out_total_jit: f32,
    donor_avail: f32,
    acceptor_in_total: f32,
    acceptor_in_total_jit: f32,
    acceptor_freecap: f32,
    jitter: f32,
) -> f32 {
    budget_term_bf(donor_out_total, donor_out_total_jit, donor_avail, jitter)
        .min(budget_term_bf(acceptor_in_total, acceptor_in_total_jit, acceptor_freecap, jitter))
        .min(1.0)
}

/// `scalar_math::grain_jitter_strength`, taking the two floats it needs directly instead of a
/// `&[f32]` cell-props slice + index -- kernel C's per-span scratch already has `grain_size` and
/// `granular_share` unpacked into their own arrays, and arbitration needs this evaluated for BOTH
/// candidate donors of an edge (see `kernel_c.rs`), not just one cell picked by a branch. Already
/// branch-free in the original (`.clamp()`/`.max()` are not branches); reproduced verbatim.
#[inline]
pub fn grain_jitter_strength_bf(grain_size: f32, granular_share: f32) -> f32 {
    let gran_s = (GRAIN_JITTER_SCALE * grain_size).clamp(0.0, GRAIN_JITTER_MAX) * granular_share;
    gran_s.max(0.05)
}

/// `scalar_math::edge_share_jitter`'s post-hash arithmetic, given the uniform draw `u01` already
/// read from the noise table (see `noise.rs`) instead of hashed. Note `grain_jitter_strength_bf`
/// always returns `>= 0.05` (its own unconditional `.max(0.05)` floor), so the original's
/// `if s <= 0.0 { return 1.0 }` guard is dead code in every real call -- `s` can never reach zero
/// -- and is correctly omitted here rather than reproduced as a no-op branch.
#[inline]
pub fn edge_share_jitter_bf(grain_jitter_strength: f32, u01: f32) -> f32 {
    1.0 + grain_jitter_strength * (2.0 * u01 - 1.0)
}

/// `scalar_math::stochastic_round`, given the uniform draw directly. Returns the rounded value as
/// `f32`; the caller casts to `u8` (Rust's `f32 as u8` saturates, so this is safe even if `v` is
/// not pre-clamped to `0.0..=255.0`, though kernel C always clamps first as the original does).
#[inline]
pub fn stochastic_round_bf(v: f32, u01: f32) -> f32 {
    (v + u01).floor()
}

/// `scalar_math::in_transit_at`, for a whole row at once (task rule 5): `y`, `x_start` and `n`
/// (span length) are span/loop-bounds facts fixed BEFORE the loop, not per-cell data, so the
/// boundary checks the original guards with a single `if` are hoisted here into two SPAN-LEVEL
/// clamped-index + validity-mask pairs (`below_y`/`row_below_valid`, `above_row_off`/
/// `row_above_valid`) computed once, outside the loop -- never a per-cell branch. The remaining
/// per-cell condition (`shape_mask[(y+1)*w+x] != OUTSIDE`, genuinely different per column) is a
/// multiplier mask, not a branch.
///
/// Verified against `in_transit_at`'s guard before relying on it (task rule 4): the lateral pass
/// only ever reads `edge_vel_v` (row `y` and row `y-1`, never written by this pass) and the row
/// BELOW's `heights`/`cell_props` (never row `y-1`'s heights/props) -- so processing spans in the
/// row order `row_span::build_spans` already produces (verified in `kernel_c.rs`'s module doc
/// comment) means row `y+1` is always still the untouched pre-pass state when this runs.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn in_transit_row_bf(
    x_start: usize,
    n: usize,
    y: usize,
    w: usize,
    h_grid: usize,
    heights: &[f32],
    cell_props: &[f32],
    edge_vel_v: &[f32],
    shape_mask: &[u8],
    out: &mut [f32],
) {
    let below_y = (y + 1).min(h_grid - 1);
    let below_row_off = below_y * w;
    let row_below_valid = mask_lt((y + 1) as f32, h_grid as f32);
    let above_row_off = if y > 0 { (y - 1) * w } else { 0 };
    let row_above_valid = mask_gt(y as f32, 0.0);

    for j in 0..n {
        let x = x_start + j;
        let idx = y * w + x;
        let below_idx = below_row_off + x;
        let above_idx = above_row_off + x;
        let interior = mask_gt(x as f32, 0.0) * mask_lt((x + 1) as f32, w as f32);
        let below_inside = mask_ne(shape_mask[below_idx] as f32, MASK_OUTSIDE as f32);

        let h_below = heights[below_idx];
        let wet_below = cell_props[below_idx * 4 + PROP_WETNESS];
        let liq_below = crate::scalar_math::liquidity(wet_below);
        let cap_below = crate::scalar_math::cell_capacity_for(wet_below);
        let _ = liq_below; // liquidity is only an intermediate of cell_capacity_for here
        let downstream_route = edge_vel_v[idx].max(0.0) + (cap_below - h_below).max(0.0);
        let raw = edge_vel_v[above_idx].max(0.0).min(downstream_route);

        out[j] = raw * interior * below_inside * row_below_valid * row_above_valid;
    }
}

/// Same law as `in_transit_row_bf`, for a caller (`kernel_e`) whose wetness lives in its own
/// contiguous `Vec<f32>` (`StateE::prop[PROP_WETNESS]`) rather than interleaved `cell_props` --
/// identical arithmetic, just one slice instead of `cell_props[idx*4+PROP_WETNESS]`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn in_transit_row_bf_soa(
    x_start: usize,
    n: usize,
    y: usize,
    w: usize,
    h_grid: usize,
    heights: &[f32],
    wetness: &[f32],
    edge_vel_v: &[f32],
    shape_mask: &[u8],
    out: &mut [f32],
) {
    let below_y = (y + 1).min(h_grid - 1);
    let below_row_off = below_y * w;
    let row_below_valid = mask_lt((y + 1) as f32, h_grid as f32);
    let above_row_off = if y > 0 { (y - 1) * w } else { 0 };
    let row_above_valid = mask_gt(y as f32, 0.0);

    for j in 0..n {
        let x = x_start + j;
        let idx = y * w + x;
        let below_idx = below_row_off + x;
        let above_idx = above_row_off + x;
        let interior = mask_gt(x as f32, 0.0) * mask_lt((x + 1) as f32, w as f32);
        let below_inside = mask_ne(shape_mask[below_idx] as f32, MASK_OUTSIDE as f32);

        let h_below = heights[below_idx];
        let wet_below = wetness[below_idx];
        let cap_below = crate::scalar_math::cell_capacity_for(wet_below);
        let downstream_route = edge_vel_v[idx].max(0.0) + (cap_below - h_below).max(0.0);
        let raw = edge_vel_v[above_idx].max(0.0).min(downstream_route);

        out[j] = raw * interior * below_inside * row_below_valid * row_above_valid;
    }
}
