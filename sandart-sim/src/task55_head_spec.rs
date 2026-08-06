//! Task #55, step 1: an ISOLATION SPEC for the pressure field.
//!
//! Every prior attempt at #55 was measured through end-to-end levelling metrics ("how many
//! ticks to halve a level difference"), which conflate the pressure FIELD with the transport
//! that consumes it. Nothing in this codebase ever asserted "given this static configuration,
//! the field should be THIS." This module is that missing assertion.
//!
//! **This is a specification of correct physics, not a test suite that must pass.** Most of the
//! specs below are EXPECTED TO FAIL against today's field -- a failing spec is the deliverable,
//! a measured, reproducible statement of what is wrong. Physics in `physics.rs` is never touched
//! from this file; only the scenarios and the assertions are ours.
//!
//! # The quantity under test
//!
//! Real hydrostatic pressure is `p = rho*g*(eta - z)`, and hydraulic head `head = z + p/(rho*g)`
//! is, for a body at rest, just `eta` -- constant throughout a connected body at rest, equal to
//! its free-surface elevation. Every spec here computes exactly the head expression the shipped
//! elliptic solve already uses (`physics.rs`, `elliptic_liquid_level_pass`'s own doc comment):
//!
//! ```text
//! head(i) = heights[i] * depth_scale + column_depth[i] - row(i) * depth_scale
//! ```
//!
//! `depth_scale = REFERENCE_GRID_HEIGHT / w` is what makes this quantity resolution-invariant,
//! so it is the natural tolerance unit: every tolerance below is expressed as a physically
//! justified multiple of `depth_scale`, never picked by trying values until something passes.
//!
//! # Structure and the ratchet
//!
//! Each `spec_*` function returns `Result<(), String>` -- `Err` carries the measured deviation
//! (numbers, location, expectation), never a panic. A thin `#[test] #[ignore = "SPEC for #55:
//! ..."]` wrapper exists per spec so any one of them can be run in isolation with `--ignored`.
//!
//! `test_task55_head_spec_scoreboard` (NOT ignored) calls every spec and asserts the set of
//! currently-PASSING names equals a hard-coded `expected_passing` set. This is what keeps `cargo
//! test` green today while making every deviation visible: a still-broken spec failing is
//! expected and encoded; the scoreboard test only turns red if the PASS/FAIL boundary MOVES.
//! When a fix lands and a spec starts passing, move its name INTO `expected_passing` -- never
//! remove a name FROM it to silence a regression the other way. Widening a tolerance, deleting a
//! spec, or `#[ignore]`-ing the scoreboard test to make this go green is exactly the failure mode
//! this ratchet exists to catch.

use super::*;

/// `depth_scale` as defined identically in `recompute_column_depth` and the elliptic solve:
/// `REFERENCE_GRID_HEIGHT / w`. The natural per-resolution unit for every tolerance below.
fn depth_scale(w: usize) -> f32 {
    REFERENCE_GRID_HEIGHT as f32 / w as f32
}

/// The exact head expression specified by the task brief and already used at
/// `elliptic_liquid_level_pass`'s call site (`physics.rs:1586`):
/// `head(i) = heights[i] * depth_scale + column_depth[i] - row(i) * depth_scale`.
fn head_at(idx: usize, w: usize, heights: &[f32], column_depth: &[f32]) -> f32 {
    let ds = depth_scale(w);
    heights[idx] * ds + column_depth[idx] - (idx / w) as f32 * ds
}

/// Tolerance for specs that assert an algebraically EXACT identity (uniform head down an open
/// resting column, Pascal's principle across a connected body at rest -- both exact in
/// continuous hydrostatics, with no discretization error expected from our scenarios since every
/// interface in them lands exactly on a cell boundary). The only real source of deviation is f32
/// summation noise: `column_depth` accumulates up to roughly `0.9 * h` additions of terms sized
/// `~depth_scale`, so the running sum reaches magnitude `~0.9 * h * depth_scale = 0.9 *
/// REFERENCE_GRID_HEIGHT` regardless of `w` (that constancy is the whole point of `depth_scale`).
/// Per-addition f32 rounding is bounded by `f32::EPSILON * magnitude`; worst-case accumulated
/// error over `n` additions is `n * f32::EPSILON * magnitude`, which at `h = 512`, `n ~ 460`,
/// `magnitude ~ 460` comes to roughly `460 * 1.2e-7 * 460 ~= 0.025`. `0.02 * depth_scale(w)`
/// tracks that bound (it scales with the same `depth_scale` the underlying sums scale with) while
/// giving it no more than ~1x headroom -- tight enough that a real physics deviation (which every
/// FAIL below measures in whole units of `depth_scale`, not fractions of it) cannot hide under it.
fn identity_tol(w: usize) -> f32 {
    0.02 * depth_scale(w)
}

/// Maps a fraction of the grid extent to an interior row/column index, clamped to
/// `recompute_column_depth`'s active interior range (`1..=n-2`) so every scenario cell this
/// module reads is one the solver actually writes.
fn frac_idx(frac: f32, n: usize) -> usize {
    ((frac * n as f32).round() as usize).clamp(1, n.saturating_sub(2))
}

/// Runs `recompute_column_depth` (production's own function, untouched) over a static scenario:
/// no ticks, no velocity, no in-flight mass. `external_mass_this_tick` and `edge_vel_v` are zero
/// throughout, which also means `in_transit_at`'s subtraction never engages here (it reads
/// `edge_vel_v[c].max(0.0)`, so an all-zero `edge_vel_v` makes it return exactly `0.0` for every
/// cell) -- this static field measures the RESTING-material term alone, see `spec_free_fall_...`
/// for why that matters.
///
/// `cell_props` is filled with `MaterialMode::Water`'s own preset (`lib.rs`:
/// `preset_props() => (1.00, 0.00, 0.00, 0.00)`) at every cell -- fully liquid water, matching
/// the brief's instruction not to invent a synthetic material.
fn run_column_depth(w: usize, h: usize, mask: &[u8], heights: &[f32]) -> Vec<f32> {
    let cell_count = w * h;
    let zeros = vec![0.0f32; cell_count];
    let mut cell_props = vec![0.0f32; cell_count * 4];
    for c in 0..cell_count {
        cell_props[c * 4 + PROP_WETNESS] = 1.0;
    }
    let mut column_depth = vec![0.0f32; cell_count];
    recompute_column_depth(
        w,
        h,
        mask,
        heights,
        heights, // heightmap_data: no prior tick exists, so the frozen snapshot is just `heights`
        &zeros,
        &cell_props,
        &zeros,
        &mut column_depth,
    );
    column_depth
}

const SWEEP_W: [usize; 4] = [64, 128, 256, 512];

// ---------------------------------------------------------------------------------------------
// Scenario builders
// ---------------------------------------------------------------------------------------------

/// A plain rectangular vessel (walls on all sides), water filled flat from `fill_row` down to
/// `floor_row`, open air (mask INSIDE, height 0) from `top_row` up to `fill_row - 1`. Every
/// bound is a fraction of `w`/`h`, not a fixed cell count, per the task brief.
struct OpenColumnScenario {
    mask: Vec<u8>,
    heights: Vec<f32>,
    left: usize,
    right: usize,
    fill_row: usize,
    floor_row: usize,
}

fn build_open_column(w: usize, h: usize) -> OpenColumnScenario {
    let left = frac_idx(0.30, w);
    let right = frac_idx(0.70, w) + 1; // exclusive upper bound, keep the column several cells wide
    let top_row = frac_idx(0.10, h);
    let fill_row = frac_idx(0.50, h);
    let floor_row = frac_idx(0.90, h);

    let mut mask = vec![crate::MASK_OUTSIDE; w * h];
    for y in top_row..=floor_row {
        for x in left..right {
            mask[y * w + x] = crate::MASK_INSIDE;
        }
    }
    let mut heights = vec![0.0f32; w * h];
    for y in fill_row..=floor_row {
        for x in left..right {
            heights[y * w + x] = 1.0;
        }
    }
    OpenColumnScenario { mask, heights, left, right, fill_row, floor_row }
}

/// A slab of water `BLOB_THICKNESS_FRAC` of the grid tall, either floating in mid-air over a
/// large genuine gap or resting on the container floor. In BOTH cases the mask is INSIDE all
/// the way down to the same floor; the only difference is where the material sits.
///
/// The two variants exist to be COMPARED. Measuring the floating one alone invites the reply
/// that a nonzero reading is just bookkeeping -- that `h[i] * depth_scale + column_depth[i]`
/// means "pressure at the bottom face including this cell's own material", so of course a
/// single cell reads one cell's worth. Running the identical slab resting and floating removes
/// that escape: if the two read the SAME number, the field is provably blind to support, and
/// the magnitude is the full weight of the slab rather than a one-cell offset.
///
/// The slab is deliberately many cells thick for the same reason. A one-cell blob can only ever
/// be wrong by one cell, which is the least convincing measurement available; an N-cell slab is
/// wrong by N, and N grows with the scenario, which is unmistakably structural.
struct FloatingBlobScenario {
    mask: Vec<u8>,
    heights: Vec<f32>,
    blob_x: usize,
    /// The slab's BOTTOM row -- the cell whose bottom face bears the whole slab's weight, and
    /// so the one where a support-blind field is most visibly wrong.
    blob_row: usize,
    /// How many full cells of material sit at and above `blob_row`. The analytic pressure at
    /// `blob_row`'s bottom face is exactly this many `depth_scale`s when resting, and exactly
    /// zero when free-falling.
    slab_cells: usize,
}

/// Slab thickness as a fraction of grid height, so the scenario means the same thing at every
/// resolution (a fixed cell count would be a different physical object at each `w`).
const BLOB_THICKNESS_FRAC: f32 = 0.10;

fn build_floating_blob_variant(w: usize, h: usize, resting: bool) -> FloatingBlobScenario {
    let left = frac_idx(0.30, w);
    let right = frac_idx(0.70, w) + 1;
    let top_row = frac_idx(0.05, h);
    let floor_row = frac_idx(0.90, h);
    let slab_cells = ((BLOB_THICKNESS_FRAC * h as f32).round() as usize).max(2);

    // Floating: the slab's bottom sits well above the floor, with a large genuine air gap
    // beneath it. Resting: the slab's bottom IS the floor.
    let blob_row = if resting { floor_row } else { frac_idx(0.45, h) };

    let mut mask = vec![crate::MASK_OUTSIDE; w * h];
    for y in top_row..=floor_row {
        for x in left..right {
            mask[y * w + x] = crate::MASK_INSIDE;
        }
    }
    let mut heights = vec![0.0f32; w * h];
    for k in 0..slab_cells {
        let y = blob_row - k;
        for x in left..right {
            heights[y * w + x] = 1.0;
        }
    }
    FloatingBlobScenario { mask, heights, blob_x: left, blob_row, slab_cells }
}

/// A tall open shaft on the left, connected at its very bottom row to a horizontal channel that
/// runs to the right underneath a solid roof (the row directly above the channel is OUTSIDE
/// mask). The whole connected body -- shaft plus channel -- is filled with water at rest.
struct RoofScenario {
    mask: Vec<u8>,
    heights: Vec<f32>,
    shaft_x: usize,
    floor_row: usize,
    channel_far_x: usize,
}

fn build_roof_scenario(w: usize, h: usize) -> RoofScenario {
    let shaft_x0 = frac_idx(0.10, w);
    let shaft_x1 = frac_idx(0.25, w);
    let channel_end = frac_idx(0.60, w) + 1; // exclusive upper bound
    let top_row = frac_idx(0.05, h);
    let fill_row = frac_idx(0.30, h);
    let floor_row = frac_idx(0.85, h);

    let mut mask = vec![crate::MASK_OUTSIDE; w * h];
    for y in top_row..=floor_row {
        for x in shaft_x0..shaft_x1 {
            mask[y * w + x] = crate::MASK_INSIDE;
        }
    }
    for x in shaft_x1..channel_end {
        mask[floor_row * w + x] = crate::MASK_INSIDE;
    }

    let mut heights = vec![0.0f32; w * h];
    for y in fill_row..=floor_row {
        for x in shaft_x0..shaft_x1 {
            heights[y * w + x] = 1.0;
        }
    }
    for x in shaft_x1..channel_end {
        heights[floor_row * w + x] = 1.0;
    }

    RoofScenario { mask, heights, shaft_x: shaft_x0, floor_row, channel_far_x: channel_end - 1 }
}

/// Builds the real `SandboxShape::UTubeFlowThrough` mask via `eval_sandbox_shape` (the
/// production shape evaluator), then fills every in-mask cell whose row lies at or below
/// `fill_dy_frac` (a fraction of `h` below the shape's own vertical center, matching
/// `U_TUBE_RECTS`'s own dy convention) with water. `fill_dy_frac = 0.10` submerges the basin
/// (`U_TUBE_RECTS[1]`, dy 0.36..0.42, entirely below 0.10) and the lower portion of BOTH the
/// reservoir (dy -0.40..0.36) and the right arm (dy 0.02..0.36), so both arms share one free
/// surface at dy=0.10 -- exactly what a body at rest looks like.
fn build_u_tube_filled(w: usize, h: usize, fill_dy_frac: f32) -> (Vec<u8>, Vec<f32>) {
    let mut mask = vec![crate::MASK_OUTSIDE; w * h];
    for y in 0..h {
        for x in 0..w {
            let (inside, _safe) = eval_sandbox_shape(
                x,
                y,
                w,
                h,
                crate::SandboxShape::UTubeFlowThrough,
                0.05,
                1.0,
                8,
                false,
            );
            if inside {
                mask[y * w + x] = crate::MASK_INSIDE;
            }
        }
    }
    let mut heights = vec![0.0f32; w * h];
    let center_y = h as f32 / 2.0;
    for y in 0..h {
        let dy_frac = (y as f32 - center_y) / h as f32;
        if dy_frac >= fill_dy_frac {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    heights[idx] = 1.0;
                }
            }
        }
    }
    (mask, heights)
}

// ---------------------------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------------------------

/// Spec 1: every submerged in-mask cell of a plain resting vessel must have the same head, equal
/// to the free-surface elevation. *Expected to PASS* -- this is the open-vertical-column case
/// where `column_depth` and true hydrostatic pressure agree by construction.
fn spec_uniform_head_in_resting_open_column() -> Result<(), String> {
    let mut table = String::new();
    let mut fail = false;
    for &w in &SWEEP_W {
        let h = w;
        let s = build_open_column(w, h);
        let cd = run_column_depth(w, h, &s.mask, &s.heights);
        let ds = depth_scale(w);
        let tol = identity_tol(w);
        let reference_head = head_at(s.fill_row * w + s.left, w, &s.heights, &cd);
        let mut max_dev = 0.0f32;
        let mut loc = (s.left, s.fill_row);
        for y in s.fill_row..=s.floor_row {
            for x in s.left..s.right {
                let idx = y * w + x;
                let dev = (head_at(idx, w, &s.heights, &cd) - reference_head).abs();
                if dev > max_dev {
                    max_dev = dev;
                    loc = (x, y);
                }
            }
        }
        table.push_str(&format!(
            "w={w}: max|Δhead|={max_dev:.5} ref-rows ({:.5} local cells) tol={tol:.5} at (x={},y={}) reference_head={reference_head:.4}\n",
            max_dev / ds,
            loc.0,
            loc.1
        ));
        if max_dev > tol {
            fail = true;
        }
    }
    if fail {
        return Err(format!(
            "spec_uniform_head_in_resting_open_column: head is NOT uniform through a plain \
             resting water column -- this is the textbook base case every other spec builds on; \
             if this fails the field is broken even here.\n{table}"
        ));
    }
    Ok(())
}

/// Spec 2: head being uniform in spec 1 must be because pressure grows by exactly one
/// `depth_scale` per row of full cells (a *scale* check), not because of some compensating
/// *offset* error that happens to cancel out only in the uniform-head comparison. Asserts the
/// per-row increment of `p := heights[i] * depth_scale + column_depth[i]` directly.
fn spec_pressure_is_linear_in_depth() -> Result<(), String> {
    let mut table = String::new();
    let mut fail = false;
    for &w in &SWEEP_W {
        let h = w;
        let s = build_open_column(w, h);
        let cd = run_column_depth(w, h, &s.mask, &s.heights);
        let ds = depth_scale(w);
        let tol = identity_tol(w);
        let x = s.left;
        let mut max_slope_dev = 0.0f32;
        let mut worst_row = s.fill_row;
        let mut slope_sum = 0.0f32;
        let mut count = 0usize;
        for y in s.fill_row..s.floor_row {
            let p_y = s.heights[y * w + x] * ds + cd[y * w + x];
            let p_y1 = s.heights[(y + 1) * w + x] * ds + cd[(y + 1) * w + x];
            let slope = p_y1 - p_y; // expect exactly `ds`: one full row of pressure per row of fill
            slope_sum += slope;
            count += 1;
            let dev = (slope - ds).abs();
            if dev > max_slope_dev {
                max_slope_dev = dev;
                worst_row = y;
            }
        }
        let avg_slope_in_rows = slope_sum / count as f32 / ds;
        table.push_str(&format!(
            "w={w}: avg slope={avg_slope_in_rows:.5} depth_scale/row (expect 1.0), max|Δslope|={:.5} ref-rows ({:.5} local cells) tol={tol:.5} at row={worst_row}\n",
            max_slope_dev,
            max_slope_dev / ds
        ));
        if max_slope_dev > tol {
            fail = true;
        }
    }
    if fail {
        return Err(format!(
            "spec_pressure_is_linear_in_depth: per-row pressure increment is not exactly one \
             depth_scale -- either a scale error (slope != 1) or an offset error is present; see \
             per-resolution slopes below.\n{table}"
        ));
    }
    Ok(())
}

/// Spec 3: a slab of water with empty space beneath it -- genuinely unsupported, nothing between
/// it and a floor far below -- must show zero pressure (`p := head(i) - z(i) = heights[i] *
/// depth_scale + column_depth[i]`, using `z(i) = -row(i) * depth_scale`, the same datum used
/// inside `head` itself). This is the user's own words: "free fall takes the pressure away."
///
/// Measured as a PAIRED COMPARISON against the identical slab resting on the floor, because the
/// absolute number alone is contestable and the ratio is not. `h[i] * depth_scale +
/// column_depth[i]` means "pressure at the bottom face including this cell's own material", so a
/// bare nonzero reading on a one-cell blob could be dismissed as bookkeeping. Running the same
/// slab both ways closes that off: a ratio of exactly 1.000 says the field cannot distinguish
/// supported from unsupported material at all, and a multi-cell slab makes the error scale with
/// the slab rather than sit at a fixed one-cell offset.
///
/// `recompute_column_depth` does subtract `in_transit_at` (in-flight mass), so in principle this
/// spec COULD partially pass if that subtraction masked the residual; this static scenario keeps
/// `edge_vel_v` at all-zero throughout (see `run_column_depth`'s doc comment), which makes that
/// subtraction a guaranteed no-op, so what is measured here is the mechanism's floor: how much
/// pressure a purely-geometric, velocity-blind read of "resting material above" reports for
/// material with nothing at all beneath it.
fn spec_free_fall_has_zero_pressure() -> Result<(), String> {
    let mut table = String::new();
    let mut fail = false;
    for &w in &SWEEP_W {
        let h = w;
        let ds = depth_scale(w);
        let tol = identity_tol(w);

        // The identical slab, once resting on the floor and once floating over a large gap.
        let falling = build_floating_blob_variant(w, h, false);
        let resting = build_floating_blob_variant(w, h, true);
        let cd_falling = run_column_depth(w, h, &falling.mask, &falling.heights);
        let cd_resting = run_column_depth(w, h, &resting.mask, &resting.heights);

        let idx_f = falling.blob_row * w + falling.blob_x;
        let idx_r = resting.blob_row * w + resting.blob_x;
        let p_falling = falling.heights[idx_f] * ds + cd_falling[idx_f];
        let p_resting = resting.heights[idx_r] * ds + cd_resting[idx_r];

        // Analytic: a resting slab `slab_cells` tall bears its full weight at its bottom face;
        // a free-falling one bears nothing at all.
        let expected_resting = falling.slab_cells as f32 * ds;

        table.push_str(&format!(
            "w={w}: slab={} cells | FALLING p={:.4} ref-rows ({:.3} cells, want 0) | \
             RESTING p={:.4} ({:.3} cells, want {:.3}) | falling/resting ratio={:.4} | tol={tol:.5}\n",
            falling.slab_cells,
            p_falling,
            p_falling / ds,
            p_resting,
            p_resting / ds,
            expected_resting / ds,
            if p_resting.abs() > 0.0 { p_falling / p_resting } else { f32::NAN },
        ));

        if p_falling.abs() > tol {
            fail = true;
        }
    }
    if fail {
        return Err(format!(
            "spec_free_fall_has_zero_pressure: a genuinely unsupported slab of water -- nothing \
             whatever between it and a floor far below -- reports the SAME pressure at its bottom \
             face as the identical slab resting on that floor. Not a one-cell bookkeeping offset: \
             the ratio is 1.000, and the magnitude is the slab's full weight, so it grows with the \
             slab. `recompute_column_depth` only ever looks UPWARD -- 'what rests above a cell' -- \
             and never asks whether anything supports the cell from below, so resting and falling \
             material of identical local geometry are indistinguishable to this field. \
             `edge_vel_v` is all-zero in this static spec, so the `in_transit_at` subtraction this \
             function performs is a guaranteed no-op; what is measured here is the mechanism's own \
             floor, not something that subtraction is failing to mask. `support_fraction` (#58, \
             shipped) already answers exactly the question this field never asks.\n{table}"
        ));
    }
    Ok(())
}

/// Spec 4 (the single most important spec in this set): cells in a roofed horizontal channel,
/// connected at the bottom of a tall resting water column, must read the SAME head as the tall
/// column -- that is Pascal's principle. *Expected to FAIL*: `column_depth` only ever looks
/// straight up, so a cell under a low roof sees none of the water piled up beside it in the
/// shaft, even though it is the exact same connected body at the exact same elevation. This is
/// precisely why no siphon or roofed-channel transmission is possible with today's field.
fn spec_pascal_under_a_roof() -> Result<(), String> {
    let mut table = String::new();
    let mut fail = false;
    for &w in &SWEEP_W {
        let h = w;
        let s = build_roof_scenario(w, h);
        let cd = run_column_depth(w, h, &s.mask, &s.heights);
        let ds = depth_scale(w);
        let tol = identity_tol(w);
        let shaft_idx = s.floor_row * w + s.shaft_x;
        let channel_idx = s.floor_row * w + s.channel_far_x;
        let head_shaft = head_at(shaft_idx, w, &s.heights, &cd);
        let head_channel = head_at(channel_idx, w, &s.heights, &cd);
        let dev = (head_shaft - head_channel).abs();
        table.push_str(&format!(
            "w={w}: head_shaft={head_shaft:.3} head_channel_under_roof={head_channel:.3} \
             |Δ|={dev:.3} ref-rows ({:.3} local cells) tol={tol:.4} (shaft overburden at its own \
             bottom was {:.3} local cells = column_depth[shaft-bottom], i.e. exactly the amount \
             missing under the roof)\n",
            dev / ds,
            cd[shaft_idx] / ds
        ));
        if dev > tol {
            fail = true;
        }
    }
    if fail {
        return Err(format!(
            "spec_pascal_under_a_roof: head under the roof does NOT equal the tall column's \
             free-surface head. This is the missing-Pascal defect: `column_depth` counts only \
             material directly above a cell, so a roofed channel gets none of the overburden of \
             a column standing beside it, even at rest, even though they are one connected body \
             at the same elevation.\n{table}"
        ));
    }
    Ok(())
}

/// Spec 5: the general form of spec 4, on the real `SandboxShape::UTubeFlowThrough` vessel
/// (`U_TUBE_RECTS`, added in 53516eb) filled to a level that submerges the basin and part of both
/// arms, so the whole body is genuinely one connected mass at rest. Asserts `max(head) -
/// min(head)` over that body is within tolerance; a real hydrostatic field would have exactly
/// one value.
fn spec_head_is_single_valued_across_a_connected_body() -> Result<(), String> {
    let fill_dy_frac = 0.10_f32;
    let mut table = String::new();
    let mut fail = false;
    for &w in &SWEEP_W {
        let h = w;
        let (mask, heights) = build_u_tube_filled(w, h, fill_dy_frac);
        let wet: Vec<bool> = (0..w * h).map(|i| heights[i] > 0.0).collect();

        // Scenario-validity self-check (not the physics claim under test): confirm the wet set
        // really is one connected body, so a FAIL below can only mean the physics is wrong, never
        // that the scenario silently fell apart into disjoint puddles.
        let start = wet.iter().position(|&b| b).ok_or_else(|| {
            format!(
                "spec_head_is_single_valued_across_a_connected_body: w={w}: SCENARIO INVALID -- \
                 no wet cells at all; fill_dy_frac={fill_dy_frac} is below every U_TUBE_RECTS \
                 rect's y-range."
            )
        })?;
        let mut visited = vec![false; w * h];
        let mut stack = vec![start];
        visited[start] = true;
        let mut reached = 0usize;
        while let Some(idx) = stack.pop() {
            reached += 1;
            let x = idx % w;
            let y = idx / w;
            for (nx, ny) in
                [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)]
            {
                if nx < w && ny < h {
                    let nidx = ny * w + nx;
                    if !visited[nidx] && wet[nidx] {
                        visited[nidx] = true;
                        stack.push(nidx);
                    }
                }
            }
        }
        let total_wet = wet.iter().filter(|&&b| b).count();
        if reached != total_wet {
            return Err(format!(
                "spec_head_is_single_valued_across_a_connected_body: w={w}: SCENARIO INVALID, not \
                 the physics under test -- the fill_dy_frac={fill_dy_frac} wet set is not one \
                 connected body (flood fill from the first wet cell reached {reached} of \
                 {total_wet} wet cells). Fix the scenario before trusting this spec's verdict."
            ));
        }

        let cd = run_column_depth(w, h, &mask, &heights);
        let ds = depth_scale(w);
        let tol = identity_tol(w);
        let mut max_head = f32::MIN;
        let mut max_loc = (0usize, 0usize);
        let mut min_head = f32::MAX;
        let mut min_loc = (0usize, 0usize);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let idx = y * w + x;
                if wet[idx] {
                    let hv = head_at(idx, w, &heights, &cd);
                    if hv > max_head {
                        max_head = hv;
                        max_loc = (x, y);
                    }
                    if hv < min_head {
                        min_head = hv;
                        min_loc = (x, y);
                    }
                }
            }
        }
        let spread = max_head - min_head;
        table.push_str(&format!(
            "w={w}: spread={spread:.3} ref-rows ({:.3} local cells) tol={tol:.4}; \
             max_head={max_head:.3} at (x={},y={}); min_head={min_head:.3} at (x={},y={})\n",
            spread / ds,
            max_loc.0,
            max_loc.1,
            min_loc.0,
            min_loc.1
        ));
        if spread > tol {
            fail = true;
        }
    }
    if fail {
        return Err(format!(
            "spec_head_is_single_valued_across_a_connected_body: head is NOT single-valued across \
             the connected U-tube body (Pascal's principle violated) -- the general form of \
             spec_pascal_under_a_roof, on the real UTubeFlowThrough vessel.\n{table}"
        ));
    }
    Ok(())
}

/// Spec 6: the SAME scenario (the open resting column, expressed purely as fractions of `w`/`h`)
/// must produce the SAME head value, after `depth_scale` normalisation, at every grid resolution.
/// There is prior history of resolution bugs in exactly this quantity (see `depth_scale`'s own
/// doc comment history in `physics.rs`), so this gets its own spec independent of spec 1.
fn spec_head_field_is_resolution_invariant() -> Result<(), String> {
    // The only source of cross-resolution disagreement in an otherwise-exact formula is that
    // `frac_idx` rounds `fill_row`/`floor_row` to the nearest integer independently at each `w`,
    // so the ACTUAL fraction represented differs from the target by up to 0.5 rows at every
    // resolution. One row of rounding error, in reference-row units, is exactly `depth_scale(w)`
    // -- and the coarsest grid in this sweep (w=64) has the largest `depth_scale`, so it bounds
    // the worst case. `depth_scale(64)` is the tolerance: any cross-resolution deviation beyond
    // one row's worth of rounding at the coarsest grid under test reflects a genuine
    // resolution-DEPENDENT bug in the field itself, not integer placement of the fill line.
    let tol = depth_scale(64);
    let mut table = String::new();
    let mut heads = Vec::new();
    for &w in &SWEEP_W {
        let h = w;
        let s = build_open_column(w, h);
        let cd = run_column_depth(w, h, &s.mask, &s.heights);
        let hv = head_at(s.fill_row * w + s.left, w, &s.heights, &cd);
        table.push_str(&format!("w={w}: head={hv:.4} ref-rows\n"));
        heads.push(hv);
    }
    let max_head = heads.iter().cloned().fold(f32::MIN, f32::max);
    let min_head = heads.iter().cloned().fold(f32::MAX, f32::min);
    let max_dev = max_head - min_head;
    table.push_str(&format!(
        "max cross-resolution |Δhead|={max_dev:.4} ref-rows, tol={tol:.4}\n"
    ));
    if max_dev > tol {
        return Err(format!(
            "spec_head_field_is_resolution_invariant: the SAME physical scenario (same fractions \
             of w/h) produces different head values at different grid resolutions by more than \
             one row's worth of fill-line rounding at the coarsest resolution under test -- the \
             field is not resolution-invariant.\n{table}"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Ignored per-spec wrappers -- run any one alone with `--ignored`
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "SPEC for #55: hydraulic head must be uniform through a plain resting open column"]
fn test_spec_uniform_head_in_resting_open_column() {
    if let Err(e) = spec_uniform_head_in_resting_open_column() {
        panic!("{e}");
    }
}

#[test]
#[ignore = "SPEC for #55: pressure must grow by exactly one depth_scale per row of depth"]
fn test_spec_pressure_is_linear_in_depth() {
    if let Err(e) = spec_pressure_is_linear_in_depth() {
        panic!("{e}");
    }
}

#[test]
#[ignore = "SPEC for #55: an unsupported free-falling blob of water must show zero pressure"]
fn test_spec_free_fall_has_zero_pressure() {
    if let Err(e) = spec_free_fall_has_zero_pressure() {
        panic!("{e}");
    }
}

#[test]
#[ignore = "SPEC for #55: a roofed channel connected to a tall column must share its head (Pascal)"]
fn test_spec_pascal_under_a_roof() {
    if let Err(e) = spec_pascal_under_a_roof() {
        panic!("{e}");
    }
}

#[test]
#[ignore = "SPEC for #55: head must be single-valued across a whole connected U-tube body at rest"]
fn test_spec_head_is_single_valued_across_a_connected_body() {
    if let Err(e) = spec_head_is_single_valued_across_a_connected_body() {
        panic!("{e}");
    }
}

#[test]
#[ignore = "SPEC for #55: the head field must be resolution-invariant across w=64/128/256/512"]
fn test_spec_head_field_is_resolution_invariant() {
    if let Err(e) = spec_head_field_is_resolution_invariant() {
        panic!("{e}");
    }
}

// ---------------------------------------------------------------------------------------------
// The scoreboard -- NOT ignored, runs under plain `cargo test`
// ---------------------------------------------------------------------------------------------

/// See this module's doc comment for the ratchet design. Calling convention: every spec returns
/// `Result`, so no `catch_unwind` is needed to keep one spec's failure from stopping the others.
#[test]
fn test_task55_head_spec_scoreboard() {
    let specs: &[(&str, fn() -> Result<(), String>)] = &[
        ("spec_uniform_head_in_resting_open_column", spec_uniform_head_in_resting_open_column),
        ("spec_pressure_is_linear_in_depth", spec_pressure_is_linear_in_depth),
        ("spec_free_fall_has_zero_pressure", spec_free_fall_has_zero_pressure),
        ("spec_pascal_under_a_roof", spec_pascal_under_a_roof),
        (
            "spec_head_is_single_valued_across_a_connected_body",
            spec_head_is_single_valued_across_a_connected_body,
        ),
        ("spec_head_field_is_resolution_invariant", spec_head_field_is_resolution_invariant),
    ];

    // Ratchet set: the specs CURRENTLY PASSING against shipped physics, measured directly (see
    // this task's report for the numbers). When a fix lands and a spec starts passing, move its
    // name INTO this set -- never remove a name FROM it to make a still-broken spec's failure
    // go away; that would defeat the entire point of this module.
    let expected_passing: &[&str] = &[
        "spec_uniform_head_in_resting_open_column",
        "spec_pressure_is_linear_in_depth",
        "spec_head_field_is_resolution_invariant",
    ];

    let mut actual_passing: Vec<&str> = Vec::new();
    let mut report = String::new();
    for (name, f) in specs {
        match f() {
            Ok(()) => {
                actual_passing.push(name);
                report.push_str(&format!("PASS  {name}\n"));
            }
            Err(e) => {
                report.push_str(&format!("FAIL  {name}\n      {e}\n\n"));
            }
        }
    }
    actual_passing.sort_unstable();
    let mut expected_sorted = expected_passing.to_vec();
    expected_sorted.sort_unstable();

    assert_eq!(
        actual_passing, expected_sorted,
        "\nTask #55 head-field spec scoreboard changed.\n\
         Currently passing: {actual_passing:?}\n\
         Expected passing:  {expected_sorted:?}\n\
         When a spec starts passing, move its name INTO `expected_passing` in this test -- never \
         remove a name from `expected_passing` to silence a regression the other way.\n\n\
         Full table:\n{report}"
    );
}
