//! Scalar math ported verbatim from `sandart-sim/src/physics.rs`'s lateral (cross-gravity) edge
//! and its helpers, restricted to the production-DEFAULT configuration this task specifies:
//! `head_field_active = false`, `multiplicative_lateral_gate` off, `pressure_sensitive_flow =
//! false`. That eliminates three whole branches (the head-field driving term, the multiplicative
//! conveyance form, and the pressure-rate factor) -- only the plain ADDITIVE branch
//! (`h + k*LATERAL_PRESSURE_SCALE*janssen_depth + dispersion`) is ported.
//!
//! Used directly by kernel R (the per-edge scalar reference) and as the scalar fallback inside
//! kernels A and B wherever a hash, a mask boundary, or the last column of a row makes
//! vectorising not worth it (see those kernels' own module doc comments for exactly where).

use crate::consts::*;

#[inline]
pub fn liquidity(wetness: f32) -> f32 {
    let t = ((wetness - 0.65) / (0.85 - 0.65)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn cell_capacity_for(wetness: f32) -> f32 {
    let l = liquidity(wetness);
    1.5 * (1.0 - l) + 1.0 * l
}

#[inline]
pub fn k_of_liquidity(liq: f32) -> f32 {
    liq + (1.0 - liq) * LATERAL_EARTH_PRESSURE_K
}

#[inline]
pub fn janssen_effective_depth(column_depth: f32, liquidity: f32) -> f32 {
    let saturating = JANSSEN_DEPTH_SCALE * (1.0 - (-column_depth / JANSSEN_DEPTH_SCALE).exp());
    liquidity * column_depth + (1.0 - liquidity) * saturating
}

/// `physics::wave_params`. Only wetness <= 0.75 (`(0.08, 0.76)`) and the ramp up to 0.85 matter
/// for the two shipped snapshots (Water is 1.0 throughout, DrySand is 0.0), but the full ramp is
/// ported so a future snapshot with intermediate wetness is not silently wrong.
#[inline]
pub fn wave_params(wetness: f32) -> (f32, f32) {
    if wetness <= 0.75 {
        (0.08, 0.76)
    } else if wetness <= 0.85 {
        let t = (wetness - 0.75) / 0.10;
        (0.08 + (0.18 - 0.08) * t, 0.76 + (0.92 - 0.76) * t)
    } else if wetness <= 0.90 {
        let t = (wetness - 0.85) / 0.05;
        (0.18 + (0.22 - 0.18) * t, 0.92 + (0.88 - 0.92) * t)
    } else if wetness <= 0.95 {
        let t = (wetness - 0.90) / 0.05;
        (0.22 + (0.16 - 0.22) * t, 0.88 + (0.86 - 0.88) * t)
    } else {
        let t = ((wetness - 0.95) / 0.05).min(1.0);
        (0.16 + (0.24 - 0.16) * t, 0.86 + (0.98 - 0.86) * t)
    }
}

/// `physics::in_transit_at`, specialised to the LATERAL edge's donor/acceptor read (`c` always
/// has an inside cell below it in every scenario the two snapshots exercise at the row this is
/// called for -- see the guard, ported unchanged).
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn in_transit_at(
    c: usize,
    w: usize,
    h: usize,
    temp_heights: &[f32],
    cell_props: &[f32],
    edge_vel_v: &[f32],
    shape_mask: &[u8],
) -> f32 {
    let cx = c % w;
    let cy = c / w;
    if !(cx > 0 && cx + 1 < w && cy > 0 && cy + 1 < h
        && shape_mask[(cy + 1) * w + cx] != MASK_OUTSIDE)
    {
        return 0.0;
    }
    let below = c + w;
    // The snapshot's `heights` IS the frozen pre-tick snapshot for this stand-in pass (see
    // `lib.rs` module doc comment), so `temp_heights[below].max(heightmap_data[below])` in the
    // original collapses to `temp_heights[below]` alone here -- both arrays are one and the same
    // input.
    let h_below = temp_heights[below];
    let cap_below = cell_capacity_for(cell_props[below * 4 + PROP_WETNESS]);
    let downstream_route = edge_vel_v[c].max(0.0) + (cap_below - h_below).max(0.0);
    edge_vel_v[c - w].max(0.0).min(downstream_route)
}

/// `physics::flux_edge_candidate`, unchanged.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn flux_edge_candidate(
    head_a: f32,
    head_b: f32,
    c_sq: f32,
    damping: f32,
    tau: f32,
    avail_a: f32,
    avail_b: f32,
    max_accept_fwd: f32,
    max_accept_bwd: f32,
    weight: f32,
    v_e_prev: f32,
) -> f32 {
    let driving = head_a - head_b;
    let yielded = if driving > tau {
        driving - tau
    } else if driving < -tau {
        driving + tau
    } else {
        0.0
    };
    let raw = (v_e_prev + c_sq * yielded) * damping;
    let v = raw.clamp(-1.0, 1.0);
    weight
        * if v > 0.0 {
            v.min(avail_a).min(max_accept_fwd)
        } else if v < 0.0 {
            -((-v).min(avail_b).min(max_accept_bwd))
        } else {
            0.0
        }
}

/// `physics::edge_sleeps`, unchanged (the `#[cfg(test)]` instrumentation call is dropped -- it
/// is a diagnostic counter, not part of the math).
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn edge_sleeps(
    driving: f32,
    tau: f32,
    v_e: f32,
    avail_a: f32,
    avail_b: f32,
    room_a: f32,
    room_b: f32,
) -> bool {
    if (avail_a <= 0.0 || room_b <= 0.0) && (avail_b <= 0.0 || room_a <= 0.0) {
        true
    } else {
        v_e == 0.0 && driving.abs() <= tau
    }
}

/// The 5-round integer hash shared by every per-edge/per-cell random draw in the lateral pass
/// (`edge_share_jitter`, the dispersion roll, the lock roll, `fall_flow_jitter`). `physics.rs`
/// inlines this same sequence at each call site rather than naming it; named here once since
/// every caller in this file needs the identical bit pattern.
#[inline]
fn hash_u01(mut h: u32) -> f32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

/// `physics::grain_jitter_strength`, unchanged.
#[inline]
pub fn grain_jitter_strength(cell_props: &[f32], cell: usize) -> f32 {
    let granular_share = (1.0 - liquidity(cell_props[cell * 4 + PROP_WETNESS])).clamp(0.0, 1.0);
    let grain_size = cell_props[cell * 4 + PROP_GRAIN_SIZE];
    let gran_s = (GRAIN_JITTER_SCALE * grain_size).clamp(0.0, GRAIN_JITTER_MAX) * granular_share;
    gran_s.max(0.05)
}

/// `physics::edge_share_jitter`, unchanged.
#[inline]
pub fn edge_share_jitter(cell_props: &[f32], donor: usize, edge_key: usize, salt: u32, time_seed: u32) -> f32 {
    let s = grain_jitter_strength(cell_props, donor);
    if s <= 0.0 {
        return 1.0;
    }
    let h = time_seed ^ (edge_key as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x2545_F491);
    let u = hash_u01(h);
    1.0 + s * (2.0 * u - 1.0)
}

/// `physics::budget_term`, unchanged.
#[inline]
pub fn budget_term(raw_total: f32, jit_total: f32, budget: f32, jitter: f32) -> f32 {
    if raw_total > budget && jit_total > 0.0 {
        (budget * jitter / jit_total).max(0.0)
    } else {
        1.0
    }
}

/// `physics::edge_arbitration_scale`, unchanged.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn edge_arbitration_scale(
    donor_out_total: f32,
    donor_out_total_jit: f32,
    donor_avail: f32,
    acceptor_in_total: f32,
    acceptor_in_total_jit: f32,
    acceptor_freecap: f32,
    jitter: f32,
) -> f32 {
    budget_term(donor_out_total, donor_out_total_jit, donor_avail, jitter)
        .min(budget_term(acceptor_in_total, acceptor_in_total_jit, acceptor_freecap, jitter))
        .min(1.0)
}

/// The per-edge dispersion draw at the lateral edge's collect site (`disp_roll`/`dispersion` in
/// `settle_tick`), and the per-cell `seed` it (and the lock roll) are derived from. `phase >= 2`'s
/// extra XOR term is omitted: this crate only ever ports `PHASE == 1`.
#[inline]
pub fn cell_seed(x: usize, y: usize, time_seed: u32) -> u32 {
    (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213)
}

#[inline]
pub fn dispersion_roll(seed: u32, nb_idx: usize, tau: f32) -> f32 {
    let disp_roll = ((seed ^ (nb_idx as u32).wrapping_mul(823)) & 0xFF) as f32 / 255.0;
    (disp_roll - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau
}

#[inline]
pub fn lock_roll(seed: u32, nb_idx: usize) -> f32 {
    ((seed ^ (nb_idx as u32).wrapping_mul(577)) & 0xFFFF) as f32 / 65535.0
}

/// `physics::stochastic_round`, unchanged.
#[inline]
pub fn stochastic_round(v: f32, entropy: u32) -> u8 {
    let h = hash_u01(entropy);
    (v + h) as u8
}

/// `physics::advect_properties`, unchanged (mixes ONE source into ONE destination -- kernel R
/// calls this directly per edge, exactly like the real per-edge APPLY; kernels A/B instead do the
/// "one shot, many inflows" Jacobi mix the task's design calls for, see `kernel_a`/`kernel_b`).
pub fn advect_properties(colors: &mut [u8], props: &mut [f32], src: usize, dst: usize, flow: f32, h_dst: f32) {
    let total = h_dst + flow;
    if total < 1e-6 {
        return;
    }
    let src_base = src * 4;
    let dst_base = dst * 4;
    if h_dst < 1e-4 {
        for ch in 0..4 {
            colors[dst_base + ch] = colors[src_base + ch];
            props[dst_base + ch] = props[src_base + ch];
        }
    } else {
        let w_keep = h_dst / total;
        let w_arrive = flow / total;
        for ch in 0..3 {
            let blended = (colors[dst_base + ch] as f32 * w_keep + colors[src_base + ch] as f32 * w_arrive)
                .clamp(0.0, 255.0);
            let entropy = flow.to_bits() ^ (dst as u32).wrapping_mul(2_654_435_761) ^ (ch as u32).wrapping_mul(97);
            colors[dst_base + ch] = stochastic_round(blended, entropy);
        }
        colors[dst_base + 3] = 255;
        for ch in 0..4 {
            props[dst_base + ch] = props[dst_base + ch] * w_keep + props[src_base + ch] * w_arrive;
        }
    }
}
