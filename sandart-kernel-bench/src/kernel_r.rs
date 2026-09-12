//! Kernel R: the scalar reference. A faithful, structural port of `settle_tick`'s section "2b.
//! RED-BLACK EDGE COLOURING" (the lateral/cross-gravity edge) followed by its "ARBITRATE + APPLY"
//! section, restricted to `PHASE == 1` (the base pass; see `consts::PHASE`) and the
//! production-default config (`head_field_active=false`, `multiplicative_lateral_gate` off,
//! `pressure_sensitive_flow=false` -- see `scalar_math`'s module doc comment).
//!
//! Structurally identical to the original: per-edge COLLECT into `touched_h` (colour 0 then
//! colour 1, exactly like the real red-black sweep), list-driven ARBITRATE, per-edge APPLY with
//! in-place `advect_properties`. See `lib.rs`'s module doc comment for how this was validated
//! against a real lateral pass.

use crate::consts::*;
use crate::scalar_math::*;
use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// Reusable scratch so repeated passes (the "run 200 times" equivalence/timing loop) don't pay a
/// fresh `w*h`-sized allocation every call. Only the entries touched by the previous pass are
/// cleared (via the touched-index lists), mirroring the fact that a lateral pass over ~550 of
/// 4096 blocks touches a small fraction of the grid.
pub struct Scratch {
    cell_avail: Vec<f32>,
    cell_freecap: Vec<f32>,
    cell_out_total: Vec<f32>,
    cell_in_total: Vec<f32>,
    cell_out_total_jit: Vec<f32>,
    cell_in_total_jit: Vec<f32>,
    cand_h: Vec<f32>,
    touched_h: Vec<usize>,
    touched_cells: Vec<usize>,
}

impl Scratch {
    pub fn new(w: usize, h: usize) -> Self {
        let n = w * h;
        Scratch {
            cell_avail: vec![0.0; n],
            cell_freecap: vec![0.0; n],
            cell_out_total: vec![0.0; n],
            cell_in_total: vec![0.0; n],
            cell_out_total_jit: vec![0.0; n],
            cell_in_total_jit: vec![0.0; n],
            cand_h: vec![0.0; n],
            touched_h: Vec::new(),
            touched_cells: Vec::new(),
        }
    }

    fn clear(&mut self) {
        for &i in &self.touched_cells {
            self.cell_avail[i] = 0.0;
            self.cell_freecap[i] = 0.0;
            self.cell_out_total[i] = 0.0;
            self.cell_in_total[i] = 0.0;
            self.cell_out_total_jit[i] = 0.0;
            self.cell_in_total_jit[i] = 0.0;
        }
        for &i in &self.touched_h {
            self.cand_h[i] = 0.0;
        }
        self.touched_h.clear();
        self.touched_cells.clear();
    }
}

/// Runs one lateral pass over `state.sim_blocks`, mutating `state.heights`, `state.cell_props`,
/// `state.cell_colors` and `state.edge_vel_h` in place. Returns the total realised mass flow
/// (sum of `|final_flux|` over edges above `MIN_FLUX`) as a cheap checksum.
///
/// `gravity_dir.x` is always `0.0` for both shipped scenarios (vertical gravity only), so it is
/// hardcoded rather than threaded through from the snapshot (which does not carry it).
pub fn run_pass(state: &mut State, scratch: &mut Scratch) -> f64 {
    scratch.clear();
    let w = state.w;
    let h = state.h;
    let block_size = state.block_size;
    let cols = state.cols;
    let time_seed = state.time_seed;
    const GRAVITY_DIR_X: f32 = 0.0;

    let is_inside = |mask: &[u8], x: usize, y: usize| mask[y * w + x] != MASK_OUTSIDE;

    let mut oversubscribed = false;

    for colour in 0..2usize {
        for &b in &state.sim_blocks {
            let b = b as usize;
            let bx = b % cols;
            let by = b / cols;
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                let row_offset = y * w;
                let first_x = if start_x % 2 == colour { start_x } else { start_x + 1 };
                let mut x = first_x;
                while x < end_x {
                    let center_idx = row_offset + x;
                    if is_inside(&state.shape_mask, x, y)
                        && x + 1 < w
                        && is_inside(&state.shape_mask, x + 1, y)
                    {
                        let wetness = state.cell_props[center_idx * 4 + PROP_WETNESS];
                        let cell_liquidity = liquidity(wetness);
                        let granular_share = 1.0 - cell_liquidity;
                        let cell_capacity = cell_capacity_for(wetness);
                        let seed = cell_seed(x, y, time_seed);

                        let nb_idx = center_idx + 1;
                        let h_a = state.heights[center_idx];
                        let h_b = state.heights[nb_idx];
                        let cap_b = cell_capacity_for(state.cell_props[nb_idx * 4 + PROP_WETNESS]);

                        let threshold_prop = state.cell_props[center_idx * 4 + PROP_THRESHOLD];
                        let tau = GRANULAR_TAU_SCALE * threshold_prop * granular_share;

                        let dispersion = dispersion_roll(seed, nb_idx, tau);

                        let liq_b = liquidity(state.cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let k_a = k_of_liquidity(cell_liquidity);
                        let k_b = k_of_liquidity(liq_b);
                        let depth_a = janssen_effective_depth(state.column_depth[center_idx], cell_liquidity);
                        let depth_b = janssen_effective_depth(state.column_depth[nb_idx], liq_b);

                        let head_a =
                            h_a + GRAVITY_DIR_X * GRAVITY_HEAD_SCALE + k_a * LATERAL_PRESSURE_SCALE * depth_a + dispersion;
                        let head_b_full = h_b + k_b * LATERAL_PRESSURE_SCALE * depth_b;
                        let tau_eff = tau;

                        let lock = lock_roll(seed, nb_idx) < GRAVITY_LOCK_CHANCE * granular_share;

                        if lock
                            || edge_sleeps(
                                head_a - head_b_full,
                                tau_eff,
                                state.edge_vel_h[center_idx],
                                h_a,
                                h_b,
                                cell_capacity - h_a,
                                cap_b - h_b,
                            )
                        {
                            if state.edge_vel_h[center_idx] != 0.0 {
                                state.edge_vel_h[center_idx] = 0.0;
                            }
                        } else {
                            let (c_sq, damping) = wave_params(wetness);
                            let pressure_weight = 1.0f32;

                            let avail_a = (h_a
                                - in_transit_at(center_idx, w, h, &state.heights, &state.cell_props, &state.edge_vel_v, &state.shape_mask))
                                .max(0.0);
                            let avail_b = (h_b
                                - in_transit_at(nb_idx, w, h, &state.heights, &state.cell_props, &state.edge_vel_v, &state.shape_mask))
                                .max(0.0);
                            let max_accept_fwd = (cap_b - h_b).max(0.0);
                            let max_accept_bwd = (cell_capacity - h_a).max(0.0);

                            let candidate = flux_edge_candidate(
                                head_a,
                                head_b_full,
                                c_sq,
                                damping,
                                tau_eff,
                                avail_a,
                                avail_b,
                                max_accept_fwd,
                                max_accept_bwd,
                                pressure_weight,
                                state.edge_vel_h[center_idx],
                            );

                            scratch.cand_h[center_idx] = candidate;
                            scratch.touched_h.push(center_idx);
                            scratch.cell_avail[center_idx] = avail_a;
                            scratch.cell_freecap[center_idx] = (cell_capacity - h_a).max(0.0);
                            scratch.cell_avail[nb_idx] = avail_b;
                            scratch.cell_freecap[nb_idx] = (cap_b - h_b).max(0.0);
                            scratch.touched_cells.push(center_idx);
                            scratch.touched_cells.push(nb_idx);

                            let (donor, acceptor, mag) = if candidate >= 0.0 {
                                (center_idx, nb_idx, candidate)
                            } else {
                                (nb_idx, center_idx, -candidate)
                            };
                            scratch.cell_out_total[donor] += mag;
                            scratch.cell_in_total[acceptor] += mag;
                            oversubscribed |= scratch.cell_out_total[donor] > scratch.cell_avail[donor]
                                || scratch.cell_in_total[acceptor] > scratch.cell_freecap[acceptor];
                        }
                    }
                    x += 2;
                }
            }
        }
    }

    if oversubscribed {
        for &idx in &scratch.touched_h {
            let candidate = scratch.cand_h[idx];
            let (donor, acceptor, mag) = if candidate >= 0.0 {
                (idx, idx + 1, candidate)
            } else {
                (idx + 1, idx, -candidate)
            };
            let jit = edge_share_jitter(&state.cell_props, donor, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
            scratch.cell_out_total_jit[donor] += mag * jit;
            scratch.cell_in_total_jit[acceptor] += mag * jit;
        }
    }

    let mut total_flow = 0.0f64;
    for &idx in &scratch.touched_h {
        let raw = scratch.cand_h[idx];
        let a_idx = idx;
        let b_idx = idx + 1;
        let (donor, acceptor) = if raw >= 0.0 { (a_idx, b_idx) } else { (b_idx, a_idx) };
        let scale = if oversubscribed {
            let jit = edge_share_jitter(&state.cell_props, donor, idx, EDGE_SALT_H.wrapping_add(PHASE), time_seed);
            edge_arbitration_scale(
                scratch.cell_out_total[donor],
                scratch.cell_out_total_jit[donor],
                scratch.cell_avail[donor],
                scratch.cell_in_total[acceptor],
                scratch.cell_in_total_jit[acceptor],
                scratch.cell_freecap[acceptor],
                jit,
            )
        } else {
            1.0
        };
        let final_flux = raw * scale;
        state.edge_vel_h[idx] = final_flux;
        if final_flux > MIN_FLUX {
            advect_properties(&mut state.cell_colors, &mut state.cell_props, a_idx, b_idx, final_flux, state.heights[b_idx]);
            state.heights[a_idx] -= final_flux;
            state.heights[b_idx] += final_flux;
            total_flow += final_flux as f64;
        } else if final_flux < -MIN_FLUX {
            let mag = -final_flux;
            advect_properties(&mut state.cell_colors, &mut state.cell_props, b_idx, a_idx, mag, state.heights[a_idx]);
            state.heights[b_idx] -= mag;
            state.heights[a_idx] += mag;
            total_flow += mag as f64;
        }
    }

    total_flow
}

/// Cell count inside `state.sim_blocks` -- the denominator for ns/cell/pass timing.
pub fn simulated_cell_count(state: &State) -> usize {
    let mut n = 0usize;
    for &b in &state.sim_blocks {
        let b = b as usize;
        let bx = b % state.cols;
        let by = b / state.cols;
        let start_x = bx * state.block_size;
        let end_x = ((bx + 1) * state.block_size).min(state.w);
        let start_y = by * state.block_size;
        let end_y = ((by + 1) * state.block_size).min(state.h);
        n += (end_x - start_x) * (end_y - start_y);
    }
    n
}
