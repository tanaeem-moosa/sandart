use crate::grid::Heightmap;
use glam::Vec2;

/// Bounding coordinates to optimize Cellular Automata settling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
    pub active: bool,
}

/// Active marble state passed to the physics CA simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveMarbleInfo {
    pub pos: Vec2,
    pub vel: f32,
    pub vel_vec: Vec2,
}

use crate::{PROP_WETNESS, PROP_THRESHOLD, PROP_FLOW_RATE, PROP_GRAIN_SIZE};

/// Advect color and properties from src cell to dst cell based on the flow amount and dst cell's height before arrival
pub fn advect_properties(colors: &mut [f32], props: &mut [f32], src: usize, dst: usize, flow: f32, h_dst: f32) {
    let total = h_dst + flow;
    if total < 1e-6 {
        return;
    }
    let src_base = src * 4;
    let dst_base = dst * 4;

    if h_dst < 1e-4 {
        // Empty destination cell: inherit 100% of source color and properties
        for ch in 0..4 {
            colors[dst_base + ch] = colors[src_base + ch];
            props[dst_base + ch] = props[src_base + ch];
        }
    } else {
        let w_keep = h_dst / total;
        let w_arrive = flow / total;

        for ch in 0..3 {
            colors[dst_base + ch] = (
                colors[dst_base + ch] * w_keep
                + colors[src_base + ch] * w_arrive
            ).clamp(0.0, 255.0);
        }
        colors[dst_base + 3] = 255.0; // opaque alpha

        for ch in 0..4 {
            props[dst_base + ch] = props[dst_base + ch] * w_keep + props[src_base + ch] * w_arrive;
        }
    }
}

/// Helper function to add sand to a cell, clamping it at max_height (glass top)
/// and distributing any excess volume to its available 4-way neighbors, with properties advection.
fn add_sand_with_limit_properties(
    heightmap: &mut Heightmap,
    cell_colors: &mut [f32],
    cell_props: &mut [f32],
    src_idx: usize,
    idx: usize,
    w: usize,
    h: usize,
    amount: f32,
    max_height: f32,
) {
    if amount <= 0.0 {
        return;
    }
    let current_h = heightmap.data[idx];
    if current_h + amount <= max_height {
        advect_properties(cell_colors, cell_props, src_idx, idx, amount, current_h);
        heightmap.data[idx] = current_h + amount;
    } else {
        let allowed = (max_height - current_h).max(0.0);
        advect_properties(cell_colors, cell_props, src_idx, idx, allowed, current_h);
        heightmap.data[idx] = current_h + allowed;
        let mut excess = amount - allowed;
        if excess > 1e-6 {
            // Distribute excess to neighbors that are below the max_height
            let x = idx % w;
            let y = idx / w;
            
            let mut neighbors = [0usize; 4];
            let mut num_neighbors = 0;
            if x > 0 { neighbors[num_neighbors] = idx - 1; num_neighbors += 1; }
            if x + 1 < w { neighbors[num_neighbors] = idx + 1; num_neighbors += 1; }
            if y > 0 { neighbors[num_neighbors] = idx - w; num_neighbors += 1; }
            if y + 1 < h { neighbors[num_neighbors] = idx + w; num_neighbors += 1; }

            // Filter neighbors that have room (height < max_height)
            let mut room_neighbors = [(0usize, 0.0f32); 4];
            let mut num_room_neighbors = 0;
            for i in 0..num_neighbors {
                let n_idx = neighbors[i];
                let nh = heightmap.data[n_idx];
                if nh < max_height {
                    room_neighbors[num_room_neighbors] = (n_idx, max_height - nh);
                    num_room_neighbors += 1;
                }
            }

            if num_room_neighbors == 0 {
                // If all neighbors are full, distribute to all neighbors equally (overflowing slightly)
                let num = num_neighbors as f32;
                let share = excess / num;
                for i in 0..num_neighbors {
                    let n_idx = neighbors[i];
                    advect_properties(cell_colors, cell_props, idx, n_idx, share, heightmap.data[n_idx]);
                    heightmap.data[n_idx] += share;
                }
            } else {
                // Distribute to room_neighbors proportional to their room
                let mut distributed = false;
                for _ in 0..3 {
                    if excess <= 1e-6 {
                        distributed = true;
                        break;
                    }
                    if num_room_neighbors == 0 {
                        break;
                    }
                    let share = excess / num_room_neighbors as f32;
                    let mut next_room = [(0usize, 0.0f32); 4];
                    let mut next_num_room = 0;
                    for i in 0..num_room_neighbors {
                        let (n_idx, room) = room_neighbors[i];
                        if room > 0.0 {
                            let to_add = share.min(room);
                            advect_properties(cell_colors, cell_props, idx, n_idx, to_add, heightmap.data[n_idx]);
                            heightmap.data[n_idx] += to_add;
                            excess -= to_add;
                            let new_room = room - to_add;
                            if new_room > 0.0 {
                                next_room[next_num_room] = (n_idx, new_room);
                                next_num_room += 1;
                            }
                        }
                    }
                    room_neighbors = next_room;
                    num_room_neighbors = next_num_room;
                }
                if !distributed && excess > 1e-6 {
                    let num = num_neighbors as f32;
                    let share = excess / num;
                    for i in 0..num_neighbors {
                        let n_idx = neighbors[i];
                        advect_properties(cell_colors, cell_props, idx, n_idx, share, heightmap.data[n_idx]);
                        heightmap.data[n_idx] += share;
                    }
                }
            }
        }
    }
}

/// Continuous "how liquid is this cell" weight in [0, 1], derived from `wetness` via a
/// smoothstep ramp centered on the `wetness >= 0.75` branch cut used to select the wave solver
/// (`physics.rs` `settle_tick`, `if wetness >= 0.75 && !gravity_active`).
///
/// That branch selection itself stays a hard binary switch (Sandbox liquids must keep going
/// through the wave solver, and that fork is out of scope for this phase). `liquidity` is used
/// instead to interpolate the *continuous* CA parameters (droplet quantization, threshold/alpha,
/// gravity_push strength, transfer coefficient, and cell capacity) so that a cell whose `wetness`
/// drifts a hair across 0.75 under property advection (see `advect_properties`) does not suddenly
/// flip between "flows like liquid" and "frozen solid" parameters (defect C5). At the extremes
/// (wetness <= 0.65 or >= 0.85) this reproduces the exact pre-existing granular/liquid parameter
/// values, so materials that never approach the cut are bit-identical to before.
fn liquidity(wetness: f32) -> f32 {
    let t = ((wetness - 0.65) / (0.85 - 0.65)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Incompressibility cap for a cell of the given `wetness`: how much material one cell may hold.
///
/// Granular materials keep the historical 1.5 packing (load-bearing for the sand-pile height
/// tests); liquids cap at 1.0 (C1). Interpolated by `liquidity` so there is no hard cut.
/// Gravitational head, in units of "one saturated cell of fill", per cell of travel per unit of
/// `gravity_dir`. The unified head is `H = h + Phi`, with `Phi(r) = -(g . r) * GRAVITY_HEAD_SCALE`.
///
/// At the shipped Sand-fall gravity of 0.04 this makes the head drop across one row exactly 1.0,
/// which is the natural unit: it is what makes "fall into the empty cell below" outrank "spread
/// into the empty cell beside" by precisely the weight of one saturated cell. Because `Phi` is
/// proportional to `|g|`, the gravity slider moves behaviour continuously between the Sandbox
/// (`Phi == 0`, pure free-surface wave) and Sand-fall regimes instead of flipping between two
/// solvers (defect C6).
const GRAVITY_HEAD_SCALE: f32 = 25.0;

fn cell_capacity_for(wetness: f32) -> f32 {
    let l = liquidity(wetness);
    1.5 * (1.0 - l) + 1.0 * l
}

/// Conservative per-edge flux update — the Phase 5 (Option B) replacement for the per-cell
/// Laplacian wave update.
///
/// For an edge `e = (a, b)` with gravitational heads `head_a`/`head_b` (`H = h + Phi(g)`; `Phi` is
/// zero when gravity is out-of-plane, so `H = h` reduces to a pure free-surface head):
///
/// ```text
/// yielded = sign(H_a - H_b) * max(|H_a - H_b| - tau, 0)     // tau = yield stress (0 for liquid)
/// v_e    <- (v_e + c_sq * yielded) * damping                // per-edge momentum
/// flux    = clamp(v_e, -(donor b limits), +(donor a limits))
/// h_a -= flux ; h_b += flux
/// ```
///
/// Two properties this buys over the old formulation:
///
/// 1. **Mass conservation by construction.** Every edge debits exactly what it credits, so the
///    total is invariant no matter *which* blocks the LOD scheduler chose to run this tick. The
///    old per-cell form only telescoped to zero if every cell in the domain updated in the same
///    pass, which `will_simulate[b]` explicitly breaks (defect C7).
/// 2. **No unilateral clamp.** The old form ended in `(h + v).clamp(0.0, 1.0)`, an edit with no
///    counterparty: flooring a negative excursion to 0 *adds* mass and capping at 1.0 *discards*
///    it. Here the donor limit (`h_donor`) and the acceptor limit (`cap - h_acceptor`) only ever
///    *reduce a transfer*, which cannot change the total.
///
/// Reduction to the old wave solver at `tau = 0`, `Phi = 0`: summing the four edge fluxes incident
/// on a cell gives `Δh_c = c_sq * (h_l + h_r + h_t + h_b - 4 h_c)` with the same damped-momentum
/// history, i.e. exactly the old `v_new = (v + c_sq * laplacian) * damping; h += v_new`. Ripples
/// and sloshing are preserved; only the leak is gone.
///
/// `*v_e` is set to the *realised* flux rather than the raw integrated velocity. That is the
/// anti-windup term: an edge whose donor is empty or whose acceptor is full would otherwise
/// accumulate unbounded head every tick and then discharge it in one burst the instant the
/// constraint lifts.
///
/// `avail_a` / `avail_b` are the donor limits: how much of each endpoint's mass is actually
/// available to move across *this* edge. They are the cell's full height for a gravity-aligned
/// edge, but for a cross-gravity edge the mass that arrived from upstream during this tick is
/// still in transit — it is unsupported, exerts no hydrostatic pressure, and must not be able to
/// push sideways. Subtracting it is what distinguishes a falling stream (everything it holds
/// arrived this tick, so nothing spreads) from a settled pool (nothing arrived, so all of it
/// levels), without either a free-fall special case or a column-pressure sweep.
///
/// `weight` scales the realised flux. It is the `liquidity` share of the donor cell when the
/// granular CA and this solver are both contributing to the same edge, so that a cell whose
/// `wetness` drifts across the old `>= 0.75` cut hands over between the two solvers continuously
/// instead of switching regime (defect C5). It is 1.0 wherever this solver acts alone.
///
/// Returns the signed flux (positive = `a` -> `b`).
#[allow(clippy::too_many_arguments)]
fn flux_edge(
    a_b: usize,
    b_b: usize,
    a_idx: usize,
    b_idx: usize,
    head_a: f32,
    head_b: f32,
    c_sq: f32,
    damping: f32,
    tau: f32,
    cap_a: f32,
    cap_b: f32,
    avail_a: f32,
    avail_b: f32,
    weight: f32,
    v_e: &mut f32,
    temp_heights: &mut [f32],
    cell_colors: &mut [f32],
    cell_props: &mut [f32],
    modified: &mut Vec<bool>,
    next_displacements: &mut Vec<f32>,
    total_flow: &mut f32,
    flow_occurred: &mut bool,
) -> f32 {
    let driving = head_a - head_b;
    let yielded = if driving > tau {
        driving - tau
    } else if driving < -tau {
        driving + tau
    } else {
        0.0
    };

    let v = (*v_e + c_sq * yielded) * damping;

    let h_a = temp_heights[a_idx];
    let h_b = temp_heights[b_idx];

    // Donor mass and acceptor capacity, in the direction the velocity actually points.
    let flux = weight * if v > 0.0 {
        v.min(avail_a).min((cap_b - h_b).max(0.0))
    } else if v < 0.0 {
        -((-v).min(avail_b).min((cap_a - h_a).max(0.0)))
    } else {
        0.0
    };

    *v_e = flux;

    // Below this the transfer is pure f32 noise; skipping it is still exactly conservative
    // (nothing is added or removed), it just avoids an advect_properties call per edge per tick.
    const MIN_FLUX: f32 = 1e-7;
    if flux > MIN_FLUX {
        activate_neighbor(a_b, flux, modified, next_displacements);
        activate_neighbor(b_b, flux, modified, next_displacements);
        advect_properties(cell_colors, cell_props, a_idx, b_idx, flux, temp_heights[b_idx]);
        temp_heights[a_idx] -= flux;
        temp_heights[b_idx] += flux;
        *total_flow += flux;
        *flow_occurred = true;
    } else if flux < -MIN_FLUX {
        let mag = -flux;
        activate_neighbor(a_b, mag, modified, next_displacements);
        activate_neighbor(b_b, mag, modified, next_displacements);
        advect_properties(cell_colors, cell_props, b_idx, a_idx, mag, temp_heights[a_idx]);
        temp_heights[b_idx] -= mag;
        temp_heights[a_idx] += mag;
        *total_flow += mag;
        *flow_occurred = true;
    }

    flux
}

fn wave_params(wetness: f32) -> (f32, f32) {
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

fn get_ca_params(
    wetness: f32,
    threshold_prop: f32,
    flow_rate_prop: f32,
    grain_size: f32,
    higher_neighbors: usize,
    sliding_active: bool,
    closest_marble_vel: f32,
    gravity_active: bool,
) -> (f32, f32, f32, Option<f32>) {
    // Oobleck shear-thickening
    if wetness >= 0.50 && wetness < 0.65 {
        let t = ((closest_marble_vel - 0.03) / 0.12).clamp(0.0, 1.0);
        let t_steep = t * t;
        let threshold = 0.005 + (0.32 - 0.005) * t_steep;
        let alpha = 0.40 + (0.005 - 0.40) * t_steep;
        let lock_chance = 0.02 + (0.98 - 0.02) * t_steep;
        return (threshold, alpha, lock_chance, None);
    }

    // Continuous liquid weight for this cell (see `liquidity` doc comment). Used below to blend
    // the granular and liquid CA parameters instead of hard-switching on `wetness >= 0.75`.
    let liquidity = liquidity(wetness);

    // Quantization size (droplet beading for liquids under gravity, discrete grains in sandbox).
    // Gated on `liquidity > 0.0` (wetness > 0.65) rather than the hard `wetness >= 0.75` cut, so
    // a cell drifting across the old cut doesn't flip discretely between "beaded" and "smooth".
    let quantize_size = if liquidity > 0.0 && gravity_active {
        Some(0.025) // Droplet/bead quantization for liquids under gravity
    } else if wetness < 0.30 && !gravity_active {
        if grain_size >= 0.60 {
            Some(0.035)
        } else if grain_size >= 0.40 {
            Some(0.01)
        } else if grain_size >= 0.08 {
            Some(0.015)
        } else {
            None
        }
    } else {
        None
    };

    // Hysteresis threshold (lower repose threshold during gravity settling for natural sliding/funneling)
    let mut threshold = if wetness < 0.15 && sliding_active {
        0.5 * threshold_prop
    } else {
        threshold_prop
    };

    // Flow rate (alpha) (faster settling when gravity is pulling sand down)
    let mut alpha = flow_rate_prop;

    if gravity_active {
        threshold *= 0.35; // Lower friction/repose angle in Sand-fall mode for realistic fluid flow

        // Phase 5: the liquid (threshold = 0.0, alpha = 0.75) blend that used to live here is
        // gone. Under gravity a cell's liquid share is now carried by the conservative edge-flux
        // solver in `settle_tick` and the CA carries only the complementary `1 - liquidity`
        // share, so the CA no longer has to impersonate a liquid at all. That deleted the whole
        // Phase 2 tuning cluster — `liquid_alpha = 0.75` (which passed L1 and L3 only inside a
        // narrow 0.70-0.80 band), plus the 0.70 free-fall and 0.90 lateral transfer coefficients
        // and the `liquid_can_still_fall` gate below. In the flux form the equivalent limits are
        // not coefficients at all: they are the donor's mass and the acceptor's capacity, which
        // are physical quantities rather than tuned ones.
        //
        // C5 (a material drifting across the old `wetness >= 0.75` cut must not change regime)
        // is still handled continuously — the handover is now between the two *solvers*, by the
        // same `liquidity` weight, rather than between two parameter sets inside one solver.
        alpha = (alpha * 1.5).min(0.8);
    }

    // Lock chance
    let lock_chance = if gravity_active {
        0.05 // Low locking under gravity so sand avalanches smoothly into a natural hill
    } else if wetness < 0.05 {
        if flow_rate_prop >= 0.21 {
            // DrySand / CoarseSand stochastic locking
            if higher_neighbors >= 3 { 0.80 } else { 0.10 }
        } else {
            // FinePowder / MoonDust
            let t = ((threshold_prop - 0.05) / 0.15).clamp(0.0, 1.0);
            0.02 + (0.40 - 0.02) * t
        }
    } else if wetness < 0.30 {
        // Snow / KineticSand
        let t = ((wetness - 0.05) / 0.25).clamp(0.0, 1.0);
        0.30 + (0.75 - 0.30) * t
    } else {
        // WetSand / ButterCream
        let t = ((wetness - 0.30) / 0.40).clamp(0.0, 1.0);
        0.15 + (0.20 - 0.15) * t
    };

    (threshold, alpha, lock_chance, quantize_size)
}


/// Displace sand along a line segment from start to end, carving a groove
/// and depositing the displaced volume into the surrounding ridge area.
pub fn displace_line(
    heightmap: &mut Heightmap,
    cell_colors: &mut [f32],
    cell_props: &mut [f32],
    start: Vec2,
    end: Vec2,
    radius: f32,
    active_bounds: &mut ActiveBounds,
) {
    if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }

    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return;
    }

    // Convert coordinates to grid space
    let ax = (start.x + 1.0) * 0.5 * w as f32;
    let ay = (1.0 - start.y) * 0.5 * h as f32;
    let bx = (end.x + 1.0) * 0.5 * w as f32;
    let by = (1.0 - end.y) * 0.5 * h as f32;

    let r_grid = radius * (w as f32 / 2.0);
    let r_grid_clamped = r_grid.min(w as f32);

    // Define ridge width (60% of the marble radius)
    let w_grid = r_grid_clamped * 0.6;
    let total_radius = r_grid_clamped + w_grid;
    let total_radius_clamped = total_radius.min(w as f32);

    // Early out if the swept area is completely outside the grid
    let min_center_x = ax.min(bx);
    let max_center_x = ax.max(bx);
    let min_center_y = ay.min(by);
    let max_center_y = ay.max(by);

    if max_center_x < -total_radius_clamped
        || min_center_x > w as f32 + total_radius_clamped
        || max_center_y < -total_radius_clamped
        || min_center_y > h as f32 + total_radius_clamped
    {
        return;
    }

    // Safe bounding box calculations in float space before casting to usize
    let min_x_float = (min_center_x - total_radius_clamped)
        .clamp(0.0, w as f32)
        .floor();
    let max_x_float = (max_center_x + total_radius_clamped)
        .clamp(0.0, w as f32)
        .ceil();
    let min_y_float = (min_center_y - total_radius_clamped)
        .clamp(0.0, h as f32)
        .floor();
    let max_y_float = (max_center_y + total_radius_clamped)
        .clamp(0.0, h as f32)
        .ceil();

    let min_x = min_x_float as usize;
    let max_x = (max_x_float as usize).min(w - 1);
    let min_y = min_y_float as usize;
    let max_y = (max_y_float as usize).min(h - 1);

    // Update settling active bounding box
    let padding = 15;
    let pad_min_x = min_x.saturating_sub(padding);
    let pad_max_x = max_x.saturating_add(padding).min(w - 1);
    let pad_min_y = min_y.saturating_sub(padding);
    let pad_max_y = max_y.saturating_add(padding).min(h - 1);

    if active_bounds.active {
        active_bounds.min_x = active_bounds.min_x.min(pad_min_x);
        active_bounds.max_x = active_bounds.max_x.max(pad_max_x);
        active_bounds.min_y = active_bounds.min_y.min(pad_min_y);
        active_bounds.max_y = active_bounds.max_y.max(pad_max_y);
    } else {
        active_bounds.min_x = pad_min_x;
        active_bounds.max_x = pad_max_x;
        active_bounds.min_y = pad_min_y;
        active_bounds.max_y = pad_max_y;
        active_bounds.active = true;
    }

    // Segment vector
    let vx = bx - ax;
    let vy = by - ay;
    let len_sq = vx * vx + vy * vy;
    let len = if len_sq >= 1e-6 { len_sq.sqrt() } else { 0.0 };
    let inv_len_sq = if len_sq >= 1e-6 { 1.0 / len_sq } else { 0.0 };

    let r_groove_sq = r_grid_clamped * r_grid_clamped;

    // Ridge ray sampling offsets
    let d1 = r_grid_clamped + w_grid * 0.25;
    let d2 = r_grid_clamped + w_grid * 0.50;
    let d3 = r_grid_clamped + w_grid * 0.75;

    // Scan bounding box to carve the groove and displace sand radially/perpendicularly
    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row_offset = y * w;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;

            // Distance to segment AB (used for carving)
            let (closest_x, closest_y) = if len_sq < 1e-6 {
                (ax, ay)
            } else {
                let t = (((px - ax) * vx + (py - ay) * vy) * inv_len_sq).clamp(0.0, 1.0);
                (ax + t * vx, ay + t * vy)
            };

            let dx = px - closest_x;
            let dy = py - closest_y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq < r_groove_sq {
                let dist = dist_sq.sqrt();
                // Spherical groove profile: z_groove = R - sqrt(R^2 - d^2)
                let h_target = r_grid_clamped - (r_groove_sq - dist_sq).max(0.0).sqrt();
                let h_target_profile = (h_target / r_grid_clamped) * crate::DEFAULT_SAND_HEIGHT;

                let current_idx = row_offset + x;
                let current_h = heightmap.data[current_idx];

                let wetness = cell_props[current_idx * 4 + PROP_WETNESS];

                // Continuous residual_factor mapping based on wetness
                let residual_factor = if wetness >= 0.50 && wetness < 0.65 {
                    let speed = (end - start).length();
                    let t = (speed / 0.01).clamp(0.0, 1.0);
                    0.50 * t * t
                } else if wetness >= 0.70 {
                    0.0
                } else if wetness < 0.45 {
                    0.20 + (0.35 - 0.20) * (wetness / 0.45)
                } else {
                    0.35 * (1.0 - (wetness - 0.45) / 0.25)
                };

                // Scale target height relative to the current height to support multi-pass clearing
                let h_target_norm = residual_factor * current_h.max(h_target_profile) + (1.0 - residual_factor) * h_target_profile;

                // Add a tiny micro-texture noise to the groove base
                let seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                let noise = (((seed & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.05; // Range [-0.025, 0.025]
                let h_target_noisy = (h_target_norm + noise).clamp(0.0, 1.0);

                if current_h > h_target_noisy {
                    let diff = current_h - h_target_noisy;
                    heightmap.data[current_idx] = h_target_noisy;

                    // Projection on the infinite line (used for perpendicular displacement origin/direction)
                    let (closest_line_x, closest_line_y) = if len_sq < 1e-6 {
                        (ax, ay)
                    } else {
                        let t_unclamped = ((px - ax) * vx + (py - ay) * vy) * inv_len_sq;
                        (ax + t_unclamped * vx, ay + t_unclamped * vy)
                    };

                    let dx_line = px - closest_line_x;
                    let dy_line = py - closest_line_y;
                    let dist_line_sq = dx_line * dx_line + dy_line * dy_line;
                    let dist_line = dist_line_sq.sqrt();

                    // Distribute diff: perpendicular to motion if moving, radial if stationary
                    let (dir_x, dir_y) = if len_sq >= 1e-6 && len > 1e-4 {
                        if dist_line > 1e-4 {
                            (dx_line / dist_line, dy_line / dist_line)
                        } else {
                            // Default perpendicular direction if exactly on the line
                            (-vy / len, vx / len)
                        }
                    } else {
                        if dist > 1e-4 {
                            (dx / dist, dy / dist)
                        } else {
                            (1.0, 0.0)
                        }
                    };

                    // Perturb sample distances with coordinate-locked noise to simulate clumped deposition
                    let base_seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                    let seed_d1 = base_seed ^ 12345;
                    let noise_d1 = (((seed_d1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d1_p = (d1 + noise_d1).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d2 = base_seed ^ 67890;
                    let noise_d2 = (((seed_d2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d2_p = (d2 + noise_d2).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d3 = base_seed ^ 54321;
                    let noise_d3 = (((seed_d3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d3_p = (d3 + noise_d3).clamp(r_grid_clamped, total_radius_clamped);

                    // Calculate target coordinates
                    let rx1 = (closest_line_x + dir_x * d1_p).floor() as isize;
                    let ry1 = (closest_line_y + dir_y * d1_p).floor() as isize;

                    let rx2 = (closest_line_x + dir_x * d2_p).floor() as isize;
                    let ry2 = (closest_line_y + dir_y * d2_p).floor() as isize;

                    let rx3 = (closest_line_x + dir_x * d3_p).floor() as isize;
                    let ry3 = (closest_line_y + dir_y * d3_p).floor() as isize;

                    // Perturb weights based on the destination cell coordinates (rx, ry)
                    let seed_w1 =
                        (rx1.max(0) as u32).wrapping_mul(1299689) ^ (ry1.max(0) as u32).wrapping_mul(314159) ^ 9991;
                    let seed_w2 =
                        (rx2.max(0) as u32).wrapping_mul(1299689) ^ (ry2.max(0) as u32).wrapping_mul(314159) ^ 9992;
                    let seed_w3 =
                        (rx3.max(0) as u32).wrapping_mul(1299689) ^ (ry3.max(0) as u32).wrapping_mul(314159) ^ 9993;

                    let nf1 = 1.0 + (((seed_w1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6; // +/- 30% variation
                    let nf2 = 1.0 + (((seed_w2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;
                    let nf3 = 1.0 + (((seed_w3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;

                    let mut w1 = 0.5 * nf1;
                    let mut w2 = (1.0 / 3.0) * nf2;
                    let mut w3 = (1.0 / 6.0) * nf3;

                    let sum_w = w1 + w2 + w3;
                    if sum_w > 0.0 {
                        let inv_sum = 1.0 / sum_w;
                        w1 *= inv_sum;
                        w2 *= inv_sum;
                        w3 *= inv_sum;
                    } else {
                        w1 = 0.5;
                        w2 = 1.0 / 3.0;
                        w3 = 1.0 / 6.0;
                    }

                    let rx1_clamped = rx1.clamp(0, w as isize - 1) as usize;
                    let ry1_clamped = ry1.clamp(0, h as isize - 1) as usize;
                    let dest1_idx = ry1_clamped * w + rx1_clamped;
                    let h_above1 = (heightmap.data[dest1_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx2_clamped = rx2.clamp(0, w as isize - 1) as usize;
                    let ry2_clamped = ry2.clamp(0, h as isize - 1) as usize;
                    let dest2_idx = ry2_clamped * w + rx2_clamped;
                    let h_above2 = (heightmap.data[dest2_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx3_clamped = rx3.clamp(0, w as isize - 1) as usize;
                    let ry3_clamped = ry3.clamp(0, h as isize - 1) as usize;
                    let dest3_idx = ry3_clamped * w + rx3_clamped;
                    let h_above3 = (heightmap.data[dest3_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    // Scale factor for asymptotic decay based on marble diameter/height in heightmap units
                    let scale = 2.0 * (radius / 0.018).max(0.1);
                    
                    let x1 = h_above1 / scale;
                    let m1 = 1.0 / (1.0 + x1 * x1 * x1 * x1);

                    let x2 = h_above2 / scale;
                    let m2 = 1.0 / (1.0 + x2 * x2 * x2 * x2);

                    let x3 = h_above3 / scale;
                    let m3 = 1.0 / (1.0 + x3 * x3 * x3 * x3);

                    let mut forward_vol = 0.0f32;
                    let mut forward_dest_idx = 0;
                    if len_sq >= 1e-6 && len > 1e-4 {
                        let forward_dist = r_grid_clamped * 1.05; // Just in front of the marble boundary
                        let fx = (px + (vx / len) * forward_dist).floor() as isize;
                        let fy = (py + (vy / len) * forward_dist).floor() as isize;
                        let fx_clamped = fx.clamp(0, w as isize - 1) as usize;
                        let fy_clamped = fy.clamp(0, h as isize - 1) as usize;
                        forward_dest_idx = fy_clamped * w + fx_clamped;
                        forward_vol = (diff * 0.10).min(0.10);
                    }

                    let side_diff = diff - forward_vol;
                    let deposited_volume = side_diff * (w1 * m1 + w2 * m2 + w3 * m3) + forward_vol;
                    if deposited_volume > 1e-6 {
                        heightmap.data[current_idx] = current_h - deposited_volume;
                        if side_diff > 0.0 {
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest1_idx, w, h, side_diff * w1 * m1, 1.5);
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest2_idx, w, h, side_diff * w2 * m2, 1.5);
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest3_idx, w, h, side_diff * w3 * m3, 1.5);
                        }
                        if forward_vol > 0.0 {
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, forward_dest_idx, w, h, forward_vol, 1.5);
                        }
                    } else {
                        // Restore height to conserve volume if no deposition can happen
                        heightmap.data[current_idx] = current_h;
                    }
                }
            }
        }
    }
}

/// Deterministic pseudo-random float in [0, 1) from an integer seed. Used to give
/// procedurally-generated shape features (staircase steps, etc.) organic variation
/// while staying stable across repeated shape_mask regenerations.
fn step_hash(n: u32) -> f32 {
    let h = n.wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
    let h = h ^ (h >> 15);
    (h % 10000) as f32 / 10000.0
}

pub fn eval_sandbox_shape(
    cx: usize,
    cy: usize,
    w: usize,
    h: usize,
    shape: crate::SandboxShape,
    neck_width: f32,
    hourglass_curve: f32,
) -> (bool, bool) {
    let center_x = w as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let dx = cx as f32 - center_x;
    let dy = cy as f32 - center_y;
    let w_f = w as f32;
    let h_f = h as f32;

    let r_x = 0.46 * w_f;
    let r_y = 0.46 * h_f;
    let r_x_sq = r_x * r_x;
    let r_oval_y_sq = (0.35 * h_f) * (0.35 * h_f);
    let safe_r_x = r_x - 1.5;
    let safe_r_y = r_y - 1.5;
    let safe_circle_r_sq = safe_r_x * safe_r_x;

    match shape {
        crate::SandboxShape::Circle => {
            let dist_sq = dx * dx + dy * dy;
            (dist_sq < r_x_sq, dist_sq < safe_circle_r_sq)
        }
        crate::SandboxShape::Square => {
            let adx = dx.abs();
            let ady = dy.abs();
            (adx < r_x && ady < r_y, adx < safe_r_x && ady < safe_r_y)
        }
        crate::SandboxShape::Oval => {
            let oval_val = (dx * dx) / r_x_sq + (dy * dy) / r_oval_y_sq;
            (oval_val < 1.0, oval_val < 0.98)
        }
        crate::SandboxShape::Hourglass => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;

            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                let inside = dx.abs() < allowed_hw;
                let safe_allowed_hw = (allowed_hw - 1.5).max(1.0);
                let is_safe = dx.abs() < safe_allowed_hw && dy_abs < (chamber_h - 1.5);
                (inside, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::MultiStageHourglass => {
            let chamber_h = 0.42 * h_f;
            if dy.abs() >= chamber_h {
                return (false, false);
            }
            let max_hw = 0.35 * w_f;
            let neck_hw = (neck_width * w_f).max(3.0);

            let (start_center, end_center, stage_y0) = if dy < -0.14 * h_f {
                (-0.12 * w_f, -0.12 * w_f, -0.42 * h_f)
            } else if dy < 0.14 * h_f {
                (-0.12 * w_f, 0.12 * w_f, -0.14 * h_f)
            } else {
                (0.12 * w_f, 0.0, 0.14 * h_f)
            };

            let t = ((dy - stage_y0) / (0.28 * h_f)).clamp(0.0, 1.0);
            // Smoothstep easing (zero derivative at t=0 and t=1) gives each stage's
            // center-line a gentle S-curve, and keeps the slope continuous across stage
            // boundaries so the three chambers join in a smooth serpentine flow instead
            // of the sharp angular kinks a plain linear blend produces.
            let t_curve = t * t * (3.0 - 2.0 * t);
            let center_x = start_center + t_curve * (end_center - start_center);
            let allowed_hw = neck_hw + (1.0 - t_curve) * (max_hw - neck_hw);

            let dx_local = dx - center_x;
            let inside = dx_local.abs() < allowed_hw;
            let is_safe = dx_local.abs() < (allowed_hw - 1.5).max(1.0);
            (inside, is_safe)
        }
        crate::SandboxShape::GaltonBoard => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                if dx.abs() >= allowed_hw {
                    return (false, false);
                }
                
                if dy > 6.0 && dy < 0.38 * h_f {
                    let spacing = 8.0;
                    let row = ((dy - 6.0) / spacing) as i32;
                    let peg_radius_sq = 1.8 * 1.8;
                    let row_y = 6.0 + row as f32 * spacing;
                    if (dy - row_y).abs() < 2.0 {
                        let count = row + 3;
                        let offset_x = if row % 2 == 1 { spacing * 0.5 } else { 0.0 };
                        let start_x = - (count as f32 - 1.0) * spacing * 0.5 + offset_x;
                        for i in 0..count {
                            let peg_x = start_x + i as f32 * spacing;
                            let pdx = dx - peg_x;
                            let pdy = dy - row_y;
                            if pdx * pdx + pdy * pdy < peg_radius_sq {
                                return (false, false);
                            }
                        }
                    }
                }
                let is_safe = dx.abs() < (allowed_hw - 1.5).max(1.0) && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::StaircaseCascade => {
            let max_hw = 0.42 * w_f;
            let max_hh = 0.42 * h_f;
            if dx.abs() >= max_hw || dy.abs() >= max_hh {
                return (false, false);
            }

            // Procedurally-varied alternating sloped stair shelves: more steps than the
            // original fixed 4, each with a slightly randomized slope (deterministic per
            // step index) plus a randomized gap ("hole") sand can filter straight through,
            // in addition to the usual open side at the end of each shelf.
            let step_count: i32 = 8;
            let attach_limit = 0.20 * w_f;
            let y_start = -0.36 * h_f;
            let step_spacing = 0.72 * h_f / (step_count as f32 - 1.0);

            for k in 0..step_count {
                let y_k = y_start + k as f32 * step_spacing;
                let slope_mag = 0.10 + 0.10 * step_hash(k as u32 * 3);
                let slope = if k % 2 == 0 { slope_mag } else { -slope_mag };
                let y_shelf = y_k + dx * slope;
                if (dy - y_shelf).abs() < 3.5 {
                    let is_left_attached = k % 2 == 0;

                    let (span_lo, span_hi) = if is_left_attached {
                        (-max_hw + 4.0, attach_limit - 4.0)
                    } else {
                        (-attach_limit + 4.0, max_hw - 4.0)
                    };
                    let hole_center = span_lo + step_hash(k as u32 * 3 + 1) * (span_hi - span_lo);
                    let hole_width = (0.05 + 0.05 * step_hash(k as u32 * 3 + 2)) * w_f;
                    let in_hole = (dx - hole_center).abs() < hole_width * 0.5;

                    if is_left_attached && dx < attach_limit && !in_hole {
                        return (false, false);
                    } else if !is_left_attached && dx > -attach_limit && !in_hole {
                        return (false, false);
                    }
                }
            }

            let is_safe = dx.abs() < (max_hw - 2.0) && dy.abs() < (max_hh - 2.0);
            (true, is_safe)
        }
        crate::SandboxShape::ProceduralFunnel => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                if dx.abs() >= allowed_hw {
                    return (false, false);
                }
                if dy > -0.32 * h_f && dy < 0.32 * h_f {
                    // Higher-frequency, 4-octave noise than before packs in more, smaller
                    // stalactite/stalagmite obstacles instead of a few large blobby ones.
                    let cave_val = (
                        (dx * 0.14).sin()
                        + (dy * 0.16).cos()
                        + (dx * 0.05 + dy * 0.07).sin()
                        + (dx * 0.24 - dy * 0.21).cos()
                    ).abs();
                    if cave_val > 1.35 && dx.abs() > 6.0 {
                        return (false, false);
                    }
                }
                let is_safe = dx.abs() < (allowed_hw - 1.5).max(1.0) && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::MultiNeckHourglass => {
            // Two genuinely separate necks, spread wide apart, rather than one center
            // opening barely split by a small barrier. Each neck is its own mini funnel
            // (same taper shape as the classic Hourglass); their wide tops overlap near
            // the chamber walls to form a single continuous top/bottom chamber, and they
            // pull apart into two distinct openings approaching the pinch line, forming a
            // "W" (draining) / "M" (refilling) silhouette.
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.30 * w_f;
            let neck_offset = 0.15 * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let neck_hw = neck_width * w_f;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                let dist_left = (dx + neck_offset).abs();
                let dist_right = (dx - neck_offset).abs();
                if dist_left >= allowed_hw && dist_right >= allowed_hw {
                    return (false, false);
                }
                let safe_hw = (allowed_hw - 1.5).max(1.0);
                let is_safe = (dist_left < safe_hw || dist_right < safe_hw) && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
    }
}

/// Mark a neighbor block as modified (needing redraw/copy-back this frame) and bump its
/// next-frame displacement estimate, without touching the buffer belonging to the block
/// currently being simulated (which would corrupt a block that hasn't run yet this frame).
fn activate_neighbor(neighbor_b: usize, flow: f32, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>) {
    modified[neighbor_b] = true;
    if next_displacements[neighbor_b] < flow {
        next_displacements[neighbor_b] = flow;
    }
}

/// Apply a mass transfer of `flow` from `center_idx` to `neighbor_idx`: activates both the
/// source and destination blocks, advects color and material properties, updates
/// `temp_heights`, and accumulates the per-tick bookkeeping (`total_flow`, `cell_flowed`,
/// `flow_occurred`). This is the body shared by the two flow sites in the granular CA path
/// (the avalanche-collapse safety check and the main slope-driven flow below it) — they differ
/// only in the guard condition that decides whether to call this at all, so the guard stays at
/// each call site rather than being folded in here.
#[allow(clippy::too_many_arguments)]
fn try_move(
    b: usize,
    center_idx: usize,
    neighbor_idx: usize,
    flow: f32,
    w: usize,
    block_size: usize,
    cols: usize,
    temp_heights: &mut [f32],
    cell_colors: &mut [f32],
    cell_props: &mut [f32],
    modified: &mut Vec<bool>,
    next_displacements: &mut Vec<f32>,
    total_flow: &mut f32,
    cell_flowed: &mut bool,
    flow_occurred: &mut bool,
) {
    let nx = neighbor_idx % w;
    let ny = neighbor_idx / w;
    let neighbor_b = (ny / block_size) * cols + (nx / block_size);

    activate_neighbor(b, flow, modified, next_displacements);
    activate_neighbor(neighbor_b, flow, modified, next_displacements);

    advect_properties(cell_colors, cell_props, center_idx, neighbor_idx, flow, temp_heights[neighbor_idx]);
    temp_heights[center_idx] -= flow;
    temp_heights[neighbor_idx] += flow;
    *total_flow += flow;
    *cell_flowed = true;
    *flow_occurred = true;
}

/// Perform a single gravity flow/settling iteration inside the active bounding box.
pub fn settle_tick(
    heightmap: &mut Heightmap,
    temp_heights: &mut Vec<f32>,
    cell_colors: &mut Vec<f32>,
    cell_props: &mut Vec<f32>,
    sliding: &mut Vec<bool>,
    active_bounds: &mut ActiveBounds,
    active_blocks: &mut Vec<crate::BlockActivity>,
    last_displacements: &mut Vec<f32>,
    last_simulated_ticks: &mut Vec<u32>,
    budget_n: usize,
    block_size: usize,
    active_marbles: &[ActiveMarbleInfo],
    time_seed: u32,
    edge_vel_h: &mut Vec<f32>,
    edge_vel_v: &mut Vec<f32>,
    shape_mask: &[u8],
    tick_count: u32,
    gravity_dir: glam::Vec2,
) -> f32 {
    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return 0.0;
    }

    // Safety checks to prevent panics if heights or sliding buffer are resized
    if temp_heights.len() != heightmap.data.len() {
        temp_heights.resize(heightmap.data.len(), crate::DEFAULT_SAND_HEIGHT);
    }
    if sliding.len() != heightmap.data.len() {
        sliding.resize(heightmap.data.len(), false);
    }
    // Per-edge momentum. `edge_vel_h[i]` belongs to the horizontal edge between cell `i` and
    // cell `i + 1`; `edge_vel_v[i]` to the vertical edge between cell `i` and cell `i + w`. Each
    // edge is owned (and therefore integrated exactly once per pass) by its lower-index cell.
    if edge_vel_h.len() != heightmap.data.len() {
        edge_vel_h.resize(heightmap.data.len(), 0.0);
    }
    if edge_vel_v.len() != heightmap.data.len() {
        edge_vel_v.resize(heightmap.data.len(), 0.0);
    }

    let cols = (w + block_size - 1) / block_size;
    let rows = (h + block_size - 1) / block_size;
    let expected_len = cols * rows;

    if last_displacements.len() != expected_len {
        last_displacements.resize(expected_len, 0.0);
    }
    if last_simulated_ticks.len() != expected_len {
        last_simulated_ticks.resize(expected_len, 0);
    }
    if active_blocks.len() != expected_len {
        active_blocks.resize(expected_len, crate::BlockActivity::Inactive);
    }

    let gravity_active = gravity_dir.length_squared() > 1e-6;

    // Constants from the design doc
    const MUST_SIMULATE_THRESHOLD: f32 = 0.1;
    const MAX_STALENESS: u32 = 30;
    const FLOW_INACTIVE_THRESHOLD: f32 = 3e-4;

    // 1. Identify MUST, STALE, and REST blocks, and calculate priorities
    let mut must_simulate = Vec::new();
    let mut stale_simulate = Vec::new();
    let mut rest_candidates = Vec::new();

    let active_threshold = if gravity_active { 1e-4 } else { MUST_SIMULATE_THRESHOLD };
    for b in 0..expected_len {
        let displacement = last_displacements[b];
        let staleness = tick_count.saturating_sub(last_simulated_ticks[b]).min(MAX_STALENESS);

        if displacement >= active_threshold {
            must_simulate.push(b);
        } else if staleness >= MAX_STALENESS {
            stale_simulate.push(b);
        } else if displacement > 0.0 {
            // Priority function: staleness * displacement
            let priority = (staleness as f32) * displacement;
            rest_candidates.push((b, priority));
        }
    }

    // Quick exit check if no blocks are active
    if must_simulate.is_empty() && stale_simulate.is_empty() && rest_candidates.is_empty() {
        active_bounds.active = false;
        active_blocks.fill(crate::BlockActivity::Inactive);
        return 0.0;
    }

    let total_always = must_simulate.len() + stale_simulate.len();
    let remaining_budget = if budget_n > total_always {
        budget_n - total_always
    } else {
        0
    };

    let mut budget_simulate = Vec::new();
    if remaining_budget > 0 && !rest_candidates.is_empty() {
        let n = remaining_budget.min(rest_candidates.len());
        rest_candidates.select_nth_unstable_by(n - 1, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 0..n {
            budget_simulate.push(rest_candidates[i].0);
        }
    }

    let mut will_simulate = vec![false; expected_len];
    for &b in &must_simulate {
        will_simulate[b] = true;
    }
    for &b in &stale_simulate {
        will_simulate[b] = true;
    }
    for &b in &budget_simulate {
        will_simulate[b] = true;
    }

    // Update active_blocks for HUD statistics
    active_blocks.fill(crate::BlockActivity::Inactive);
    for &b in &must_simulate {
        active_blocks[b] = crate::BlockActivity::Fast;
    }
    for &b in &stale_simulate {
        active_blocks[b] = crate::BlockActivity::Slow;
    }
    for &b in &budget_simulate {
        active_blocks[b] = crate::BlockActivity::Medium;
    }

    // Use precomputed shape mask instead of per-frame eval_sandbox_shape
    // shape_mask values: 0 = OUTSIDE (wall), 1 = INSIDE (safe), 2 = BOUNDARY (inside, near wall)
    let is_inside = |cx: usize, cy: usize| -> bool {
        shape_mask[cy * w + cx] != crate::MASK_OUTSIDE
    };

    let mut modified = will_simulate.clone();

    // 1. Copy heightmap to working buffer at start of frame
    temp_heights.copy_from_slice(&heightmap.data);

    let mut total_flow = 0.0f32;
    let mut next_displacements = vec![0.0f32; expected_len];
    let mut flow_occurred = false;

    // 2. Continuous per-cell solver (loop over active blocks)
    let gravity_active = gravity_dir.length_squared() > 1e-6;
    let b_len = expected_len;
    // Directional operator split for liquid under gravity.
    //
    //   phase 0 — the liquid solver's gravity-aligned edges only, scanned *against* gravity.
    //   phase 1 — everything else: the granular CA, the Sandbox (g = 0) liquid solver, and the
    //             liquid solver's cross-gravity edges.
    //
    // Both halves of the split are ordering choices, not tuned coefficients, and each fixes a
    // distinct failure of the naive fused pass:
    //
    // *Why the directions are separated.* A cell in free fall and a cell in a settled pool
    // present the same fill difference to their lateral neighbours — a full cell beside an empty
    // one — so a fused pass cannot tell "falling" from "resting" and spreads both, fanning a
    // 4-cell stream out to 33. What actually distinguishes them is that the falling cell has
    // somewhere to go *along* gravity and the pooled cell does not. Resolving the gravity-aligned
    // edges first makes that physical: by the time lateral edges are evaluated, a falling cell has
    // already handed its mass to the cell below and has nothing left to give sideways, while a
    // pooled cell still holds all of it and levels out. That is the hydrostatic statement "no
    // lateral pressure without support", obtained from the update order instead of from a
    // free-fall special case.
    //
    // *Why phase 0 runs against gravity.* Sweeping down-gravity is Gauss-Seidel in the flow
    // direction: row y donates into row y+1, then row y+1 — already topped up — donates into
    // y+2, so one pass cascades a parcel the whole height of the grid and the stream arrives as
    // a stretched 0.10-fill smear. Sweeping bottom-to-top empties the acceptor before the donor
    // is considered, which is the CFL-respecting order: mass advances at most one cell per tick
    // and a saturated stream stays saturated (peak fill 1.0).
    for phase in 0..2usize {
        // phase 0 only exists for in-plane gravity; at g = 0 there is no gravity-aligned
        // direction and the Sandbox liquid solver handles both of its edges in phase 1.
        if phase == 0 && !gravity_active {
            continue;
        }
        // True when phase 0 should walk rows bottom-to-top (the usual case: gravity points at
        // +y, i.e. down the grid).
        let against_gravity_is_up = gravity_dir.y >= 0.0;
    for idx_b in 0..b_len {
        let b = if phase == 0 {
            // Reverse of the main block order along the gravity axis.
            let by_fwd = idx_b / cols;
            let by = if against_gravity_is_up { rows - 1 - by_fwd } else { by_fwd };
            let bx_idx = idx_b % cols;
            let bx = if (tick_count + by as u32) % 2 == 0 {
                bx_idx
            } else {
                cols - 1 - bx_idx
            };
            by * cols + bx
        } else if gravity_active && gravity_dir.y > 0.0 {
            // Under downward gravity, process blocks top-to-bottom so falling sand advects across block boundaries without trapping
            let by = idx_b / cols;
            let bx_idx = idx_b % cols;
            let bx = if (tick_count + by as u32) % 2 == 0 {
                bx_idx
            } else {
                cols - 1 - bx_idx
            };
            by * cols + bx
        } else if tick_count % 2 == 0 {
            idx_b
        } else {
            b_len - 1 - idx_b
        };
        if !will_simulate[b] {
            continue;
        }

        let bx = b % cols;
        let by = b / cols;
        let start_x = bx * block_size;
        let end_x = ((bx + 1) * block_size).min(w);
        let start_y = by * block_size;
        let end_y = ((by + 1) * block_size).min(h);

        let x_len = end_x - start_x;
        let y_len = end_y - start_y;
        for idy in 0..y_len {
            let y = if phase == 0 {
                if against_gravity_is_up { end_y - 1 - idy } else { start_y + idy }
            } else if gravity_active && gravity_dir.y > 0.0 {
                start_y + idy
            } else if tick_count % 2 == 0 {
                end_y - 1 - idy
            } else {
                start_y + idy
            };
            let row_offset = y * w;
            for idx in 0..x_len {
                let x = if gravity_active {
                    if (tick_count + y as u32) % 2 == 0 {
                        start_x + idx
                    } else {
                        end_x - 1 - idx
                    }
                } else if tick_count % 2 == 0 {
                    start_x + idx
                } else {
                    end_x - 1 - idx
                };
                let center_idx = row_offset + x;

                let mask_val = shape_mask[center_idx];
                let inside = mask_val != crate::MASK_OUTSIDE;

                if !inside {
                    continue;
                }

                let wetness = cell_props[center_idx * 4 + PROP_WETNESS];

                if phase == 0 {
                    // Gravity-aligned liquid pass — see the operator-split note above.
                    // `wetness <= 0.65` is the cheap early-out that keeps a pure-sand frame from
                    // paying for this pass: it is exactly where `liquidity` reaches zero.
                    if wetness <= 0.65 {
                        continue;
                    }
                    let cell_liquidity = liquidity(wetness);
                    if cell_liquidity > 0.0
                        && x > 0 && x + 1 < w && y > 0 && y + 1 < h
                        && is_inside(x, y + 1)
                    {
                        let (c_sq, damping) = wave_params(wetness);
                        let nb_idx = center_idx + w;
                        let nb_b = ((y + 1) / block_size) * cols + bx;
                        flux_edge(
                            b, nb_b, center_idx, nb_idx,
                            temp_heights[center_idx] + gravity_dir.y * GRAVITY_HEAD_SCALE,
                            temp_heights[nb_idx],
                            c_sq, damping, 0.0,
                            cell_capacity_for(wetness),
                            cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]),
                            temp_heights[center_idx], temp_heights[nb_idx],
                            cell_liquidity,
                            &mut edge_vel_v[center_idx],
                            temp_heights, cell_colors, cell_props,
                            &mut modified, &mut next_displacements,
                            &mut total_flow, &mut flow_occurred,
                        );
                    }
                    continue;
                }

                if wetness >= 0.75 && !gravity_active {
                    // --- Conservative edge-flux liquid solver (replaces the per-cell wave
                    //     update; see `flux_edge`) ---
                    //
                    // Each cell integrates only the two edges it *owns* — the one to its right
                    // and the one below it — so every edge in the domain is integrated exactly
                    // once per pass, by its lower-index endpoint. The left/top edges of this cell
                    // are owned by its left/top neighbours and were (or will be) handled there.
                    //
                    // Neumann reflection at the shape boundary is now structural: an edge whose
                    // far side is outside the mask simply does not exist, so no flux crosses it
                    // and no mass can be stranded in a wall cell (the old formulation mirrored
                    // `h_center` across the wall to get a zero-gradient; skipping the edge is the
                    // same boundary condition expressed on the flux instead of the height).
                    //
                    // *Jacobi driving.* The head difference that integrates an edge's velocity is
                    // read from `heightmap.data`, the tick's frozen starting heights, never from
                    // `temp_heights` mid-sweep. `temp_heights` is mutated four times per cell per
                    // pass (once by each incident edge) and `heightmap.data` is not written until
                    // the copy-back in step 3, so it is a stable snapshot at zero cost — the same
                    // one the Sandbox granular CA below already reads.
                    //
                    // This is not a style preference, it is the stability condition. Driving the
                    // velocities from the live buffer makes the update Gauss-Seidel with a
                    // direction-alternating sweep, and Gauss-Seidel on a wave equation is not
                    // merely less accurate — it is a *gain*. Linearising the 1-D chain at Water's
                    // (c_sq, damping) = (0.24, 0.98) gives a per-tick spectral radius of 1.20 for
                    // the swept form against 0.994 for this one: the sweep injected ~20% of
                    // amplitude per tick while the damping removed 2%, so a ripple grew until it
                    // hit the cell cap and stuck there (peak 0.80 -> pinned at 1.0000 in under 50
                    // ticks) instead of decaying back to a flat pool. Raising the cap did not
                    // help; it only moved the ceiling and made the directional bias visible as
                    // 33x worse left/right asymmetry.
                    //
                    // The clamps below stay live on purpose. Only the *dynamics* need the
                    // snapshot; the donor-mass and acceptor-capacity limits inside `flux_edge`
                    // are a safety limiter and must see what the other three edges have already
                    // taken this pass, or a cell could be drained twice over. Because every edge
                    // still debits exactly what it credits, Jacobi ordering costs nothing in
                    // conservation — that property is structural in the flux form, not a
                    // consequence of the sweep order.
                    //
                    // Gravity-driven liquid deliberately does *not* get this treatment: under
                    // gravity the solver is doing advection down a hydrostatic head, where the
                    // ordering (gravity-aligned edges first, swept against gravity) is load
                    // bearing for CFL. Gauss-Seidel is only wrong for the conservative,
                    // energy-carrying case, which is exactly this g = 0 branch.
                    let (c_sq, damping) = wave_params(wetness);
                    let cap_c = cell_capacity_for(wetness);
                    let mut max_flux = 0.0f32;
                    let head_c = heightmap.data[center_idx];

                    if x + 1 < w && is_inside(x + 1, y) {
                        let nb_idx = center_idx + 1;
                        let nb_b = by * cols + (x + 1) / block_size;
                        let f = flux_edge(
                            b, nb_b, center_idx, nb_idx,
                            head_c, heightmap.data[nb_idx],
                            c_sq, damping, 0.0,
                            cap_c, cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]),
                            temp_heights[center_idx], temp_heights[nb_idx], 1.0,
                            &mut edge_vel_h[center_idx],
                            temp_heights, cell_colors, cell_props,
                            &mut modified, &mut next_displacements,
                            &mut total_flow, &mut flow_occurred,
                        );
                        max_flux = max_flux.max(f.abs());
                    }

                    if y + 1 < h && is_inside(x, y + 1) {
                        let nb_idx = center_idx + w;
                        let nb_b = ((y + 1) / block_size) * cols + bx;
                        let f = flux_edge(
                            b, nb_b, center_idx, nb_idx,
                            head_c, heightmap.data[nb_idx],
                            c_sq, damping, 0.0,
                            cap_c, cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]),
                            temp_heights[center_idx], temp_heights[nb_idx], 1.0,
                            &mut edge_vel_v[center_idx],
                            temp_heights, cell_colors, cell_props,
                            &mut modified, &mut next_displacements,
                            &mut total_flow, &mut flow_occurred,
                        );
                        max_flux = max_flux.max(f.abs());
                    }

                    // Block-activation bookkeeping, unchanged in spirit from the old wave path:
                    // `max_flux` plays the role the per-cell velocity used to, and a cell whose
                    // height still differs from the resting bed keeps its neighbourhood awake so
                    // a travelling wave is not cut off at a block boundary.
                    let disturbance = (temp_heights[center_idx] - crate::DEFAULT_SAND_HEIGHT).abs();
                    if max_flux > 3e-4 || disturbance > 1e-4 {
                        flow_occurred = true;
                        let flow_val = max_flux.max(disturbance);
                        activate_neighbor(b, flow_val, &mut modified, &mut next_displacements);
                        if bx > 0 { activate_neighbor(b - 1, flow_val, &mut modified, &mut next_displacements); }
                        if bx + 1 < cols { activate_neighbor(b + 1, flow_val, &mut modified, &mut next_displacements); }
                        if by > 0 { activate_neighbor(b - cols, flow_val, &mut modified, &mut next_displacements); }
                        if by + 1 < rows { activate_neighbor(b + cols, flow_val, &mut modified, &mut next_displacements); }
                    }
                } else {
                    // --- Cellular Automata (Sand settling behavior) ---
                    // CA requires accessing neighbors at offset 1, so we must be inside the grid boundaries
                    if x == 0 || x + 1 >= w || y == 0 || y + 1 >= h {
                        sliding[center_idx] = false;
                        continue;
                    }

                    // Continuous liquid weight for this cell (see `liquidity` doc comment).
                    // Computed once per center cell and reused by both the avalanche safety
                    // valve below and the main neighbor flow loop further down, so the acceptor
                    // capacity (C1, incompressibility) is enforced consistently everywhere a
                    // neighbor can receive mass in a single tick.
                    let cell_liquidity = liquidity(wetness);
                    let cell_capacity = 1.5 * (1.0 - cell_liquidity) + 1.0 * cell_liquidity;
                    // Complement of the liquid share handled by the edge-flux solver below.
                    // Exactly 1.0 for any granular material (liquidity == 0), so the CA path is
                    // bit-identical to before for sand.
                    let granular_share = if gravity_active { 1.0 - cell_liquidity } else { 1.0 };

                    // --- Liquid share: the same conservative edge-flux solver as the g = 0
                    //     branch above, but with a non-zero gravitational head Phi ---
                    //
                    // `H = h + Phi(g, r)` is the unified head. In Sandbox the grid plane is
                    // horizontal, gravity is perpendicular to it and Phi is identically zero, so
                    // `H = h` and the solver degenerates to the free-surface wave. In Sand-fall
                    // the grid is a vertical cross-section and gravity is in-plane, so Phi is a
                    // linear ramp along `g` and the head difference across a downhill edge picks
                    // up `|g| * GRAVITY_HEAD_SCALE` on top of the fill difference. Nothing else
                    // about the update changes: the same clamp on donor mass and acceptor
                    // capacity that makes ripples conservative at g = 0 is what produces
                    // hydrostatic stacking and level pools at g > 0.
                    //
                    // Weighted by `cell_liquidity`, with the granular CA below carrying the
                    // complementary `1 - cell_liquidity`, so the handover across the old
                    // `wetness >= 0.75` cut is continuous (C5) and a pure granular cell
                    // (liquidity == 0) is bit-identical to before.
                    if gravity_active && cell_liquidity > 0.0 && x + 1 < w && is_inside(x + 1, y) {
                        let (c_sq, damping) = wave_params(wetness);
                        let nb_idx = center_idx + 1;
                        let nb_b = by * cols + (x + 1) / block_size;
                        // Mass that arrived from upstream during phase 0 is still falling; it is
                        // unsupported and cannot push sideways (see `flux_edge`'s `avail_*`).
                        // `edge_vel_v[i - w]` is exactly the flux phase 0 realised on the
                        // gravity-aligned edge feeding cell `i`, and `edge_vel_v[i]` the flux it
                        // realised on the edge draining `i`.
                        //
                        // The inflow alone is the right limit for a free-falling parcel and the
                        // wrong one for a supported parcel, and it used to be subtracted
                        // unconditionally. A cell standing on a full column — or on the container
                        // floor, or on casing — bears the hydrostatic head of everything in it and
                        // must spread sideways at the normal rate however hard it is being fed
                        // from above. Subtracting the inflow there re-suppressed the motion the
                        // phase ordering already suppresses, and did so *permanently* under any
                        // continuous feed: a cell under a running pour receives from above on
                        // every single tick, so `avail_*` never recovered and lateral flow was
                        // dead at every depth of the pour rather than only in its falling part.
                        //
                        // The limit is therefore not the inflow but the amount of that inflow
                        // that can actually keep going down:
                        //
                        //     in_transit = min(inflow, outflow + room_below)
                        //
                        // `outflow` is what already left through the bottom this tick and
                        // `room_below` is the free space still under the cell, so the second term
                        // is everything the cell has any downstream route for. Inflow beyond it
                        // landed on a column that cannot take it any further: it is at rest, and
                        // it presses sideways like any other resting mass.
                        //
                        // The tempting simpler test — "is the cell below full?" — does not work
                        // here, and it is worth recording why. A *saturated* falling stream passes
                        // it at every interior cell: phase 0 sweeps bottom-to-top, so each stream
                        // cell hands `f` downward and is refilled by `f` from above, leaving every
                        // cell (hence every cell's below-neighbour) back at capacity by the time
                        // phase 1 reads it. By height alone a saturated stream is
                        // indistinguishable from a standing column; gating on height alone fanned
                        // the stream from 8 cells wide to 16. The `outflow` term is what separates
                        // them: the stream moved its whole content down, the pooled cell moved
                        // nothing. And `room_below` is what keeps the *front* of a stream falling
                        // — its edge momentum has not spun up, so it moves little downward on the
                        // tick it appears, but the empty space beneath it is a route all the same.
                        //
                        // Every case in free fall reproduces the old value exactly (in the stream
                        // interior `outflow = inflow` and `room_below = 0`; at the front
                        // `room_below` is a whole cell), so this is a strict relaxation confined
                        // to genuinely supported liquid. Reachable only when `cell_liquidity >
                        // 0.0`, so granular cells are untouched by construction.
                        let in_transit = |c: usize| -> f32 {
                            let cx = c % w;
                            let cy = c / w;
                            // No edge below (off-grid, or the cell below is casing): the cell is
                            // resting on the container, so there is no downstream route at all.
                            // `edge_vel_v[c]` is stale in that case — phase 0 skips exactly these
                            // edges, and its guard is mirrored here — so it must not be read.
                            if !(cx > 0 && cx + 1 < w && cy > 0 && cy + 1 < h && is_inside(cx, cy + 1))
                            {
                                return 0.0;
                            }
                            let below = c + w;
                            let h_below = temp_heights[below].max(heightmap.data[below]);
                            let cap_below = cell_capacity_for(cell_props[below * 4 + PROP_WETNESS]);
                            let downstream_route =
                                edge_vel_v[c].max(0.0) + (cap_below - h_below).max(0.0);
                            edge_vel_v[c - w].max(0.0).min(downstream_route)
                        };
                        let avail_a = (temp_heights[center_idx] - in_transit(center_idx)).max(0.0);
                        let avail_b = (temp_heights[nb_idx] - in_transit(nb_idx)).max(0.0);
                        flux_edge(
                            b, nb_b, center_idx, nb_idx,
                            temp_heights[center_idx] + gravity_dir.x * GRAVITY_HEAD_SCALE,
                            temp_heights[nb_idx],
                            c_sq, damping, 0.0,
                            cell_capacity, cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]),
                            avail_a, avail_b,
                            cell_liquidity,
                            &mut edge_vel_h[center_idx],
                            temp_heights, cell_colors, cell_props,
                            &mut modified, &mut next_displacements,
                            &mut total_flow, &mut flow_occurred,
                        );
                    }

                    let h_center = if gravity_active {
                        temp_heights[center_idx].max(heightmap.data[center_idx])
                    } else {
                        heightmap.data[center_idx]
                    };

                    // Load neighbor heights and find minimum
                    let h_left = if gravity_active { temp_heights[center_idx - 1].max(heightmap.data[center_idx - 1]) } else { heightmap.data[center_idx - 1] };
                    let h_right = if gravity_active { temp_heights[center_idx + 1].max(heightmap.data[center_idx + 1]) } else { heightmap.data[center_idx + 1] };
                    let h_top = if gravity_active { temp_heights[center_idx - w].max(heightmap.data[center_idx - w]) } else { heightmap.data[center_idx - w] };
                    let h_bottom = if gravity_active { temp_heights[center_idx + w].max(heightmap.data[center_idx + w]) } else { heightmap.data[center_idx + w] };

                    let min_h = h_left.min(h_right).min(h_top).min(h_bottom);

                    let threshold_prop = cell_props[center_idx * 4 + PROP_THRESHOLD];
                    let flow_rate_prop = cell_props[center_idx * 4 + PROP_FLOW_RATE];
                    let grain_size = cell_props[center_idx * 4 + PROP_GRAIN_SIZE];

                    let threshold_min = if wetness < 0.15 {
                        0.5 * threshold_prop
                    } else {
                        threshold_prop
                    };

                    // Fast-path shortcut (disabled when gravity is active to allow flow on flat beds)
                    if gravity_dir.length_squared() < 1e-6 && h_center - min_h <= threshold_min {
                        sliding[center_idx] = false;
                        continue;
                    }

                    let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);
                    
                    let neighbors_info = if gravity_active && gravity_dir.y > 0.0 {
                        if (tick_count + x as u32 + y as u32) % 2 == 0 {
                            [
                                (center_idx + w, 0.0, 1.0),  // Bottom (Gravity first)
                                (center_idx - 1, -1.0, 0.0), // Left
                                (center_idx + 1, 1.0, 0.0),  // Right
                                (center_idx - w, 0.0, -1.0), // Top
                            ]
                        } else {
                            [
                                (center_idx + w, 0.0, 1.0),  // Bottom (Gravity first)
                                (center_idx + 1, 1.0, 0.0),  // Right
                                (center_idx - 1, -1.0, 0.0), // Left
                                (center_idx - w, 0.0, -1.0), // Top
                            ]
                        }
                    } else if (tick_count + x as u32 + y as u32) % 2 == 0 {
                        [
                            (center_idx - 1, -1.0, 0.0), // Left
                            (center_idx + 1, 1.0, 0.0),  // Right
                            (center_idx - w, 0.0, -1.0), // Top
                            (center_idx + w, 0.0, 1.0),  // Bottom
                        ]
                    } else {
                        [
                            (center_idx + 1, 1.0, 0.0),  // Right
                            (center_idx - 1, -1.0, 0.0), // Left
                            (center_idx - w, 0.0, -1.0), // Top
                            (center_idx + w, 0.0, 1.0),  // Bottom
                        ]
                    };

                    let mut cell_flowed = false;

                    // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
                    let mut avalanche_checked = false;
                    for &(neighbor_idx, ndx, ndy) in &neighbors_info {
                        let gravity_dot = ndx * gravity_dir.x + ndy * gravity_dir.y;
                        if gravity_active && gravity_dot < -0.01 {
                            continue;
                        }

                        let h_neighbor = if gravity_active { temp_heights[neighbor_idx].max(heightmap.data[neighbor_idx]) } else { heightmap.data[neighbor_idx] };
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope > 0.20 {
                            let mut flow = (0.10 * (geom_slope - 0.20)).max(0.0);

                            // Never transfer mass into a neighbour outside the shape mask, for any
                            // material. Such a cell is skipped by `if !inside { continue }` at the
                            // top of this loop and is never simulated again, so anything landing
                            // there is a silent, permanent leak: total mass is still conserved (so
                            // the leak is invisible to the mass-conservation tests) but the sand or
                            // liquid is frozen inside a wall forever. The renderer draws
                            // MASK_OUTSIDE as opaque casing, which hides it visually too.
                            if !is_inside(neighbor_idx % w, neighbor_idx / w) {
                                flow = 0.0;
                            }

                            if flow > 0.0 {
                                let current_temp_center = temp_heights[center_idx];
                                let current_temp_neighbor = temp_heights[neighbor_idx];
                                let temp_diff = current_temp_center - current_temp_neighbor;
                                // Same acceptor capacity as the main flow loop below (C1): this
                                // avalanche safety valve bypasses the normal threshold/alpha flow
                                // computation entirely, so without this clamp it could push a
                                // liquid neighbor above the incompressibility cap on its own.
                                let max_dst_room = (cell_capacity - current_temp_neighbor).max(0.0);
                                let clamped_flow = flow.min(temp_diff * 0.4).min(max_dst_room).max(0.0)
                                    * granular_share;
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    try_move(
                                        b, center_idx, neighbor_idx, clamped_flow, w, block_size, cols,
                                        temp_heights, cell_colors, cell_props,
                                        &mut modified, &mut next_displacements,
                                        &mut total_flow, &mut cell_flowed, &mut flow_occurred,
                                    );
                                }
                            }
                            avalanche_checked = true;
                        }
                    }
                    if !gravity_active && avalanche_checked {
                        sliding[center_idx] = cell_flowed;
                        continue;
                    }

                    // Cell-invariant properties
                    let mut higher_neighbors = 0;
                    for &(n_idx, _, _) in &neighbors_info {
                        let h_n = if gravity_active { temp_heights[n_idx].max(heightmap.data[n_idx]) } else { heightmap.data[n_idx] };
                        if h_n >= h_center - 1e-4 {
                            higher_neighbors += 1;
                        }
                    }

                    let mut closest_marble_idx = None;
                    let mut min_dist_to_marble = f32::MAX;
                    if !active_marbles.is_empty() {
                        let cell_x = (x as f32 / w as f32) * 2.0 - 1.0;
                        let cell_y = 1.0 - (y as f32 / h as f32) * 2.0;
                        let cell_pos = Vec2::new(cell_x, cell_y);

                        for (idx, m) in active_marbles.iter().enumerate() {
                            let dist = (cell_pos - m.pos).length();
                            if dist < min_dist_to_marble {
                                min_dist_to_marble = dist;
                                closest_marble_idx = Some(idx);
                            }
                        }
                    }

                    let closest_marble_vel = if let Some(idx) = closest_marble_idx {
                        active_marbles[idx].vel
                    } else {
                        0.0
                    };

                    let (threshold, alpha, lock_chance, quantize_size) = get_ca_params(
                        wetness,
                        threshold_prop,
                        flow_rate_prop,
                        grain_size,
                        higher_neighbors,
                        sliding[center_idx],
                        closest_marble_vel,
                        gravity_active,
                    );

                    // `cell_liquidity` / `cell_capacity` computed above (right after the CA
                    // branch was entered); reused below to blend the gravity_push multiplier,
                    // the transfer coefficient, and the acceptor cell capacity.
                    for &(neighbor_idx, ndx, ndy) in &neighbors_info {
                        let h_neighbor = if gravity_active { temp_heights[neighbor_idx].max(heightmap.data[neighbor_idx]) } else { heightmap.data[neighbor_idx] };
                        let geom_slope = h_center - h_neighbor;
                        let gravity_dot = ndx * gravity_dir.x + ndy * gravity_dir.y;
                        
                        // Under gravity, sand cannot flow upwards against gravity
                        if gravity_active && gravity_dot < -0.01 {
                            continue;
                        }

                        let h_below = if center_idx + w < temp_heights.len() {
                            temp_heights[center_idx + w].max(heightmap.data[center_idx + w])
                        } else {
                            0.0
                        };
                        let is_below_inside = y + 1 < h && is_inside(x, y + 1);
                        let is_free_fall = gravity_active && is_below_inside && h_below < 0.10;

                        // Downward pull. Phase 5 removed the x40 liquid multiplier that used to
                        // be blended in here: a liquid's downhill drive is now the gravitational
                        // head Phi in the flux solver, not a fictitious slope bonus in the CA.
                        let mut gravity_push = gravity_dot * 4.0;
                        
                        // Sideways lateral term — the granular stochastic dispersion/splashing
                        // that builds the bed heap and scatters sand in free fall. Phase 5
                        // deleted the liquid counterpart that used to be blended in here (a term
                        // that cancelled `geom_slope` while the cell below could still accept
                        // mass): a liquid's lateral motion is now the flux solver's cross-gravity
                        // edge, gated by the in-transit donor limit rather than by a "can it
                        // still fall?" predicate.
                        let gravity_len = gravity_dir.length();
                        if gravity_len > 1e-6 {
                            let perp_x = -gravity_dir.y;
                            let perp_y = gravity_dir.x;
                            let perp_dot = (ndx * perp_x + ndy * perp_y).abs();
                            let rand_val = (seed ^ (neighbor_idx as u32).wrapping_mul(823)) & 0xFF;
                            let dispersion_noise = rand_val as f32 / 255.0;

                            gravity_push += if !is_free_fall {
                                // Lateral avalanche dispersion on bed heap to form a natural tall sand hill
                                perp_dot * 3.5 * dispersion_noise
                            } else {
                                // Always randomly scatter a little laterally in free fall for natural stream flow
                                perp_dot * 0.8 * dispersion_noise
                            };
                        }

                        let effective_slope = geom_slope + gravity_push;

                        if effective_slope <= threshold {
                            continue;
                        }

                        // C. Stochastic locking and sliding condition (bypass locking in free fall)
                        let flow_seed = (seed ^ (neighbor_idx as u32).wrapping_mul(997)) & 0xFFFF;
                        let rand_val = flow_seed as f32 / 65535.0;
                        let effective_lock_chance = if is_free_fall { 0.0 } else { lock_chance };
                        
                        if rand_val >= effective_lock_chance {
                            let alpha_noise = if gravity_active {
                                1.0 + (rand_val - 0.5) * 0.10 // Smooth laminar flow under gravity (+/- 5%)
                            } else {
                                1.0 + (rand_val - 0.5) * 0.80 // Natural stochastic noise in sandbox carving (+/- 40%)
                            };
                            let mut flow = (alpha * (effective_slope - threshold) * alpha_noise).max(0.0);
                            
                            if let Some(q) = quantize_size {
                                flow = (flow / q).round() * q;
                            }

                            if flow > 0.0 {
                                // Phase 5 removed the liquid arm of this coefficient (0.70 while
                                // the column below could still take more, 0.90 otherwise). A
                                // liquid's per-tick transfer is no longer a fraction of the donor
                                // chosen by hand — it is the donor's actual mass and the
                                // acceptor's actual free capacity, in `flux_edge`.
                                let max_transfer_coeff = if !gravity_active {
                                    0.40
                                } else if is_free_fall && gravity_dot > 0.0 {
                                    let rand_ff = ((seed ^ (neighbor_idx as u32).wrapping_mul(1543)) & 0xFFFF) as f32 / 65535.0;
                                    0.80 + 0.20 * rand_ff // Random transfer between 80% and 100% in mid-air free fall
                                } else {
                                    0.20 // Sand uses lower coeff on bed to prevent wave oscillations
                                };
                                // Acceptor cell capacity (incompressibility, C1). `cell_capacity`
                                // (computed once above, per center cell) is 1.5 for granular
                                // materials (unchanged, load-bearing for sand-pile height tests)
                                // and interpolates down to 1.0 for liquids via `cell_liquidity`, so
                                // there is no hard cut. Applied to BOTH branches below (not just the
                                // "push into an equal/higher neighbor" case) because within a single
                                // tick a cell can receive inflow from more than one neighbor; without
                                // a capacity check on the downhill (geom_slope > 0) branch too,
                                // several simultaneous donors could each independently push a liquid
                                // neighbor a little past 1.0 even though none of them individually
                                // looked like overpacking.
                                let max_dst_room = (cell_capacity - temp_heights[neighbor_idx]).max(0.0);

                                let src_h = temp_heights[center_idx];
                                let mut clamped_flow = if geom_slope > 0.0 {
                                    let temp_diff = temp_heights[center_idx] - temp_heights[neighbor_idx];
                                    let flow_capped = if src_h <= 0.003 {
                                        flow.min(temp_diff).max(0.0)
                                    } else {
                                        flow.min(temp_diff * max_transfer_coeff).max(0.0)
                                    };
                                    flow_capped.min(max_dst_room)
                                } else {
                                    let max_src_flow = if src_h <= 0.003 {
                                        src_h
                                    } else {
                                        src_h * max_transfer_coeff
                                    };
                                    flow.min(max_src_flow).min(max_dst_room).max(0.0)
                                };
                                
                                // Clean sweep for tiny residual amounts to prevent Zeno's paradox trapping & floating grains.
                                // Still respects the acceptor capacity (C1): this override previously bypassed
                                // max_dst_room entirely, which let a liquid neighbor already at capacity get pushed
                                // slightly over 1.0 by every tiny-residual neighbor sweeping into it in the same tick.
                                if (clamped_flow <= FLOW_INACTIVE_THRESHOLD || is_free_fall) && src_h > 0.0 && src_h <= 0.010 && flow > 0.0 {
                                    clamped_flow = src_h.min(max_dst_room);
                                }

                                // Mask-leak fix: never let a transfer land in a neighbor outside the
                                // shape mask, for any material. Such a cell is skipped by
                                // `if !inside { continue }` at the top of this loop and is never
                                // simulated again, so anything that reaches it is a silent,
                                // permanent leak that stays frozen there forever. Total mass is
                                // still conserved, so the mass-conservation tests never saw this;
                                // and the renderer draws MASK_OUTSIDE as opaque casing, so it was
                                // invisible on screen too. For liquid it was also what pinned a
                                // "spike" of water against the box wall/floor in
                                // `test_liquid_pool_levels_flat_in_closed_box` (surface_row scans
                                // the whole grid width, including outside-mask columns).
                                if !is_inside(neighbor_idx % w, neighbor_idx / w) {
                                    clamped_flow = 0.0;
                                }

                                clamped_flow *= granular_share;

                                if clamped_flow > FLOW_INACTIVE_THRESHOLD || (src_h <= 0.001 && clamped_flow > 0.0) {
                                    try_move(
                                        b, center_idx, neighbor_idx, clamped_flow, w, block_size, cols,
                                        temp_heights, cell_colors, cell_props,
                                        &mut modified, &mut next_displacements,
                                        &mut total_flow, &mut cell_flowed, &mut flow_occurred,
                                    );
                                }
                            }
                        }
                    }

                    sliding[center_idx] = cell_flowed;
                }
            }
        }
    }
    } // end `for phase` — body left at the original indentation so the operator split reads as a
      // wrapper rather than as a 600-line reformat of the solver.

    // 3. Copy back updated blocks
    for b in 0..expected_len {
        if modified[b] {
            let bx = b % cols;
            let by = b / cols;
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                let offset = y * w;
                heightmap.data[offset + start_x..offset + end_x]
                    .copy_from_slice(&temp_heights[offset + start_x..offset + end_x]);
            }
        }
    }

    // Compute updated active bounds for this frame
    let mut min_bx = cols;
    let mut max_bx = 0;
    let mut min_by = rows;
    let mut max_by = 0;
    let mut any_modified = false;

    for b in 0..expected_len {
        if modified[b] {
            any_modified = true;
            let bx = b % cols;
            let by = b / cols;
            min_bx = min_bx.min(bx);
            max_bx = max_bx.max(bx);
            min_by = min_by.min(by);
            max_by = max_by.max(by);
        }
    }

    if any_modified {
        active_bounds.min_x = min_bx * block_size;
        active_bounds.max_x = ((max_bx + 1) * block_size - 1).min(w - 1);
        active_bounds.min_y = min_by * block_size;
        active_bounds.max_y = ((max_by + 1) * block_size - 1).min(h - 1);
        active_bounds.active = flow_occurred;
    } else {
        active_bounds.active = false;
    }

    for b in 0..expected_len {
        if !will_simulate[b] {
            next_displacements[b] = next_displacements[b].max(last_displacements[b]);
        } else {
            last_simulated_ticks[b] = tick_count;
        }
    }
    *last_displacements = next_displacements;

    total_flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawingSimulation, GRID_SIZE, MaterialMode, SandboxShape};

    fn get_test_props(mode: crate::MaterialMode, size: usize) -> Vec<f32> {
        let (wetness, threshold, flow_rate, grain_size) = match mode {
            crate::MaterialMode::DrySand => (0.00, 0.08, 0.25, 0.45),
            crate::MaterialMode::CoarseSand => (0.00, 0.11, 0.22, 0.80),
            crate::MaterialMode::KineticSand => (0.20, 0.10, 0.15, 0.35),
            crate::MaterialMode::WetSand => (0.45, 0.14, 0.08, 0.40),
            crate::MaterialMode::FinePowder => (0.00, 0.05, 0.30, 0.05),
            crate::MaterialMode::Snow => (0.05, 0.15, 0.20, 0.20),
            crate::MaterialMode::MoonDust => (0.00, 0.20, 0.20, 0.10),
            crate::MaterialMode::Oobleck => (0.55, 0.04, 0.12, 0.15),
            crate::MaterialMode::ButterCream => (0.70, 0.04, 0.15, 0.08),
            crate::MaterialMode::Water => (1.00, 0.00, 0.00, 0.00),
            crate::MaterialMode::CalmWater => (0.90, 0.00, 0.00, 0.00),
            crate::MaterialMode::Milk => (0.95, 0.00, 0.00, 0.00),
            crate::MaterialMode::VegetableOil => (0.85, 0.00, 0.00, 0.00),
            crate::MaterialMode::Yogurt => (0.75, 0.00, 0.00, 0.08),
        };
        let mut props = vec![0.0f32; size * 4];
        for chunk in props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = wetness;
            chunk[PROP_THRESHOLD] = threshold;
            chunk[PROP_FLOW_RATE] = flow_rate;
            chunk[PROP_GRAIN_SIZE] = grain_size;
        }
        props
    }

    /// Generate a shape mask for testing. Uses eval_sandbox_shape to build the mask
    /// with proper INSIDE/BOUNDARY/OUTSIDE classification.
    fn make_test_mask(
        w: usize,
        h: usize,
        shape: SandboxShape,
        neck_width: f32,
        hourglass_curve: f32,
    ) -> Vec<u8> {
        let mut mask = vec![crate::MASK_OUTSIDE; w * h];
        // Pass 1: inside/outside
        for y in 0..h {
            for x in 0..w {
                let (inside, _) = eval_sandbox_shape(x, y, w, h, shape, neck_width, hourglass_curve);
                mask[y * w + x] = if inside { crate::MASK_INSIDE } else { crate::MASK_OUTSIDE };
            }
        }
        // Pass 2: mark boundary cells
        let snapshot = mask.clone();
        for y in 0..h {
            for x in 0..w {
                if snapshot[y * w + x] == crate::MASK_INSIDE {
                    let has_outside =
                        (x == 0 || snapshot[y * w + x - 1] == crate::MASK_OUTSIDE) ||
                        (x + 1 >= w || snapshot[y * w + x + 1] == crate::MASK_OUTSIDE) ||
                        (y == 0 || snapshot[(y - 1) * w + x] == crate::MASK_OUTSIDE) ||
                        (y + 1 >= h || snapshot[(y + 1) * w + x] == crate::MASK_OUTSIDE);
                    if has_outside {
                        mask[y * w + x] = crate::MASK_BOUNDARY;
                    }
                }
            }
        }
        mask
    }

    /// Bundles the many mutable buffers settle_tick needs so the liquid-gravity
    /// characterisation tests below don't have to repeat ~10 lines of boilerplate
    /// allocation each. Used only by the L1-L10 tests added in Phase 0.
    struct TestSim {
        hm: Heightmap,
        temp_heights: Vec<f32>,
        cell_colors: Vec<f32>,
        cell_props: Vec<f32>,
        sliding: Vec<bool>,
        bounds: ActiveBounds,
        active_blocks: Vec<crate::BlockActivity>,
        last_displacements: Vec<f32>,
        last_simulated_ticks: Vec<u32>,
        edge_vel_h: Vec<f32>,
        edge_vel_v: Vec<f32>,
        mask: Vec<u8>,
        block_size: usize,
        tick_count: u32,
    }

    impl TestSim {
        fn new(w: usize, h: usize, props: Vec<f32>, mask: Vec<u8>, block_size: usize) -> Self {
            let cols = (w + block_size - 1) / block_size;
            let rows = (h + block_size - 1) / block_size;
            let expected_len = cols * rows;
            TestSim {
                hm: Heightmap::new(w, h, 0.0),
                temp_heights: vec![0.0; w * h],
                cell_colors: vec![0.0f32; w * h * 4],
                cell_props: props,
                sliding: vec![false; w * h],
                bounds: ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true },
                active_blocks: vec![crate::BlockActivity::Inactive; expected_len],
                last_displacements: vec![1.0; expected_len],
                last_simulated_ticks: vec![0; expected_len],
                edge_vel_h: vec![0.0; w * h],
                edge_vel_v: vec![0.0; w * h],
                mask,
                block_size,
                tick_count: 0,
            }
        }

        fn tick(&mut self, gravity_dir: glam::Vec2, budget_n: usize) -> f32 {
            let flow = settle_tick(
                &mut self.hm,
                &mut self.temp_heights,
                &mut self.cell_colors,
                &mut self.cell_props,
                &mut self.sliding,
                &mut self.bounds,
                &mut self.active_blocks,
                &mut self.last_displacements,
                &mut self.last_simulated_ticks,
                budget_n,
                self.block_size,
                &[],
                12345u32.wrapping_add(self.tick_count),
                &mut self.edge_vel_h,
                &mut self.edge_vel_v,
                &self.mask,
                self.tick_count,
                gravity_dir,
            );
            self.tick_count += 1;
            flow
        }

        fn mass(&self) -> f64 {
            self.hm.data.iter().map(|&v| v as f64).sum()
        }
    }

    #[test]
    fn test_draw_point_out_of_bounds() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Drawing completely offscreen should not panic or modify the heightmap
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(5.0, 5.0),
            Vec2::new(5.0, 5.0),
            0.1,
            &mut bounds,
        );

        // Assert that heightmap data is unchanged
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Position marble so it sits on the left boundary
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            0.05,
            &mut bounds,
        );

        // Check that some points are carved below 0.1, and bounds are respected
        let mut modified_count = 0;
        for &val in hm.as_slice() {
            if val < 0.1 {
                modified_count += 1;
            }
        }
        assert!(modified_count > 0);
        assert!(bounds.active);
    }

    #[test]
    fn test_draw_line_interpolation() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Draw a line from (-0.5, 0.0) to (0.5, 0.0)
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-0.5, 0.0),
            Vec2::new(0.5, 0.0),
            0.05,
            &mut bounds,
        );

        // Helper to convert pos to grid index
        let norm_to_grid = |pos: Vec2| {
            let x = ((pos.x + 1.0) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            let y = ((1.0 - pos.y) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            (x, y)
        };

        // Verify that the path is continuous by checking that the center points are drawn
        let (cx1, cy1) = norm_to_grid(Vec2::new(-0.5, 0.0));
        let (cx2, cy2) = norm_to_grid(Vec2::new(0.0, 0.0));
        let (cx3, cy3) = norm_to_grid(Vec2::new(0.5, 0.0));

        assert!(hm.get(cx1, cy1) < 0.03);
        assert!(hm.get(cx2, cy2) < 0.03);
        assert!(hm.get(cx3, cy3) < 0.03);
    }

    #[test]
    fn test_draw_point_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(1e18, 1e18),
            Vec2::new(1e18, 1e18),
            0.1,
            &mut bounds,
        );
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_multipass_carving() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::DrySand, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Pass 1: carving at (0.0, 0.0) with DrySand properties
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
        );

        let center_idx = 256 * 512 + 256;
        let h1 = hm.data[center_idx];
        // Expect height to be approximately 20% of 0.35 = 0.07
        assert!((h1 - 0.07).abs() < 0.035, "First pass height should be ~0.07, got {}", h1);

        // Pass 2: carving again at (0.0, 0.0)
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
        );
        let h2 = hm.data[center_idx];
        // Expect height to be approximately 20% of h1 = 0.20 * 0.07 = 0.014
        assert!((h2 - 0.014).abs() < 0.035, "Second pass height should be ~0.014, got {}", h2);
        assert!(h2 < h1, "Second pass should carve deeper than first pass");
    }

    #[test]
    fn test_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.4);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement along a path
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-0.2, 0.2),
            Vec2::new(0.2, -0.2),
            0.03,
            &mut bounds,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-1e18, 0.0),
            Vec2::new(1e18, 0.0),
            0.1,
            &mut bounds,
        );
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut hm = Heightmap::new(512, 512, 0.70);
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement at a single point to trigger local saturation in the inner ridge
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.02,
            &mut bounds,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_settling_flow_and_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);

        let center_idx = 256 * 512 + 256;
        hm.data[center_idx] = 0.8;

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        let mut edge_vel_h = vec![0.0; 512 * 512];
        let mut edge_vel_v = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 256];
        let mut last_simulated_ticks = vec![0; 256];
        let budget_n = 256;
        let mut flow_occurred = false;
        let mut sliding = vec![false; 512 * 512];

        let mask = make_test_mask(512, 512, crate::SandboxShape::Circle, 0.04, 1.0);
        for i in 0..10 {
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                &[],
                12345,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                glam::Vec2::ZERO,
            );
            if flow > 0.0 {
                flow_occurred = true;
            }
        }

        assert!(flow_occurred, "Sand should flow down from the peak");

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(
            diff < 1e-5,
            "Settling did not conserve volume! diff = {}",
            diff
        );
        assert!(
            hm.data[center_idx] < 0.8,
            "Peak should be lower after flowing"
        );
    }

    #[test]
    fn test_settling_deactivation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        let mut cell_colors = vec![0.0f32; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 512 * 512];
        let mut edge_vel_v = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = Vec::new();
        let mut last_simulated_ticks = Vec::new();
        let budget_n = 256;
        let mut sliding = vec![false; 512 * 512];

        let mask = make_test_mask(512, 512, crate::SandboxShape::Circle, 0.04, 1.0);
        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut cell_props,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            budget_n,
            32,
            &[],
            12345,
            &mut edge_vel_h,
            &mut edge_vel_v,
            &mask,
            0,
            glam::Vec2::ZERO,
        );
        assert_eq!(flow, 0.0);
        assert!(!bounds.active, "Settling should deactivate when stable");
    }

    #[test]
    fn test_material_presets_and_avalanche() {
        use crate::MaterialMode;
        
        let materials = [
            MaterialMode::ButterCream,
            MaterialMode::DrySand,
            MaterialMode::Snow,
            MaterialMode::KineticSand,
            MaterialMode::WetSand,
            MaterialMode::FinePowder,
            MaterialMode::Oobleck,
            MaterialMode::MoonDust,
            MaterialMode::Water,
            MaterialMode::Milk,
            MaterialMode::VegetableOil,
            MaterialMode::CalmWater,
            MaterialMode::Yogurt,
            MaterialMode::CoarseSand,
        ];

        for &mat in &materials {
            let mut hm = Heightmap::new(64, 64, 0.5);
            let mut temp_heights = vec![0.5; 64 * 64];
            let mut cell_colors = vec![0.0f32; 64 * 64 * 4];
            let mut cell_props = get_test_props(mat, 64 * 64);
            let mut sliding = vec![false; 64 * 64];
            let mut bounds = ActiveBounds {
                min_x: 10,
                max_x: 54,
                min_y: 10,
                max_y: 54,
                active: true,
            };

            // Set a steep spike at center that exceeds the avalanche threshold (0.20 slope)
            let center_idx = 32 * 64 + 32;
            hm.data[center_idx] = 1.0;
            hm.data[center_idx - 1] = 0.5; // slope = 0.5 > 0.20

            let mut edge_vel_h = vec![0.0; 64 * 64];
            let mut edge_vel_v = vec![0.0; 64 * 64];
            let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
            let mut last_displacements = vec![1.0; 4];
            let mut last_simulated_ticks = vec![0; 4];
            let budget_n = 256;
            let mask = make_test_mask(64, 64, crate::SandboxShape::Circle, 0.04, 1.0);
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                &[ActiveMarbleInfo { pos: Vec2::ZERO, vel: 0.1, vel_vec: Vec2::new(0.1, 0.0) }],
                9999,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                0,
                glam::Vec2::ZERO,
            );

            assert!(flow > 0.0, "Material {:?} should flow under steep slope", mat);
        }
    }

    #[test]
    fn test_color_conservation() {
        let mut hm = Heightmap::new(128, 128, 0.5);
        // Put a peak in the center so sand flows
        let center_idx = 64 * 128 + 64;
        hm.data[center_idx] = 1.0;

        let mut cell_colors = vec![0.0f32; 128 * 128 * 4];
        let mut cell_props = vec![0.0f32; 128 * 128 * 4];
        // Initialize cell_colors and cell_props with a mixed striped pattern
        for y in 0..128 {
            for x in 0..128 {
                let idx = y * 128 + x;
                if (x / 16) % 2 == 0 {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.45;

                    cell_colors[idx * 4 + 0] = 200.0; // Reddish DrySand
                    cell_colors[idx * 4 + 1] = 100.0;
                    cell_colors[idx * 4 + 2] = 50.0;
                    cell_colors[idx * 4 + 3] = 255.0;
                } else {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.45;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.14;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.08;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.40;

                    cell_colors[idx * 4 + 0] = 50.0; // Bluish WetSand
                    cell_colors[idx * 4 + 1] = 100.0;
                    cell_colors[idx * 4 + 2] = 200.0;
                    cell_colors[idx * 4 + 3] = 255.0;
                }
            }
        }

        // Calculate initial total colors (Red, Green, Blue masses)
        let calculate_color_masses = |colors: &[f32], hmap: &Heightmap| -> (f64, f64, f64) {
            let mut r_mass = 0.0f64;
            let mut g_mass = 0.0f64;
            let mut b_mass = 0.0f64;
            for (idx, &h) in hmap.as_slice().iter().enumerate() {
                let r = colors[idx * 4 + 0] as f64;
                let g = colors[idx * 4 + 1] as f64;
                let b = colors[idx * 4 + 2] as f64;
                r_mass += r * h as f64;
                g_mass += g * h as f64;
                b_mass += b * h as f64;
            }
            (r_mass, g_mass, b_mass)
        };

        let (initial_r, initial_g, initial_b) = calculate_color_masses(&cell_colors, &hm);

        let mut temp_heights = vec![0.5; 128 * 128];
        let mut sliding = vec![false; 128 * 128];
        let mut bounds = ActiveBounds {
            min_x: 60,
            max_x: 68,
            min_y: 60,
            max_y: 68,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 128 * 128];
        let mut edge_vel_v = vec![0.0; 128 * 128];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 16];
        let mut last_simulated_ticks = vec![0; 16];

        let mask = make_test_mask(128, 128, crate::SandboxShape::Circle, 0.04, 1.0);
        // Settle a bit to trigger flows
        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut cell_props,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            256,
            32,
            &[],
            12345,
            &mut edge_vel_h,
            &mut edge_vel_v,
            &mask,
            0,
            glam::Vec2::ZERO,
        );

        assert!(flow > 0.0, "Settling flow must occur for the test");

        // Calculate final total colors
        let (final_r, final_g, final_b) = calculate_color_masses(&cell_colors, &hm);

        let diff_r = (final_r - initial_r).abs() / initial_r;
        let diff_g = (final_g - initial_g).abs() / initial_g;
        let diff_b = (final_b - initial_b).abs() / initial_b;

        assert!(diff_r < 0.005, "Red color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_r * 100.0, initial_r, final_r);
        assert!(diff_g < 0.005, "Green color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_g * 100.0, initial_g, final_g);
        assert!(diff_b < 0.005, "Blue color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_b * 100.0, initial_b, final_b);
    }

    #[test]
    fn test_advect_properties_weighted() {
        let mut cell_colors = vec![0.0f32; 8];
        let mut cell_props = vec![0.0f32; 8];

        // Cell 0: Red, Wet Sand-ish
        cell_colors[0..4].copy_from_slice(&[200.0, 100.0, 50.0, 255.0]);
        cell_props[0..4].copy_from_slice(&[0.5, 0.1, 0.15, 0.3]);

        // Cell 1: Blue, Dry Sand-ish
        cell_colors[4..8].copy_from_slice(&[50.0, 100.0, 200.0, 255.0]);
        cell_props[4..8].copy_from_slice(&[0.0, 0.08, 0.25, 0.45]);

        // Advect from 0 to 1 with flow = 0.2, and dst height h_dst = 0.2
        advect_properties(&mut cell_colors, &mut cell_props, 0, 1, 0.2, 0.2);

        // Expected colors (weighted average):
        // Red = (50 * 0.5 + 200 * 0.5) = 125
        // Green = 100
        // Blue = (200 * 0.5 + 50 * 0.5) = 125
        assert_eq!(cell_colors[4], 125.0);
        assert_eq!(cell_colors[5], 100.0);
        assert_eq!(cell_colors[6], 125.0);

        // Expected properties (weighted average):
        // wetness = (0.0 * 0.5 + 0.5 * 0.5) = 0.25
        // threshold = (0.08 * 0.5 + 0.1 * 0.5) = 0.09
        // flow_rate = (0.25 * 0.5 + 0.15 * 0.5) = 0.20
        // grain_size = (0.45 * 0.5 + 0.3 * 0.5) = 0.375
        assert_eq!(cell_props[4], 0.25);
        assert_eq!(cell_props[5], 0.09);
        assert_eq!(cell_props[6], 0.20);
        assert_eq!(cell_props[7], 0.375);
    }

    #[test]
    fn test_displace_line_advects() {
        let mut hm = Heightmap::new(128, 128, 0.5);
        let mut cell_colors = vec![100.0f32; 128 * 128 * 4];
        let mut cell_props = vec![0.5f32; 128 * 128 * 4];
        let mut active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 127,
            min_y: 0,
            max_y: 127,
            active: true,
        };

        // Source center area has different properties & colors
        for y in 60..68 {
            for x in 60..68 {
                let idx = y * 128 + x;
                cell_colors[idx * 4 + 0] = 200.0;
                cell_props[idx * 4 + PROP_WETNESS] = 0.1;
            }
        }

        // Draw a line through the center
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(0.0, 0.0),
            Vec2::new(0.1, 0.1),
            0.05,
            &mut active_bounds,
        );

        // Check that some cell outside the immediate line segment but within radius received advected properties
        // We will sum the red color and wetness in the ridge and assert change.
        let mut changed = false;
        for y in 0..128 {
            for x in 0..128 {
                let idx = y * 128 + x;
                // Exclude the starting zone
                if (x < 60 || x >= 68) || (y < 60 || y >= 68) {
                    if cell_colors[idx * 4 + 0] != 100.0 || cell_props[idx * 4 + PROP_WETNESS] != 0.5 {
                        changed = true;
                        break;
                    }
                }
            }
        }
        assert!(changed, "Properties/colors must have advected to surrounding cells during displacement");
    }

    #[test]
    fn test_property_and_color_conservation() {
        let mut sim = DrawingSimulation::new();
        // Set up alternating stripes of DrySand and WetSand properties, and mixed colors
        let mut cell_props = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
        // This buffer goes through the external set_cell_colors(&[u8]) API below, so it
        // stays u8 (not the internal f32 source of truth) — it's exercising the boundary.
        let mut cell_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                // Alternating stripes of DrySand and WetSand properties
                if (x / 32) % 2 == 0 {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.45;

                    cell_colors[idx * 4 + 0] = 200; // Reddish DrySand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 50;
                    cell_colors[idx * 4 + 3] = 255;
                } else {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.45;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.14;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.08;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.40;

                    cell_colors[idx * 4 + 0] = 50; // Bluish WetSand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 200;
                    cell_colors[idx * 4 + 3] = 255;
                }
            }
        }
        sim.set_cell_props(&cell_props);
        sim.set_cell_colors(&cell_colors);

        // Put several heaps of sand to force movement
        sim.heightmap.data.fill(0.1);
        for cy in [GRID_SIZE / 4, GRID_SIZE / 2, (3 * GRID_SIZE) / 4] {
            for cx in [GRID_SIZE / 4, GRID_SIZE / 2, (3 * GRID_SIZE) / 4] {
                let c_idx = cy * GRID_SIZE + cx;
                sim.heightmap.data[c_idx] = 1.0;
            }
        }

        // Calculate initial total property masses and color masses
        let calculate_masses = |s: &DrawingSimulation| -> (f64, f64, f64, f64, f64, f64, f64) {
            let mut wet_mass = 0.0f64;
            let mut thresh_mass = 0.0f64;
            let mut flow_mass = 0.0f64;
            let mut grain_mass = 0.0f64;
            let mut r_mass = 0.0f64;
            let mut g_mass = 0.0f64;
            let mut b_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let w = s.cell_props[idx * 4 + PROP_WETNESS] as f64;
                let t = s.cell_props[idx * 4 + PROP_THRESHOLD] as f64;
                let f = s.cell_props[idx * 4 + PROP_FLOW_RATE] as f64;
                let gr = s.cell_props[idx * 4 + PROP_GRAIN_SIZE] as f64;
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                let bl = s.cell_colors[idx * 4 + 2] as f64;
                wet_mass += w * h as f64;
                thresh_mass += t * h as f64;
                flow_mass += f * h as f64;
                grain_mass += gr * h as f64;
                r_mass += r * h as f64;
                g_mass += g * h as f64;
                b_mass += bl * h as f64;
            }
            (wet_mass, thresh_mass, flow_mass, grain_mass, r_mass, g_mass, b_mass)
        };

        let (init_wet, init_thresh, init_flow, init_grain, init_r, init_g, init_b) = calculate_masses(&sim);

        // Run 100 simulation steps with a moving marble
        let mut targets = [None; 5];
        for i in 0..100 {
            let angle = i as f32 * 0.15;
            let radius = i as f32 * 0.005;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.02,
                MaterialMode::DrySand, // preset parameter is ignored for properties after init
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_wet, final_thresh, final_flow, final_grain, final_r, final_g, final_b) = calculate_masses(&sim);

        let diff_wet = (final_wet - init_wet).abs() / init_wet;
        let diff_thresh = (final_thresh - init_thresh).abs() / init_thresh;
        let diff_flow = (final_flow - init_flow).abs() / init_flow;
        let diff_grain = (final_grain - init_grain).abs() / init_grain;
        let diff_r = (final_r - init_r).abs() / init_r;
        let diff_g = (final_g - init_g).abs() / init_g;
        let diff_b = (final_b - init_b).abs() / init_b;

        // Properties and colors must be conserved within 0.8%
        assert!(diff_wet < 0.008, "Wetness mass leaked! diff = {:.5}%, init = {}, final = {}", diff_wet * 100.0, init_wet, final_wet);
        assert!(diff_thresh < 0.008, "Threshold mass leaked! diff = {:.5}%, init = {}, final = {}", diff_thresh * 100.0, init_thresh, final_thresh);
        assert!(diff_flow < 0.008, "Flow rate mass leaked! diff = {:.5}%, init = {}, final = {}", diff_flow * 100.0, init_flow, final_flow);
        assert!(diff_grain < 0.008, "Grain size mass leaked! diff = {:.5}%, init = {}, final = {}", diff_grain * 100.0, init_grain, final_grain);
        assert!(diff_r < 0.008, "Red color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_r * 100.0, init_r, final_r);
        assert!(diff_g < 0.008, "Green color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_g * 100.0, init_g, final_g);
        assert!(diff_b < 0.008, "Blue color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_b * 100.0, init_b, final_b);
    }

    #[test]
    fn test_hourglass_boundary_math() {
        let w_f = 512.0;
        let h_f = 512.0;
        let center_x = w_f / 2.0;
        let center_y = h_f / 2.0;
        let chamber_h = 0.40 * h_f;
        let max_hw = 0.35 * w_f;
        let neck_hw = 0.04 * w_f;

        let is_inside = |cx: usize, cy: usize| -> bool {
            let dx = cx as f32 - center_x;
            let dy = cy as f32 - center_y;
            
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t * (max_hw - neck_hw);
                dx.abs() < allowed_hw
            } else {
                false
            }
        };

        // Center of upper chamber (256, 156)
        assert!(is_inside(256, 156));
        // Center of lower chamber (256, 356)
        assert!(is_inside(256, 356));
        // Inside the neck (256, 256 = center)
        assert!(is_inside(256, 256));
        // Inside upper chamber but offset horizontally
        assert!(is_inside(256 + 50, 156));
        // Outside chamber horizontally
        assert!(!is_inside(256 + 150, 156));
        // Completely outside vertically
        assert!(!is_inside(256, 20));
    }

    #[test]
    fn test_gravity_bias_flow() {
        let mut hm = Heightmap::new(64, 64, 0.35);
        let mut temp_heights = vec![0.35; 64 * 64];
        let mut cell_colors = vec![0.0f32; 64 * 64 * 4];
        let mut cell_props = get_test_props(MaterialMode::DrySand, 64 * 64);
        let mut sliding = vec![false; 64 * 64];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 64 * 64];
            let mut edge_vel_v = vec![0.0; 64 * 64];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];
        
        // Put gravity pulling downwards (+Y direction) - matching UI default strength (0.04)
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        
        let initial_sum: f32 = hm.data.iter().sum();

        let mask = make_test_mask(64, 64, SandboxShape::Circle, 0.04, 1.0);
        // Run 50 ticks of gravity settling
        for i in 0..50 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        let final_sum: f32 = hm.data.iter().sum();
        // Mass conservation
        assert!((final_sum - initial_sum).abs() / initial_sum < 1e-4);

        // Sand should have accumulated in the bottom half of the circle
        let top_half_sum: f32 = hm.data[0..32*64].iter().sum();
        let bottom_half_sum: f32 = hm.data[32*64..64*64].iter().sum();
        assert!(bottom_half_sum > top_half_sum, "Sand did not flow downward under gravity!");
    }


    #[test]
    fn test_hourglass_flow_after_flip() {
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.15 * w as f32; // Wide neck to speed up test flow
        let hourglass_curve = 0.6;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill only a shallow layer in the upper chamber just above the neck
        for y in 0..h {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            for x in 0..w {
                let dx = x as f32 - center_x;
                if dy_abs < chamber_h {
                    let t = dy_abs / chamber_h;
                    let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                    if dx.abs() < allowed_hw && dy < 0.0 && dy > -6.0 {
                        hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let initial_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let initial_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(initial_top_sum > 10.0);
        assert_eq!(initial_bottom_sum, 0.0);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, hourglass_curve);
        // Run 500 ticks to let almost all sand flow downward into the bottom chamber
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        let mid_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let mid_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // Sand should have flowed downward into the bottom chamber
        assert!(mid_bottom_sum > initial_top_sum * 0.40, "Not enough sand flowed to bottom! bottom_sum={}, init_top={}", mid_bottom_sum, initial_top_sum);
        assert!(mid_top_sum < initial_top_sum * 0.60);

        // Swap heights vertically (simulate flip)
        for y in 0..h / 2 {
            let y2 = h - 1 - y;
            for x in 0..w {
                hm.data.swap(y * w + x, y2 * w + x);
                temp_heights.swap(y * w + x, y2 * w + x);
            }
        }

        let post_flip_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let post_flip_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // After flip, sand is back in the top chamber (allow tiny epsilon for floating point swap ordering)
        assert!((post_flip_top_sum - mid_bottom_sum).abs() < 1e-4, "Top sum mismatch: {} vs {}", post_flip_top_sum, mid_bottom_sum);
        assert!((post_flip_bottom_sum - mid_top_sum).abs() < 1e-4, "Bottom sum mismatch: {} vs {}", post_flip_bottom_sum, mid_top_sum);

        // Run another 500 ticks with downward gravity
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + 500 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                (500 + i) as u32,
                gravity_dir,
            );
        }

        let final_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // Sand should have flowed downward again
        assert!(final_bottom_sum > post_flip_bottom_sum + (post_flip_top_sum * 0.30), "Sand did not flow downward after flip! init_bottom={}, final_bottom={}, post_flip_top={}", post_flip_bottom_sum, final_bottom_sum, post_flip_top_sum);
    }

    #[test]
    fn test_hourglass_statistical_symmetry() {
        // Initialize a symmetric grid with sand concentrated in the middle column
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);
        
        // Put a single block of sand at the top middle
        for y in 2..20 {
            for x in 30..34 {
                hm.data[y * w + x] = 1.0;
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];
        
        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 40 ticks of gravity settling
        for i in 0..40 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        // Measure center of mass along X axis
        let mut total_mass = 0.0f32;
        let mut weighted_x = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let val = hm.data[y * w + x];
                if val > 0.0 {
                    total_mass += val;
                    weighted_x += (x as f32) * val;
                }
            }
        }

        let center_of_mass_x = weighted_x / total_mass;
        let geometric_center_x = (w as f32 - 1.0) / 2.0; // 31.5

        // Center of mass should be extremely close to the geometric center (perfect symmetry)
        let bias = (center_of_mass_x - geometric_center_x).abs();
        assert!(bias < 0.25, "Found horizontal symmetry bias: {}", bias);
    }

    #[test]
    fn test_liquid_gravity_flows_downward() {
        // Verify that the wave-propagation solver (wetness >= 0.75) moves liquid
        // downward under gravity, not upward.
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0; // 32
        let center_y = h as f32 / 2.0; // 32
        let r = 0.46 * w as f32;       // 29.44
        let r_sq = r * r;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill only the top half of the circle with liquid
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                if dx * dx + dy * dy < r_sq && dy < -2.0 {
                    // Upper half of circle (dy < 0 means above center)
                    hm.data[y * w + x] = 0.8;
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        // Use Water material (wetness=1.0)
        let mut cell_props = get_test_props(MaterialMode::Water, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let initial_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let initial_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(initial_top_sum > initial_bottom_sum, "Initial state should have more liquid on top");

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 500 ticks of gravity settling (liquid CA is slower than wave)
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        // After settling under gravity, the bottom half should have MORE liquid
        // than the top half (liquid flows downward)
        let final_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let final_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(
            final_bottom_sum > final_top_sum,
            "Liquid should flow downward under gravity! top={}, bottom={}",
            final_top_sum, final_bottom_sum
        );
    }

    // =========================================================================================
    // Phase 0 characterisation tests (liquid-gravity overhaul safety net).
    //
    // L1-L3 and L5-L10 encode the *intended* correct behaviour for liquids under gravity and
    // are marked #[ignore] because they FAIL on today's code — that failure is the point: they
    // are the target later phases must turn green. L4 encodes an invariant that already holds
    // today (mass conservation of the CA gravity path) and is kept active as a regression guard.
    //
    // See scratchpad/liquid-gravity-proposal.md for the full diagnosis (defects C1-C9) this
    // suite is built against. Do NOT tune constants in physics.rs to make any of the ignored
    // tests below pass — that is explicitly out of scope for Phase 0.
    // =========================================================================================

    #[test]
    // Phase 2 (C2 fix): un-ignored. The fictitious lateral dispersion term is gone. Was: surface
    // spread = 47 rows (min=10, max=57) across 61 columns, up to 37 partially-filled cells in a
    // single column. Now: spread = 1 row (min=50, max=51), max 1 partially-filled cell/column.
    //
    // Phase 5: the mechanism underneath this changed and the tuning it needed went away. Levelling
    // is no longer a `liquid_alpha`/`max_transfer_coeff` pair sized to converge inside this test's
    // tick budget; it is what a conservative edge flux does on its own once cells have a capacity.
    // Two neighbouring columns of a pool present a real head difference at their surface row, the
    // flux moves mass down that gradient, and the acceptor's `cap - h` stops it at level. The
    // Phase 2 constants that used to set the convergence rate are deleted, and the result still
    // lands at spread = 1.
    fn test_liquid_pool_levels_flat_in_closed_box() {
        // A closed 64x64 box, Water poured into a 12-wide x 56-tall column, settled under
        // downward gravity for a long time. In a correct liquid solver this becomes a flat
        // pool with a single clean surface row per column.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        for y in 4..60 {
            for x in 6..18 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..1500 {
            sim.tick(gravity_dir, 256);
        }

        // Scan the WHOLE box, not just the original pour columns, since liquid disperses
        // sideways well beyond its starting footprint (C2).
        let mut surface_rows = Vec::new();
        let mut max_partial_in_a_column = 0;
        for x in 0..w {
            let mut surface_row: Option<usize> = None;
            let mut partial_count = 0;
            for y in 0..h {
                let val = sim.hm.data[y * w + x];
                if val > 0.5 && surface_row.is_none() {
                    surface_row = Some(y);
                }
                if val > 0.02 && val < 0.98 {
                    partial_count += 1;
                }
            }
            if let Some(sr) = surface_row {
                surface_rows.push(sr);
            }
            max_partial_in_a_column = max_partial_in_a_column.max(partial_count);
        }
        let min_row = *surface_rows.iter().min().unwrap();
        let max_row = *surface_rows.iter().max().unwrap();
        let spread = max_row - min_row;
        println!(
            "test_liquid_pool_levels_flat_in_closed_box: surface spread={} (min={}, max={}), \
             n_columns={}, max_partial_in_a_column={}",
            spread, min_row, max_row, surface_rows.len(), max_partial_in_a_column
        );

        // Measured today: spread=47, max_partial_in_a_column=37.
        assert!(spread <= 1, "Pool surface is not flat: spread={} rows", spread);
        assert!(
            max_partial_in_a_column <= 1,
            "Column has {} partially-filled cells, expected at most 1 (a single meniscus row)",
            max_partial_in_a_column
        );
    }

    #[test]
    // Phase 1 (C1 fix): liquid cells must respect CELL_CAPACITY = 1.0 (no cell above 1.0 + 1e-3)
    // and the occupied-cell footprint (h > 0.5) must be within 5% of the initial pour, i.e. no
    // phantom compression/shrinkage. Was ignored before Phase 1: max h = 1.502198 (cells packed
    // to the CA's 1.5 cap), occupied count 672 -> 466 (-30.65%). After the liquid-only capacity
    // fix (physics.rs get_ca_params / settle_tick, gated on `liquidity(wetness)`): max h = 1.0,
    // shrink = -0.30% (footprint grew slightly, well within tolerance).
    fn test_liquid_is_incompressible() {
        // Same pool as test_liquid_pool_levels_flat_in_closed_box: a closed box, Water poured
        // into a column, settled under gravity. A real (incompressible) liquid can never exceed
        // fill fraction 1.0 per cell, and settling should not make cells "disappear" (shrink the
        // occupied footprint) since mass is conserved.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        for y in 4..60 {
            for x in 6..18 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let initial_occupied: usize = 12 * 56;
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..1500 {
            sim.tick(gravity_dir, 256);
        }

        let max_h = sim.hm.data.iter().cloned().fold(0.0f32, f32::max);
        let final_occupied = sim.hm.data.iter().filter(|&&v| v > 0.5).count();
        let shrink_pct = 100.0 * (initial_occupied as f64 - final_occupied as f64) / initial_occupied as f64;
        println!(
            "test_liquid_is_incompressible: max_h={:.6}, occupied init={} final={} shrink={:.2}%",
            max_h, initial_occupied, final_occupied, shrink_pct
        );

        // Measured today: max_h=1.502198, shrink=30.65%.
        assert!(sim.hm.data.iter().all(|&v| v <= 1.0 + 1e-3), "Cell exceeded capacity: max_h={:.6}", max_h);
        assert!(
            shrink_pct.abs() < 5.0,
            "Occupied footprint changed by {:.2}%, expected within 5% (incompressible liquid)",
            shrink_pct
        );
    }

    #[test]
    // Phase 2 (C2 fix): un-ignored — a falling column stopped fanning out sideways.
    // Was: width=19, peak_h=0.3166 after 40 ticks. Phase 2: width=8, peak_h=0.7987.
    //
    // Phase 5: same width, but peak fill is now 1.0000 — the stream is genuinely saturated rather
    // than a narrow smear. Both properties come out of the update *order* rather than out of the
    // `liquid_can_still_fall` predicate and the 0.70 transfer coefficient Phase 2 used, which are
    // both deleted (see the operator-split note in `settle_tick`):
    //   - width, because gravity-aligned edges resolve before cross-gravity ones, so a falling
    //     cell has already handed its mass downward and has nothing left to spread;
    //   - peak fill, because the gravity-aligned sweep runs bottom-to-top, which is the
    //     CFL-respecting direction and stops a single pass from cascading a parcel down the whole
    //     grid and stretching it thin.
    fn test_liquid_stream_stays_coherent() {
        // A 64x96 box with a 4-cell-wide continuous source (a "tap") pouring at the top.
        // A coherent stream should stay narrow as it falls; today's dispersion noise
        // scatters it into a wide, thin sheet instead.
        let w = 64;
        let h = 96;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..40 {
            for y in 6..10 {
                for x in 30..34 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
            sim.tick(gravity_dir, 256);
        }

        // Densest (widest) row and peak fill anywhere in the mid-air band, well clear of the
        // source (y=6..10) and the box floor (box bottom is around y=92).
        let mut max_width = 0usize;
        let mut peak_h = 0.0f32;
        for y in 15..70 {
            let mut min_x = None;
            let mut max_x = None;
            for x in 0..w {
                let val = sim.hm.data[y * w + x];
                if val > 0.05 {
                    if min_x.is_none() { min_x = Some(x); }
                    max_x = Some(x);
                    peak_h = peak_h.max(val);
                }
            }
            if let (Some(mn), Some(mx)) = (min_x, max_x) {
                max_width = max_width.max(mx - mn + 1);
            }
        }
        println!("test_liquid_stream_stays_coherent: max_width={}, peak_h={:.4}", max_width, peak_h);

        // Measured today: max_width=19, peak_h=0.3166.
        assert!(max_width <= 8, "Stream cross-section too wide: {} cells", max_width);
        assert!(peak_h >= 0.5, "Stream peak fill too low: {:.4}", peak_h);
    }

    #[test]
    // Companion to `test_liquid_stream_stays_coherent`, and its deliberate opposite. That test
    // pins *falling* water narrow; this one pins *supported* water spreading, and — the point —
    // it measures while the liquid is still flowing rather than after it has settled.
    //
    // This is the case every other liquid test missed. They all settle with the inflow switched
    // off, so `edge_vel_v` decays to zero, the cross-gravity donor limit recovers, the pool
    // levels, and the end state looks right. The defect only existed during active flow: the
    // in-transit subtraction on the lateral edge was applied unconditionally, so in any
    // continuously fed body of liquid — a pour, or an upper chamber draining into a pool — every
    // cell received from above on every tick and `avail_*` never recovered. Lateral flow was
    // throttled at every depth, and the liquid stood up in vertical sheets against the casing
    // with a hollow between them instead of keeping a level surface: the user's "water walls".
    //
    // Metric: the number of *enclosed voids* — cells inside the shape that are essentially empty
    // (h <= 0.05) but have liquid (h > 0.5) somewhere to their left AND somewhere to their right
    // in the same row, with no casing in between. A level free surface has none; a pair of
    // standing walls with a drained channel between them has one per row of the channel, so the
    // count is a direct read of how wall-like the liquid is right now.
    //
    // Measured on a full hourglass upper chamber draining into the empty lower one, at the tick
    // where the drain is fully developed:
    //                            tick 120   tick 160   sum over 400 ticks
    //   before (unconditional):     223         41           38437
    //   after  (this fix):           94          0           30060
    //
    // `test_liquid_stream_stays_coherent` is the counterweight and is unchanged by the fix
    // (max_width 8, peak_h 1.0000, both before and after). Removing the in-transit limit
    // altogether does drive this test's tick-120 count to near zero, but it also blows that
    // stream out from 8 cells wide to 59 — see the note on `in_transit` in `settle_tick` for why
    // the limit has to survive for genuinely free-falling liquid.
    fn test_liquid_flowing_liquid_does_not_stand_in_walls() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // Fill the whole upper chamber; the lower one starts empty, so the neck feeds a column
        // that is continuously fed from above for the entire measurement window.
        for y in 0..h / 2 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }

        let count_voids = |sim: &TestSim| -> usize {
            let mut voids = 0;
            for y in 1..h - 1 {
                let mut liquid_to_the_left = false;
                for x in 0..w {
                    if mask[y * w + x] == crate::MASK_OUTSIDE {
                        // Casing breaks the row into independent spans.
                        liquid_to_the_left = false;
                        continue;
                    }
                    let v = sim.hm.data[y * w + x];
                    if v > 0.5 {
                        liquid_to_the_left = true;
                        continue;
                    }
                    if !liquid_to_the_left || v > 0.05 {
                        continue;
                    }
                    let liquid_to_the_right = (x + 1..w)
                        .take_while(|&x2| mask[y * w + x2] != crate::MASK_OUTSIDE)
                        .any(|x2| sim.hm.data[y * w + x2] > 0.5);
                    if liquid_to_the_right {
                        voids += 1;
                    }
                }
            }
            voids
        };

        let mut at_120 = 0;
        let mut at_160 = 0;
        let mut total = 0;
        let initial_mass = sim.mass();
        for t in 0..400 {
            sim.tick(gravity_dir, 256);
            let voids = count_voids(&sim);
            total += voids;
            if t + 1 == 120 {
                at_120 = voids;
            }
            if t + 1 == 160 {
                at_160 = voids;
            }
        }
        println!(
            "test_liquid_flowing_liquid_does_not_stand_in_walls: voids@120={} voids@160={} \
             total={} mass {:.3} -> {:.3}",
            at_120, at_160, total, initial_mass, sim.mass()
        );

        // Measured before the fix: 223 / 41 / 38437.
        assert!(
            at_120 <= 150,
            "Draining liquid is standing in walls: {} enclosed void cells at tick 120",
            at_120
        );
        assert!(
            at_160 <= 20,
            "Draining liquid is still standing in walls: {} enclosed void cells at tick 160",
            at_160
        );
        assert!(
            total <= 34_000,
            "Draining liquid spent too long in walls: {} void cell-ticks over 400 ticks",
            total
        );
    }

    #[test]
    fn test_liquid_mass_conserved_under_gravity() {
        // Regression guard (Phase 0): this invariant already holds today and must keep holding
        // through every later phase. Water poured into the upper chamber of an hourglass,
        // 2000 gravity ticks, total mass must be conserved.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy > -6.0 {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < 22.4 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }
        let initial_mass = sim.mass();
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..2000 {
            sim.tick(gravity_dir, 256);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "test_liquid_mass_conserved_under_gravity: init={:.6} final={:.6} rel_err={:.8}",
            initial_mass, final_mass, rel_err
        );
        // Measured today: rel_err ~= 1.2e-6.
        assert!(rel_err < 1e-4, "Mass not conserved under gravity: rel_err={:.8}", rel_err);
    }

    #[test]
    // Phase 5 (C7 fix): un-ignored. The sandbox liquid solver now conserves mass even when the
    // block LOD scheduler only simulates a fraction of the blocks per tick. Before the fix
    // (128x128 Water "dome", gravity=0, budget_n=4 of 16 blocks, 600 ticks) this measured
    // rel_err = +13.85% — mass INCREASED. (The liquid-gravity-proposal.md design doc reports
    // -1.345% for a similar but not identical setup; the sign disagrees, and this reproduction
    // is the one to trust.) Two independent causes, both structural rather than tunable:
    //   1. each cell adjusted *itself* by its own Laplacian, which only telescopes to zero over
    //      the domain if every cell updates in the same pass — `will_simulate[b]` gates blocks
    //      by frame budget, so it does not;
    //   2. the trailing `.clamp(0.0, 1.0)` was a unilateral edit with no counterparty: flooring
    //      a negative excursion to 0 adds mass, capping at 1.0 discards it.
    // Replaced by the per-edge flux form (`flux_edge`), where every edge debits exactly what it
    // credits and the donor/acceptor limits can only ever *reduce a transfer*. After the fix:
    // rel_err = -7e-9 (f32 rounding on the debit/credit pair), vs the required 1e-4.
    fn test_liquid_mass_conserved_in_sandbox_under_lod() {
        let w = 128;
        let h = 128;
        let block_size = 32; // 4x4 = 16 blocks
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, block_size);
        assert_eq!(sim.active_blocks.len(), 16, "Expected 16 blocks at 128x128 with block_size=32");

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                if dx * dx + dy * dy < 20.0 * 20.0 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }
        let initial_mass = sim.mass();
        // Sandbox mode: gravity = 0 (routes wetness >= 0.75 through the wave solver), and a
        // throttled budget so only 4 of 16 blocks simulate per tick.
        for _ in 0..600 {
            sim.tick(glam::Vec2::ZERO, 4);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass) / initial_mass;
        println!(
            "test_liquid_mass_conserved_in_sandbox_under_lod: init={:.6} final={:.6} rel_err={:.6}",
            initial_mass, final_mass, rel_err
        );
        assert!(rel_err.abs() < 1e-4, "Mass not conserved under partial-block LOD: rel_err={:.6}", rel_err);
    }

    // =======================================================================================
    // Sandbox (gravity = 0) wave dynamics.
    //
    // Until these existed there was no test of liquid *behaviour* at g = 0 at all. Every other
    // liquid test is gravity-oriented except `test_liquid_mass_conserved_in_sandbox_under_lod`,
    // which weighs the pool and never looks at it. That blind spot let `cce3b571` ship a solver
    // whose Sandbox ripples *grew* ~20% in amplitude per tick until they pinned against the cell
    // cap — the user's "used to ripple and reflect, now fully chaotic" — through 59 green tests,
    // because mass stayed perfect the whole time. Conservation and dynamics are independent
    // properties and each needs its own test.
    // =======================================================================================

    /// A flat Sandbox pool: every in-mask cell filled to `level`, gravity to be passed as zero.
    fn wave_pool(w: usize, h: usize, block_size: usize, shape: SandboxShape, level: f32) -> TestSim {
        let mask = make_test_mask(w, h, shape, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, block_size);
        for i in 0..w * h {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                sim.hm.data[i] = level;
            }
        }
        sim
    }

    /// Radially symmetric gaussian crest centred on `(bx, by)`.
    fn add_bump(sim: &mut TestSim, w: usize, h: usize, bx: f32, by: f32, amp: f32, sigma: f32) {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if sim.mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                let dx = x as f32 - bx;
                let dy = y as f32 - by;
                sim.hm.data[i] += amp * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            }
        }
    }

    /// Crest that is uniform in y, so the dynamics reduce to a 1-D channel along x. Used by the
    /// reflection test, where a radial ripple would confound "bounced off the wall" with
    /// "spread out sideways".
    fn add_band_bump(sim: &mut TestSim, w: usize, h: usize, bx: f32, amp: f32, sigma: f32) {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if sim.mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                let dx = x as f32 - bx;
                sim.hm.data[i] += amp * (-(dx * dx) / (2.0 * sigma * sigma)).exp();
            }
        }
    }

    /// Level the pool must relax to once the ripple is gone: all the mass, spread evenly over
    /// the cells that can hold it. Derived from the state rather than hard-coded so it stays
    /// correct if the mask area changes.
    fn wave_rest_level(sim: &TestSim) -> f32 {
        let mut total = 0.0f64;
        let mut n = 0usize;
        for i in 0..sim.hm.data.len() {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                total += sim.hm.data[i] as f64;
                n += 1;
            }
        }
        (total / n.max(1) as f64) as f32
    }

    /// Ripple amplitude: the largest departure from the resting pool level, in either direction.
    fn wave_amplitude(sim: &TestSim, rest: f32) -> f32 {
        let mut a = 0.0f32;
        for i in 0..sim.hm.data.len() {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                a = a.max((sim.hm.data[i] - rest).abs());
            }
        }
        a
    }

    /// Ripple amplitude restricted to the vertical strip `x in [x0, x1)`.
    fn wave_amplitude_in_band(sim: &TestSim, w: usize, h: usize, rest: f32, x0: usize, x1: usize) -> f32 {
        let mut a = 0.0f32;
        for y in 0..h {
            for x in x0..x1.min(w) {
                let i = y * w + x;
                if sim.mask[i] != crate::MASK_OUTSIDE {
                    a = a.max((sim.hm.data[i] - rest).abs());
                }
            }
        }
        a
    }

    #[test]
    // THE regression test for this bug. A crest dropped on a still pool must lose amplitude and
    // settle back to a flat pool; it is a damped wave, and the only source of energy is the
    // initial disturbance.
    //
    // The `cce3b571` edge-flux solver drove each edge's velocity from `temp_heights`, the buffer
    // it was concurrently writing, so a cell's four incident edges each saw whatever the previous
    // ones had already done and the pass became Gauss-Seidel with a direction-alternating sweep.
    // Gauss-Seidel on a wave equation is a gain, not just a loss of accuracy: linearising the
    // 1-D chain at Water's (c_sq, damping) = (0.24, 0.98) gives a per-tick spectral radius of
    // 1.20 for the swept form versus 0.994 for the snapshot form. 20% growth per tick against 2%
    // damping, so this test measured `maxh` climbing 0.80 -> 1.0000 by tick 50 and pinning there
    // for the remaining 350 — a pool of saturated cells, which is what the user saw as "fully
    // chaotic". Raising the cell cap to 3.0 only moved the ceiling (peak pinned at 3.0000) and
    // made the sweep bias 33x more visible, confirming injection rather than clipping.
    //
    // Driving the velocities from `heightmap.data` instead — frozen for the whole tick, since the
    // copy-back happens after the sweep — restores Jacobi ordering without touching the flux
    // form, so conservation (checked separately below) is unaffected.
    fn test_sandbox_wave_decays_to_flat_pool() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.30, 12.0);
        let rest = wave_rest_level(&sim);

        let mut samples = vec![(0u32, wave_amplitude(&sim, rest))];
        for t in 1..=400u32 {
            sim.tick(glam::Vec2::ZERO, 16);
            if t % 50 == 0 {
                samples.push((t, wave_amplitude(&sim, rest)));
            }
        }
        println!(
            "test_sandbox_wave_decays_to_flat_pool: rest={:.4} amplitude {:?}",
            rest,
            samples.iter().map(|&(t, a)| (t, (a * 1e4).round() / 1e4)).collect::<Vec<_>>()
        );

        // The envelope must come down at every sample. A growing solver fails this on the first
        // interval; a solver that merely stalls fails it later.
        for pair in samples.windows(2) {
            let ((t0, a0), (t1, a1)) = (pair[0], pair[1]);
            assert!(
                a1 < a0,
                "Sandbox ripple did not decay between tick {} and {}: {:.6} -> {:.6}. \
                 A growing amplitude means the wave update is injecting energy (Gauss-Seidel \
                 ordering); see this test's comment.",
                t0, t1, a0, a1
            );
        }
        let final_amp = samples.last().unwrap().1;
        assert!(
            final_amp < 0.25 * samples[0].1,
            "Sandbox ripple still holds {:.1}% of its initial amplitude after 400 ticks \
             ({:.6} of {:.6}); it should have relaxed toward the {:.4} rest level",
            100.0 * final_amp / samples[0].1, final_amp, samples[0].1, rest
        );
    }

    #[test]
    // A centred disturbance in a left-right symmetric domain must stay centred. Any bias in the
    // update order — and the solver's block, row and column sweeps all flip on `tick_count % 2`
    // — shows up here long before it is visible as instability.
    //
    // This is deliberately a *separate* assertion from the decay test: raising the cell cap to
    // 3.0 while the solver was still Gauss-Seidel left the pool "stable-looking" at its new
    // ceiling but drove asymmetry from 0.0029 to 0.0996, 33x worse. Amplitude and symmetry fail
    // independently, so they are tested independently.
    fn test_sandbox_wave_stays_left_right_symmetric() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        // Centred on (w-1)/2, the exact axis of the mirror map x -> w-1-x, so the initial
        // condition is *bit* symmetric and any asymmetry that appears later is the solver's.
        add_bump(&mut sim, w, h, (w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0, 0.30, 12.0);

        // Mirror error of the height field itself, normalised by total mass. Compared against
        // the field's own reflection rather than a left/right mass split, which is far coarser:
        // equal masses either side says nothing about equal *shapes* either side.
        let mirror_error = |s: &TestSim| -> f64 {
            let (mut diff, mut total) = (0.0f64, 0.0f64);
            for y in 0..h {
                for x in 0..w {
                    let (i, j) = (y * w + x, y * w + (w - 1 - x));
                    if s.mask[i] == crate::MASK_OUTSIDE || s.mask[j] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    diff += (s.hm.data[i] - s.hm.data[j]).abs() as f64;
                    total += s.hm.data[i] as f64;
                }
            }
            if total > 0.0 { diff / total } else { 0.0 }
        };

        let initial = mirror_error(&sim);
        assert!(initial < 1e-9, "test setup is not mirror symmetric: {:.3e}", initial);

        let mut worst = 0.0f64;
        let mut trace = Vec::new();
        for t in 1..=400u32 {
            sim.tick(glam::Vec2::ZERO, 16);
            let e = mirror_error(&sim);
            worst = worst.max(e);
            if t % 100 == 0 {
                trace.push((t, e));
            }
        }
        let final_err = mirror_error(&sim);
        println!(
            "test_sandbox_wave_stays_left_right_symmetric: worst={:.3e} final={:.3e} trace={:?}",
            worst, final_err,
            trace.iter().map(|&(t, e)| (t, format!("{:.2e}", e))).collect::<Vec<_>>()
        );

        // Some asymmetry is unavoidable: the sweep order alternates every tick, so a symmetric
        // pair of cells is not visited in the same relative order on every tick. What must not
        // happen is for that to *accumulate*.
        assert!(
            worst < 1e-2,
            "Centred disturbance went lopsided: mirror error reached {:.3e}. A directional \
             sweep bias in the wave update is the cause to look for.",
            worst
        );
        assert!(
            final_err < 0.25 * worst,
            "Mirror error is not transient — it peaked at {:.3e} and is still {:.3e} after 400 \
             ticks, so the bias is accumulating rather than washing out",
            worst, final_err
        );
    }

    #[test]
    // Conservation, on the same disturbance the decay test uses. `cce3b571` bought this at the
    // cost of stability (-3.93% drift over 400 ticks before it, ~0% after), and the fix for the
    // stability half must not hand the drift back: Jacobi ordering changes *when* an edge's
    // velocity is read, not the fact that the edge debits exactly what it credits.
    fn test_sandbox_wave_conserves_mass() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.30, 12.0);
        let initial = sim.mass();
        for _ in 0..400 {
            sim.tick(glam::Vec2::ZERO, 16);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial) / initial;
        println!(
            "test_sandbox_wave_conserves_mass: init={:.6} final={:.6} rel_err={:.3e}",
            initial, final_mass, rel_err
        );
        assert!(
            rel_err.abs() < 1e-4,
            "Sandbox ripple leaked mass: rel_err={:.6}", rel_err
        );
    }

    #[test]
    // Reflection — "ripples that would reflect", the other half of the user's report.
    //
    // A y-uniform crest near the left wall of a square pool makes the problem a 1-D channel, so
    // the wave that leaves the crest has nowhere to go but the far wall and back; a radial bump
    // would let "spread out sideways" masquerade as "bounced".
    //
    // Two distinct failure modes are ruled out by looking at both ends of the channel:
    //   * absorbed into the wall  -> the far band rings up and the near band never rings again;
    //   * piled against the wall  -> the far band rings up and stays up.
    // A real reflection is the far band rising and then falling *and* the near band recovering
    // after its own minimum.
    fn test_sandbox_wave_reflects_off_boundary() {
        let (w, h, bs) = (64, 64, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Square, 0.50);
        // Actual mask extent along the mid row — the Square shape insets from the grid edge.
        let (mut x_lo, mut x_hi) = (w, 0usize);
        for x in 0..w {
            if sim.mask[(h / 2) * w + x] != crate::MASK_OUTSIDE {
                x_lo = x_lo.min(x);
                x_hi = x_hi.max(x);
            }
        }
        add_band_bump(&mut sim, w, h, x_lo as f32 + 4.0, 0.30, 3.0);
        let rest = wave_rest_level(&sim);

        // Signed mean deviation of a whole column. Signed, not absolute: a crest reflecting off
        // a Neumann wall arrives as a *positive* excursion where there was a negative one, which
        // an absolute-value metric would blur into the static offset.
        let column = |s: &TestSim, x: usize| -> f32 {
            let (mut sum, mut n) = (0.0f32, 0usize);
            for y in 0..h {
                let i = y * w + x;
                if s.mask[i] != crate::MASK_OUTSIDE {
                    sum += s.hm.data[i] - rest;
                    n += 1;
                }
            }
            sum / n.max(1) as f32
        };
        let (near_x, far_x) = (x_lo + 4, x_hi - 1);

        let (mut near, mut far) = (vec![column(&sim, near_x)], vec![column(&sim, far_x)]);
        for _ in 0..400 {
            sim.tick(glam::Vec2::ZERO, 4);
            near.push(column(&sim, near_x));
            far.push(column(&sim, far_x));
        }

        let far_peak_t = (0..far.len()).max_by(|&a, &b| far[a].total_cmp(&far[b])).unwrap();
        let far_peak = far[far_peak_t];
        let far_end = far[far.len() - 1];
        // Whatever comes back to the near column *after* the wave has reached the far wall.
        let return_from = far_peak_t + 20;
        let near_return = near[return_from..].iter().cloned().fold(f32::MIN, f32::max);
        println!(
            "test_sandbox_wave_reflects_off_boundary: mask x {}..{}, rest={:.4}; \
             far start={:+.5} peak={:+.5}@t{} end={:+.5}; near at t{}={:+.5} return={:+.5}",
            x_lo, x_hi, rest, far[0], far_peak, far_peak_t, far_end,
            far_peak_t, near[far_peak_t], near_return
        );

        // The far column starts below the rest level (all the disturbance is at the near end)
        // and must stay there until the wave physically crosses the pool.
        assert!(far[0] < 0.0, "far column did not start below rest: {:+.6}", far[0]);
        assert!(
            far_peak_t > 40,
            "The far wall reacted at t={}, far sooner than a wave can cross {} cells at this \
             wave speed — that is not propagation",
            far_peak_t, far_x - near_x
        );
        assert!(
            far_peak > 0.02,
            "The disturbance never reached the far wall: that column only ever rose to {:+.6} \
             above the rest level",
            far_peak
        );
        // Reflected, not absorbed and not accumulated.
        assert!(
            far_end < 0.25 * far_peak,
            "The disturbance piled up against the far wall instead of bouncing off it: the wall \
             column peaked at {:+.6} and is still {:+.6} at the end",
            far_peak, far_end
        );
        assert!(
            near[far_peak_t] <= 0.0,
            "The near end had not gone quiet by the time the wave hit the far wall ({:+.6}), so \
             the recovery below would not prove anything",
            near[far_peak_t]
        );
        assert!(
            near_return > 0.004,
            "Nothing came back: after the wave hit the far wall at t={}, the near column only \
             ever recovered to {:+.6}. The boundary swallowed the wave instead of reflecting it",
            far_peak_t, near_return
        );
    }

    #[test]
    #[ignore = "MARKER, not a regression, and deliberately NOT fixed here: a pool already sitting \
                at cell capacity has no headroom for a crest, so it cannot ripple. Water's \
                cell_capacity_for(1.0) is exactly 1.0, and every transfer in `flux_edge` is \
                limited by the acceptor's `(cap_b - h_b).max(0.0)`, so at h == cap that limit is \
                zero on every edge in every direction. A refilling trough can therefore rise to \
                the rest level but can never overshoot it, and overshoot is what ringing IS. \
                Measured today, the same narrow trough carved into the same pool at two fill \
                levels: a half-full pool (rest 0.4964) overshoots the rest level by 1.306e-1 and \
                rings; an at-capacity pool (rest 0.9928) overshoots by 7.220e-3 — which is not a \
                damped version of the same thing, it is *exactly* the 7.220e-3 of headroom \
                between that rest level and the cap, to every digit. The crest is not attenuated, \
                it is clipped by the ceiling. Distinct from the Gauss-Seidel energy injection \
                fixed alongside these tests (that one made ripples grow; this one stops them \
                existing). The remedy is headroom — a free surface allowed above the packing \
                limit, or a capacity above the fill ceiling — which changes what a cell means and \
                belongs in its own commit."]
    fn test_sandbox_wave_at_capacity_cannot_ripple() {
        let (w, h, bs) = (64, 64, 32);
        // Carve one narrow trough and watch its centre refill. A real damped wave overshoots the
        // rest level and rings; the question is only whether there is room above rest to do it in.
        let run = |level: f32| -> (f32, f32) {
            let mut sim = wave_pool(w, h, bs, SandboxShape::Square, level);
            add_bump(&mut sim, w, h, 32.0, 32.0, -level, 2.0);
            let rest = wave_rest_level(&sim);
            let probe = 32 * w + 32;
            let mut peak = f32::MIN;
            for _ in 0..600 {
                sim.tick(glam::Vec2::ZERO, 4);
                peak = peak.max(sim.hm.data[probe]);
            }
            (rest, peak - rest)
        };

        let cap = cell_capacity_for(1.0);
        let (rest_half, over_half) = run(0.50);
        let (rest_full, over_full) = run(cap);
        println!(
            "test_sandbox_wave_at_capacity_cannot_ripple: cap={:.3}; half-full rest={:.4} \
             overshoot={:.3e}; at-capacity rest={:.4} headroom={:.3e} overshoot={:.3e}",
            cap, rest_half, over_half, rest_full, cap - rest_full, over_full
        );
        assert!(over_half > 1e-3, "control case did not ring at all: {:.3e}", over_half);
        assert!(
            over_full > 0.5 * over_half,
            "A pool at capacity cannot ripple: the same trough overshoots the rest level by \
             {:.3e} in a half-full pool but only {:.3e} at capacity, where the entire headroom \
             above rest is {:.3e}",
            over_half, over_full, cap - rest_full
        );
    }


    #[test]
    // Phase 5: un-ignored, and it came for free with the L5 fix. Toggling gravity to zero
    // mid-simulation (the shipped slider reaches 0.0 in Sand-fall mode, demo.js:710) used to
    // measure rel_err = +9.75% over 60 ticks at g=(0,0.04) followed by 300 at g=0 — mass
    // INCREASED, because the g=0 branch's `clamp(0.0, 1.0)` is asymmetric: an undershoot below 0
    // was floored to 0 without removing the corresponding mass from any neighbour, and an
    // hourglass gives many ticks of wall reflection for that to accumulate. (The design doc
    // reports -21.5%; the sign disagrees and this reproduction is the one to trust.) The edge
    // flux form has no unilateral clamp at all, so this is conservative by construction rather
    // than by tuning. After the fix: rel_err = 1.3e-8, vs the required 1e-3.
    fn test_liquid_survives_gravity_toggle() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy > -6.0 {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < 22.4 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }
        let initial_mass = sim.mass();
        for _ in 0..60 {
            sim.tick(glam::Vec2::new(0.0, 0.04), 256);
        }
        for _ in 0..300 {
            sim.tick(glam::Vec2::ZERO, 256);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "test_liquid_survives_gravity_toggle: init={:.6} final={:.6} rel_err={:.6}",
            initial_mass, final_mass, rel_err
        );
        assert!(rel_err < 1e-3, "Mass not conserved across a gravity toggle: rel_err={:.6}", rel_err);
    }

    #[test]
    #[ignore = "STILL FAILING after Phase 5, but for a different reason than before, and the \
                remaining gap looks like a defect in this test rather than in the solver. \
                Originally (C4): get_ca_params collapsed every wetness >= 0.75 material to the \
                same (threshold = 0.0, alpha = 0.50) and wave_params — the only thing that \
                distinguishes Water/CalmWater/Milk/VegOil — was unreachable under gravity, so \
                all four produced bit-identical centroids (max_sep = 0.000000). Phase 5 put the \
                gravity liquid path on the same edge-flux solver as the g = 0 path, so \
                wave_params IS now reached and the four presets are no longer identical: \
                max_sep = 0.0237. But that is still far short of the 0.5 this test demands, \
                because the metric is the centroid of the FINAL SETTLED state after 800 ticks. \
                A conservative, incompressible solver settles every liquid into the same shape \
                — that is the point of Phase 1's capacity constraint and Phase 5's conservation \
                — so the settled centroid cannot distinguish them no matter how different their \
                dynamics are. What actually differs is how fast they get there: c_sq/damping \
                span (0.08, 0.76) for Yogurt to (0.24, 0.98) for Water, roughly a 2x spread in \
                free-fall rate. That is the design doc's own alternative criterion ('settle-time \
                differing > 10%'), which this test does not implement. Deliberately left \
                failing and unmodified rather than weakened."]
    fn test_liquid_presets_are_distinguishable_under_gravity() {
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let r_sq = (0.46 * w as f32) * (0.46 * w as f32);
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mut centroids = Vec::new();
        for mat in [MaterialMode::Water, MaterialMode::CalmWater, MaterialMode::Milk, MaterialMode::VegetableOil] {
            let props = get_test_props(mat, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    let dy = y as f32 - center_y;
                    if dx * dx + dy * dy < r_sq && dy < -2.0 {
                        sim.hm.data[y * w + x] = 0.8;
                    }
                }
            }
            for _ in 0..800 {
                sim.tick(gravity_dir, 256);
            }
            let mut total = 0.0f64;
            let mut wx = 0.0f64;
            let mut wy = 0.0f64;
            for y in 0..h {
                for x in 0..w {
                    let val = sim.hm.data[y * w + x] as f64;
                    if val > 0.0 {
                        total += val;
                        wx += x as f64 * val;
                        wy += y as f64 * val;
                    }
                }
            }
            let centroid = (wx / total, wy / total);
            println!("test_liquid_presets_are_distinguishable_under_gravity: {:?} centroid={:?}", mat, centroid);
            centroids.push((mat, centroid));
        }

        // Pairwise centroid separation: at least one pair should differ by more than 0.5 cells.
        let mut max_sep = 0.0f64;
        for i in 0..centroids.len() {
            for j in (i + 1)..centroids.len() {
                let (ax, ay) = centroids[i].1;
                let (bx, by) = centroids[j].1;
                let sep = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                max_sep = max_sep.max(sep);
            }
        }
        println!("test_liquid_presets_are_distinguishable_under_gravity: max pairwise centroid separation={:.6}", max_sep);
        // Measured today: max_sep = 0.0 (bit-identical).
        assert!(max_sep > 0.5, "All liquid presets produced statistically identical results under gravity: max_sep={:.6}", max_sep);
    }

    #[test]
    // Phase 1 (C5 fix): the wetness >= 0.75 liquid/granular branch cut must be stable under
    // property advection — two runs seeded a hair on either side of 0.75 should agree closely,
    // not diverge catastrophically. Was ignored before Phase 1: wetness=0.7499 settled near
    // centroid y=41.18, wetness=0.7501 near centroid y=51.50 — a ~16% difference relative to the
    // 64-row grid. Cause: advect_properties blends with weights that don't sum to exactly 1.0 in
    // f32, and cells that drifted to wetness < 0.75 fell wholesale into the granular branch where
    // alpha = flow_rate * 1.5 = 0.0 for a Yogurt-like material (flow_rate=0.08 here), freezing
    // solid instead of flowing. Fixed by replacing the hard `wetness >= 0.75` parameter switch in
    // get_ca_params with a `liquidity(wetness)` smoothstep blend of the granular and liquid
    // (threshold, alpha) pairs, so a drift of a few 1e-4 in wetness only perturbs the blend
    // weight by a similarly tiny amount. After the fix: rel_diff ~= 0.00018 (vs required < 0.01).
    fn test_wetness_classification_is_stable_under_advection() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let r_sq = (0.46 * w as f32) * (0.46 * w as f32);

        let mut centroids_y = Vec::new();
        for wetness in [0.7499f32, 0.7501f32] {
            let mut props = vec![0.0f32; w * h * 4];
            for chunk in props.chunks_exact_mut(4) {
                chunk[PROP_WETNESS] = wetness;
                chunk[PROP_THRESHOLD] = 0.0;
                chunk[PROP_FLOW_RATE] = 0.08;
                chunk[PROP_GRAIN_SIZE] = 0.08;
            }
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    let dy = y as f32 - center_y;
                    if dx * dx + dy * dy < r_sq && dy < -2.0 {
                        sim.hm.data[y * w + x] = 0.8;
                    }
                }
            }
            for _ in 0..800 {
                sim.tick(gravity_dir, 256);
            }
            let mut total = 0.0f64;
            let mut wy = 0.0f64;
            for y in 0..h {
                for x in 0..w {
                    let val = sim.hm.data[y * w + x] as f64;
                    if val > 0.0 {
                        total += val;
                        wy += y as f64 * val;
                    }
                }
            }
            let centroid_y = wy / total;
            println!("test_wetness_classification_is_stable_under_advection: wetness={} centroid_y={:.5}", wetness, centroid_y);
            centroids_y.push(centroid_y);
        }

        let diff = (centroids_y[0] - centroids_y[1]).abs();
        let rel_diff = diff / h as f64;
        println!("test_wetness_classification_is_stable_under_advection: |diff|={:.5} rel_diff={:.5}", diff, rel_diff);
        // Measured today: rel_diff ~= 0.16 (16%).
        assert!(
            rel_diff < 0.01,
            "wetness=0.7499 and wetness=0.7501 diverge by {:.2}% of grid height, expected < 1%",
            rel_diff * 100.0
        );
    }

    #[test]
    #[ignore = "Phase 2/3 target: liquid should have essentially no angle of repose (settles \
                flat, spread <= 1 row) while granular DrySand should retain a real heap (spread \
                >= 8 rows) under identical pour conditions — this is the user's core complaint. \
                Measured today (continuous point-source pour, 400 ticks pouring + 600 ticks \
                settling): Water spread=1 (already flat, min=54 max=55) but DrySand spread=7 \
                (min=52 max=59) — DrySand falls just short of the 8-row bar too. That is a \
                second, distinct finding: under Sand-fall gravity mode DrySand's own repose \
                angle is weaker than expected (get_ca_params halves the threshold again via \
                'threshold *= 0.35' at physics.rs:226, and lock_chance drops to a flat 0.05 at \
                physics.rs:241-242 'for smooth avalanching'), so even dry sand piles flatter \
                than a real angle of repose would allow."]
    fn test_liquid_has_no_angle_of_repose() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mut spreads = Vec::new();
        for mat in [MaterialMode::Water, MaterialMode::DrySand] {
            let props = get_test_props(mat, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            // Continuous point-source pour near the top center (like a tap), so enough mass
            // accumulates for DrySand to build a real angle-of-repose cone rather than a
            // single blob settling flat by construction.
            for _ in 0..400 {
                for y in 4..8 {
                    for x in 30..34 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
                sim.tick(gravity_dir, 256);
            }
            // Let it settle without further pouring.
            for _ in 0..600 {
                sim.tick(gravity_dir, 256);
            }

            let mut surface_rows = Vec::new();
            for x in 6..58 {
                for y in 0..h {
                    if sim.hm.data[y * w + x] > 0.05 {
                        surface_rows.push(y);
                        break;
                    }
                }
            }
            let spread = if surface_rows.is_empty() {
                0
            } else {
                surface_rows.iter().max().unwrap() - surface_rows.iter().min().unwrap()
            };
            println!("test_liquid_has_no_angle_of_repose: {:?} spread={}", mat, spread);
            spreads.push(spread);
        }

        // Measured today: Water spread=1, DrySand spread=7.
        assert!(spreads[0] <= 1, "Liquid (Water) should settle nearly flat: spread={}", spreads[0]);
        assert!(spreads[1] >= 8, "DrySand should retain a real heap: spread={}", spreads[1]);
    }

    #[test]
    #[ignore = "Phase 3 target: a liquid blob impacting a floor should splash — spreading \
                laterally beyond its original width AND moving at least 1 row upward against \
                gravity. Measured today: lateral spread does happen (width 8 -> 30 within 10 \
                ticks of impact, via the same dispersion noise as C2) but upward movement is \
                impossible BY CONSTRUCTION: physics.rs:1162 and physics.rs:1248 both `continue` \
                whenever `gravity_active && gravity_dot < -0.01`, i.e. no cell may ever flow \
                against gravity in Sand-fall mode. min_row_after (59) never goes above \
                top_row_at_impact (50)."]
    fn test_liquid_splashes_on_impact() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // A compact blob close to the floor.
        for y in 50..54 {
            for x in 28..36 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let initial_width = 36 - 28;

        let mut top_row_at_impact = None;
        for _ in 0..60 {
            sim.tick(gravity_dir, 256);
            if top_row_at_impact.is_some() {
                continue;
            }
            // Detect impact: material reaches near the floor (y=58 or 59).
            let touching_floor = (20..44).any(|x| sim.hm.data[58 * w + x] > 0.05 || sim.hm.data[59 * w + x] > 0.05);
            if touching_floor {
                let min_row = (0..h)
                    .find(|&y| (20..44).any(|x| sim.hm.data[y * w + x] > 0.05))
                    .expect("material must exist somewhere once it's touching the floor");
                top_row_at_impact = Some(min_row);
            }
        }
        let top_at_impact = top_row_at_impact.expect("blob never reached the floor within 60 ticks");

        for _ in 0..10 {
            sim.tick(gravity_dir, 256);
        }

        let mut min_x = w;
        let mut max_x = 0;
        let mut min_row_after = h;
        for y in 0..h {
            for x in 0..w {
                if sim.hm.data[y * w + x] > 0.05 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_row_after = min_row_after.min(y);
                }
            }
        }
        let width_after = max_x.saturating_sub(min_x) + 1;
        let lateral_spread = width_after > initial_width;
        let upward_move = min_row_after < top_at_impact;
        println!(
            "test_liquid_splashes_on_impact: width_after={} (initial={}), min_row_after={} \
             (top_at_impact={}), lateral_spread={}, upward_move={}",
            width_after, initial_width, min_row_after, top_at_impact, lateral_spread, upward_move
        );

        // Measured today: lateral_spread=true, upward_move=false (structurally impossible).
        assert!(lateral_spread, "Blob did not spread laterally beyond its original width on impact");
        assert!(upward_move, "Blob did not move upward at all on impact (min_row_after={}, top_at_impact={})", min_row_after, top_at_impact);
    }

    #[test]
    fn test_hourglass_full_drainage() {
        // Fill the upper chamber of an hourglass with DrySand and let it settle under gravity
        // for long enough to reach a steady state, then verify (a) mass is conserved and
        // (b) the large majority of the sand has drained through the neck into the lower
        // chamber. This does NOT assert the upper chamber reaches exactly zero: measured today,
        // a residual pile (~13% of total mass) permanently rests in the upper chamber above the
        // neck once the local slope drops below the (gravity-reduced) repose threshold, and
        // upper-chamber mass is observed to plateau (stop changing tick-over-tick) well before
        // 3000 ticks — i.e. "drainage" reaches a stable end state, just not a literally empty
        // upper chamber. That residual-pile behavior is plausible for granular material and is
        // NOT something Phase 0 should "fix"; this test only pins down that it doesn't regress.
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        let center_x = 32.0;
        let center_y = 32.0;
        let chamber_h = 0.40 * 64.0;
        let max_hw = 0.35 * 64.0;
        let neck_hw = 0.04 * 64.0;

        for y in 0..64 {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.60) * (max_hw - neck_hw);
                for x in 0..64 {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw && dy < 0.0 {
                        let idx = y * w + x;
                        hm.data[idx] = 0.55;
                    }
                }
            }
        }
        let initial_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 63,
            min_y: 0,
            max_y: 63,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.60);
        for i in 0..3000u32 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345u32.wrapping_add(i),
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i,
                gravity_dir,
            );
        }

        println!("--- Sand around neck (y=30..36) ---");
        for y in 30..36 {
            for x in 28..36 {
                let idx = y * w + x;
                println!("y={:2}, x={:2}: h={:.4}", y, x, hm.data[idx]);
            }
        }

        let final_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
        let final_lower_mass: f64 = hm.data[32 * w..].iter().map(|&v| v as f64).sum();
        let mass_err = (final_mass - initial_mass).abs() / initial_mass;
        let drained_frac = final_lower_mass / initial_mass;
        println!(
            "test_hourglass_full_drainage: init_mass={:.6} final_mass={:.6} mass_err={:.8} \
             final_lower_mass={:.6} drained_frac={:.4}",
            initial_mass, final_mass, mass_err, final_lower_mass, drained_frac
        );

        // Measured today: mass_err ~= 1.1e-7, drained_frac ~= 0.869 (86.9%).
        assert!(mass_err < 1e-4, "Mass not conserved during drainage: mass_err={:.8}", mass_err);
        assert!(
            drained_frac > 0.75,
            "Less than 75% of the sand drained into the lower chamber: drained_frac={:.4}",
            drained_frac
        );
    }

    #[test]
    fn test_residual_sand_drains_to_zero() {
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        // Put a single small residual sand pixel at (32, 10)
        let src_idx = 10 * w + 32;
        hm.data[src_idx] = 0.002;

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 20 ticks of gravity settling
        for i in 0..20 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        // The source pixel should be cleanly zero (0.0) without leaving residual ghost height trapped
        assert_eq!(hm.data[src_idx], 0.0, "Residual sand was trapped! h={}", hm.data[src_idx]);
    }

    #[test]
    fn test_no_floating_sand_under_gravity() {
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill random sand in upper chamber inside hourglass boundary
        let center_x = 32.0;
        let center_y = 32.0;
        let chamber_h = 0.40 * 64.0;
        let max_hw = 0.35 * 64.0;
        let neck_hw = 0.04 * 64.0;

        for y in 5..30 {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 2..62 {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        let pseudo_rand = ((x * 17 + y * 31) % 100) as f32 / 100.0;
                        hm.data[idx] = pseudo_rand * 0.8;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.6);
        // Run gravity settling until all falling sand completes landing and flow reaches zero
        for i in 0..2000 {
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
            if i > 200 && flow == 0.0 {
                break;
            }
        }

        // Verify no cell with sand (h > 0.005) has an empty cell (h_below == 0.0) directly below it in mid-air
        for y in 2..60 {
            for x in 2..62 {
                let idx = y * w + x;
                let idx_below = (y + 1) * w + x;
                let h_curr = hm.data[idx];
                let h_below = hm.data[idx_below];

                let center_x = 32.0;
                let center_y = 32.0;
                let chamber_h = 0.40 * 64.0;
                let max_hw = 0.35 * 64.0;
                let neck_hw = 0.04 * 64.0;

                let is_in = |cx: usize, cy: usize| -> bool {
                    let dx = cx as f32 - center_x;
                    let dy = cy as f32 - center_y;
                    let dy_abs = dy.abs();
                    if dy_abs < chamber_h {
                        let t = dy_abs / chamber_h;
                        let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                        dx.abs() < allowed_hw
                    } else {
                        false
                    }
                };

                if is_in(x, y) && h_curr > 0.005 && is_in(x, y + 1) && h_below == 0.0 {
                    println!("Column x={}:", x);
                    for py in 0..15 {
                        let p_idx = py * w + x;
                        println!("y={}: h={} inside={}", py, hm.data[p_idx], is_in(x, py));
                    }
                    panic!("Found floating sand inside container at ({}, {}) with h={} and empty air below!", x, y, h_curr);
                }
            }
        }
    }

    #[test]
    fn test_falling_stream_no_block_boundary_density_spikes() {
        let w = 512;
        let h = 512;
        let block_size = 32;
        let cols = w / block_size;
        let rows = h / block_size;

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.005 * w as f32;
        let hourglass_curve = 0.6;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill upper chamber (y < center_y) up to 0.50 capacity
        for y in 0..h {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw && dy < -4.0 && dy > -0.50 * chamber_h {
                        hm.data[y * w + x] = 0.5;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: w - 1,
            min_y: 0,
            max_y: h - 1,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        let mut last_displacements = vec![1.0; cols * rows];
        let mut last_simulated_ticks = vec![0; cols * rows];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(512, 512, SandboxShape::Hourglass, hourglass_curve, 0.005);
        // Run simulation for 80 ticks
        for i in 0..80 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        // Print heights at block boundaries along stream center x=256
        let stream_x = 256;
        println!("--- Stream Height Profile at 32-pixel Block Boundaries ---");
        for by in 8..15 {
            let boundary_y = by * 32;
            let h_prev = hm.data[(boundary_y - 1) * w + stream_x];
            let h_bound = hm.data[boundary_y * w + stream_x];
            let h_next = hm.data[(boundary_y + 1) * w + stream_x];
            println!(
                "Boundary y={}: y-1={:.4}, y_bound={:.4}, y+1={:.4}",
                boundary_y, h_prev, h_bound, h_next
            );
        }

        // Classify EVERY consecutive-row height difference down the whole stream column as
        // either "at a block boundary" (the lower row index is a multiple of block_size, i.e.
        // this step crosses the seam between two 32px blocks copied back independently at
        // physics.rs:1372-1386) or "interior" (both rows are inside the same block). If the
        // block-based LOD/copy-back scheme were introducing density spikes specifically at
        // block seams, the boundary population's worst case would be anomalously large compared
        // to the interior population's worst case (which already reflects the stream's normal
        // leading-edge/settling discontinuities).
        let mut max_boundary_jump = 0.0f32;
        let mut max_interior_jump = 0.0f32;
        for y in 1..h {
            let h_prev = hm.data[(y - 1) * w + stream_x];
            let h_curr = hm.data[y * w + stream_x];
            let jump = (h_curr - h_prev).abs();
            if y % block_size == 0 {
                max_boundary_jump = max_boundary_jump.max(jump);
            } else {
                max_interior_jump = max_interior_jump.max(jump);
            }
        }
        println!(
            "max_boundary_jump={:.5} max_interior_jump={:.5} ratio={:.3}",
            max_boundary_jump, max_interior_jump, max_boundary_jump / max_interior_jump.max(1e-6)
        );

        // Measured today: max_boundary_jump=0.14356, max_interior_jump=0.38460 (ratio 0.373) —
        // boundary-adjacent jumps are actually SMALLER than the worst interior jump, i.e. no
        // block-seam-specific spike. Generous margin (2x) to absorb run-to-run tuning changes
        // that don't touch the LOD/copy-back mechanism itself.
        assert!(
            max_boundary_jump <= max_interior_jump * 2.0,
            "Block-boundary height jump ({:.5}) is anomalously large relative to the worst \
             interior jump ({:.5}) — possible density spike at a 32px block seam",
            max_boundary_jump, max_interior_jump
        );
    }

    #[test]
    fn test_hourglass_color_and_property_conservation_under_gravity() {
        // Verify that RGBA cell colors and material properties (wetness, grain size, flow rate)
        // are 100% conserved when sand flows down through the hourglass neck under gravity.
        let w = 128;
        let h = 128;
        let mut hm = Heightmap::new(w, h, 0.0);
        let mut temp_heights = vec![0.0f32; w * h];
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut cell_props = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut edge_vel_h = vec![0.0f32; w * h];
        let mut edge_vel_v = vec![0.0f32; w * h];

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * (h as f32);
        let max_hw = 0.35 * (w as f32);
        let neck_hw = 0.04 * (w as f32);

        // Fill upper chamber with two distinct colored & property layers (Red Dry Sand / Blue Wet Sand)
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy.abs() < chamber_h {
                let t = dy.abs() / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        hm.data[idx] = 0.80; // 80% initial fill height

                        if dy < -0.20 * (h as f32) {
                            // Top Layer: Red Dry Sand (Wetness = 0.0, GrainSize = 0.50)
                            cell_colors[idx * 4 + 0] = 230.0;
                            cell_colors[idx * 4 + 1] = 40.0;
                            cell_colors[idx * 4 + 2] = 40.0;
                            cell_colors[idx * 4 + 3] = 255.0;

                            cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                            cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                            cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                            cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.50;
                        } else {
                            // Bottom Layer: Blue Wet Sand (Wetness = 0.40, GrainSize = 0.30)
                            cell_colors[idx * 4 + 0] = 40.0;
                            cell_colors[idx * 4 + 1] = 80.0;
                            cell_colors[idx * 4 + 2] = 230.0;
                            cell_colors[idx * 4 + 3] = 255.0;

                            cell_props[idx * 4 + PROP_WETNESS] = 0.40;
                            cell_props[idx * 4 + PROP_THRESHOLD] = 0.12;
                            cell_props[idx * 4 + PROP_FLOW_RATE] = 0.15;
                            cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.30;
                        }
                    }
                }
            }
        }
        temp_heights.copy_from_slice(&hm.data);

        // Helper to calculate total color and property mass
        let calc_totals = |colors: &[f32], props: &[f32], hmap: &Heightmap| -> (f64, f64, f64, f64, f64) {
            let mut r_total = 0.0f64;
            let mut g_total = 0.0f64;
            let mut b_total = 0.0f64;
            let mut wet_total = 0.0f64;
            let mut grain_total = 0.0f64;
            for (idx, &height) in hmap.as_slice().iter().enumerate() {
                let h_val = height as f64;
                if h_val > 0.0 {
                    r_total += (colors[idx * 4 + 0] as f64) * h_val;
                    g_total += (colors[idx * 4 + 1] as f64) * h_val;
                    b_total += (colors[idx * 4 + 2] as f64) * h_val;
                    wet_total += (props[idx * 4 + PROP_WETNESS] as f64) * h_val;
                    grain_total += (props[idx * 4 + PROP_GRAIN_SIZE] as f64) * h_val;
                }
            }
            (r_total, g_total, b_total, wet_total, grain_total)
        };

        let (init_r, init_g, init_b, init_wet, init_grain) = calc_totals(&cell_colors, &cell_props, &hm);

        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: w - 1,
            min_y: 0,
            max_y: h - 1,
            active: true,
        };

        let expected_len = (w / 32) * (h / 32);
        let mut active_blocks = vec![crate::BlockActivity::Fast; expected_len];
        let mut last_displacements = vec![1.0; expected_len];
        let mut last_simulated_ticks = vec![0; expected_len];
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.6, 0.04);
        // Run 300 gravity ticks flowing sand down into the lower chamber
        for i in 0..300 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i as u32,
                gravity_dir,
            );
        }

        let (final_r, final_g, final_b, final_wet, final_grain) = calc_totals(&cell_colors, &cell_props, &hm);

        println!("Init Totals:  R={:.2}, G={:.2}, B={:.2}, Wet={:.2}, Grain={:.2}", init_r, init_g, init_b, init_wet, init_grain);
        println!("Final Totals: R={:.2}, G={:.2}, B={:.2}, Wet={:.2}, Grain={:.2}", final_r, final_g, final_b, final_wet, final_grain);

        // Relative error tolerances
        let r_err = (final_r - init_r).abs() / init_r;
        let g_err = (final_g - init_g).abs() / init_g;
        let b_err = (final_b - init_b).abs() / init_b;
        let wet_err = (final_wet - init_wet).abs() / init_wet;
        let grain_err = (final_grain - init_grain).abs() / init_grain;

        println!("Errors: R_err={:.9}, G_err={:.9}, B_err={:.9}, Wet_err={:.9}, Grain_err={:.9}", r_err, g_err, b_err, wet_err, grain_err);

        // Colors now advect through the f32 source of truth (`cell_colors_f32`), the same
        // weighted blend used for properties, with no per-tick rounding to u8. Measured
        // R/G/B error over these 300 ticks is ~1e-7 to 1e-9 (vs. up to 7.4% when colors were
        // blended and rounded to u8 directly on every tick) — the same float accumulation
        // error floor as the property channels below, so the same tolerance applies.
        assert!(r_err < 0.001, "Red color mass loss under gravity: err={:.6}", r_err);
        assert!(g_err < 0.001, "Green color mass loss under gravity: err={:.6}", g_err);
        assert!(b_err < 0.001, "Blue color mass loss under gravity: err={:.6}", b_err);
        assert!(wet_err < 0.001, "Wetness property loss under gravity: err={:.6}", wet_err);
        assert!(grain_err < 0.001, "Grain size property loss under gravity: err={:.6}", grain_err);
    }

    #[test]
    fn test_concentric_rings_eventually_all_drain_through_neck() {
        // Paint the upper chamber with rings centered on the neck (matching the UI's
        // "Concentric Rings" color pattern: sandart-wasm/web/demo.js generateColormap).
        // Ring 0 (nearest the neck) is green; odd rings are yellow.
        let w = 128;
        let h = 128;
        let mut hm = Heightmap::new(w, h, 0.0);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let cell_props_mode = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_props = cell_props_mode;

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.04 * w as f32;
        let ring_width = 8.0; // cells; proportionally matches the UI's 32px/512

        let mut initial_green_mass = 0.0f64;
        let mut initial_yellow_mass = 0.0f64;

        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy.abs() < chamber_h {
                let t = dy.abs() / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        hm.data[idx] = 0.60;

                        let dist = (dx * dx + dy * dy).sqrt();
                        let ring_even = ((dist / ring_width) as i64) % 2 == 0;
                        if ring_even {
                            // Green
                            cell_colors[idx * 4 + 0] = 34.0;
                            cell_colors[idx * 4 + 1] = 139.0;
                            cell_colors[idx * 4 + 2] = 34.0;
                            cell_colors[idx * 4 + 3] = 255.0;
                            initial_green_mass += hm.data[idx] as f64;
                        } else {
                            // Yellow
                            cell_colors[idx * 4 + 0] = 255.0;
                            cell_colors[idx * 4 + 1] = 215.0;
                            cell_colors[idx * 4 + 2] = 0.0;
                            cell_colors[idx * 4 + 3] = 255.0;
                            initial_yellow_mass += hm.data[idx] as f64;
                        }
                    }
                }
            }
        }

        assert!(initial_green_mass > 0.0 && initial_yellow_mass > 0.0, "Test setup should paint both colors");

        let mut temp_heights = hm.data.clone();
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };
        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let expected_len = (w / 32) * (h / 32);
        let mut active_blocks = vec![crate::BlockActivity::Fast; expected_len];
        let mut last_displacements = vec![1.0; expected_len];
        let mut last_simulated_ticks = vec![0; expected_len];
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.60);

        // Measure the height-weighted AVERAGE color strictly below the neck (lower chamber).
        // Pure ring paint is (34,139,34) green / (255,215,0) yellow; as sand mixes en route
        // to the neck, individual cells take on blended, in-between hues rather than staying
        // categorically one or the other. Tracking the weighted average RGB (and derived R:G
        // ratio, 0.0 = pure green, 1.0 = pure yellow) shows that drift directly.
        let measure_lower_chamber_avg = |colors: &[f32], hmap: &Heightmap| -> (f64, f64, f64, f64) {
            let mut r_sum = 0.0f64;
            let mut g_sum = 0.0f64;
            let mut b_sum = 0.0f64;
            let mut mass = 0.0f64;
            for y in (center_y as usize)..h {
                for x in 0..w {
                    let idx = y * w + x;
                    let hgt = hmap.data[idx] as f64;
                    if hgt <= 0.0 {
                        continue;
                    }
                    r_sum += colors[idx * 4 + 0] as f64 * hgt;
                    g_sum += colors[idx * 4 + 1] as f64 * hgt;
                    b_sum += colors[idx * 4 + 2] as f64 * hgt;
                    mass += hgt;
                }
            }
            if mass <= 0.0 {
                return (0.0, 0.0, 0.0, 0.0);
            }
            let avg_r = r_sum / mass;
            let avg_g = g_sum / mass;
            let avg_b = b_sum / mass;
            // 0.0 at pure green (34,139,34), 1.0 at pure yellow (255,215,0), interpolating on R and B.
            let yellow_frac = (((avg_r - 34.0) / (255.0 - 34.0)) + ((34.0 - avg_b) / 34.0)) / 2.0;
            (avg_r, avg_g, avg_b, yellow_frac)
        };

        for i in 0..4000u32 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mask,
                i,
                gravity_dir,
            );

            if i % 500 == 0 || i == 3999 {
                let (r, g, b, yellow_frac) = measure_lower_chamber_avg(&cell_colors, &hm);
                println!("tick {:5}: lower-chamber avg color = ({:.1}, {:.1}, {:.1})  yellow_frac={:.3}",
                    i, r, g, b, yellow_frac);
            }
        }

        let (final_r, final_g, final_b, final_yellow_frac) = measure_lower_chamber_avg(&cell_colors, &hm);
        println!("FINAL: lower-chamber avg color = ({:.1}, {:.1}, {:.1})  yellow_frac={:.3}", final_r, final_g, final_b, final_yellow_frac);
        assert!(
            final_yellow_frac > 0.05,
            "Lower chamber's average color never drifted toward yellow at all: yellow_frac={:.3}",
            final_yellow_frac
        );
    }

    #[test]
    fn test_no_mass_leaks_into_out_of_mask_cells() {
        // The granular CA flow path never checked whether the *destination* neighbor was inside
        // the shape mask before transferring into it (only the sandbox wave branch did, at the
        // `h_left`/`h_right`/`h_top`/`h_bottom` reads). A MASK_OUTSIDE cell is skipped by
        // `if !inside { continue }` at the top of the solver loop, so it is never simulated
        // again and anything landing there is frozen inside a wall permanently.
        //
        // This was invisible to every existing test: total mass is still conserved, so the
        // mass-conservation suite (including test_serpentine_no_sand_leaking, the tightest at
        // 1e-4) passes regardless. It was invisible on screen too, because the renderer draws
        // MASK_OUTSIDE as opaque casing.
        //
        // Measured before the fix: 254.05 of 2933.00 total mass (8.66%) ended up inside the
        // hourglass walls over 1500 ticks of DrySand. After: exactly 0.
        let w = 128;
        let h = 128;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.6);
        let mut hm = Heightmap::new(w, h, 0.0);
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE && (y as f32) < (h as f32 * 0.45) {
                    hm.data[idx] = 1.0;
                }
            }
        }
        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let block_size = 32;
        let (cols, rows) = (w / block_size, h / block_size);
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        let mut last_displacements = vec![1.0; cols * rows];
        let mut last_simulated_ticks = vec![0; cols * rows];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for t in 0..1500 {
            settle_tick(
                &mut hm, &mut temp_heights, &mut cell_colors, &mut cell_props,
                &mut sliding, &mut bounds, &mut active_blocks, &mut last_displacements,
                &mut last_simulated_ticks, cols * rows, block_size, &[], t as u32,
                &mut edge_vel_h,
                &mut edge_vel_v, &mask, t as u32, gravity_dir,
            );
        }
        let outside: f32 = (0..w * h).filter(|&i| mask[i] == crate::MASK_OUTSIDE).map(|i| hm.data[i]).sum();
        let total: f32 = hm.data.iter().sum();
        println!(
            "outside_mask_mass={:.4} total={:.4} frac={:.4}%",
            outside, total, 100.0 * outside / total
        );

        assert!(
            outside < 1e-3,
            "Mass leaked into MASK_OUTSIDE (wall) cells: {:.4} of {:.4} total ({:.2}%). \
             Those cells are never simulated again, so this mass is frozen inside a wall forever.",
            outside,
            total,
            100.0 * outside / total
        );
    }
}
