//! Task item 2: a cheap, vectorisable "could this edge flow?" predicate that needs no
//! `in_transit_at`, no hashes and no arbitration -- see the module doc comment on
//! `scalar_math::edge_sleeps`/`flux_edge_candidate` for the real (expensive) law this
//! approximates.
//!
//! **Why this is provably conservative (never a false negative).** `edge_sleeps` (the real gate)
//! is `starved || idle`, and an edge's candidate is forced to zero if it sleeps OR the lock roll
//! fires. This predicate computes:
//!
//! - `starved` EXACTLY as `edge_sleeps` does. Note the real caller (`kernel_a`/`kernel_e2`) feeds
//!   `edge_sleeps` the RAW heights (`scratch.h[e]`/`scratch.h[e+1]`), not the in-transit-adjusted
//!   `avail`, as its `avail_a`/`avail_b` parameters -- so the starvation test never needed
//!   `in_transit_at` in the first place, and reproducing it here is exact, not an approximation.
//! - `idle`, over-approximated: the real test is `v_e == 0.0 && driving.abs() <= tau`, where
//!   `driving = raw_driving + dispersion` and `dispersion` is a hashed draw in
//!   `[-D*tau, +D*tau]` (`D = DISPERSION_TAU_FRAC`). The MOST idle-breaking value dispersion can
//!   take pushes `|driving|` up to `|raw_driving| + D*tau`, so "idle is not GUARANTEED" (i.e. this
//!   predicate must pass the edge) whenever `|raw_driving| + D*tau > tau`, i.e.
//!   `|raw_driving| > tau*(1-D)` -- exactly `tau` minus the maximum possible dispersion magnitude,
//!   as the task specifies. This never UNDER-counts: any edge the real law could make non-idle
//!   passes; it can OVER-count (an edge that only looks non-idle for some unrealised dispersion
//!   draw) -- allowed by the task's conservatism rule.
//! - the lock roll is IGNORED entirely: it can only zero an otherwise-nonzero candidate, never
//!   create one, so ignoring it is safe in the "never false-negative" direction.
//! - `flux_edge_candidate`'s own clamping by the true (in-transit-adjusted) `avail` can also
//!   silently zero a candidate this predicate passes -- again safe, since that only removes flow,
//!   never adds it.

use crate::consts::DISPERSION_TAU_FRAC;

/// `true` if the edge between cells A and B COULD carry nonzero flux this pass. Conservative: may
/// return `true` for an edge whose real candidate ends up `0.0`, must never return `false` for an
/// edge whose real candidate is nonzero.
///
/// - `h_a`/`h_b`: raw cell heights (NOT `avail` -- see module doc comment).
/// - `freecap_a`/`freecap_b`: `(cap - h).max(0.0)`, same value `edge_sleeps`'s `room_a`/`room_b`
///   parameters receive.
/// - `head_base_a`/`head_base_b`: `h + k*LATERAL_PRESSURE_SCALE*janssen_depth`, BEFORE dispersion.
/// - `tau`: `GRANULAR_TAU_SCALE * threshold_a * granular_share_a` (the donor-side-`a` tau the real
///   edge uses for both the dispersion draw and `edge_sleeps`/`flux_edge_candidate`).
/// - `edge_vel_prev`: the edge's stored `edge_vel_h` from before this pass (`v_e`).
#[inline]
pub fn could_flow(
    h_a: f32,
    h_b: f32,
    freecap_a: f32,
    freecap_b: f32,
    head_base_a: f32,
    head_base_b: f32,
    tau: f32,
    edge_vel_prev: f32,
) -> bool {
    let starved_a = h_a <= 0.0 || freecap_b <= 0.0;
    let starved_b = h_b <= 0.0 || freecap_a <= 0.0;
    if starved_a && starved_b {
        return false;
    }
    if edge_vel_prev != 0.0 {
        return true;
    }
    let raw_driving = (head_base_a - head_base_b).abs();
    let max_dispersion = DISPERSION_TAU_FRAC * tau;
    raw_driving > tau - max_dispersion
}

/// Branch-free form of `could_flow`, for the "vectorised predicate loop" timing (task item 5) --
/// same law, expressed with `bf_math`-style float masks so it lowers the same way `kernel_e2`'s
/// stage 2 does under wasm+simd128/native autovectorisation. Returns `1.0`/`0.0` instead of
/// `bool`. Verified bit-for-bit mask-equivalent to `could_flow` in `predicate_bf_matches_scalar`.
#[inline]
pub fn could_flow_bf(
    h_a: f32,
    h_b: f32,
    freecap_a: f32,
    freecap_b: f32,
    head_base_a: f32,
    head_base_b: f32,
    tau: f32,
    edge_vel_prev: f32,
) -> f32 {
    use crate::bf_math::{mask_gt, mask_le, mask_ne};
    let starved_a = mask_le(h_a, 0.0).max(mask_le(freecap_b, 0.0));
    let starved_b = mask_le(h_b, 0.0).max(mask_le(freecap_a, 0.0));
    let starved = starved_a.min(starved_b);
    let has_momentum = mask_ne(edge_vel_prev, 0.0);
    let raw_driving = (head_base_a - head_base_b).abs();
    let max_dispersion = DISPERSION_TAU_FRAC * tau;
    let could_exceed = mask_gt(raw_driving, tau - max_dispersion);
    let not_idle = has_momentum.max(could_exceed);
    (1.0 - starved) * not_idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicate_bf_matches_scalar() {
        let mut rng = 12345u32;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng as f32 / u32::MAX as f32) * 4.0 - 1.0
        };
        for _ in 0..10000 {
            let (h_a, h_b, fc_a, fc_b, hb_a, hb_b, tau, v) =
                (next(), next(), next(), next(), next() * 5.0, next() * 5.0, next().abs(), next());
            let scalar = could_flow(h_a, h_b, fc_a, fc_b, hb_a, hb_b, tau, v);
            let bf = could_flow_bf(h_a, h_b, fc_a, fc_b, hb_a, hb_b, tau, v) != 0.0;
            assert_eq!(scalar, bf, "mismatch at h_a={h_a} h_b={h_b} fc_a={fc_a} fc_b={fc_b} hb_a={hb_a} hb_b={hb_b} tau={tau} v={v}");
        }
    }
}
