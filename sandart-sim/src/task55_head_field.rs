//! Task #55, step 2: a STATIC hydraulic head field.
//!
//! `task55_head_spec.rs` (this module's sibling) is the isolation spec this file exists to
//! satisfy. Read that file's module doc comment first for the physics and the two measured
//! defects in today's `column_depth`-derived head: (1) pressure never propagates laterally out
//! of its own column, and (2) the field never asks whether anything supports a cell from below.
//!
//! # The physics
//!
//! Hydraulic head `head = z + p/(rho*g)`. For a body of liquid at rest, head is constant
//! throughout the body and equals its free-surface elevation. That makes the static field a
//! Laplace problem on the wet graph with Dirichlet boundary conditions wherever pressure is
//! known to be zero:
//!
//! - A cell with an **exposed top face** (nothing wet-and-in-mask directly above it -- the free
//!   surface of its own column) has `p = 0` there, so its head is pinned to its own surface
//!   elevation.
//! - A cell with **nothing supporting it from below** (`support_fraction`, #58) is in free fall,
//!   so `p = 0` there too, for exactly the same reason.
//!
//! **These are the SAME condition** -- pressure at an exposed face is zero -- whether that face
//! is the cell's top (open to air) or its bottom (open to empty space below). This module
//! recognises that structurally: a single `effective_support` scalar per cell decides whether it
//! is Dirichlet-pinned (`0.0`) or free (`1.0`, relaxing to the average of its wet neighbours, or
//! blended in between). But `p` itself is always measured at a cell's BOTTOM face (`head_at`'s
//! own convention: `p := head - z`, `z(idx) = -row(idx) * depth_scale`), so the two zero-pressure
//! conditions pin to DIFFERENT numeric targets, because they zero pressure at DIFFERENT
//! geometric faces of the same cell: an exposed top means nothing ABOVE adds overburden, but the
//! cell's own weight still bears on its own bottom face (`p = heights[idx] * depth_scale`, not
//! zero -- confirmed against the legacy field, which already gets this right for a resting
//! column's topmost cell), so it pins to `own_elev = z + heights * depth_scale`. Nothing below to
//! receive that weight instead zeroes `p` at the cell's own bottom face DIRECTLY, so it pins to
//! `z` alone. When both are true at once (support is genuinely zero regardless of what sits
//! above), the zero-support target wins: it is the more local, more direct statement about that
//! specific face. `effective_support == 0.0` is the one Dirichlet branch (with either target);
//! `1.0` is a fully free interior node; anything between blends toward whichever target applies,
//! so a partially supported cell partially transmits pressure and partially free-falls, with no
//! separate code path for either extreme.
//!
//! A **solid roof** (`shape_mask == MASK_OUTSIDE`) directly above a cell is NOT an exposed top:
//! the cell is pressed against a wall, not open to atmosphere. Getting this distinction right is
//! the entire content of `spec_pascal_under_a_roof` -- a channel cell under a roof must relax
//! (propagate the shaft's head inward), not pin to its own low elevation.
//!
//! Every other wet cell relaxes toward the average of its connected wet neighbours (4-neighbour
//! Laplace over cells that both hold material and are both in-mask). At equilibrium, a connected
//! body whose only Dirichlet pins agree on one elevation settles to that single uniform head --
//! Pascal's principle, obtained by construction, not special-cased. A siphon or a roofed channel
//! then needs no code of its own.
//!
//! # What this does NOT do
//!
//! This moves no mass and does no clamping. It is a scalar relaxation over a field that is
//! returned to the caller, never fed back into `heights` or any other simulation state. No
//! existing physics function is read for anything other than `support_fraction` (read-only,
//! unmodified) and no existing function is called that itself moves material.
//!
//! # Convergence
//!
//! Plain (non-accelerated) Gauss-Seidel needs `O(w + h)` sweeps to propagate a boundary
//! condition across the whole domain, which is too slow to be worth paying at production
//! resolution without acceleration. This uses Successive Over-Relaxation (SOR) with the
//! classical optimal-omega formula for a Poisson problem on a grid of the given extent
//! (`omega = 2 / (1 + sin(pi / (max(w,h) + 1)))`), which cuts that to roughly `O(sqrt(w + h))`
//! sweeps, alternating sweep direction each iteration (as `elliptic_liquid_level_pass` already
//! does, for the same reason: a change at either end of the graph reaches the other end sooner
//! than sweeping one direction only). Dirichlet-pinned cells (`effective_support == 0.0`) are
//! written directly to their own elevation every sweep rather than left to relax toward it --
//! true boundary conditions should not lag behind the interior they are driving.
//!
//! `compute_head_field_with_stats` iterates until the largest per-sweep change drops below a
//! tolerance two orders of magnitude tighter than the spec's own `identity_tol` and returns the
//! sweep count and final residual for diagnostics; `compute_head_field` is the plain pure
//! function the brief specifies, which just discards those two numbers.

use super::*;

/// Cells with fill at or below this are treated as empty air, not material -- matches the
/// `ELLIPTIC_WET_EPS`-style epsilon used elsewhere in this file for the same purpose, but kept
/// local (not imported) since this module must not perturb anything else's behaviour.
const HEAD_FIELD_WET_EPS: f32 = 1e-6;

/// Converge until the largest per-sweep change is below this fraction of `depth_scale` -- roughly
/// one and a half orders of magnitude under `task55_head_spec::identity_tol`'s own
/// `0.02 * depth_scale`, so a spec that reads this field is never limited by relaxation residual
/// rather than by genuine physics. Measured directly: SOR on the (deliberately non-uniform-
/// boundary) free-fall-slab scenario plateaus around `~5e-4 * depth_scale` at `w = 512` --
/// consistent with the f32 noise floor at head magnitudes around `REFERENCE_GRID_HEIGHT` (roughly
/// `500 * f32::EPSILON ~= 6e-5`, times some constant-factor headroom for SOR's alternating
/// over/undershoot) rather than genuine further convergence -- so the target sits above that
/// floor with margin instead of chasing a residual f32 arithmetic cannot actually deliver.
const CONVERGENCE_TOL_FRACTION: f32 = 1e-3;

/// Safety cap on sweep count, sized generously above what the SOR omega below is expected to
/// need (`O(sqrt(w + h))`) so a scenario that fails to converge is caught by the `debug_assert`
/// below rather than looping unboundedly.
const MAX_SWEEPS_FACTOR: usize = 8;

/// Task #55, step 2. Static hydraulic head per cell, in the same reference-row units
/// `task55_head_spec::head_at` already uses (`z(i) = -row(i) * depth_scale`). A pure function:
/// no field on `DrawingSimulation` is read or written, nothing here is wired into `settle_tick`.
/// See this file's module doc comment for the physics.
pub(crate) fn compute_head_field(
    w: usize,
    h: usize,
    shape_mask: &[u8],
    heights: &[f32],
    cell_props: &[f32],
) -> Vec<f32> {
    compute_head_field_with_stats(w, h, shape_mask, heights, cell_props).0
}

/// Same computation as `compute_head_field`, additionally returning `(sweeps, final_residual)`
/// -- the largest per-cell change made by the sweep that triggered convergence (or the cap), in
/// the same reference-row head units the returned field itself is in. Exists so diagnostics and
/// the spec harness can report real convergence numbers instead of assuming them.
pub(crate) fn compute_head_field_with_stats(
    w: usize,
    h: usize,
    shape_mask: &[u8],
    heights: &[f32],
    cell_props: &[f32],
) -> (Vec<f32>, usize, f32) {
    let cell_count = w * h;
    let mut head = vec![0.0f32; cell_count];
    if w < 3 || h < 3 || cell_count == 0 {
        return (head, 0, 0.0);
    }
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    let is_inside = |idx: usize| shape_mask[idx] != crate::MASK_OUTSIDE;

    // `z_elev[idx]`: `head_at`'s own datum, `z(idx) = -row(idx) * depth_scale` -- the elevation
    // of cell `idx`'s BOTTOM face (the boundary shared with the cell below), which is exactly
    // the face `pressure_at`/`head_at`'s `p := head - z` convention measures pressure at (a
    // resting column's topmost cell reads `p = heights * depth_scale`, its own weight bearing on
    // its own bottom face, not zero -- confirmed against the legacy field, which already passes
    // `spec_uniform_head_in_resting_open_column`/`spec_pressure_is_linear_in_depth`).
    //
    // `own_elev[idx] = z_elev[idx] + heights[idx] * depth_scale`: the elevation of the TOP of
    // this cell's own material -- `head_at`'s formula with `column_depth == 0`. This is the
    // correct Dirichlet target for an EXPOSED TOP: nothing above means zero overburden, but this
    // cell's own weight still bears on its own bottom face, so `p` there is exactly one cell's
    // worth, not zero.
    // --- Transitive support (Task #55 step 2b). `support_fraction` (#58, shipped, unmodified --
    // read here, never re-derived) looks exactly ONE cell down, so it is not transitive: in a
    // slab of water falling through air, only the BOTTOM row reads as unsupported, because every
    // cell above it is "resting on" a cell below that is itself full of material. That is exactly
    // the gap `spec_free_fall_is_pressureless_throughout` measures (1.000 cells of residual
    // pressure at the slab's TOP row -- backwards-signed for a resting body, whose pressure
    // maximum sits at its base, not its crown). Fix: propagate support UPWARD through each
    // column, bottom-up, so "a cell resting on falling material is itself falling":
    //
    //   effective_support_transitive[idx] =
    //       support_fraction(idx)                                       , below is out of mask
    //       min(support_fraction(idx), effective_support_transitive[below])   , otherwise
    //
    // `support_fraction` already returns `1.0` for BOTH "out of mask below" cases -- off-grid
    // below (the world's own floor) and `MASK_OUTSIDE` below (casing/walls) -- so both collapse
    // into one plain read of it; no separate mask check duplicates its own branching, and its
    // third internal fallback branch (degenerate zero capacity below) is left alone too: it
    // already returns `1.0`, which composes correctly through the `min` below with no special
    // case needed here.
    //
    // MIN, not a product. The chain is only as strong as its weakest link: a resting column N
    // cells tall has `support_fraction == 1.0` at every link (each cell sits on a fully-packed
    // cell below), so `min` carries `1.0` all the way to its top -- required by
    // `spec_uniform_head_in_resting_open_column`. A PRODUCT of N terms at or near `1.0` would
    // decay through a tall resting column via nothing but repeated multiplication and wrongly
    // report cells near its top as unsupported, breaking that spec for no physical reason: a
    // column standing on solid ground is not "less supported" the taller it is. Do not
    // "simplify" this to a product.
    //
    // SCOPE LIMIT: this is a per-COLUMN upward propagation -- correct for the free-fall case this
    // task fixes, but NOT a general support model. It captures no LATERAL support: material
    // resting on a ledge to its side, or held up in an arch, is supported by something that is
    // not directly beneath it, and this pass is blind to that entirely. That is a
    // granular-mechanics concern (force chains, arching) belonging to the solids half of task
    // #55, not this liquid head field.
    //
    // DUPLICATION, noted deliberately: the LOD scheduler's `fresh_overburden_must_blocks`
    // (physics.rs:1001) answers a related but different question -- "might anything in this
    // 32x32 block move?", a boolean (`unsupported AND has_room_to_move`) -- and gets transitivity
    // for free over TIME via `activate_neighbor_upstream` (physics.rs:3896), which wakes the
    // block one step upstream each tick. A one-tick lag there is harmless for scheduling (the
    // block simply wakes up next tick regardless); it would be fatal here, where the field must
    // be correct WITHIN the tick it is read. Hence this separate, synchronous, whole-column pass
    // rather than reusing that mechanism.
    let mut effective_support_transitive = vec![1.0f32; cell_count];
    for y in (0..h).rev() {
        let row = y * w;
        for x in 0..w {
            let idx = row + x;
            if !is_inside(idx) {
                continue; // never read: `wet[idx]` requires `is_inside(idx)` too.
            }
            let raw = support_fraction(idx, w, h, heights, cell_props, shape_mask);
            let below_out_of_mask = y + 1 >= h || shape_mask[idx + w] == crate::MASK_OUTSIDE;
            effective_support_transitive[idx] = if below_out_of_mask {
                raw
            } else {
                raw.min(effective_support_transitive[idx + w])
            };
        }
    }

    let mut z_elev = vec![0.0f32; cell_count];
    let mut own_elev = vec![0.0f32; cell_count];
    for y in 0..h {
        let row = y * w;
        let z = -(y as f32) * depth_scale;
        for x in 0..w {
            let idx = row + x;
            z_elev[idx] = z;
            own_elev[idx] = heights[idx] * depth_scale + z;
        }
    }
    head.copy_from_slice(&own_elev);

    // Domain + boundary-condition classification. `wet[idx]`: holds material and is in-mask, so
    // it participates in the relaxation at all. `effective_support[idx]`: `0.0` means Dirichlet-
    // pinned to `pin_target[idx]`; `1.0` means a fully free interior node; anything between
    // blends the two. `pin_target[idx]` is only meaningful where `effective_support[idx] < 1.0`.
    //
    // THE UNIFICATION, precisely: an exposed top and nothing supporting a cell from below are
    // both "pressure at an exposed face is zero," but they are exposed faces on OPPOSITE sides
    // of the same cell, so in the single `p := head - z_elev` convention (always measured at a
    // cell's BOTTOM face) they pin to DIFFERENT targets. Zero support means `p` AT THIS CELL'S
    // OWN bottom face is zero directly -- `head == z_elev[idx]` -- and that is the stronger,
    // more local fact: it wins even over an exposed top on the SAME cell (nothing below to press
    // against is true regardless of what is or isn't piled on top). An exposed top with
    // something genuine below it (`support_fraction > 0`) instead means this cell's own weight
    // does reach its bottom face with nothing extra added from above, i.e. `head == own_elev`.
    // Both cases still collapse into the SAME `effective_support == 0.0` Dirichlet branch of the
    // one blend formula below; only the target value differs, and both targets are read off
    // `support_fraction` (#58, unmodified) and the exposed-top test alone -- no reimplementation
    // of either.
    let mut wet = vec![false; cell_count];
    let mut effective_support = vec![0.0f32; cell_count];
    let mut pin_target = vec![0.0f32; cell_count];
    let mut wet_order: Vec<usize> = Vec::new();
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let idx = row + x;
            if !is_inside(idx) || heights[idx] <= HEAD_FIELD_WET_EPS {
                continue;
            }
            wet[idx] = true;
            wet_order.push(idx);

            // Exposed top: no cell above at all (the world's own top boundary counts as open
            // atmosphere), or the cell above is in-mask and empty (open air/void). A cell capped
            // by a SOLID roof (`shape_mask[above] == MASK_OUTSIDE`) is excluded here on purpose
            // -- it is pressed against a wall, not free to atmosphere, which is exactly the
            // distinction `spec_pascal_under_a_roof` exists to force.
            let exposed_top = if idx < w {
                true
            } else {
                let above = idx - w;
                is_inside(above) && heights[above] <= HEAD_FIELD_WET_EPS
            };

            // `effective_support_transitive[idx]`, not a fresh one-cell `support_fraction` call:
            // see that array's own computation above for why the raw one-cell lookback is not
            // enough on its own (support is not transitive without it).
            let raw_support = effective_support_transitive[idx];
            if raw_support <= 0.0 {
                // Nothing below at all: zero pressure at THIS cell's own bottom face, regardless
                // of what sits above it.
                effective_support[idx] = 0.0;
                pin_target[idx] = z_elev[idx];
            } else if exposed_top {
                // Something genuine below to receive this cell's own weight, and nothing above
                // adding overburden: `p` at the bottom face is exactly this cell's own weight.
                effective_support[idx] = 0.0;
                pin_target[idx] = own_elev[idx];
            } else {
                effective_support[idx] = raw_support;
                pin_target[idx] = own_elev[idx]; // unused when raw_support >= 1.0 (pure interior)
            }
        }
    }

    if wet_order.is_empty() {
        return (head, 0, 0.0);
    }

    let wet_order_rev: Vec<usize> = wet_order.iter().rev().copied().collect();

    // Coarse pass: CONNECTIVITY ONLY (`uf_union`/`uf_find`, physics.rs's own union-find
    // primitives -- reused unmodified, per the task brief, for exactly the connectivity role
    // they already play in `elliptic_liquid_level_pass`'s coarse step; nothing here moves any
    // mass). Plain, unaccelerated relaxation on a chain needs O((w+h)^2) sweeps to propagate a
    // single boundary value across it -- measured directly: a 64-row resting-column scenario
    // still sat at ~6x this field's own convergence tolerance after 1000+ SOR sweeps, because
    // one Dirichlet pin has to diffuse down a long chain of purely-averaging interior nodes one
    // step of "information" per sweep. But Laplace's equation with a single-valued Dirichlet
    // boundary on a connected domain has exactly ONE solution: that value, everywhere -- Pascal's
    // principle again. So: union every pair of adjacent wet cells (the same graph the fine sweep
    // below uses), and for every component whose Dirichlet-pinned members (`effective_support ==
    // 0.0`) all agree on the same `pin_target`, jump every fully-interior member
    // (`effective_support == 1.0`) directly to it. That turns the slow case into an O(cells)
    // direct assignment. A component with disagreeing Dirichlet values (the free-fall slab: one
    // pin at its exposed top, a different one at its unsupported bottom) gets no shortcut and
    // falls through to the fine sweep below -- correctly: that chain's fixed point is a per-row
    // LINEAR interpolation between the two, not a constant, which is exactly what plain
    // relaxation converges to (fast, in practice, since a uniform-height column's own elevation
    // is already that exact linear function -- see this module's own report for the measured
    // sweep counts). Partially supported cells (`0 < effective_support < 1`) are never jumped --
    // their equilibrium genuinely depends on their own elevation, not only the component's
    // boundary value -- but still start the fine sweep from a far better neighbourhood than
    // "everywhere = own elevation," since every jumped neighbour is already exactly right.
    let mut uf_parent: Vec<usize> = (0..cell_count).collect();
    for &idx in &wet_order {
        let x = idx % w;
        let y = idx / w;
        if x + 1 < w && wet[idx + 1] {
            uf_union(&mut uf_parent, idx, idx + 1);
        }
        if y + 1 < h && wet[idx + w] {
            uf_union(&mut uf_parent, idx, idx + w);
        }
    }
    // Per-component-root Dirichlet value: `Some(v)` while every Dirichlet member seen so far
    // agrees on `v`; `None` once two disagree (mixed boundary, no shortcut for this component).
    let mut component_value: std::collections::HashMap<usize, Option<f32>> =
        std::collections::HashMap::new();
    for &idx in &wet_order {
        if effective_support[idx] > 0.0 {
            continue; // not a Dirichlet node
        }
        let root = uf_find(&mut uf_parent, idx);
        let v = pin_target[idx];
        component_value
            .entry(root)
            .and_modify(|slot| {
                if let Some(existing) = *slot {
                    if (existing - v).abs() > 1e-3 * depth_scale {
                        *slot = None;
                    }
                }
            })
            .or_insert(Some(v));
    }
    for &idx in &wet_order {
        if effective_support[idx] < 1.0 {
            continue; // only fully-interior nodes are safe to jump directly
        }
        let root = uf_find(&mut uf_parent, idx);
        if let Some(Some(v)) = component_value.get(&root) {
            head[idx] = *v;
        }
    }

    // Per-component bounding-box extent, for a per-cell SOR omega below. A component with a
    // single-valued Dirichlet boundary was just jumped directly to its answer above (converged
    // in one sweep, omega irrelevant to it); the ONLY cells left for the fine sweep to actually
    // move are inside components with disagreeing Dirichlet values (the free-fall slab: exposed
    // top vs. unsupported bottom) or partial support. Sizing omega off the FULL GRID (`w.max(h)`)
    // over-relaxes a small isolated component -- measured directly: a ~26-row free-fall slab on
    // a 256x256 grid failed to reach this function's own convergence tolerance within an 8x(w+h)
    // sweep budget using the whole-grid omega, because that omega assumes a problem roughly as
    // large as the grid itself. Using each component's OWN bounding-box extent instead sizes the
    // acceleration to the problem actually being solved.
    let mut comp_bounds: std::collections::HashMap<usize, (usize, usize, usize, usize)> =
        std::collections::HashMap::new(); // root -> (min_x, min_y, max_x, max_y)
    for &idx in &wet_order {
        let root = uf_find(&mut uf_parent, idx);
        let x = idx % w;
        let y = idx / w;
        comp_bounds
            .entry(root)
            .and_modify(|(min_x, min_y, max_x, max_y)| {
                *min_x = (*min_x).min(x);
                *min_y = (*min_y).min(y);
                *max_x = (*max_x).max(x);
                *max_y = (*max_y).max(y);
            })
            .or_insert((x, y, x, y));
    }
    // Safe to read `uf_parent` directly (no further `uf_find` needed) from here on: the
    // `comp_bounds` loop just above called `uf_find` on every wet cell in `wet_order`, so every
    // one of them is already path-compressed straight to its root.
    let omega_for = |idx: usize| -> f32 {
        let root = uf_parent[idx];
        let (min_x, min_y, max_x, max_y) = comp_bounds.get(&root).copied().unwrap_or((0, 0, 0, 0));
        let root_dim = ((max_x - min_x + 1).max(max_y - min_y + 1)) as f32;
        (2.0 / (1.0 + (std::f32::consts::PI / (root_dim + 1.0)).sin())).clamp(1.0, 1.97)
    };
    // Dense lookup, computed once: the hot sweep loop below indexes this instead of repeating
    // the `HashMap` lookup above per cell per sweep.
    let mut omega_field = vec![1.0f32; cell_count];
    for &idx in &wet_order {
        omega_field[idx] = omega_for(idx);
    }

    let converge_tol = CONVERGENCE_TOL_FRACTION * depth_scale;
    let max_sweeps = MAX_SWEEPS_FACTOR * (w + h) + 64;

    let mut sweeps = 0usize;
    let mut residual = f32::INFINITY;
    while sweeps < max_sweeps {
        let forward = sweeps % 2 == 0;
        let order: &[usize] = if forward { &wet_order[..] } else { &wet_order_rev[..] };
        let mut max_delta = 0.0f32;
        for &idx in order {
            let x = idx % w;
            let y = idx / w;
            let old = head[idx];
            let s = effective_support[idx];
            let new_val = if s <= 0.0 {
                // A true Dirichlet boundary condition: written directly every sweep rather than
                // left to relax toward it, so it never lags the interior it is driving.
                pin_target[idx]
            } else {
                let mut sum = 0.0f32;
                let mut count = 0u32;
                if x > 0 && wet[idx - 1] {
                    sum += head[idx - 1];
                    count += 1;
                }
                if x + 1 < w && wet[idx + 1] {
                    sum += head[idx + 1];
                    count += 1;
                }
                if y > 0 && wet[idx - w] {
                    sum += head[idx - w];
                    count += 1;
                }
                if y + 1 < h && wet[idx + w] {
                    sum += head[idx + w];
                    count += 1;
                }
                let avg = if count > 0 { sum / count as f32 } else { pin_target[idx] };
                let target = s * avg + (1.0 - s) * pin_target[idx];
                old + omega_field[idx] * (target - old)
            };
            let delta = (new_val - old).abs();
            if delta > max_delta {
                max_delta = delta;
            }
            head[idx] = new_val;
        }
        sweeps += 1;
        residual = max_delta;
        if max_delta < converge_tol {
            break;
        }
    }

    debug_assert!(
        residual < converge_tol,
        "compute_head_field: relaxation failed to converge within {max_sweeps} sweeps \
         (w={w} h={h}, {} wet cells): residual={residual} tol={converge_tol}",
        wet_order.len()
    );

    (head, sweeps, residual)
}
