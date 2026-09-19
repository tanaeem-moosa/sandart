//! PROTOTYPE ONLY. Round 2. Renders pictures of candidate redesigns for the "Merging cascade"
//! vessel (`SandboxShape::MultiStageHourglass`) for the user to pick between. Does not touch
//! `sandart-sim/src`, the renderer, wasm or the UI.
//!
//! ROUND 1 (kept as `*_r1_*` files) shipped A (split/merge) and C (step pools) but the actual
//! pictures did not show what round 1's report claimed:
//!   - A rendered as a regular lattice of parallel teeth; nothing visibly divided or rejoined.
//!   - C's material bypassed the pools entirely (fell straight down the sides); no pool visibly
//!     filled or spilled.
//!   - Both started BELOW today's shape's capacity fraction (22%/11% vs today's 26%), which does
//!     not fix the user's actual complaint ("not enough material").
//!
//! ROUND 2 fixes: B0 (control -- today's shape, unchanged geometry, bigger reservoir), A2 (real
//! enclosed brick chambers with two corner holes each, landing in two DIFFERENT chambers below),
//! C2 (real weir pools -- the only outlet is a lip near the top of the downstream wall, never a
//! floor drain or side gap, so a pool must nearly fill before anything reaches the next one). A
//! colour tracer (reservoir left half vs right half in two colours, which advect with the
//! material) makes dividing/rejoining and fill order verifiable from the picture, not asserted.
//!
//! Background: artifacts/tickets/24-*.md, 32-*.md, 41-*.md; `MultiStageHourglass` in
//! sandart-sim/src/physics.rs (~2940); `initialize_hourglass` in sandart-sim/src/lib.rs.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && cargo run -p sandart-sim --release --example proto_cascades'
//!
//! Writes PNGs and a README to artifacts/design/cascade-2026-09-17/.

use sandart_sim::{
    color_channel, pack_rgba, DrawingSimulation, MaterialMode, SandboxShape, MASK_BOUNDARY,
    MASK_INSIDE, MASK_OUTSIDE,
};
use std::fs;
use std::path::Path;

const GRID: usize = 256;

// -------------------------------------------------------------------------------------------
// Shared rasterization / diagnostics
// -------------------------------------------------------------------------------------------

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

fn capacity(mask: &[u8]) -> usize {
    mask.iter().filter(|&&m| m != MASK_OUTSIDE).count()
}

/// Flood-fill connectivity from every reservoir-region cell over 4-connected inside cells.
/// Returns (reached_count, total_inside_count, reached_collector).
fn connectivity(
    mask: &[u8],
    w: usize,
    h: usize,
    is_reservoir: impl Fn(f32, f32) -> bool,
    is_collector: impl Fn(f32, f32) -> bool,
) -> (usize, usize, bool) {
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let total_inside = capacity(mask);

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

/// Finds the smallest `dy` threshold such that the number of inside cells with `dy < threshold`
/// is >= `target_area`, by scanning whole rows top-to-bottom. Row-granular (not sub-cell), which
/// is fine at grid 256.
fn threshold_for_area(mask: &[u8], w: usize, h: usize, target_area: usize) -> f32 {
    let center_y = h as f32 / 2.0;
    let mut acc = 0usize;
    for y in 0..h {
        let row_count = (0..w).filter(|&x| mask[y * w + x] != MASK_OUTSIDE).count();
        acc += row_count;
        if acc >= target_area {
            return (y as f32 + 1.0) - center_y;
        }
    }
    h as f32 - center_y
}

// -------------------------------------------------------------------------------------------
// Colour tracer
// -------------------------------------------------------------------------------------------

const TRACER_LEFT: (u8, u8, u8) = (225, 40, 40); // red
const TRACER_RIGHT: (u8, u8, u8) = (40, 200, 90); // green

fn set_tracer_colors(sim: &mut DrawingSimulation, mask: &[u8], w: usize, is_reservoir: impl Fn(f32, f32) -> bool) {
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = w as f32 / 2.0;
    for y in 0..w {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            let idx = y * w + x;
            if mask[idx] == MASK_OUTSIDE || !is_reservoir(dx, dy) {
                continue;
            }
            let (r, g, b) = if dx < 0.0 { TRACER_LEFT } else { TRACER_RIGHT };
            sim.cell_colors[idx] = pack_rgba(r, g, b, 255);
        }
    }
}

fn render_tracer_frame(mask: &[u8], heights: &[f32], colors: &[u32], w: usize, h: usize) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let px = if mask[idx] == MASK_OUTSIDE {
                [60u8, 60, 64]
            } else if heights[idx] < 0.01 {
                [18, 18, 20]
            } else {
                let h_norm = (heights[idx] / 1.2).clamp(0.0, 1.0);
                let r = color_channel(colors[idx], 0) as f32 * (0.45 + 0.55 * h_norm);
                let g = color_channel(colors[idx], 1) as f32 * (0.45 + 0.55 * h_norm);
                let b = color_channel(colors[idx], 2) as f32 * (0.45 + 0.55 * h_norm);
                [r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8]
            };
            img.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    img
}

// -------------------------------------------------------------------------------------------
// Rendering (height/wetness view)
// -------------------------------------------------------------------------------------------

fn color_cell(mask: u8, height: f32, wetness: f32) -> [u8; 3] {
    if mask == MASK_OUTSIDE {
        return [60, 60, 64];
    }
    if height < 0.01 {
        return [18, 18, 20];
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

fn mask_image(mask: &[u8], w: usize, h: usize) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let m = mask[y * w + x];
            let px = match m {
                MASK_OUTSIDE => [40u8, 40, 44],
                MASK_BOUNDARY => [255, 255, 255],
                _ => [190, 190, 196],
            };
            img.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    magnify_nearest(&img, 2)
}

// -------------------------------------------------------------------------------------------
// Sim plumbing
// -------------------------------------------------------------------------------------------

fn wetness_of(material: MaterialMode) -> f32 {
    match material {
        MaterialMode::Water => 1.0,
        MaterialMode::DrySand => 0.0,
        _ => 0.5,
    }
}

fn build_sim(mask: Vec<u8>, fill: impl Fn(f32, f32) -> f32, w: usize) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.shape_mask = mask;
    sim.shape_mask_dirty = true;
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = w as f32 / 2.0;
    for y in 0..w {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            sim.heightmap.data[y * w + x] = fill(dx, dy);
        }
    }
    sim.temp_heights.copy_from_slice(&sim.heightmap.data);
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = 2.5;
    sim
}

struct Snap {
    tick: u32,
    heights: Vec<f32>,
    colors: Vec<u32>,
}

fn run_material(sim: &mut DrawingSimulation, material: MaterialMode, snapshot_ticks: &[u32]) -> (Vec<Snap>, f32) {
    sim.apply_preset(material);
    let max_ticks = *snapshot_ticks.iter().max().unwrap();
    let mut snaps = Vec::new();
    if snapshot_ticks.contains(&0) {
        snaps.push(Snap { tick: 0, heights: sim.heightmap.data.clone(), colors: sim.cell_colors.clone() });
    }
    let targets = [None; 5];
    for t in 1..=max_ticks {
        sim.update(0.016, &targets, 0.08, material, sim.sandbox_shape, 16.0, 16.0);
        if snapshot_ticks.contains(&t) {
            snaps.push(Snap { tick: t, heights: sim.heightmap.data.clone(), colors: sim.cell_colors.clone() });
        }
    }
    let final_mass: f32 = sim.heightmap.data.iter().sum();
    (snaps, final_mass)
}

/// Diagnostic only: prints mass in reservoir/network/collector every `step` ticks up to
/// `max_tick`, so sensible snapshot ticks can be picked from measured fill/spill times rather
/// than guessed (round 1's guess of 0/12/40/150 was too short to show pools filling).
fn trace_mass(
    name: &str,
    sim: &mut DrawingSimulation,
    material: MaterialMode,
    max_tick: u32,
    step: u32,
    is_reservoir: impl Fn(f32, f32) -> bool,
    is_collector: impl Fn(f32, f32) -> bool,
) {
    sim.apply_preset(material);
    let w = GRID;
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = w as f32 / 2.0;
    let targets = [None; 5];
    println!("  -- mass trace [{name}, {material:?}] --");
    for t in 0..=max_tick {
        if t % step == 0 {
            let mut res = 0.0f32;
            let mut col = 0.0f32;
            let mut mid = 0.0f32;
            for y in 0..w {
                let dy = y as f32 - center_y;
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    let hgt = sim.heightmap.data[y * w + x];
                    if is_reservoir(dx, dy) {
                        res += hgt;
                    } else if is_collector(dx, dy) {
                        col += hgt;
                    } else {
                        mid += hgt;
                    }
                }
            }
            println!("    tick {t:5}: reservoir={res:8.1} network={mid:8.1} collector={col:8.1}");
        }
        sim.update(0.016, &targets, 0.08, material, sim.sandbox_shape, 16.0, 16.0);
    }
}

// -------------------------------------------------------------------------------------------
// B0 -- control: today's MultiStageHourglass, unchanged geometry, reservoir enlarged to the
// material target (>= 50% of the capacity of the network below the reservoir, i.e. reservoir
// area >= capacity/3 of the whole shape, since here the reservoir is carved OUT OF a fixed total
// rather than added on top).
// -------------------------------------------------------------------------------------------

fn b0_base_mask() -> Vec<u8> {
    let mut sim = DrawingSimulation::new_with_size(GRID);
    sim.sandbox_shape = SandboxShape::MultiStageHourglass;
    sim.generate_shape_mask();
    sim.shape_mask
}

// -------------------------------------------------------------------------------------------
// A2 -- real split and merge: enclosed chambers ("closed bowls") in a brick layout, each row
// offset by half a chamber. Each FULL chamber is a plain box for most of its height (a real
// bowl, not a taper) with two small, FIXED-size holes near its lower corners for the last
// fraction of its height; each half-width edge chamber (brick layout's row-end chambers) gets
// one hole, on its inward side. The `cw` chamber-width unit is identical across every tier,
// which is what keeps a parent chamber's centre always exactly on a child tier's boundary --
// every hole lands inside, never on the wall of, a DIFFERENT chamber in the tier below.
// -------------------------------------------------------------------------------------------

const A_HW_FRAC: f32 = 0.42; // * w_f, half-width of the network envelope
const A_COLS0: u32 = 4; // even-tier chamber count (odd tiers get COLS0+1: COLS0-1 full + 2 half)
const A_NET_TIERS: usize = 4; // alternating 4, 5, 4, 5
const A_COL_FRAC_OF_NET: f32 = 0.20; // fraction of (tiers+collector) height given to the collector
const A_HOLE_FRAC: f32 = 0.78; // t > this: plain box; t <= this: the two (or one) holes only
const A_HOLE_HW_FRAC: f32 = 0.08; // * cw, half-width of each hole (>= 2 cells at grid 256)

fn a_tier_cols(tier: usize) -> u32 {
    if tier % 2 == 0 { A_COLS0 } else { A_COLS0 + 1 }
}

/// Chamber/collector geometry as a pure function of dy, given the network's own vertical span
/// `net_y0..net_y1` (NOT including the reservoir, which is sized and placed by the caller).
fn a_network_inside(dx: f32, dy: f32, w_f: f32, net_y0: f32, net_y1: f32) -> bool {
    if dy < net_y0 || dy >= net_y1 {
        return false;
    }
    let hw = A_HW_FRAC * w_f;
    let net_h = net_y1 - net_y0;
    let col_h = A_COL_FRAC_OF_NET * net_h;
    let tiers_h = net_h - col_h;
    let tiers_y1 = net_y0 + tiers_h;
    if dy >= tiers_y1 {
        return dx.abs() < hw; // collector: plain open box
    }

    let tier_h = tiers_h / A_NET_TIERS as f32;
    let rel = dy - net_y0;
    let tier = ((rel / tier_h).floor() as usize).min(A_NET_TIERS - 1);
    let y1 = net_y0 + (tier as f32 + 1.0) * tier_h;

    let cols = a_tier_cols(tier);
    let cw = 2.0 * hw / A_COLS0 as f32;
    let is_even = tier % 2 == 0;

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

    let half_w_slot = (b1 - b0) / 2.0;
    let center = (b0 + b1) / 2.0;
    let dx_local = dx - center;
    let t = ((y1 - dy) / tier_h).clamp(0.0, 1.0); // 1 at chamber top, 0 at its floor
    let full_chamber = (b1 - b0) > cw * 0.75;
    let hole_hw = (A_HOLE_HW_FRAC * cw).max(1.0);
    // A chamber's SLOT is shared exactly with its neighbours (no gap) -- that is what keeps a
    // parent's hole always landing inside a child's slot. But testing the box regime against the
    // full slot half-width made every "closed bowl" abut its neighbour with no wall between them
    // at all, so the top ~80% of every tier rendered as one continuous open trough, not separate
    // enclosed chambers (caught from the picture, not asserted: the mask showed one solid band
    // per tier with dark slits only in the bottom holes strip). WALL_MARGIN carves a real
    // separating wall out of each chamber's OWN half of its slot, so this is now a real gap
    // between neighbours at every height, not just at the holes.
    const WALL_MARGIN: f32 = 3.0;
    let half_w = (half_w_slot - WALL_MARGIN).max(hole_hw * 1.5);

    if t > A_HOLE_FRAC {
        return dx_local.abs() < half_w; // plain closed-bowl box
    }
    // Bottom slice: two (or one) FIXED-size holes, not a taper -- reads as literal drain holes
    // in an otherwise flat floor, not a funnel.
    let shift = 0.5 * half_w;
    if full_chamber {
        (dx_local - (-shift)).abs() < hole_hw || (dx_local - shift).abs() < hole_hw
    } else {
        let inward = if slot == 0 { 1.0 } else { -1.0 };
        debug_assert!(slot == 0 || slot == cols_here - 1, "half chamber not at a tier edge");
        (dx_local - inward * shift).abs() < hole_hw
    }
}

// -------------------------------------------------------------------------------------------
// C2 -- real step pools with a WEIR outlet. The reservoir is just pool index 0 in the same
// alternating chain (OUTER/INNER/OUTER/INNER, spilling right/left/right/left), pre-filled at
// t=0; pools 1..N are empty at t=0. Every pool-to-pool transition (including reservoir -> pool
// 1) is a weir: a short opening near the TOP of the downstream wall (so most of the pool's
// depth must fill before anything can pass) leading into a narrow chute that runs, physically
// separate from the pool's own body (a real wall gap in between), down into the next pool's own
// footprint. There is no floor drain and no gap along the pool's side below the weir -- verified
// by the connectivity check (there is exactly one path out of each pool, through its weir) and
// by the picture (a pool must visibly fill before the next one gets anything).
// -------------------------------------------------------------------------------------------

const C_HW_FRAC: f32 = 0.44; // * w_f
const C_GAP_FRAC: f32 = 0.02; // * w_f, gap at the centre between the two staircases
const C_POOL_W_FRAC: f32 = 0.30; // * w_f, width of one pool (and of the reservoir, same slot)
const C_WALL_GAP_FRAC: f32 = 0.012; // * w_f, real wall between a pool and its downstream chute
const C_CHUTE_W_FRAC: f32 = 0.05; // * w_f, chute width (>= 2 cells at grid 256)
const C_WEIR_FRAC: f32 = 0.15; // fraction of a pool's OWN height that is open at the top (weir)
const C_POOL_H_FRAC: f32 = 0.09; // * h_f, height of pools 1..N (pool 0 / reservoir height varies)
const C_N_CHAIN: usize = 4; // pool 0 (reservoir) + 3 downstream pools, then the collector
const C_TOTAL_HALF_FRAC: f32 = 0.44; // * h_f

/// All rects (both staircases mirrored) for the chain reservoir(pool0) -> pool1 -> ... -> poolN
/// -> collector, as `[dx_lo, dx_hi, dy_lo, dy_hi]`. `res_h` is the reservoir's (pool 0's) own
/// height in cells -- the free parameter the caller sizes to hit the material target. Passing
/// `res_h = 0.0` degenerates pool 0 and its weir into zero-area, which is how the "network below
/// the reservoir" capacity is measured (see `c2_network_capacity`).
fn c2_rects(w_f: f32, h_f: f32, res_h: f32) -> Vec<[f32; 4]> {
    let hw = C_HW_FRAC * w_f;
    let gap = C_GAP_FRAC * w_f;
    let pool_w = C_POOL_W_FRAC * w_f;
    let wall_gap = C_WALL_GAP_FRAC * w_f;
    let chute_w = C_CHUTE_W_FRAC * w_f;
    let pool_h = C_POOL_H_FRAC * h_f;
    let total_half = C_TOTAL_HALF_FRAC * h_f;

    let outer = (-hw, -hw + pool_w);
    let inner = (-gap - pool_w, -gap);

    let mut rects = Vec::new();
    let mut y = -total_half;
    let mut prev_lo_hi: Option<(f32, f32)> = None; // previous pool's (x_lo, x_hi)
    for i in 0..C_N_CHAIN {
        let h_i = if i == 0 { res_h } else { pool_h };
        let (x_lo, x_hi) = if i % 2 == 0 { outer } else { inner };
        let y_lo = y;
        let y_hi = y + h_i;
        rects.push([x_lo, x_hi, y_lo, y_hi]); // the pool box itself
        rects.push([-x_hi, -x_lo, y_lo, y_hi]); // mirrored right-hand pool

        // Weir + chute down to the NEXT pool (or the collector, for the last one in the chain).
        let spill_right = i % 2 == 0;
        let weir_y1 = y_lo + C_WEIR_FRAC * h_i;
        let (chute_lo, chute_hi) = if spill_right {
            let lo = x_hi + wall_gap;
            (lo, lo + chute_w)
        } else {
            let hi = x_lo - wall_gap;
            (hi - chute_w, hi)
        };
        let connector_x = if spill_right { (x_hi, chute_hi) } else { (chute_lo, x_lo) };
        // The next pool's top -- or, for the last pool in the chain, just past the collector's
        // own top so the chute unambiguously lands inside it.
        let next_top = y_hi; // pools stack with no vertical gap; collector starts at the same y.
        let chute_bottom = next_top + pool_h.min(6.0); // land solidly inside whatever is next
        rects.push([connector_x.0, connector_x.1, y_lo, weir_y1]); // weir slit near the top
        rects.push([chute_lo, chute_hi, weir_y1, chute_bottom]); // chute down to the next pool
        rects.push([-connector_x.1, -connector_x.0, y_lo, weir_y1]); // mirrored
        rects.push([-chute_hi, -chute_lo, weir_y1, chute_bottom]); // mirrored

        prev_lo_hi = Some((x_lo, x_hi));
        y = y_hi;
    }
    let _ = prev_lo_hi;
    // Shared collector, full width, from where the chain left off to the bottom of the vessel.
    rects.push([-hw, hw, y, total_half]);
    rects
}

fn c2_inside(dx: f32, dy: f32, rects: &[[f32; 4]]) -> bool {
    rects.iter().any(|r| dx >= r[0] && dx < r[1] && dy >= r[2] && dy < r[3])
}

fn c2_pool0_y0(h_f: f32) -> f32 {
    -C_TOTAL_HALF_FRAC * h_f
}

// -------------------------------------------------------------------------------------------
// Driving the simulation
// -------------------------------------------------------------------------------------------

struct DesignResult {
    name: &'static str,
    initial_mass: f32,
    network_capacity: usize,
    fraction_of_network: f32,
    water_mass_err: f32,
    sand_mass_err: f32,
}

#[allow(clippy::too_many_arguments)]
fn run_design(
    name: &'static str,
    out_dir: &Path,
    mask: Vec<u8>,
    fill: impl Fn(f32, f32) -> f32,
    is_reservoir: impl Fn(f32, f32) -> bool + Copy,
    is_collector: impl Fn(f32, f32) -> bool + Copy,
    network_capacity: usize,
    water_ticks: &[u32],
    sand_ticks: &[u32],
) -> DesignResult {
    println!("\n=== {name} ===");

    let mismatches = mirror_mismatches(&mask, GRID, GRID);
    let (reached, total_inside, reached_collector) = connectivity(&mask, GRID, GRID, is_reservoir, is_collector);
    println!(
        "  mirror mismatches: {mismatches}  |  connectivity: {reached}/{total_inside} reached, collector reached = {reached_collector}"
    );
    if mismatches > 0 {
        println!("  WARNING: shape is not left-right symmetric!");
    }
    if reached != total_inside {
        println!("  WARNING: {} inside cell(s) not reachable from reservoir -- sealed pocket(s).", total_inside - reached);
    }
    if !reached_collector {
        println!("  WARNING: collector not reachable -- the network does not drain.");
    }

    mask_image(&mask, GRID, GRID)
        .save(out_dir.join(format!("{name}_mask.png")))
        .expect("write mask png");

    let sim0 = build_sim(mask.clone(), &fill, GRID);
    let initial_mass: f32 = sim0.heightmap.data.iter().sum();
    let fraction_of_network = initial_mass / network_capacity as f32;
    println!(
        "  initial mass = {initial_mass:.1}, network capacity = {network_capacity} cells, fraction of network = {fraction_of_network:.3}"
    );

    let mut water_sim = build_sim(mask.clone(), &fill, GRID);
    set_tracer_colors(&mut water_sim, &mask, GRID, is_reservoir);
    let (water_snaps, water_final) = run_material(&mut water_sim, MaterialMode::Water, water_ticks);
    let water_mass_err = (water_final - initial_mass).abs() / initial_mass;
    println!("  water: final mass = {water_final:.1}, conservation err = {:.6}", water_mass_err);

    let mut sand_sim = build_sim(mask.clone(), &fill, GRID);
    set_tracer_colors(&mut sand_sim, &mask, GRID, is_reservoir);
    let (sand_snaps, sand_final) = run_material(&mut sand_sim, MaterialMode::DrySand, sand_ticks);
    let sand_mass_err = (sand_final - initial_mass).abs() / initial_mass;
    println!("  dry sand: final mass = {sand_final:.1}, conservation err = {:.6}", sand_mass_err);

    for (label, snaps) in [("water", &water_snaps), ("dry sand", &sand_snaps)] {
        let last = snaps.last().unwrap();
        let center_x = (GRID - 1) as f32 / 2.0;
        let center_y = GRID as f32 / 2.0;
        let mut res_mass = 0.0f32;
        let mut col_mass = 0.0f32;
        let mut mid_mass = 0.0f32;
        for y in 0..GRID {
            let dy = y as f32 - center_y;
            for x in 0..GRID {
                let dx = x as f32 - center_x;
                let hgt = last.heights[y * GRID + x];
                if is_reservoir(dx, dy) {
                    res_mass += hgt;
                } else if is_collector(dx, dy) {
                    col_mass += hgt;
                } else {
                    mid_mass += hgt;
                }
            }
        }
        println!("    [{label}] final (tick {}) distribution: reservoir={res_mass:.1} network={mid_mass:.1} collector={col_mass:.1}", last.tick);
    }

    // Normal (height/wetness) contact sheet.
    let mut tiles = Vec::new();
    for s in &water_snaps {
        tiles.push(magnify_nearest(&render_frame(&mask, &s.heights, GRID, GRID, wetness_of(MaterialMode::Water)), 2));
    }
    for s in &sand_snaps {
        tiles.push(magnify_nearest(&render_frame(&mask, &s.heights, GRID, GRID, wetness_of(MaterialMode::DrySand)), 2));
    }
    contact_sheet(&tiles, water_ticks.len().max(sand_ticks.len()))
        .save(out_dir.join(format!("{name}_contact_sheet.png")))
        .expect("write contact sheet");

    // Tracer contact sheet.
    let mut tracer_tiles = Vec::new();
    for s in &water_snaps {
        tracer_tiles.push(magnify_nearest(&render_tracer_frame(&mask, &s.heights, &s.colors, GRID, GRID), 2));
    }
    for s in &sand_snaps {
        tracer_tiles.push(magnify_nearest(&render_tracer_frame(&mask, &s.heights, &s.colors, GRID, GRID), 2));
    }
    contact_sheet(&tracer_tiles, water_ticks.len().max(sand_ticks.len()))
        .save(out_dir.join(format!("{name}_tracer_sheet.png")))
        .expect("write tracer sheet");

    DesignResult {
        name,
        initial_mass,
        network_capacity,
        fraction_of_network,
        water_mass_err,
        sand_mass_err,
    }
}

fn main() {
    let out_dir = Path::new("artifacts/design/cascade-2026-09-17");
    fs::create_dir_all(out_dir).expect("create output dir");

    let w_f = GRID as f32;
    let h_f = GRID as f32;
    let do_trace = std::env::var("TRACE_MASS").is_ok();

    // ===================================================================================
    // B0 -- control
    // ===================================================================================
    let base_mask = b0_base_mask();
    let base_total_capacity = capacity(&base_mask);
    // reservoir_area >= 0.5 * (total - reservoir_area)  =>  reservoir_area >= total / 3.
    // Aim comfortably above the floor (0.40 of total) so rounding to whole rows doesn't leave it
    // just under 50% of the remaining network.
    let b0_target_area = (base_total_capacity as f32 * 0.40).ceil() as usize;
    let b0_threshold = threshold_for_area(&base_mask, GRID, GRID, b0_target_area);
    let b0_is_reservoir = move |_dx: f32, dy: f32| dy < b0_threshold;
    let b0_is_collector = move |_dx: f32, dy: f32| dy > 0.30 * h_f; // today's bottom chamber
    let b0_network_capacity = base_total_capacity
        - base_mask
            .iter()
            .enumerate()
            .filter(|&(i, &m)| {
                m != MASK_OUTSIDE && {
                    let y = i / GRID;
                    (y as f32 - h_f / 2.0) < b0_threshold
                }
            })
            .count();
    let base_mask_for_fill = base_mask.clone();
    let b0_fill = move |dx: f32, dy: f32| {
        if !b0_is_reservoir(dx, dy) {
            return 0.0;
        }
        // Must also check the actual mask -- `dy < threshold` alone says nothing about whether
        // this (dx, dy) cell is INSIDE the vessel at all (e.g. above/beside the funnel's own
        // walls). Missing this check was a real bug in an earlier version of this file: it
        // filled OUTSIDE cells to 1.0 too, inflating the reported reservoir mass past the
        // vessel's own total capacity.
        let x = (dx + (GRID as f32 - 1.0) / 2.0).round();
        let y = (dy + GRID as f32 / 2.0).round();
        if x < 0.0 || y < 0.0 || x as usize >= GRID || y as usize >= GRID {
            return 0.0;
        }
        if base_mask_for_fill[y as usize * GRID + x as usize] != MASK_OUTSIDE { 1.0 } else { 0.0 }
    };

    if do_trace {
        let mut sim = build_sim(base_mask.clone(), &b0_fill, GRID);
        trace_mass("B0", &mut sim, MaterialMode::Water, 3000, 100, b0_is_reservoir, b0_is_collector);
    }
    let water_ticks_b0 = [0u32, 300, 700, 1100];
    let sand_ticks_b0 = [0u32, 400, 1000, 2200];
    let b0_result = run_design(
        "B0_control",
        out_dir,
        base_mask,
        b0_fill,
        b0_is_reservoir,
        b0_is_collector,
        b0_network_capacity,
        &water_ticks_b0,
        &sand_ticks_b0,
    );

    // ===================================================================================
    // A2 -- real split and merge
    // ===================================================================================
    // The reservoir sits ABOVE the network within one FIXED total vertical span (0.46h either
    // side of centre -- comfortably inside the grid, unlike an earlier version of this file that
    // let the reservoir extend the vessel upward past the top of the 256-cell grid, silently
    // clipping most of it and reporting a tiny fraction as a result). Reservoir height and
    // network height trade off against each other within that fixed span, and the network's own
    // capacity depends on its height, so this is a small fixed-point iteration rather than a
    // closed form.
    let a_total_half = 0.46 * h_f;
    let a_res_w = 2.0 * A_HW_FRAC * w_f;
    let mut a_res_h = 20.0f32;
    let mut a_network_cap = 0usize;
    for _ in 0..5 {
        let net_y0 = -a_total_half + a_res_h;
        let net_y1 = a_total_half;
        a_network_cap = capacity(&rasterize(GRID, GRID, |dx, dy| a_network_inside(dx, dy, w_f, net_y0, net_y1)));
        let target_area = (0.5 * a_network_cap as f32).ceil();
        a_res_h = (target_area / a_res_w).ceil();
    }
    let a_net_y0 = -a_total_half + a_res_h;
    let a_net_y1 = a_total_half;
    let a_res_y1 = a_net_y0; // reservoir sits directly above the network's own top edge
    let a_res_y0 = -a_total_half;

    let a_inside = move |dx: f32, dy: f32| {
        if dy >= a_res_y0 && dy < a_res_y1 {
            return dx.abs() < a_res_w / 2.0;
        }
        a_network_inside(dx, dy, w_f, a_net_y0, a_net_y1)
    };
    let a_mask = rasterize(GRID, GRID, a_inside);
    let a_is_reservoir = move |_dx: f32, dy: f32| dy < a_res_y1;
    let a_col_h = A_COL_FRAC_OF_NET * (a_net_y1 - a_net_y0);
    let a_is_collector = move |_dx: f32, dy: f32| dy >= a_net_y1 - a_col_h;
    let a_fill = move |dx: f32, dy: f32| if a_is_reservoir(dx, dy) && a_inside(dx, dy) { 1.0 } else { 0.0 };

    if do_trace {
        let mut sim = build_sim(a_mask.clone(), a_fill, GRID);
        trace_mass("A2", &mut sim, MaterialMode::Water, 3000, 100, a_is_reservoir, a_is_collector);
    }
    let water_ticks_a2 = [0u32, 150, 350, 900];
    let sand_ticks_a2 = [0u32, 300, 700, 1800];
    let a2_result = run_design(
        "A2_split_merge",
        out_dir,
        a_mask,
        a_fill,
        a_is_reservoir,
        a_is_collector,
        a_network_cap,
        &water_ticks_a2,
        &sand_ticks_a2,
    );

    // ===================================================================================
    // C2 -- real step pools (weir outlet)
    // ===================================================================================
    // Same fixed-point problem as A2: the collector's own height is `2*total_half - res_h -
    // (N-1)*pool_h`, i.e. it SHRINKS as the reservoir grows, because the whole chain shares one
    // fixed vertical budget. Measuring network capacity once at `res_h = 0` (as an earlier
    // version of this file did) measures a collector far bigger than what is left over once the
    // reservoir is actually sized, so the reservoir massively overshot its target and crowded
    // pools 2/3 and the collector down to almost nothing -- caught from the picture (only one
    // pool below the reservoir was visible; everything else had been squeezed past the point of
    // being distinguishable), not asserted. Iterate to a fixed point instead. Reservoir area is
    // computed geometrically (it's a plain axis-aligned box) rather than by re-rasterizing.
    let c_pool_w = C_POOL_W_FRAC * w_f;
    let mut c_res_h = 20.0f32;
    let mut c_network_cap = 0usize;
    for _ in 0..6 {
        let rects = c2_rects(w_f, h_f, c_res_h);
        let full_cap = capacity(&rasterize(GRID, GRID, |dx, dy| c2_inside(dx, dy, &rects)));
        let reservoir_area_est = (2.0 * c_pool_w * c_res_h).round() as usize;
        c_network_cap = full_cap.saturating_sub(reservoir_area_est);
        let target = (0.5 * c_network_cap as f32).ceil();
        c_res_h = (target / (2.0 * c_pool_w)).ceil();
    }
    let c_rects_cached = c2_rects(w_f, h_f, c_res_h);
    let c_mask = rasterize(GRID, GRID, |dx, dy| c2_inside(dx, dy, &c_rects_cached));
    let c_res_y0 = c2_pool0_y0(h_f);
    let c_res_y1 = c_res_y0 + c_res_h;
    let c_is_reservoir = move |_dx: f32, dy: f32| dy < c_res_y1 && dy >= c_res_y0;
    // Collector starts after the whole chain (reservoir + 3 pools).
    let c_pool_h = C_POOL_H_FRAC * h_f;
    let c_collector_y0 = c_res_y1 + (C_N_CHAIN as f32 - 1.0) * c_pool_h;
    let c_is_collector = move |_dx: f32, dy: f32| dy >= c_collector_y0;
    let c_fill = move |dx: f32, dy: f32| {
        if c_is_reservoir(dx, dy) && c2_inside(dx, dy, &c_rects_cached) { 1.0 } else { 0.0 }
    };

    if do_trace {
        let mut sim = build_sim(c_mask.clone(), &c_fill, GRID);
        trace_mass("C2", &mut sim, MaterialMode::Water, 3000, 100, c_is_reservoir, c_is_collector);
    }
    let water_ticks_c2 = [0u32, 120, 300, 600];
    let sand_ticks_c2 = [0u32, 300, 700, 1500];
    let c2_result = run_design(
        "C2_step_pools",
        out_dir,
        c_mask,
        c_fill,
        c_is_reservoir,
        c_is_collector,
        c_network_cap,
        &water_ticks_c2,
        &sand_ticks_c2,
    );

    println!("\n=== summary ===");
    for r in [&b0_result, &a2_result, &c2_result] {
        println!(
            "{}: mass={:.1} network_capacity={} fraction_of_network={:.3} water_err={:.6} sand_err={:.6}",
            r.name, r.initial_mass, r.network_capacity, r.fraction_of_network, r.water_mass_err, r.sand_mass_err
        );
    }
}
