//! Task #55, step 2 (REBUILT AGAIN): a hydraulic head field computed by MAX-PROPAGATION.
//!
//! `task55_head_spec.rs` (this module's sibling) is the isolation spec this file exists to
//! satisfy. Read that file's module doc comment first for the physics and the two measured
//! defects in today's `column_depth`-derived head: (1) pressure never propagates laterally out
//! of its own column, and (2) the field never asks whether anything supports a cell from below.
//!
//! # The physics
//!
//! Hydraulic head `head = z + p/(rho*g)`. For a body of liquid at rest, head is constant
//! throughout the body and equals its free-surface elevation. That makes the field, at
//! equilibrium, the solution to a Laplace problem on the wet graph with Dirichlet boundary
//! conditions wherever pressure is known to be zero:
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
//! is Dirichlet-pinned (`0.0`) or free (`1.0`, taking the max over its wet neighbours, or
//! blended in between). But `p` itself is always measured at a cell's BOTTOM face (`head_at`'s
//! own convention: `p := head - z`, `z(idx) = -row(idx) * depth_scale`), so the two zero-pressure
//! conditions pin to DIFFERENT numeric targets, because they zero pressure at DIFFERENT
//! geometric faces of the same cell: an exposed top means nothing ABOVE adds overburden, but the
//! cell's own weight still bears on its own bottom face (`p = heights * depth_scale`, not
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
//! Every other wet cell takes the MAX of its own local hydrostatic head and the head of its
//! connected wet neighbours (4-neighbour, over cells that both hold material and are both
//! in-mask). A connected body whose only Dirichlet pins agree on one elevation lands on that
//! single uniform head -- Pascal's principle, obtained by construction, not special-cased. A
//! siphon or a roofed channel then needs no code of its own.
//!
//! # What this does NOT do
//!
//! This moves no mass and does no clamping. `advance_head_field` mutates only the persistent
//! `head` buffer the caller passes it (`DrawingSimulation::head_field` in production; a plain
//! local `Vec` in the spec harness) -- it never touches `heights` or any other simulation state,
//! and no existing physics function is read for anything other than `support_fraction`
//! (read-only, unmodified).
//!
//! # Redesign: MAX-propagation, not averaging
//!
//! Two designs came before this one and both failed for the same underlying reason, so the reason
//! is worth stating before the fix.
//!
//! The FIRST version (`compute_head_field`) solved the Laplace problem to convergence from
//! scratch on every call, via SOR with a per-component coarse union-find jump. It did not
//! converge at production scale: a w=512 draining vessel (47315 wet cells) still sat at residual
//! 0.0436 against a 0.001 tolerance after 8256 sweeps. The union-find jump hid this, because it
//! only ever fires when EVERY Dirichlet pin in a component agrees -- i.e. exactly the trivial
//! case. Every interesting configuration has disagreeing pins by construction and fell through to
//! plain SOR. The accelerator accelerated only the case that did not need accelerating.
//!
//! The SECOND version kept the averaging update but stopped demanding convergence within a call,
//! carrying the field tick to tick as persistent state and running a fixed sweep budget per tick.
//! That fixed the *requirement* without fixing the *rate*, and made the rate visible: at w=512 the
//! textbook base case (`spec_uniform_head_in_resting_open_column`, one flat pin over a free
//! column) was still 47.9 reference-rows off after the spec harness's whole tick budget.
//!
//! THE REASON BOTH FAILED: averaging is DIFFUSION. A Gauss-Seidel/SOR sweep drives the field by
//! the discrete Laplacian, whose settling time over an `N`-cell chain is `O(N^2)` sweeps -- not
//! the `O(N)` wavefront-arrival time the sweep-count argument was built on. Those are two
//! different quantities and conflating them is what made `8` sweeps/tick look sufficient. At
//! `N = 512` the gap between them is a factor of 512.
//!
//! THE FIX IS THE OPERATOR, NOT THE SOLVER. Head does not obey a Laplace equation on the wet
//! graph. At rest it is CONSTANT through a connected body, equal to that body's free-surface
//! elevation -- which is a MAXIMUM statement, not an averaging one. So the update is a max:
//!
//! ```text
//! head[i] = max( own_local_hydrostatic[i], max over connected wet neighbours j of head[j] )
//! ```
//!
//! That is Bellman-Ford, not Jacobi. It converges in `O(graph diameter)` sweeps -- and better
//! than that in practice, because a Gauss-Seidel-ordered max sweep propagates a value arbitrarily
//! far ALONG THE SWEEP DIRECTION in a single pass (the same mechanism that makes a two-pass
//! chamfer distance transform exact). The cost is not the number of cells on the path, it is the
//! number of times the path REVERSES direction: a straight column needs one sweep, a U-tube about
//! two. `O(N^2)` sweeps becomes `O(1)`-ish sweeps. Three orders of magnitude, from changing the
//! operator and nothing else.
//!
//! There is no `omega` here, and there must not be: over-relaxation is an averaging-solver
//! acceleration and has no meaning for a max. Extrapolating past a max produces a value no
//! neighbour holds.
//!
//! ## Why the self-term is LOCAL, never HISTORY
//!
//! `max` is monotone. If `head[i]` took the max against its OWN PREVIOUS VALUE, the field could
//! only ever ratchet upward: drain a reservoir and every cell it once fed keeps that head
//! forever. So the field is RE-SEEDED from local geometry at the top of every call, and the
//! previous tick's values are never an input to the max.
//!
//! This is what makes falling pressure work, and it is worth being explicit about because it is
//! the property the averaging design never had. The max over a connected component is attained at
//! that component's HIGHEST free surface; nothing else in the component can exceed it. Lower that
//! surface and its `own_elev` drops, and since no cell is holding the old value up, the whole
//! component follows in the same one-to-few sweeps a RISE would take. Decrease propagates exactly
//! as fast as increase.
//!
//! The consequence for the caller: `head` is a scratch buffer, not carried state. It stays a
//! caller-owned persistent allocation (so this stays allocation-free per tick, and so
//! `head_field_to_pressure` has something to read between calls), but every wet cell in it is
//! overwritten on entry. The field is therefore a PURE FUNCTION of the current mask + heightmap +
//! material -- no hysteresis, no path dependence, and in particular no dependence on material
//! FLOW. Pressure is geometry.
//!
//! ## Why the pins do NOT take the max
//!
//! Dirichlet-pinned cells are WRITTEN, not maxed. This is load-bearing twice over.
//!
//! It is what keeps free fall pressureless: a falling cell adjacent to a supported column would
//! otherwise inherit that column's head through the max. Being written every sweep, it cannot.
//!
//! And it is what makes anything move at all. If every cell took the max, head would be uniform
//! across each connected body, `grad(head)` would be zero everywhere, and the transport step this
//! field exists to drive would have nothing to read. The gap between a pinned LOW free surface and
//! the HIGH interior beneath it IS the driving head -- and, in a U-tube, IS the siphon.

use super::*;

/// Cells with fill at or below this are treated as empty air, not material -- matches the
/// `ELLIPTIC_WET_EPS`-style epsilon used elsewhere in this file for the same purpose, but kept
/// local (not imported) since this module must not perturb anything else's behaviour.
const HEAD_FIELD_WET_EPS: f32 = 1e-6;

/// UPPER BOUND on the max-propagation sweeps `advance_head_field` runs per call. This is a CAP on
/// a loop that exits as soon as a sweep changes nothing, NOT a fixed budget that is always spent:
/// under the max operator the typical cost is 2-3 sweeps (see the module doc comment), and the
/// early exit is what keeps it there. Under the deleted averaging design the same name meant a
/// fixed spend, because averaging never reached a sweep that changed nothing.
///
/// WHY A CAP IS THE RIGHT SHAPE, given the field must converge WITHIN one call: the previous
/// design deliberately did not converge per call, treating a partially-relaxed field as the
/// normal mid-relaxation state and letting equilibrium arrive over many ticks. Max-propagation
/// makes that unnecessary AND unsafe. Unnecessary because convergence now costs a handful of
/// sweeps rather than `O(N^2)` of them. Unsafe because the field is re-seeded from local geometry
/// every call (module doc comment, "the self-term is LOCAL, never HISTORY"), so ticks no longer
/// ACCUMULATE progress -- a call that stopped short would throw its own partial answer away, and
/// looping more ticks would never finish what one tick left undone. Converge here or not at all.
///
/// WHY 32: sweeps alternate direction (forward raster, then reverse), and what a sweep costs is
/// one traversal of the wet set, so the real question is how many DIRECTION REVERSALS the longest
/// connectivity path in a scenario contains -- not how many cells it has. A resting column is 1,
/// a U-tube about 2, and a pathological serpentine cave channel is bounded by its number of
/// switchbacks. `32` is well above anything the shipped container shapes or the procedural cave
/// generator produce, while still bounding the worst case at a cost comparable to the fixed `8`
/// the previous design spent unconditionally on EVERY call. It is a safety bound, not a tuning
/// knob: no spec's pass/fail boundary sits near it, and the measured sweep counts (see this
/// task's report) sit an order of magnitude below.
///
/// Cost is `O(wet_cells)` per sweep, independent of grid resolution.
pub(crate) const HEAD_FIELD_SWEEPS_PER_TICK: usize = 32;

/// A sweep whose largest single-cell change is at or below this (as a fraction of `depth_scale`)
/// counts as having changed nothing, ending the sweep loop. Under a pure max over a fixed set of
/// values the converged state is reached EXACTLY (values are copied, not blended, so the final
/// sweep's delta is bit-for-bit `0.0`); this is nonzero only because partially supported cells
/// (`0 < effective_support < 1`) blend their neighbour max toward their pin target, which is real
/// arithmetic and can leave a last-bit wobble. Two orders of magnitude under `identity_tol`'s own
/// `0.02 * depth_scale`, so no spec is ever limited by this rather than by physics.
const HEAD_FIELD_SWEEP_SETTLE_FRACTION: f32 = 1e-4;

/// Recompute the hydraulic head field `head` for this tick: classify Dirichlet pins (exposed-top
/// / unsupported-bottom, transitive support -- see the module doc comment), seed every wet cell
/// from its own local hydrostatic head, then max-propagate to convergence (up to
/// `HEAD_FIELD_SWEEPS_PER_TICK` sweeps).
///
/// `head` is WRITTEN, not read: every wet cell is overwritten by the seed on entry, so the
/// previous call's values never influence this one. It remains a caller-owned persistent
/// allocation purely so this stays allocation-free per tick and so `head_field_to_pressure` has
/// something to read between calls -- NOT because it carries state. See the module doc comment
/// ("the self-term is LOCAL, never HISTORY") for why reading it back would make the field ratchet
/// upward and never fall.
///
/// `head.len()` must already equal `w * h`; callers own resizing/reinitialising the persistent
/// buffer (`DrawingSimulation::head_field` resizes and zero-fills exactly where `column_depth`
/// does -- construction, `reset()`, and a grid-size change; see those call sites). A length
/// mismatch here is a caller bug, not a normal runtime condition, so it is a `debug_assert!`
/// (kept -- unlike the deleted convergence assert, this one guards a programming invariant this
/// module's own contract requires, not a physical property that is expected to take many ticks to
/// approach) with a safe no-op fallback in release.
///
/// Returns the largest single-cell change made by this tick's relaxation, in the same
/// reference-row head units `head` itself is in. Diagnostic only -- nothing in this module or its
/// caller requires this to reach any particular value before the next tick; see the module doc
/// comment on why convergence is never something this function waits for.
pub(crate) fn advance_head_field(
    w: usize,
    h: usize,
    shape_mask: &[u8],
    heights: &[f32],
    cell_props: &[f32],
    head: &mut [f32],
) -> f32 {
    let cell_count = w * h;
    debug_assert_eq!(
        head.len(),
        cell_count,
        "advance_head_field: `head` (len {}) must already be sized to w*h ({}) -- caller owns \
         resizing the persistent buffer, see this function's own doc comment",
        head.len(),
        cell_count
    );
    if w < 3 || h < 3 || cell_count == 0 || head.len() != cell_count {
        return 0.0;
    }
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    let is_inside = |idx: usize| shape_mask[idx] != crate::MASK_OUTSIDE;

    // --- Transitive support (unchanged from the previous version of this module; see the module
    // doc comment's own explanation of why this is bottom-up-per-column and MIN, not a product).
    // `support_fraction` (#58, shipped) looks exactly one cell down, so it is not transitive on
    // its own: in a slab of water falling through air, only the bottom row reads as unsupported,
    // because every cell above it is "resting on" a cell below that is itself full of material.
    // Propagating support upward through each column bottom-up fixes that: "a cell resting on
    // falling material is itself falling."
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

    // `z_elev[idx]`: `head_at`'s own datum, `z(idx) = -row(idx) * depth_scale` -- the elevation
    // of cell `idx`'s BOTTOM face. `own_elev[idx] = z_elev[idx] + heights[idx] * depth_scale`:
    // the elevation of the top of this cell's own material -- the correct Dirichlet target for an
    // EXPOSED TOP (see module doc comment).
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

    // Domain + boundary-condition classification -- unchanged physics from the previous version
    // of this module (see the module doc comment for the exposed-top / unsupported-bottom
    // unification). `wet[idx]`: holds material and is in-mask. `effective_support[idx]`: `0.0`
    // means Dirichlet-pinned to `pin_target[idx]`; `1.0` means a fully free interior node;
    // anything between blends the two.
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

            // THERE IS DELIBERATELY NO "EXPOSED TOP" PIN HERE. An earlier version pinned any cell
            // whose top face was open to air at `own_elev = z + heights * depth_scale`. That pin
            // was REDUNDANT at equilibrium and ACTIVELY WRONG in transit, and deleting it is what
            // lets a free surface rise:
            //
            //   REDUNDANT, because `own_elev` is already this cell's self-term in the max below,
            //   and for a column's topmost cell it is the largest `own_elev` in that column --
            //   i.e. the free-surface elevation. A resting column therefore relaxes to exactly the
            //   value the pin used to impose, with the pin gone. Every static spec measures an
            //   equilibrium, which is why all seven passed either way and none of them caught this.
            //
            //   WRONG IN TRANSIT, because a Dirichlet pin is WRITTEN rather than maxed, so it also
            //   PREVENTED a surface cell from ever reading a higher head from the body beneath it.
            //   For a full cell `own_elev = z + depth_scale`, which is EXACTLY the `z` of the air
            //   cell directly above (dry cells hold `head = z`, air pressure being zero). So the
            //   driving head across every water/air interface in the domain was identically
            //   `z_air - own_elev = 0` -- not small, not resolution-dependent, but structurally
            //   zero. Material could be pushed UP TO a free surface and never THROUGH it, so no
            //   siphon could ever climb and no surface could ever rise. Measured on the U-tube at
            //   w=512: the cell one below the right arm's surface carried the reservoir's head
            //   (-204, a drive of -236 across the submerged edge), while the surface edge itself
            //   read exactly 0.00.
            //
            // The `p = 0` boundary condition belongs to the ATMOSPHERE, not to the topmost water
            // cell, and the atmosphere already carries it: a dry cell holds `head = z`, so the
            // interface now compares the body's head against the air cell's own elevation, which
            // is the physically meaningful question ("can this body lift water to that height?").
            //
            // The FREE-FALL pin below is a different condition and stays. It is about support from
            // BELOW, not exposure above, and nothing here weakens it.
            let raw_support = effective_support_transitive[idx];
            if raw_support <= 0.0 {
                // Nothing below at all: zero pressure at THIS cell's own bottom face, regardless
                // of what sits above it. Written, not maxed, so a falling cell cannot inherit head
                // from a supported column beside it.
                effective_support[idx] = 0.0;
                pin_target[idx] = z_elev[idx];
            } else {
                effective_support[idx] = raw_support;
                pin_target[idx] = own_elev[idx]; // unused when raw_support >= 1.0 (pure interior)
            }
        }
    }

    // COLD SEED: every wet cell is overwritten with its OWN LOCAL hydrostatic head, discarding
    // whatever the previous call left here. See the module doc comment ("the self-term is LOCAL,
    // never HISTORY") for why this is not an optimisation to be skipped: `max` is monotone, so a
    // field that read its own previous value could only ever ratchet upward and would never fall
    // when a reservoir drains.
    //
    // `own_elev = z + heights * depth_scale` is the correct seed for a free cell and is a strict
    // LOWER bound on its converged head: a supported cell's own weight bears on its own bottom
    // face, so `p >= heights * depth_scale` there, and Pascal's principle (a connected body
    // standing higher elsewhere) can only ever ADD to that. The `+ heights` term is load-bearing
    // and is NOT an off-by-one -- `p` is measured at the BOTTOM face, so the material bearing on
    // it includes the cell's own. Omitting it leaves every interior cell exactly one cell of head
    // below the exposed-top pin above it, which is precisely the ~1.0-reference-row residual that
    // stalled `spec_pascal_under_a_roof` and `spec_uniform_head_in_resting_open_column` under the
    // previous design.
    //
    // Pinned cells (`effective_support <= 0.0`) seed to their pin target instead, which for the
    // unsupported case is `z` alone -- LOWER than `own_elev`, which is exactly right and is why
    // this is a branch rather than a max: a body in free fall carries no contact force anywhere
    // in it. `spec_free_fall_has_zero_pressure` and `spec_free_fall_is_pressureless_throughout`
    // both rest on that.
    // DRY CELLS FIRST, and this is not defensive tidying -- it is a correctness requirement of
    // the transport consumer. A dry cell holds no material, so its pressure is zero and its head
    // is its own elevation: `head = z + p = z`.
    //
    // The previous version left dry cells at whatever the buffer happened to hold (`0.0` after a
    // reset), and `settle_tick`'s driving-head sites read `head_field[nb_idx]` UNCONDITIONALLY
    // once both endpoints pass the liquid gate -- including when the neighbour is the empty cell
    // a body is about to fall into. That read returned `0.0` against a wet cell's genuine
    // `z = -row * depth_scale` (about `-1200` at row 48, w=64), making `head_a - head_b` large
    // and NEGATIVE: every free-fall edge was driven UPWARD, slept, and moved nothing. Measured as
    // a completely frozen simulation with the transport toggle on -- `total_flow = 0.0000` over
    // 150 ticks, a blob that never fell and a vessel that never drained.
    //
    // Seeding `z` everywhere costs one `O(cells)` pass and makes the field TOTAL rather than
    // defined-only-on-wet-cells, so there is no longer any index a consumer can read and get a
    // value that is not a head. It is also exactly consistent with `head_field_to_pressure`,
    // which independently forces dry and out-of-mask cells to `0.0`: `p = head - z = z - z = 0`
    // agrees with that by construction instead of by a second special case.
    //
    // This runs BEFORE the "no wet cells at all" early-out below, deliberately: an empty domain is
    // exactly the case where every read a consumer makes lands on a dry cell, so it is the last
    // place that should be left holding stale values.
    for (idx, slot) in head.iter_mut().enumerate() {
        *slot = z_elev[idx];
    }

    if wet_order.is_empty() {
        return 0.0;
    }

    // Then wet cells, overwriting the `z` seed with their own local hydrostatic head.
    for &idx in &wet_order {
        head[idx] =
            if effective_support[idx] <= 0.0 { pin_target[idx] } else { own_elev[idx] };
    }

    // MAX-PROPAGATION SWEEPS. Alternating direction, Gauss-Seidel (each cell reads its
    // neighbours' CURRENT values), so a value propagates arbitrarily far along the sweep
    // direction within a single pass -- the cost is the number of times a connectivity path
    // REVERSES direction, not its length. Exits as soon as a sweep changes nothing; see
    // `HEAD_FIELD_SWEEPS_PER_TICK` for why the cap is a safety bound rather than a budget.
    //
    // No `omega` here, deliberately: over-relaxation is an averaging-solver acceleration and has
    // no meaning for a max (extrapolating past a max produces a value no neighbour holds).
    let wet_order_rev: Vec<usize> = wet_order.iter().rev().copied().collect();
    let settle_tol = HEAD_FIELD_SWEEP_SETTLE_FRACTION * depth_scale;

    let mut residual = 0.0f32;
    for sweep in 0..HEAD_FIELD_SWEEPS_PER_TICK {
        let forward = sweep % 2 == 0;
        let order: &[usize] = if forward { &wet_order[..] } else { &wet_order_rev[..] };
        let mut max_delta = 0.0f32;
        for &idx in order {
            let x = idx % w;
            let y = idx / w;
            let old = head[idx];
            let s = effective_support[idx];
            let new_val = if s <= 0.0 {
                // A true Dirichlet boundary condition: WRITTEN every sweep, never maxed against
                // its neighbours. Load-bearing twice over (module doc comment): it is what stops
                // a falling cell inheriting head from a supported column beside it, and it is
                // what leaves a gradient for transport to read at all.
                pin_target[idx]
            } else {
                // The one collapsed rule. No `+/- 1` for vertical neighbours: head carries
                // elevation (`head = z + p`), so "pressure rises by one going down" and "falls by
                // one going up" are the SAME statement, `head[below] == head[above]`, and the
                // per-row increment reappears only when the field is read back as pressure
                // (`head_field_to_pressure`, `p = head - z`).
                let mut best = own_elev[idx];
                if x > 0 && wet[idx - 1] {
                    best = best.max(head[idx - 1]);
                }
                if x + 1 < w && wet[idx + 1] {
                    best = best.max(head[idx + 1]);
                }
                if y > 0 && wet[idx - w] {
                    best = best.max(head[idx - w]);
                }
                if y + 1 < h && wet[idx + w] {
                    best = best.max(head[idx + w]);
                }
                if s >= 1.0 {
                    best
                } else {
                    // Partial support (#58's graded `support_fraction`) partially transmits
                    // pressure and partially free-falls, with no separate code path for either
                    // extreme -- the same blend the previous design used, with `best` in place of
                    // the neighbour average.
                    s * best + (1.0 - s) * pin_target[idx]
                }
            };
            let delta = (new_val - old).abs();
            if delta > max_delta {
                max_delta = delta;
            }
            head[idx] = new_val;
        }
        residual = max_delta;
        if max_delta <= settle_tol {
            break;
        }
    }

    residual
}

/// TASK #63. Hydrostatic head carried by one cell, expressed in CELLS OF HEAD -- i.e. `p / ds`
/// where `p = head - z` is the same pressure `head_field_to_pressure` computes and
/// `ds = depth_scale` is one grid cell's worth of elevation. A return of `3.0` means "this cell
/// bears three cells' depth of water"; the value is therefore resolution-independent by
/// construction, which is why the rate law built on it (`pressure_rate_factor` in `physics.rs`)
/// needs no per-resolution constant.
///
/// **`0.0` means UNSUPPORTED, not "almost no water".** Reading this as a continuum bottoming out
/// at zero is the one way to misuse it. `advance_head_field` WRITES (never maxes) `head = z` at
/// every cell it classified as free-falling, so an unsupported cell returns exactly `0.0`. Every
/// SUPPORTED wet cell instead takes at least its own local hydrostatic term
/// `own_elev = z + height * ds` into the max, so it returns at least `height > 0` cells of head --
/// strictly positive, however thin the film. The two states are therefore cleanly separated at
/// zero, and a caller that wants to exempt free fall can test `<= 0.0` exactly rather than
/// carrying a second support mask alongside the field. Dry cells also read `0.0` (they hold
/// `head = z` too), which is harmless for that use: a dry cell has no mass to donate.
///
/// Cheap by construction -- `head[idx] / ds + row` -- so it is safe to call per edge inside the
/// solver's hot loop, unlike `head_field_to_pressure`, which allocates a whole-grid `Vec`.
#[inline]
pub(crate) fn cells_of_head_at(idx: usize, w: usize, head: &[f32]) -> f32 {
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    head[idx] / depth_scale + (idx / w) as f32
}

/// Task #55 step 2, visualisation (2.32): converts an ALREADY-COMPUTED head field (the persistent
/// `head` buffer `advance_head_field` maintains) into a PRESSURE-like quantity, for the pressure
/// heat-map debug overlay's "new field" source (`DrawingSimulation::pressure_heatmap_head_field`,
/// read in `pressure_field_texels`). `head` is an ELEVATION and is not comparable on
/// `column_depth`'s scale; `p := head - z` IS, using this module's own `z_elev` convention
/// (`z(idx) = -row(idx) * depth_scale`) -- exactly `task55_head_spec::pressure_at`'s definition,
/// applied to this field, so the two sources can be pushed through `pressure_field_texels`'s
/// existing normalisation and colour ramp unchanged and read on the same scale.
///
/// UNLIKE the previous version of this module's `compute_head_pressure_field`, this does NOT run
/// any relaxation itself -- it is a pure, cheap (`O(cells)`) read-and-convert over whatever `head`
/// currently holds, because the field is now persistent simulation state maintained elsewhere
/// (`advance_head_field`, called once per tick from `settle_tick` while `head_field_transport` is
/// active -- see that field's doc comment in `lib.rs`), not something this function computes on
/// demand. If the persistent field was never advanced (transport has never been turned on since
/// the last reset/resize), this reads whatever `head` was initialised to (zero -- see
/// `DrawingSimulation::head_field`'s own doc comment), which is not a meaningful pressure reading;
/// that is a property of when the caller chose to maintain the field, not of this conversion.
///
/// Cells with no material and cells outside the shape mask are forced to `0.0` explicitly --
/// never left to whatever stale value `head` happens to hold there (a cell that dried out keeps
/// whatever head value it last had; `advance_head_field` only ever touches currently-wet cells).
/// This matches `column_depth`'s own convention for the same cells: `recompute_column_depth` only
/// ever writes an in-mask interior cell, leaving every other slot at its buffer default of `0.0`
/// -- so the two sources agree outside a filled body, not only inside one.
pub(crate) fn head_field_to_pressure(
    w: usize,
    h: usize,
    shape_mask: &[u8],
    heights: &[f32],
    head: &[f32],
) -> Vec<f32> {
    if w == 0 || h == 0 || head.len() != w * h {
        return vec![0.0f32; head.len()];
    }
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    (0..w * h)
        .map(|idx| {
            if shape_mask[idx] == crate::MASK_OUTSIDE || heights[idx] <= HEAD_FIELD_WET_EPS {
                0.0
            } else {
                let z = -((idx / w) as f32) * depth_scale;
                head[idx] - z
            }
        })
        .collect()
}
