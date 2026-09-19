//! PROTOTYPE ONLY. Renders pictures of two candidate redesigns for the "Merging cascade" vessel
//! (`SandboxShape::MultiStageHourglass`) for the user to pick between, per the task that asked
//! for this file. Does not touch `sandart-sim/src`, the renderer, wasm or the UI.
//!
//! User's verdict on today's shape: "it does not start with enough material to have an
//! interesting simulation." Candidates A (split-and-merge brick chambers) and C (step pools that
//! fill then spill) are both designed to give a real top reservoir.
//!
//! Background: artifacts/tickets/24-*.md, 32-*.md, 41-*.md; `MultiStageHourglass` in
//! sandart-sim/src/physics.rs (~2940); `initialize_hourglass` in sandart-sim/src/lib.rs.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && cargo run -p sandart-sim --release --example proto_cascades'
//!
//! Writes PNGs and a README to artifacts/design/cascade-2026-09-17/.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, MASK_BOUNDARY, MASK_INSIDE, MASK_OUTSIDE};
use std::fs;
use std::path::Path;

const GRID: usize = 256;

// -------------------------------------------------------------------------------------------
// Shared rasterization / diagnostics
// -------------------------------------------------------------------------------------------

/// Same two-pass semantics as `DrawingSimulation::rasterize_shape_mask`: pass 1 evaluates
/// inside/outside at integer cell centres (`out_size == sim size`, so no fractional pixel-centre
/// remap is needed -- this is exactly the bit-identical-at-sim-size case that function's own doc
/// comment describes); pass 2 marks any INSIDE cell with an OUTSIDE 4-neighbour as MASK_BOUNDARY.
fn rasterize(w: usize, h: usize, inside_fn: impl Fn(f32, f32) -> bool) -> Vec<u8> {
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let mut mask = vec![MASK_OUTSIDE; w * h];
    for y in 0..h {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            mask[y * w + x] = if inside_fn(dx, dy) { MASK_INSIDE } else { MASK_OUTSIDE };
        }
    }
    let snapshot = mask.clone();
    for y in 0..h {
        for x in 0..w {
            if snapshot[y * w + x] != MASK_INSIDE {
                continue;
            }
            let has_outside = (x == 0 || snapshot[y * w + x - 1] == MASK_OUTSIDE)
                || (x + 1 >= w || snapshot[y * w + x + 1] == MASK_OUTSIDE)
                || (y == 0 || snapshot[(y - 1) * w + x] == MASK_OUTSIDE)
                || (y + 1 >= h || snapshot[(y + 1) * w + x] == MASK_OUTSIDE);
            if has_outside {
                mask[y * w + x] = MASK_BOUNDARY;
            }
        }
    }
    mask
}

/// Mirror-symmetry check about the true axis `(w-1)/2`: for integer cell index `x`, the mirror
/// cell is `w-1-x` exactly (this is the same check `test_vessel_masks_are_left_right_symmetric`
/// makes on the real shapes). Returns the number of mismatched (inside-ness differs) cells.
fn mirror_mismatches(mask: &[u8], w: usize, h: usize) -> usize {
    let mut mismatches = 0;
    for y in 0..h {
        for x in 0..w {
            let mx = w - 1 - x;
            let a = mask[y * w + x] != MASK_OUTSIDE;
            let b = mask[y * w + mx] != MASK_OUTSIDE;
            if a != b {
                mismatches += 1;
            }
        }
    }
    mismatches
}

/// Flood-fill connectivity from every reservoir-region cell (any inside cell with `dy < res_y1`)
/// over 4-connected inside cells (MASK_INSIDE or MASK_BOUNDARY). Returns
/// (reached_count, total_inside_count, reached_collector) where `reached_collector` is whether
/// any cell with `dy >= collector_y0` was reached -- this is the "no sealed pockets, and the
/// network actually reaches the collector" check the task asks for.
fn connectivity(
    mask: &[u8],
    w: usize,
    h: usize,
    is_reservoir: impl Fn(f32, f32) -> bool,
    is_collector: impl Fn(f32, f32) -> bool,
) -> (usize, usize, bool) {
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let total_inside = mask.iter().filter(|&&m| m != MASK_OUTSIDE).count();

    let mut visited = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for y in 0..h {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            let idx = y * w + x;
            if mask[idx] != MASK_OUTSIDE && is_reservoir(dx, dy) && !visited[idx] {
                visited[idx] = true;
                stack.push(idx);
            }
        }
    }
    let mut reached_collector = false;
    while let Some(idx) = stack.pop() {
        let (x, y) = (idx % w, idx / w);
        let dx = x as f32 - center_x;
        let dy = y as f32 - center_y;
        if is_collector(dx, dy) {
            reached_collector = true;
        }
        let neighbors = [
            (x.checked_sub(1), Some(y)),
            (Some(x + 1).filter(|&v| v < w), Some(y)),
            (Some(x), y.checked_sub(1)),
            (Some(x), Some(y + 1).filter(|&v| v < h)),
        ];
        for (nx, ny) in neighbors {
            if let (Some(nx), Some(ny)) = (nx, ny) {
                let nidx = ny * w + nx;
                if mask[nidx] != MASK_OUTSIDE && !visited[nidx] {
                    visited[nidx] = true;
                    stack.push(nidx);
                }
            }
        }
    }
    let reached = visited.iter().filter(|&&v| v).count();
    (reached, total_inside, reached_collector)
}

// -------------------------------------------------------------------------------------------
// Design A -- split and merge (offset-brick chambers, Pascal-triangle-like)
// -------------------------------------------------------------------------------------------
//
// Layout (top to bottom, as a pure function of dy so Flip inverts the whole structure):
//   reservoir (open box, full network width) -> 4 tiers of chambers, alternating 6 and 7
//   chambers per tier (7 = 6's chambers offset by half a chamber width, with two half-width
//   chambers at the walls) -> collector (open box, full network width).
//
// Each FULL-width chamber is the union of two tapered funnels sharing its top (so the top is
// fully open across the chamber) and narrowing to two separate necks near its bottom corners --
// one feeding the chamber below-left, one feeding the chamber below-right, i.e. the offset
// lattice below. Half-width edge chambers (the two end chambers of an odd/7-chamber tier) get
// only the inward-facing funnel, since they only have one neighbour below.
//
// The `cw` unit (chamber width) is the SAME constant across every tier -- an even tier's `n`
// chambers of width `cw`, and an odd tier's `n+1` chambers (`n-1` full width `cw`, two half-width
// `cw/2` at the ends) both span exactly `n*cw` -- which is what keeps a parent chamber's centre
// always exactly on a child tier's chamber boundary, guaranteeing every neck lands inside (not
// on the wall of) the tier below.

const A_TOTAL_HALF_FRAC: f32 = 0.44; // * h_f, half the vessel's total vertical extent
const A_RES_FRAC: f32 = 0.18; // fraction of the full vertical extent given to the reservoir
const A_COL_FRAC: f32 = 0.16; // fraction given to the collector
const A_HW_FRAC: f32 = 0.42; // * w_f, half-width of the network envelope
const A_COLS0: u32 = 6; // even-tier chamber count
const A_NET_TIERS: usize = 4; // alternating 6, 7, 6, 7

fn a_tier_cols(tier: usize) -> u32 {
    if tier % 2 == 0 { A_COLS0 } else { A_COLS0 + 1 }
}

/// (total_half, res_y1, net_y1, net_h, hw) -- the boundaries a pure function of h_f/w_f needs.
fn a_geometry(w_f: f32, h_f: f32) -> (f32, f32, f32, f32, f32) {
    let total_half = A_TOTAL_HALF_FRAC * h_f;
    let full = 2.0 * total_half;
    let res_h = A_RES_FRAC * full;
    let col_h = A_COL_FRAC * full;
    let net_h = full - res_h - col_h;
    let res_y1 = -total_half + res_h;
    let net_y1 = res_y1 + net_h;
    let hw = A_HW_FRAC * w_f;
    (total_half, res_y1, net_y1, net_h, hw)
}

fn a_inside(dx: f32, dy: f32, w_f: f32, h_f: f32) -> bool {
    let (total_half, res_y1, net_y1, net_h, hw) = a_geometry(w_f, h_f);
    if dy < -total_half || dy >= total_half {
        return false;
    }
    if dy < res_y1 || dy >= net_y1 {
        // reservoir (above) or collector (below): plain open box, full network width.
        return dx.abs() < hw;
    }

    let tier_h = net_h / A_NET_TIERS as f32;
    let rel = dy - res_y1;
    let tier = ((rel / tier_h).floor() as usize).min(A_NET_TIERS - 1);
    let y1 = res_y1 + (tier as f32 + 1.0) * tier_h;

    let cols = a_tier_cols(tier);
    let cw = 2.0 * hw / A_COLS0 as f32;
    let is_even = tier % 2 == 0;

    // Find this dx's chamber [b0, b1) and remember its slot index (needed to know whether a
    // half-width edge chamber is the LEFT or RIGHT end, i.e. which single funnel it gets).
    let (b0, b1, slot, cols_here) = if is_even {
        let u = ((dx + hw) / cw).clamp(0.0, cols as f32);
        let k = (u.floor() as i32).clamp(0, cols as i32 - 1) as u32;
        (-hw + k as f32 * cw, -hw + (k as f32 + 1.0) * cw, k, cols)
    } else {
        let boundary = |i: u32| -> f32 {
            if i == 0 {
                -hw
            } else if i == cols {
                hw
            } else {
                -hw + (i as f32 - 0.5) * cw
            }
        };
        let mut slot = cols - 1;
        for i in 0..cols {
            if dx < boundary(i + 1) {
                slot = i;
                break;
            }
        }
        (boundary(slot), boundary(slot + 1), slot, cols)
    };
    if dx < b0 || dx >= b1 {
        return false;
    }

    let half_w = (b1 - b0) / 2.0;
    let center = (b0 + b1) / 2.0;
    let t = ((y1 - dy) / tier_h).clamp(0.0, 1.0); // 1 at chamber top (widest), 0 at neck row
    let full_chamber = (b1 - b0) > cw * 0.75;
    let neck_hw = (0.10 * cw).max(1.0);

    // A "wedge" tapers linearly from (center_top, radius_top) at t=1 down to
    // (center_bottom, radius_bottom) at t=0 -- letting the CENTRE itself migrate (not just the
    // radius shrink) is what makes a taper genuinely visible across the full tier height instead
    // of the chamber's own vertical side walls staying put and only a thin notch appearing near
    // the very bottom (the bug in an earlier version of this file: two FIXED-centre wedges each
    // already reaching past the opposite wall at t=1 makes the union wall-bound, i.e. visually a
    // flat-sided box, for most of the tier).
    let wedge = |dx_local: f32, center_top: f32, radius_top: f32, center_bottom: f32| -> bool {
        let c = center_bottom + t * (center_top - center_bottom);
        let r = neck_hw + t * (radius_top - neck_hw);
        (dx_local - c).abs() < r
    };
    let dx_local = dx - center;

    if full_chamber {
        // Two wedges, each exactly HALF the chamber wide at the top (so their union is exactly
        // the full chamber -- no overshoot, no wall-binding), narrowing to a neck near its own
        // quarter point (the "lower corner") at the bottom.
        let shift = 0.5 * half_w;
        wedge(dx_local, -shift, half_w / 2.0, -shift) || wedge(dx_local, shift, half_w / 2.0, shift)
    } else {
        // Half-width edge chamber: ONE wedge, full chamber wide at the top, its centre sliding
        // from the chamber's true centre (0) at the top to the inward corner at the bottom --
        // the only funnel this chamber needs, since it has only one neighbour below.
        let inward = if slot == 0 { 1.0 } else { -1.0 };
        debug_assert!(slot == 0 || slot == cols_here - 1, "half chamber not at a tier edge");
        wedge(dx_local, 0.0, half_w, inward * 0.5 * half_w)
    }
}

fn a_fill(dx: f32, dy: f32, w_f: f32, h_f: f32) -> f32 {
    let (_total_half, res_y1, _net_y1, _net_h, _hw) = a_geometry(w_f, h_f);
    if dy < res_y1 && a_inside(dx, dy, w_f, h_f) { 1.0 } else { 0.0 }
}

// -------------------------------------------------------------------------------------------
// Design C -- step pools (fill then spill)
// -------------------------------------------------------------------------------------------
//
// Two mirrored staircases of wide, shallow pools (left and right), each independently zigzagging
// between an OUTER x-range (near its own wall) and an INNER x-range (near the centre gap),
// stepping down the vessel, both draining into one shared collector at the bottom. Mirroring the
// left staircase's rects gives the right staircase for free, so the whole picture is exactly
// symmetric about dx = 0 by construction, while each individual staircase still alternates left
// and right within its own half.
//
// The "fill then spill" behaviour needs no separate lip parameter: consecutive pools are stacked
// directly (pool i's bottom edge is pool i+1's top edge) with PARTIALLY OVERLAPPING x-ranges.
// Where they overlap, pool i has no floor -- it is already open into pool i+1 below -- so that
// strip is a permanent drain. Everywhere else in pool i's footprint, pool i's own bottom edge IS
// a real floor (pool i+1 does not start until a lower y), so material lands there and must
// reach the overlap strip (by water self-levelling, or by a dry-sand slope towards it) before it
// can descend further. That's a real basin with a real, if partial, floor -- not a re-badged
// funnel.

const C_HW_FRAC: f32 = 0.44; // * w_f
const C_GAP_FRAC: f32 = 0.02; // * w_f, gap at the centre between the two staircases
const C_POOL_W_FRAC: f32 = 0.234; // * w_f, width of one pool -- tuned so OUTER/INNER overlap by
                                   // about a fifth of a pool's width (a real spillway notch, not
                                   // half the pool: an earlier version overlapped ~60% and every
                                   // pool read as a narrow vertical drainpipe rather than a wide
                                   // shallow basin).
const C_RES_W_FRAC: f32 = 0.8; // reservoir width as a fraction of a pool's width -- narrower than
                                // the pool it feeds so there is a visible step down into pool 0,
                                // rather than reservoir and pool 0 reading as one fused block.
const C_TOTAL_HALF_FRAC: f32 = 0.44; // * h_f
const C_RES_Y1_FRAC: f32 = -0.28; // * h_f, reservoir/pools boundary
const C_POOLS_Y1_FRAC: f32 = 0.08; // * h_f, pools/collector boundary (4 pools * 0.10h each)
const C_N_POOLS: usize = 4;

/// All rects as `[dx_lo, dx_hi, dy_lo, dy_hi]` in absolute cell units, already including the
/// mirrored right-hand pools -- a pure function of `w_f`/`h_f` (no `flipped`/dy-sign handling is
/// needed here beyond what the caller applies, same convention as `eval_sandbox_shape_at`).
fn c_rects(w_f: f32, h_f: f32) -> Vec<[f32; 4]> {
    let hw = C_HW_FRAC * w_f;
    let gap = C_GAP_FRAC * w_f;
    let pool_w = C_POOL_W_FRAC * w_f;
    let total_half = C_TOTAL_HALF_FRAC * h_f;
    let res_y0 = -total_half;
    let res_y1 = C_RES_Y1_FRAC * h_f;
    let pools_y1 = C_POOLS_Y1_FRAC * h_f;
    let col_y1 = total_half;

    let outer = (-hw, -hw + pool_w);
    let inner = (-gap - pool_w, -gap);
    // The reservoir sits flush against the OUTER wall -- as far as possible from pool 0's drain
    // notch (which is on its INNER/centre-facing side, where it overlaps `inner`) -- so material
    // must actually cross pool 0's floor to reach the drain instead of landing right beside it.
    // An earlier version centred the reservoir over the whole pool and it ended up straddling
    // the drain notch already, so everything just fell straight through with no visible pooling.
    let res_w = C_RES_W_FRAC * pool_w;
    let reservoir = (outer.0, outer.0 + res_w);

    let pool_h = (pools_y1 - res_y1) / C_N_POOLS as f32;
    let mut rects = Vec::new();
    for i in 0..C_N_POOLS {
        let y0 = res_y1 + i as f32 * pool_h;
        let y1 = y0 + pool_h;
        let (xlo, xhi) = if i % 2 == 0 { outer } else { inner };
        rects.push([xlo, xhi, y0, y1]);
        rects.push([-xhi, -xlo, y0, y1]); // mirrored right-hand pool
    }
    // Two reservoir compartments (one per staircase), narrower than pool 0 so the step down is
    // visible, centred over pool 0's own span.
    rects.push([reservoir.0, reservoir.1, res_y0, res_y1]);
    rects.push([-reservoir.1, -reservoir.0, res_y0, res_y1]);
    // Shared collector, full width.
    rects.push([-hw, hw, pools_y1, col_y1]);
    rects
}

fn c_inside(dx: f32, dy: f32, rects: &[[f32; 4]]) -> bool {
    rects.iter().any(|r| dx >= r[0] && dx < r[1] && dy >= r[2] && dy < r[3])
}

fn c_reservoir_rects(w_f: f32, h_f: f32) -> [[f32; 4]; 2] {
    let all = c_rects(w_f, h_f);
    // The last 3 rects pushed are [res_left, res_right, collector]; reservoirs are the two
    // before the collector.
    let n = all.len();
    [all[n - 3], all[n - 2]]
}

fn c_fill(dx: f32, dy: f32, w_f: f32, h_f: f32) -> f32 {
    for r in c_reservoir_rects(w_f, h_f) {
        if dx >= r[0] && dx < r[1] && dy >= r[2] && dy < r[3] {
            return 1.0;
        }
    }
    0.0
}

// -------------------------------------------------------------------------------------------
// Rendering
// -------------------------------------------------------------------------------------------

fn color_cell(mask: u8, height: f32, wetness: f32) -> [u8; 3] {
    if mask == MASK_OUTSIDE {
        return [60, 60, 64]; // outside: dark grey
    }
    if height < 0.01 {
        return [18, 18, 20]; // empty inside: near-black
    }
    let h_norm = (height / 1.2).clamp(0.0, 1.0);
    let tan = [210.0f32, 180.0, 140.0];
    let blue = [60.0f32, 110.0, 200.0];
    let mut rgb = [0u8; 3];
    for i in 0..3 {
        let base = tan[i] + (blue[i] - tan[i]) * wetness.clamp(0.0, 1.0);
        let v = base * (0.30 + 0.70 * h_norm);
        rgb[i] = v.clamp(0.0, 255.0) as u8;
    }
    rgb
}

fn render_frame(mask: &[u8], heights: &[f32], w: usize, h: usize, wetness: f32) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let px = color_cell(mask[idx], heights[idx], wetness);
            img.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    img
}

fn magnify_nearest(img: &image::RgbImage, factor: u32) -> image::RgbImage {
    let (w, h) = img.dimensions();
    let mut out = image::RgbImage::new(w * factor, h * factor);
    for y in 0..h {
        for x in 0..w {
            let p = *img.get_pixel(x, y);
            for dy in 0..factor {
                for dx in 0..factor {
                    out.put_pixel(x * factor + dx, y * factor + dy, p);
                }
            }
        }
    }
    out
}

fn contact_sheet(tiles: &[image::RgbImage], cols: usize) -> image::RgbImage {
    let cols = cols.min(tiles.len().max(1));
    let rows = (tiles.len() + cols - 1) / cols;
    let (tw, th) = tiles.first().map(|t| (t.width(), t.height())).unwrap_or((1, 1));
    let pad = 8u32;
    let mut sheet = image::RgbImage::from_pixel(
        cols as u32 * (tw + pad) + pad,
        rows as u32 * (th + pad) + pad,
        image::Rgb([235, 235, 235]),
    );
    for (i, tile) in tiles.iter().enumerate() {
        let (col, row) = (i % cols, i / cols);
        let ox = pad + col as u32 * (tw + pad);
        let oy = pad + row as u32 * (th + pad);
        image::imageops::overlay(&mut sheet, tile, ox as i64, oy as i64);
    }
    sheet
}

// -------------------------------------------------------------------------------------------
// Driving the simulation
// -------------------------------------------------------------------------------------------

struct DesignResult {
    name: &'static str,
    initial_mass: f32,
    capacity: usize,
    capacity_fraction: f32,
    water_mass_err: f32,
    sand_mass_err: f32,
}

fn build_sim(mask: Vec<u8>, fill: impl Fn(f32, f32, f32, f32) -> f32, w: usize) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.shape_mask = mask;
    sim.shape_mask_dirty = true;
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = w as f32 / 2.0;
    let w_f = w as f32;
    let h_f = w as f32;
    for y in 0..w {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            sim.heightmap.data[y * w + x] = fill(dx, dy, w_f, h_f);
        }
    }
    sim.temp_heights.copy_from_slice(&sim.heightmap.data);
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = 2.5; // page default; a no-op for dry sand per its own tooltip
    sim
}

/// Runs `sim` for `max_ticks` updates under `material`, snapshotting (mask clone once outside,
/// heights clone) at every tick number in `snapshot_ticks`. Returns one (tick, heights) pair per
/// requested snapshot, in order, plus the final mass.
fn run_material(
    sim: &mut DrawingSimulation,
    material: MaterialMode,
    snapshot_ticks: &[u32],
) -> (Vec<(u32, Vec<f32>)>, f32) {
    sim.apply_preset(material);
    let max_ticks = *snapshot_ticks.iter().max().unwrap();
    let mut snapshots = Vec::new();
    if snapshot_ticks.contains(&0) {
        snapshots.push((0u32, sim.heightmap.data.clone()));
    }
    let targets = [None; 5];
    for t in 1..=max_ticks {
        sim.update(0.016, &targets, 0.08, material, sim.sandbox_shape, 16.0, 16.0);
        if snapshot_ticks.contains(&t) {
            snapshots.push((t, sim.heightmap.data.clone()));
        }
    }
    let final_mass: f32 = sim.heightmap.data.iter().sum();
    (snapshots, final_mass)
}

fn wetness_of(material: MaterialMode) -> f32 {
    match material {
        MaterialMode::Water => 1.0,
        MaterialMode::DrySand => 0.0,
        _ => 0.5,
    }
}

fn run_design(
    name: &'static str,
    out_dir: &Path,
    mask: Vec<u8>,
    fill: impl Fn(f32, f32, f32, f32) -> f32,
    is_reservoir: impl Fn(f32, f32) -> bool,
    is_collector: impl Fn(f32, f32) -> bool,
) -> DesignResult {
    println!("\n=== {name} ===");

    let mismatches = mirror_mismatches(&mask, GRID, GRID);
    let (reached, total_inside, reached_collector) =
        connectivity(&mask, GRID, GRID, &is_reservoir, &is_collector);
    println!(
        "  mirror mismatches: {mismatches}  |  connectivity: {reached}/{total_inside} inside cells reached from reservoir, collector reached = {reached_collector}"
    );
    if mismatches > 0 {
        println!("  WARNING: shape is not left-right symmetric!");
    }
    if reached != total_inside {
        println!(
            "  WARNING: {} inside cell(s) NOT reachable from the reservoir -- sealed pocket(s).",
            total_inside - reached
        );
    }
    if !reached_collector {
        println!("  WARNING: collector not reachable from reservoir -- the network does not drain.");
    }

    // Bare mask picture.
    let mask_img = {
        let mut img = image::RgbImage::new(GRID as u32, GRID as u32);
        for y in 0..GRID {
            for x in 0..GRID {
                let m = mask[y * GRID + x];
                let px = match m {
                    MASK_OUTSIDE => [40u8, 40, 44],
                    MASK_BOUNDARY => [255, 255, 255],
                    _ => [190, 190, 196],
                };
                img.put_pixel(x as u32, y as u32, image::Rgb(px));
            }
        }
        magnify_nearest(&img, 2)
    };
    mask_img
        .save(out_dir.join(format!("{name}_mask.png")))
        .expect("write mask png");

    let sim = build_sim(mask.clone(), &fill, GRID);
    let initial_mass: f32 = sim.heightmap.data.iter().sum();
    let capacity = mask.iter().filter(|&&m| m != MASK_OUTSIDE).count();
    let capacity_fraction = initial_mass / capacity as f32;
    println!(
        "  initial mass = {initial_mass:.1}, capacity = {capacity} cells, fraction = {:.3}",
        capacity_fraction
    );

    // Water run.
    let water_ticks = [0u32, 12, 40, 150];
    let mut water_sim = build_sim(mask.clone(), &fill, GRID);
    let (water_snaps, water_final) = run_material(&mut water_sim, MaterialMode::Water, &water_ticks);
    let water_mass_err = (water_final - initial_mass).abs() / initial_mass;
    println!(
        "  water: final mass = {water_final:.1}, conservation err = {:.6}",
        water_mass_err
    );

    // Dry sand run.
    let sand_ticks = [0u32, 50, 200, 800];
    let mut sand_sim = build_sim(mask.clone(), &fill, GRID);
    let (sand_snaps, sand_final) = run_material(&mut sand_sim, MaterialMode::DrySand, &sand_ticks);
    let sand_mass_err = (sand_final - initial_mass).abs() / initial_mass;
    println!(
        "  dry sand: final mass = {sand_final:.1}, conservation err = {:.6}",
        sand_mass_err
    );

    // Region breakdown at the final tick, to spot stuck material.
    for (label, snaps, wet) in [
        ("water", &water_snaps, wetness_of(MaterialMode::Water)),
        ("dry sand", &sand_snaps, wetness_of(MaterialMode::DrySand)),
    ] {
        let (_t, heights) = snaps.last().unwrap();
        let center_x = (GRID - 1) as f32 / 2.0;
        let center_y = GRID as f32 / 2.0;
        let mut res_mass = 0.0f32;
        let mut col_mass = 0.0f32;
        let mut mid_mass = 0.0f32;
        for y in 0..GRID {
            let dy = y as f32 - center_y;
            for x in 0..GRID {
                let dx = x as f32 - center_x;
                let hgt = heights[y * GRID + x];
                if is_reservoir(dx, dy) {
                    res_mass += hgt;
                } else if is_collector(dx, dy) {
                    col_mass += hgt;
                } else {
                    mid_mass += hgt;
                }
            }
        }
        println!(
            "    [{label}, wetness {wet}] final distribution: reservoir={res_mass:.1} network={mid_mass:.1} collector={col_mass:.1}"
        );
    }

    // Contact sheet: rows = water / dry sand, columns = the 4 ticks.
    let mut tiles = Vec::new();
    for (_t, heights) in &water_snaps {
        tiles.push(magnify_nearest(
            &render_frame(&mask, heights, GRID, GRID, wetness_of(MaterialMode::Water)),
            2,
        ));
    }
    for (_t, heights) in &sand_snaps {
        tiles.push(magnify_nearest(
            &render_frame(&mask, heights, GRID, GRID, wetness_of(MaterialMode::DrySand)),
            2,
        ));
    }
    let sheet = contact_sheet(&tiles, 4);
    sheet
        .save(out_dir.join(format!("{name}_contact_sheet.png")))
        .expect("write contact sheet");

    DesignResult {
        name,
        initial_mass,
        capacity,
        capacity_fraction,
        water_mass_err,
        sand_mass_err,
    }
}

fn main() {
    let out_dir = Path::new("artifacts/design/cascade-2026-09-17");
    fs::create_dir_all(out_dir).expect("create output dir");

    let w_f = GRID as f32;
    let h_f = GRID as f32;

    // --- Baseline: today's MultiStageHourglass at its shipped defaults (n=8 chambers,
    // neck_width=0.005, hourglass_curve=0.6), using the REAL production code path
    // (generate_shape_mask + initialize_hourglass) rather than a reimplementation, so this
    // shows today's actual start exactly as the app ships it. ---
    println!("\n=== baseline (MultiStageHourglass, shipped defaults) ===");
    let mut base_sim = DrawingSimulation::new_with_size(GRID);
    base_sim.sandbox_shape = SandboxShape::MultiStageHourglass;
    base_sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    base_sim.lateral_substeps = 2.5;
    base_sim.initialize_hourglass(); // regenerates shape_mask AND fills tier 0
    let base_mask = base_sim.shape_mask.clone();
    let base_initial_mass: f32 = base_sim.heightmap.data.iter().sum();
    let base_capacity = base_mask.iter().filter(|&&m| m != MASK_OUTSIDE).count();
    let base_fraction = base_initial_mass / base_capacity as f32;
    println!(
        "  initial mass = {base_initial_mass:.1}, capacity = {base_capacity} cells, fraction = {:.3}",
        base_fraction
    );
    let base_mismatches = mirror_mismatches(&base_mask, GRID, GRID);
    println!("  mirror mismatches: {base_mismatches} (expect 0 -- shipped shape is already fixed)");

    let base_mask_img = {
        let mut img = image::RgbImage::new(GRID as u32, GRID as u32);
        for y in 0..GRID {
            for x in 0..GRID {
                let m = base_mask[y * GRID + x];
                let px = match m {
                    MASK_OUTSIDE => [40u8, 40, 44],
                    MASK_BOUNDARY => [255, 255, 255],
                    _ => [190, 190, 196],
                };
                img.put_pixel(x as u32, y as u32, image::Rgb(px));
            }
        }
        magnify_nearest(&img, 2)
    };
    base_mask_img
        .save(out_dir.join("baseline_mask.png"))
        .expect("write mask png");

    let water_ticks = [0u32, 12, 40, 150];
    let sand_ticks = [0u32, 50, 200, 800];

    let mut base_water_sim = DrawingSimulation::new_with_size(GRID);
    base_water_sim.sandbox_shape = SandboxShape::MultiStageHourglass;
    base_water_sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    base_water_sim.lateral_substeps = 2.5;
    base_water_sim.initialize_hourglass();
    let (base_water_snaps, base_water_final) =
        run_material(&mut base_water_sim, MaterialMode::Water, &water_ticks);
    let base_water_err = (base_water_final - base_initial_mass).abs() / base_initial_mass;
    println!("  water: final mass = {base_water_final:.1}, conservation err = {:.6}", base_water_err);

    let mut base_sand_sim = DrawingSimulation::new_with_size(GRID);
    base_sand_sim.sandbox_shape = SandboxShape::MultiStageHourglass;
    base_sand_sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    base_sand_sim.lateral_substeps = 2.5;
    base_sand_sim.initialize_hourglass();
    let (base_sand_snaps, base_sand_final) =
        run_material(&mut base_sand_sim, MaterialMode::DrySand, &sand_ticks);
    let base_sand_err = (base_sand_final - base_initial_mass).abs() / base_initial_mass;
    println!("  dry sand: final mass = {base_sand_final:.1}, conservation err = {:.6}", base_sand_err);

    let mut base_tiles = Vec::new();
    for (_t, heights) in &base_water_snaps {
        base_tiles.push(magnify_nearest(
            &render_frame(&base_mask, heights, GRID, GRID, wetness_of(MaterialMode::Water)),
            2,
        ));
    }
    for (_t, heights) in &base_sand_snaps {
        base_tiles.push(magnify_nearest(
            &render_frame(&base_mask, heights, GRID, GRID, wetness_of(MaterialMode::DrySand)),
            2,
        ));
    }
    contact_sheet(&base_tiles, 4)
        .save(out_dir.join("baseline_contact_sheet.png"))
        .expect("write contact sheet");

    // --- Design A ---
    let a_mask = rasterize(GRID, GRID, |dx, dy| a_inside(dx, dy, w_f, h_f));
    let (_th, a_res_y1, _n1, _n2, _hw) = a_geometry(w_f, h_f);
    let (_th2, _r1, a_net_y1, _n3, _hw2) = a_geometry(w_f, h_f);
    let a_result = run_design(
        "A_split_merge",
        out_dir,
        a_mask,
        |dx, dy, w, h| a_fill(dx, dy, w, h),
        move |_dx, dy| dy < a_res_y1,
        move |_dx, dy| dy >= a_net_y1,
    );

    // --- Design C ---
    let c_rects_cached = c_rects(w_f, h_f);
    let c_mask = rasterize(GRID, GRID, |dx, dy| c_inside(dx, dy, &c_rects_cached));
    let c_res_y1 = C_RES_Y1_FRAC * h_f;
    let c_pools_y1 = C_POOLS_Y1_FRAC * h_f;
    let c_result = run_design(
        "C_step_pools",
        out_dir,
        c_mask,
        |dx, dy, w, h| c_fill(dx, dy, w, h),
        move |_dx, dy| dy < c_res_y1,
        move |_dx, dy| dy >= c_pools_y1,
    );

    println!("\n=== summary ===");
    println!(
        "baseline: mass={base_initial_mass:.1} capacity_fraction={:.3} water_err={:.6} sand_err={:.6}",
        base_fraction, base_water_err, base_sand_err
    );
    for r in [&a_result, &c_result] {
        println!(
            "{}: mass={:.1} capacity={} capacity_fraction={:.3} water_err={:.6} sand_err={:.6}",
            r.name, r.initial_mass, r.capacity, r.capacity_fraction, r.water_mass_err, r.sand_mass_err
        );
    }
}
