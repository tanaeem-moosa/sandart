//! PROTOTYPE ONLY. Renders pictures of candidate "network of chambers joined by channels"
//! vessels, replacing the old "Merging cascade" (`SandboxShape::MultiStageHourglass`, deleted
//! from the codebase). Does not touch `sandart-sim/src`, the renderer, wasm, the UI, or
//! `proto_cascades.rs` (another agent owns that file).
//!
//! ROUND 1 of this file prototyped N1/N2/N3 (crossing junctions / split-recombine / layering) --
//! their PNGs and README section stay in `artifacts/design/network-2026-09-19/` as the record,
//! but the user rejected all three ("None of these are good") and this file no longer builds
//! them. See git history for that code if it's ever needed again.
//!
//! ROUND 2 (this file, now): a concrete spec from the user. **12 rectangular chambers, a fixed
//! 3-row x 4-column grid (top/middle/bottom, 4 per row), joined by pipes.** The interesting
//! variable is the PIPE CONFIGURATION (and, per a follow-up clarification, the chamber FLOOR
//! shape) over that same fixed grid. Geometry is still data for one small rasterizer -- see
//! `Shape`, `shape_inside`, `network_inside` -- now with a third shape (`Shape::Poly`, an
//! arbitrary simple polygon) added specifically so a chamber's floor can be sloped, not just
//! flat.
//!
//! Two follow-up clarifications from the user, both load-bearing:
//!   1. Chambers need not be strictly rectangular -- sloped floors are a deliberate design
//!      variable, not a workaround to hide. G6/G7 below are the SAME pipe configuration with a
//!      flat vs. sloped floor, specifically so the cost/benefit is visible side by side.
//!   2. **Pipes merging is fine and wanted** -- "that creates interesting effects." Two pipes (or
//!      two colours) meeting in one pipe or junction chamber is a deliberate design element here,
//!      not a planar-mask accident to route around. G3 is built specifically to merge a red
//!      column with a green column early and show what the tracer does there.
//!
//! Variants (all on the same 12-chamber grid; only the pipe list -- and, for G6/G7, the chamber
//! floor shape -- changes):
//!   G1 -- straight: each chamber feeds the one directly below. Flat floor, centred pipe mouth.
//!   G2 -- diagonal shift: each feeds the chamber one column across; the wrap (last column back
//!         to the first) is a genuine long diagonal that crosses the others -- left as a real
//!         crossing/merge, per clarification 2.
//!   G3 -- merge: a red top column and a green top column are deliberately routed into the SAME
//!         shared chamber, twice, so the tracer shows the merge directly.
//!   G4 -- split: every chamber's floor has two pipes, feeding two different chambers below.
//!   G5 -- crossing fan: top row reverses column order into the middle row (a full 4-way crossing
//!         "own idea" variant), then runs straight into the bottom row for contrast.
//!   G6 -- straight, flat floor, CORNER pipe mouth (vs. G1's centred mouth) -- sand-drainage A/B.
//!   G7 -- straight, SLOPED floor, same corner pipe mouth as G6 -- sand-drainage A/B, continued.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && CARGO_BUILD_JOBS=2 cargo run -p sandart-sim --release --example proto_networks'
//!
//! Writes PNGs and a README to artifacts/design/network-2026-09-19/ (N1/N2/N3's files untouched).

use sandart_sim::{
    color_channel, pack_rgba, DrawingSimulation, MaterialMode, MASK_BOUNDARY, MASK_INSIDE,
    MASK_OUTSIDE,
};
use std::fs;
use std::path::Path;

const GRID: usize = 256;

// -------------------------------------------------------------------------------------------
// Shape primitives: a chamber (rounded box) or a channel (capsule / thick line segment). A
// network is just `Vec<Shape>`; a point is inside the network if it is inside ANY shape. This is
// the whole rasterizer -- deliberately small and shape-agnostic, because this is the prototype of
// how the shipped vessel would be authored (a list of chambers and channels, not bespoke per-tier
// math like the old MultiStageHourglass).
// -------------------------------------------------------------------------------------------

#[derive(Clone)]
enum Shape {
    /// Rounded box: centre (cx, cy), half-extents (hx, hy), corner radius r (r <= min(hx, hy)).
    Chamber { cx: f32, cy: f32, hx: f32, hy: f32, r: f32 },
    /// Capsule: line segment (x0,y0)-(x1,y1), half-width hw (rounded ends).
    Channel { x0: f32, y0: f32, x1: f32, y1: f32, hw: f32 },
    /// Arbitrary simple polygon (closed, points in order). Used for sloped-floor chambers --
    /// everything else about a chamber (rounded corners, uniform depth) assumes a flat floor, so
    /// a slope needs its own shape rather than a parameter on `Chamber`.
    Poly(Vec<(f32, f32)>),
}

fn rounded_box_inside(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> bool {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    let outside_dist = (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - r;
    outside_dist <= 0.0
}

fn capsule_inside(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, hw: f32) -> bool {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len_sq = (dx * dx + dy * dy).max(1e-6);
    let t = (((x - x0) * dx + (y - y0) * dy) / len_sq).clamp(0.0, 1.0);
    let (px, py) = (x0 + t * dx, y0 + t * dy);
    let (ex, ey) = (x - px, y - py);
    (ex * ex + ey * ey).sqrt() <= hw
}

/// Standard ray-casting point-in-polygon test. `pts` need not be convex, just a simple closed
/// loop (implicitly closed -- the last point connects back to the first).
fn poly_inside(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let n = pts.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn shape_inside(s: &Shape, x: f32, y: f32) -> bool {
    match s {
        &Shape::Chamber { cx, cy, hx, hy, r } => rounded_box_inside(x, y, cx, cy, hx, hy, r),
        &Shape::Channel { x0, y0, x1, y1, hw } => capsule_inside(x, y, x0, y0, x1, y1, hw),
        Shape::Poly(pts) => poly_inside(x, y, pts),
    }
}

fn network_inside(shapes: &[Shape], x: f32, y: f32) -> bool {
    shapes.iter().any(|s| shape_inside(s, x, y))
}

// -------------------------------------------------------------------------------------------
// Shared rasterization / diagnostics (same approach as proto_cascades.rs -- duplicated rather
// than imported, since examples cannot depend on each other).
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
/// `max_tick`, so snapshot ticks can be picked from measured fill/spill times rather than guessed.
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
// Driving the simulation
// -------------------------------------------------------------------------------------------

struct DesignResult {
    name: &'static str,
    initial_mass: f32,
    network_capacity: usize,
    fraction_of_network: f32,
    water_mass_err: f32,
    sand_mass_err: f32,
    mirror_mismatches: usize,
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
        "  mirror mismatches: {mismatches} (asymmetry is EXPECTED/allowed for these designs) | connectivity: {reached}/{total_inside} reached, collector reached = {reached_collector}"
    );
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
        mirror_mismatches: mismatches,
    }
}

// -------------------------------------------------------------------------------------------
// The 12-chamber grid: 3 rows (top/middle/bottom) x 4 columns, fixed across every variant. Only
// the pipe list -- and, for G6/G7, the chamber floor shape -- changes per variant.
// -------------------------------------------------------------------------------------------

const G_ROWS: usize = 3;
const G_COLS: usize = 4;
const G_HW_X_FRAC: f32 = 0.44; // * w_f, half-width of the whole grid envelope
const G_TOTAL_HALF_Y_FRAC: f32 = 0.46; // * h_f, half-height of the whole grid envelope
// The top row gets a bigger share of the vertical budget than middle/bottom. With three EQUAL
// rows, the reservoir (4 top chambers) is capped at ~4/(8 chambers + pipes) of the network, i.e.
// well under 50% before a single pipe is even added -- measured at 0.491 for the sparsest variant
// (G1, 8 pipes) and as low as 0.474 once a variant's pipe list gets bigger (G4, 16 pipes). Giving
// row 0 a larger slice fixes this at the geometry level instead of inflating the fill some other
// way.
const G_ROW_FRACS: [f32; 3] = [0.42, 0.29, 0.29];
const G_CHAMBER_FILL_X: f32 = 0.82; // fraction of a column's own width slot the chamber fills
const G_CHAMBER_FILL_Y: f32 = 0.68; // fraction of a row's own height slot the chamber fills
const G_CHAMBER_R: f32 = 3.0; // corner radius -- small, so chambers still read as rectangular
const G_PIPE_HW: f32 = 5.0; // pipe half-width (10 cells -- well over the 3-cell/6-cell minimum)
const G_INSET: f32 = 4.0; // how far a pipe's mouth is inset from the chamber's own wall

struct Grid {
    hw_x: f32,
    total_half_y: f32,
    col_width: f32,
    row_y0: [f32; G_ROWS],
    row_y1: [f32; G_ROWS],
    chamber_hx: f32,
    chamber_hy: [f32; G_ROWS],
}

impl Grid {
    fn new(w_f: f32, h_f: f32) -> Self {
        let hw_x = G_HW_X_FRAC * w_f;
        let total_half_y = G_TOTAL_HALF_Y_FRAC * h_f;
        let col_width = 2.0 * hw_x / G_COLS as f32;
        let total_h = 2.0 * total_half_y;
        let mut row_y0 = [0.0f32; G_ROWS];
        let mut row_y1 = [0.0f32; G_ROWS];
        let mut y = -total_half_y;
        for r in 0..G_ROWS {
            row_y0[r] = y;
            y += G_ROW_FRACS[r] * total_h;
            row_y1[r] = y;
        }
        let mut chamber_hy = [0.0f32; G_ROWS];
        for r in 0..G_ROWS {
            chamber_hy[r] = (row_y1[r] - row_y0[r]) * 0.5 * G_CHAMBER_FILL_Y;
        }
        Grid { hw_x, total_half_y, col_width, row_y0, row_y1, chamber_hx: col_width * 0.5 * G_CHAMBER_FILL_X, chamber_hy }
    }
    fn col_c(&self, col: usize) -> f32 {
        -self.hw_x + self.col_width * (col as f32 + 0.5)
    }
    fn row_c(&self, row: usize) -> f32 {
        (self.row_y0[row] + self.row_y1[row]) * 0.5
    }
    /// The boundary between row 0 (top/reservoir) and row 1 -- everything above is "reservoir".
    fn reservoir_boundary(&self) -> f32 {
        self.row_y1[0]
    }
    /// The boundary between row 1 and row 2 (bottom/collector) -- everything below is "collector".
    fn collector_boundary(&self) -> f32 {
        self.row_y1[1]
    }
    fn chamber(&self, row: usize, col: usize) -> Shape {
        Shape::Chamber { cx: self.col_c(col), cy: self.row_c(row), hx: self.chamber_hx, hy: self.chamber_hy[row], r: G_CHAMBER_R }
    }
    /// A sloped-floor chamber: flat top and sides, but the floor is a single straight ramp from
    /// a shallow corner (0.35 of the way down) to a full-depth corner at the outlet side. No
    /// corner rounding -- see the module doc comment (clarification 1: slope is a real, visible
    /// design variable here, not a parameter tucked away on the normal chamber).
    fn sloped_chamber(&self, row: usize, col: usize, outlet_side: f32) -> Shape {
        let cx = self.col_c(col);
        let cy = self.row_c(row);
        let (hx, hy) = (self.chamber_hx, self.chamber_hy[row]);
        let shallow_y = cy + hy * 0.35;
        let deep_y = cy + hy;
        let (bl_y, br_y) = if outlet_side < 0.0 { (deep_y, shallow_y) } else { (shallow_y, deep_y) };
        Shape::Poly(vec![(cx - hx, cy - hy), (cx + hx, cy - hy), (cx + hx, br_y), (cx - hx, bl_y)])
    }
    fn all_chambers_flat(&self) -> Vec<Shape> {
        (0..G_ROWS).flat_map(|r| (0..G_COLS).map(move |c| (r, c))).map(|(r, c)| self.chamber(r, c)).collect()
    }
    fn all_chambers_sloped(&self, outlet_side: f32) -> Vec<Shape> {
        (0..G_ROWS).flat_map(|r| (0..G_COLS).map(move |c| (r, c))).map(|(r, c)| self.sloped_chamber(r, c, outlet_side)).collect()
    }
    /// A point on chamber (row, col)'s own floor, `xfrac` in [-1, 1] across its own width.
    fn floor_pt(&self, row: usize, col: usize, xfrac: f32) -> (f32, f32) {
        (self.col_c(col) + xfrac * self.chamber_hx * 0.9, self.row_c(row) + self.chamber_hy[row] - G_INSET)
    }
    /// A point on chamber (row, col)'s own ceiling (top edge), `xfrac` in [-1, 1].
    fn top_pt(&self, row: usize, col: usize, xfrac: f32) -> (f32, f32) {
        (self.col_c(col) + xfrac * self.chamber_hx * 0.9, self.row_c(row) - self.chamber_hy[row] + G_INSET)
    }
    /// A point on chamber (row, col)'s own side wall. `side` -1.0 = left wall, +1.0 = right wall.
    fn side_pt(&self, row: usize, col: usize, side: f32, yfrac: f32) -> (f32, f32) {
        (self.col_c(col) + side * (self.chamber_hx - G_INSET), self.row_c(row) + yfrac * self.chamber_hy[row] * 0.6)
    }
    fn pipe_floor_to_top(&self, from: (usize, usize, f32), to: (usize, usize, f32)) -> Shape {
        let (x0, y0) = self.floor_pt(from.0, from.1, from.2);
        let (x1, y1) = self.top_pt(to.0, to.1, to.2);
        Shape::Channel { x0, y0, x1, y1, hw: G_PIPE_HW }
    }
    fn pipe_side_to_side(&self, from: (usize, usize, f32, f32), to: (usize, usize, f32, f32)) -> Shape {
        let (x0, y0) = self.side_pt(from.0, from.1, from.2, from.3);
        let (x1, y1) = self.side_pt(to.0, to.1, to.2, to.3);
        Shape::Channel { x0, y0, x1, y1, hw: G_PIPE_HW }
    }
}

// -------------------------------------------------------------------------------------------
// Variant pipe lists. Chambers are identical across all of these (`Grid::all_chambers_flat`,
// except G7 which uses `all_chambers_sloped`) -- each function below returns PIPES ONLY.
// -------------------------------------------------------------------------------------------

/// G1 -- straight: every chamber feeds the one directly below it. Flat floor, centred mouth.
fn pipes_g1_straight(g: &Grid) -> Vec<Shape> {
    let mut v = Vec::new();
    for c in 0..G_COLS {
        v.push(g.pipe_floor_to_top((0, c, 0.0), (1, c, 0.0)));
        v.push(g.pipe_floor_to_top((1, c, 0.0), (2, c, 0.0)));
    }
    v
}

/// G2 -- diagonal shift: column c feeds column c+1 one row down, wrapping column 3 back to
/// column 0. The wrap is a genuine long diagonal that crosses the three short ones -- left as a
/// real crossing/merge (clarification 2: merging pipes is a wanted effect, not something to
/// route around). This also means every column has exactly one incoming pipe at every row, so
/// full connectivity falls out for free with no extra connector pipes needed.
fn pipes_g2_diagonal_shift(g: &Grid) -> Vec<Shape> {
    let mut v = Vec::new();
    for c in 0..G_COLS {
        let to = (c + 1) % G_COLS;
        // sign: which way the mouth/entry lean -- rightward for the three short shifts (to > c),
        // leftward for the one wrap (to < c, column 3 back to column 0).
        let sign = if to > c { 1.0 } else { -1.0 };
        v.push(g.pipe_floor_to_top((0, c, 0.3 * sign), (1, to, -0.3 * sign)));
        v.push(g.pipe_floor_to_top((1, c, 0.3 * sign), (2, to, -0.3 * sign)));
    }
    v
}

/// G3 -- merge: deliberately routes a RED top column and a GREEN top column into the SAME
/// shared chamber, twice (clarification 2's dedicated "colours actually meet" variant). Columns
/// 0-1 start red, 2-3 start green (see `set_tracer_colors` below), so pairing (0,2) -> mid col 1
/// and (1,3) -> mid col 2 guarantees both merge points combine one red source and one green
/// source. Mid columns 0 and 3 get no direct feed from this pairing, so a lateral connector pipe
/// (side wall to side wall, not floor-to-top) keeps them reachable -- an ordinary "connected
/// vessels" link, not a crossing workaround. The same pattern repeats mid -> bottom.
fn pipes_g3_merge(g: &Grid) -> Vec<Shape> {
    let mut v = Vec::new();
    // top -> mid: (0,2) -> mid col 1, (1,3) -> mid col 2.
    v.push(g.pipe_floor_to_top((0, 0, 0.5), (1, 1, -0.4)));
    v.push(g.pipe_floor_to_top((0, 2, -0.5), (1, 1, 0.4)));
    v.push(g.pipe_floor_to_top((0, 1, 0.5), (1, 2, -0.4)));
    v.push(g.pipe_floor_to_top((0, 3, -0.5), (1, 2, 0.4)));
    v.push(g.pipe_side_to_side((1, 0, 1.0, 0.0), (1, 1, -1.0, 0.0)));
    v.push(g.pipe_side_to_side((1, 2, 1.0, 0.0), (1, 3, -1.0, 0.0)));
    // mid -> bottom: same merge pattern one row down.
    v.push(g.pipe_floor_to_top((1, 0, 0.5), (2, 1, -0.4)));
    v.push(g.pipe_floor_to_top((1, 2, -0.5), (2, 1, 0.4)));
    v.push(g.pipe_floor_to_top((1, 1, 0.5), (2, 2, -0.4)));
    v.push(g.pipe_floor_to_top((1, 3, -0.5), (2, 2, 0.4)));
    v.push(g.pipe_side_to_side((2, 0, 1.0, 0.0), (2, 1, -1.0, 0.0)));
    v.push(g.pipe_side_to_side((2, 2, 1.0, 0.0), (2, 3, -1.0, 0.0)));
    v
}

/// G4 -- split: every chamber's floor has TWO pipes, feeding two different chambers below (the
/// last column feeds itself and its inward neighbour instead of wrapping, to keep every run
/// short and local).
fn pipes_g4_split(g: &Grid) -> Vec<Shape> {
    let fan = |from_row: usize, to_row: usize, v: &mut Vec<Shape>| {
        for c in 0..G_COLS - 1 {
            v.push(g.pipe_floor_to_top((from_row, c, -0.3), (to_row, c, 0.3)));
            v.push(g.pipe_floor_to_top((from_row, c, 0.3), (to_row, c + 1, -0.3)));
        }
        let last = G_COLS - 1;
        v.push(g.pipe_floor_to_top((from_row, last, 0.3), (to_row, last, -0.3)));
        v.push(g.pipe_floor_to_top((from_row, last, -0.3), (to_row, last - 1, 0.3)));
    };
    let mut v = Vec::new();
    fan(0, 1, &mut v);
    fan(1, 2, &mut v);
    v
}

/// G5 -- crossing fan ("own idea"): top row reverses column order into the middle row (col c ->
/// mid col 3-c), a deliberate 4-way crossing/merge in the middle of the vessel; middle -> bottom
/// then runs straight (no crossing) so the picture contrasts a crossing stage against a clean one.
fn pipes_g5_crossing_fan(g: &Grid) -> Vec<Shape> {
    let mut v = Vec::new();
    for c in 0..G_COLS {
        v.push(g.pipe_floor_to_top((0, c, 0.0), (1, G_COLS - 1 - c, 0.0)));
    }
    for c in 0..G_COLS {
        v.push(g.pipe_floor_to_top((1, c, 0.0), (2, c, 0.0)));
    }
    v
}

/// G6/G7 share this pipe list (same configuration, per the sand-drainage A/B request) -- only
/// the chamber floor shape differs between the two (flat for G6, sloped for G7). The pipe mouth
/// sits at a corner (xfrac 0.7) rather than centred, which for G7 is also where the sloped floor
/// is deepest.
fn pipes_corner_outlet(g: &Grid) -> Vec<Shape> {
    let mut v = Vec::new();
    let xf = 0.7;
    for c in 0..G_COLS {
        v.push(g.pipe_floor_to_top((0, c, xf), (1, c, xf)));
        v.push(g.pipe_floor_to_top((1, c, xf), (2, c, xf)));
    }
    v
}

fn main() {
    let out_dir = Path::new("artifacts/design/network-2026-09-19");
    fs::create_dir_all(out_dir).expect("create output dir");

    let w_f = GRID as f32;
    let h_f = GRID as f32;
    let g = Grid::new(w_f, h_f);
    let do_trace = std::env::var("TRACE_MASS").is_ok();

    let is_reservoir = {
        let b = g.reservoir_boundary();
        move |_dx: f32, dy: f32| dy < b
    };
    let is_collector = {
        let b = g.collector_boundary();
        move |_dx: f32, dy: f32| dy >= b
    };

    // Report the geometric drop/run ratio for a representative diagonal pipe (G2/G4/G5's short
    // shift) and the long G2 wrap, against the ~0.089 (~5 degree) dry-sand repose floor, rather
    // than asserting steepness -- see the printed numbers below and the dry-sand traces/pictures
    // for whether each actually keeps flowing.
    // A floor-to-top pipe's endpoints are each inset INTO their own chamber, so measure the
    // actual drop directly from a representative pipe's own endpoints (row 1 -> row 2, the
    // smaller of the two gaps now that row 0 is enlarged) rather than re-deriving it from row
    // geometry by hand.
    let (fx0, fy0) = g.floor_pt(1, 0, 0.0);
    let (tx0, ty0) = g.top_pt(2, 0, 0.0);
    let short_drop = ty0 - fy0;
    let short_run = g.col_width;
    let _ = (fx0, tx0);
    let wrap_run = (G_COLS - 1) as f32 * g.col_width;
    println!(
        "geometry: short diagonal drop/run = {:.3}/{:.1} = {:.3} ({:.1} deg); long wrap drop/run = {:.3}/{:.1} = {:.3} ({:.1} deg); repose floor = 0.089 (~5.1 deg)",
        short_drop, short_run, short_drop / short_run, (short_drop / short_run).atan().to_degrees(),
        short_drop, wrap_run, short_drop / wrap_run, (short_drop / wrap_run).atan().to_degrees()
    );

    struct Variant {
        name: &'static str,
        chambers: Vec<Shape>,
        pipes: Vec<Shape>,
        water_ticks: [u32; 4],
        sand_ticks: [u32; 4],
    }

    let variants = vec![
        Variant { name: "G1_straight_flat_center", chambers: g.all_chambers_flat(), pipes: pipes_g1_straight(&g), water_ticks: [0, 150, 600, 2200], sand_ticks: [0, 400, 1500, 4500] },
        Variant { name: "G2_diagonal_shift", chambers: g.all_chambers_flat(), pipes: pipes_g2_diagonal_shift(&g), water_ticks: [0, 300, 900, 2200], sand_ticks: [0, 300, 900, 3500] },
        Variant { name: "G3_merge", chambers: g.all_chambers_flat(), pipes: pipes_g3_merge(&g), water_ticks: [0, 300, 900, 2200], sand_ticks: [0, 300, 900, 3500] },
        Variant { name: "G4_split", chambers: g.all_chambers_flat(), pipes: pipes_g4_split(&g), water_ticks: [0, 200, 700, 1500], sand_ticks: [0, 400, 1200, 2800] },
        Variant { name: "G5_crossing_fan", chambers: g.all_chambers_flat(), pipes: pipes_g5_crossing_fan(&g), water_ticks: [0, 300, 900, 2500], sand_ticks: [0, 300, 900, 3500] },
        Variant { name: "G6_straight_flat_corner", chambers: g.all_chambers_flat(), pipes: pipes_corner_outlet(&g), water_ticks: [0, 150, 600, 2200], sand_ticks: [0, 400, 1500, 4500] },
        Variant { name: "G7_straight_sloped_corner", chambers: g.all_chambers_sloped(1.0), pipes: pipes_corner_outlet(&g), water_ticks: [0, 150, 600, 2200], sand_ticks: [0, 400, 1500, 4500] },
    ];

    let mut results = Vec::new();
    let mut all_masks = Vec::new();
    for variant in variants {
        let mut shapes = variant.chambers;
        shapes.extend(variant.pipes);
        let shapes = std::rc::Rc::new(shapes);
        let inside = {
            let shapes = shapes.clone();
            move |dx: f32, dy: f32| network_inside(&shapes, dx, dy)
        };
        let mask = rasterize(GRID, GRID, inside.clone());
        let total_cap = capacity(&mask);
        let reservoir_cap = {
            let center_y = h_f / 2.0;
            mask.iter().enumerate().filter(|&(i, &m)| m != MASK_OUTSIDE && (i / GRID) as f32 - center_y < g.reservoir_boundary()).count()
        };
        let network_cap = total_cap - reservoir_cap;
        let fill = {
            let inside = inside.clone();
            let is_reservoir = is_reservoir.clone();
            move |dx: f32, dy: f32| if is_reservoir(dx, dy) && inside(dx, dy) { 1.0 } else { 0.0 }
        };

        if do_trace {
            let mut sim = build_sim(mask.clone(), fill.clone(), GRID);
            trace_mass(variant.name, &mut sim, MaterialMode::Water, 2500, 50, is_reservoir.clone(), is_collector.clone());
            let mut sim = build_sim(mask.clone(), fill.clone(), GRID);
            trace_mass(variant.name, &mut sim, MaterialMode::DrySand, 3500, 100, is_reservoir.clone(), is_collector.clone());
        }

        all_masks.push(mask_image(&mask, GRID, GRID));
        let result = run_design(
            variant.name,
            out_dir,
            mask,
            fill,
            is_reservoir.clone(),
            is_collector.clone(),
            network_cap,
            &variant.water_ticks,
            &variant.sand_ticks,
        );
        results.push(result);
    }

    contact_sheet(&all_masks, 4)
        .save(out_dir.join("G_all_masks_comparison.png"))
        .expect("write comparison sheet");

    println!("\n=== summary ===");
    for r in &results {
        println!(
            "{}: mass={:.1} network_capacity={} fraction_of_network={:.3} water_err={:.6} sand_err={:.6} mirror_mismatches={}",
            r.name, r.initial_mass, r.network_capacity, r.fraction_of_network, r.water_mass_err, r.sand_mass_err, r.mirror_mismatches
        );
    }
}
