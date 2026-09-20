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
//! ROUND 2 prototyped G1-G7 (pipe config + floor shape variants over the same 12-chamber grid) --
//! their PNGs and README section also stay as the record. The user picked **G4** ("every
//! chamber's floor has two pipes") as the family to keep, so this file no longer builds G1-G3/
//! G5-G7 either; see git history for that code (including `Shape::Poly`'s sloped-chamber use,
//! `all_chambers_sloped`/`sloped_chamber`, kept in the file but currently unused).
//!
//! ROUND 3 (this file, now): **the routing is now the only variable.** Same fixed 12-chamber
//! grid, same start (top row full, left pair red, right pair green); the new, uniform rule is
//! that EVERY chamber in rows 1 and 2 has exactly two outlet pipes, each feeding one chamber of
//! the row below -- and, so the rule is uniform all the way down, every bottom-row chamber also
//! gets two outlets, both into one shared collector pool below row 2 (a new dedicated region,
//! `Grid::collector_shape`, rather than "row 2 and below" being the collector as in Round 2). A
//! variant is just the pair of target columns chosen for each chamber -- see `RowRoute` and
//! `build_route_pipes`.
//!
//! Six routing variants over the identical chamber/outlet-count rule:
//!   R1 -- G4 as-is (the baseline the user liked), stated as a routing table.
//!   R2 -- neighbours: each feeds itself-below and its right neighbour, wrapping col 3 -> col 0.
//!   R3 -- wide spread: each feeds (col-1, col+2) mod 4 -- never its own column.
//!   R4 -- converging: every column's two outlets are BOTH of the two centre columns.
//!   R5 -- butterfly: row 0->1 stride 2, row 1->2 stride 1 -- the textbook claim is every top
//!         chamber can reach every bottom chamber; verified from the mixing table, not assumed.
//!   R6 -- own idea: neighbours into row 1, then converging into row 2.
//!
//! Mixing is now MEASURED, not just looked at: `mixing_table` reports, per bottom chamber (and
//! the collector), the red/green mass split at the final snapshot -- read off the tracer colour
//! itself via `green_fraction` (linear in the red channel between the two pure tracer colours) --
//! and the average |deviation from 50/50| across the 4 bottom chambers (lower = more mixed).
//! `worst_pipe_ratio` checks EVERY pipe's drop/run against the ~0.089 repose floor, not just a
//! representative one, since these routing tables include longer cross-vessel runs than Round 2.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && CARGO_BUILD_JOBS=2 cargo run -p sandart-sim --release --example proto_networks'
//!
//! Writes PNGs and a README to artifacts/design/network-2026-09-19/ (N1-N3/G1-G7's files untouched).

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
    row_bounds: Option<(f32, f32)>, // (reservoir/row1 boundary is is_reservoir's own; this is (row1/row2, row2/collector) for the finer per-row residue breakdown)
) -> (DesignResult, Snap, Snap) {
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
        let mut row1_mass = 0.0f32;
        let mut row2_mass = 0.0f32;
        for y in 0..GRID {
            let dy = y as f32 - center_y;
            for x in 0..GRID {
                let dx = x as f32 - center_x;
                let hgt = last.heights[y * GRID + x];
                if is_reservoir(dx, dy) {
                    res_mass += hgt;
                } else if is_collector(dx, dy) {
                    col_mass += hgt;
                } else if let Some((row1_row2, _)) = row_bounds {
                    if dy < row1_row2 {
                        row1_mass += hgt;
                    } else {
                        row2_mass += hgt;
                    }
                } else {
                    row1_mass += hgt;
                }
            }
        }
        if row_bounds.is_some() {
            println!("    [{label}] final (tick {}) distribution: reservoir={res_mass:.1} row1={row1_mass:.1} row2={row2_mass:.1} collector={col_mass:.1}", last.tick);
        } else {
            println!("    [{label}] final (tick {}) distribution: reservoir={res_mass:.1} network={row1_mass:.1} collector={col_mass:.1}", last.tick);
        }
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

    let water_final_snap = water_snaps.into_iter().last().unwrap();
    let sand_final_snap = sand_snaps.into_iter().last().unwrap();
    (
        DesignResult {
            name,
            initial_mass,
            network_capacity,
            fraction_of_network,
            water_mass_err,
            sand_mass_err,
            mirror_mismatches: mismatches,
        },
        water_final_snap,
        sand_final_snap,
    )
}

// -------------------------------------------------------------------------------------------
// The 12-chamber grid: 3 rows (top/middle/bottom) x 4 columns, fixed across every variant. Only
// the pipe list -- and, for G6/G7, the chamber floor shape -- changes per variant.
// -------------------------------------------------------------------------------------------

const G_ROWS: usize = 3;
const G_COLS: usize = 4;
const G_HW_X_FRAC: f32 = 0.44; // * w_f, half-width of the whole grid envelope
const G_TOTAL_HALF_Y_FRAC: f32 = 0.46; // * h_f, half-height of the whole grid envelope
// The top row gets a bigger share of the vertical budget than middle/bottom, and a dedicated
// collector zone now sits below row 2 (Round 3: bottom-row chambers get two real outlet pipes
// into a shared pool, per the user's "give them two outlets as well so the rule is uniform").
// With rows/collector all equal, the reservoir (4 top chambers) is capped well under 50% of the
// network before a single pipe is even added -- enlarging row 0's share fixes this at the
// geometry level. Re-tuned for Round 3's bigger network (more pipes + the new collector box);
// verified >= 0.50 for every R1-R6 variant, printed per run.
const G_ROW_FRACS: [f32; 3] = [0.40, 0.19, 0.19]; // + G_COLLECTOR_FRAC = 1.0
const G_COLLECTOR_FRAC: f32 = 0.22;
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
    collector_y0: f32,
    collector_y1: f32,
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
        let collector_y0 = y;
        y += G_COLLECTOR_FRAC * total_h;
        let collector_y1 = y;
        let mut chamber_hy = [0.0f32; G_ROWS];
        for r in 0..G_ROWS {
            chamber_hy[r] = (row_y1[r] - row_y0[r]) * 0.5 * G_CHAMBER_FILL_Y;
        }
        Grid { hw_x, total_half_y, col_width, row_y0, row_y1, collector_y0, collector_y1, chamber_hx: col_width * 0.5 * G_CHAMBER_FILL_X, chamber_hy }
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
    /// The boundary between row 2 (bottom) and the dedicated collector pool below it.
    fn collector_boundary(&self) -> f32 {
        self.collector_y0
    }
    /// The boundary between row 1 (middle) and row 2 (bottom), for the finer per-row dry-sand
    /// residue breakdown (reservoir / row1 / row2 / collector) requested in Round 3.
    fn mid_boundary(&self) -> f32 {
        self.row_y1[1]
    }
    fn collector_shape(&self) -> Shape {
        Shape::Chamber { cx: 0.0, cy: (self.collector_y0 + self.collector_y1) / 2.0, hx: self.hw_x, hy: (self.collector_y1 - self.collector_y0) / 2.0, r: G_CHAMBER_R }
    }
    /// One of a bottom chamber's two entry points into the shared collector pool, near that
    /// chamber's own column. `side` -1.0 = left entry, +1.0 = right entry.
    fn collector_entry(&self, col: usize, side: f32) -> (f32, f32) {
        (self.col_c(col) + side * 0.15 * self.col_width, self.collector_y0 + G_INSET)
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
// ROUND 3: the user picked G4 ("every chamber's floor has two pipes") as the family and asked
// for variations that hold the RULE fixed and vary only the ROUTING -- which two chambers below
// each pipe pair feeds. `RowRoute` is that routing table: `table[c] = (a, b)`, the two target
// columns chamber (row, c) feeds one row down. Applied identically to both the top->middle and
// middle->bottom transitions (except where a variant deliberately uses a different table per
// row, e.g. R5's butterfly). Per the user's uniformity request, bottom-row chambers ALSO get two
// outlets each -- both into the one shared collector pool below row 2, via `apply_bottom_to_collector`.
// -------------------------------------------------------------------------------------------

type RowRoute = [(usize, usize); 4];

/// Which way an incoming pipe's mouth should lean, based on the column it's coming from.
fn lean(target: usize, source: usize) -> f32 {
    if target > source {
        0.35
    } else if target < source {
        -0.35
    } else {
        0.0
    }
}

/// Adds 2 pipes per source chamber in `from_row` (columns 0..4), routed per `table`, into
/// `to_row`. The two outlets always leave from a source chamber's own floor at xfrac -0.35/+0.35
/// (two visibly separate mouths), matching the "every chamber has two outlets" rule.
fn apply_route(g: &Grid, from_row: usize, to_row: usize, table: &RowRoute, v: &mut Vec<Shape>) {
    for c in 0..G_COLS {
        let (a, b) = table[c];
        v.push(g.pipe_floor_to_top((from_row, c, -0.35), (to_row, a, lean(a, c))));
        v.push(g.pipe_floor_to_top((from_row, c, 0.35), (to_row, b, lean(b, c))));
    }
}

/// Every bottom-row (row 2) chamber gets its own two outlets too, both landing in the ONE shared
/// collector pool near that chamber's own column -- "give them two outlets as well so the rule
/// is uniform" (the user's words). This is the same for every variant; it is not part of the
/// routing experiment.
fn apply_bottom_to_collector(g: &Grid, v: &mut Vec<Shape>) {
    for c in 0..G_COLS {
        for side in [-1.0f32, 1.0] {
            let (x0, y0) = g.floor_pt(2, c, side * 0.35);
            let (x1, y1) = g.collector_entry(c, side);
            v.push(Shape::Channel { x0, y0, x1, y1, hw: G_PIPE_HW });
        }
    }
}

/// A side-to-side connector between two same-row chambers, used ONLY where a routing table
/// leaves a column with no direct feed (e.g. R4's convergence). An ordinary connected-vessels
/// link, not a crossing workaround.
fn lateral_fix(g: &Grid, row: usize, col_a: usize, col_b: usize, v: &mut Vec<Shape>) {
    v.push(g.pipe_side_to_side((row, col_a, 1.0, 0.0), (row, col_b, -1.0, 0.0)));
}

fn build_route_pipes(g: &Grid, table0: &RowRoute, table1: &RowRoute, laterals: &[(usize, usize, usize)]) -> Vec<Shape> {
    let mut v = Vec::new();
    apply_route(g, 0, 1, table0, &mut v);
    apply_route(g, 1, 2, table1, &mut v);
    apply_bottom_to_collector(g, &mut v);
    for &(row, a, b) in laterals {
        lateral_fix(g, row, a, b, &mut v);
    }
    v
}

// R1 = G4 as-is (the baseline the user liked): each column feeds itself and its right neighbour;
// the last column feeds itself and its LEFT neighbour instead of wrapping. Full column coverage,
// no laterals needed.
const R1_TABLE: RowRoute = [(0, 1), (1, 2), (2, 3), (3, 2)];
// R2 neighbours: each feeds itself-below and its right neighbour, wrapping column 3 -> column 0
// (a genuine crossing, left in per "pipes merging is fine"). Full coverage, no laterals.
const R2_TABLE: RowRoute = [(0, 1), (1, 2), (2, 3), (3, 0)];
// R3 wide spread: each feeds (c-1, c+2) mod 4 -- neither target is the source's own column, and
// several are 2-3 columns away. Full coverage, no laterals.
const R3_TABLE: RowRoute = [(3, 2), (0, 3), (1, 0), (2, 1)];
// R4 converging: every column's two outlets are BOTH the two centre columns (1, 2), so the two
// colours are driven into the same middle chambers regardless of which half they start in.
// Columns 0 and 3 get no direct feed from this table, so a lateral connector is added at every
// row this table is applied to.
const R4_TABLE: RowRoute = [(1, 2), (1, 2), (1, 2), (1, 2)];
// R5 butterfly: row 0->1 uses stride 2 (the classic first butterfly stage: c and c+2 swap
// halves), row 1->2 uses stride 1 (R2's table) -- so a top chamber's material can reach every
// bottom chamber via SOME path, the textbook claim for a butterfly network. Verified, not
// assumed, from the mixing table below.
const R5_ROW0: RowRoute = [(0, 2), (1, 3), (2, 0), (3, 1)];
const R5_ROW1: RowRoute = R2_TABLE;
// R6 (own, informed by R1-R5): neighbours into row 1 (full coverage, like R2), then converge
// into row 2 (like R4) -- tests whether a clean stage followed by a convergent stage mixes
// better than either alone. Row 1 needs no laterals (R2's table covers it); row 2 does (R4's
// table only hits columns 1-2).
const R6_ROW0: RowRoute = R2_TABLE;
const R6_ROW1: RowRoute = R4_TABLE;

/// The shallowest (worst-case) drop/run ratio among all `Shape::Channel`s in `shapes`, against
/// the ~0.089 (~5.1 degree) dry-sand repose floor -- checked for every pipe, not just a
/// representative one, since Round 3's routing tables include long cross-vessel runs R1-R5
/// didn't have.
fn worst_pipe_ratio(shapes: &[Shape]) -> (f32, f32, f32) {
    let mut worst_ratio = f32::INFINITY;
    let mut worst_drop = 0.0;
    let mut worst_run = 0.0;
    for s in shapes {
        if let Shape::Channel { x0, y0, x1, y1, .. } = s {
            let run = (x1 - x0).abs();
            let drop = (y1 - y0).abs();
            if run < 0.5 {
                continue; // near-vertical pipe: no meaningful run, can't be the shallow case
            }
            let ratio = drop / run;
            if ratio < worst_ratio {
                worst_ratio = ratio;
                worst_drop = drop;
                worst_run = run;
            }
        }
    }
    (worst_ratio, worst_drop, worst_run)
}

// -------------------------------------------------------------------------------------------
// Measuring mixing: for each of the 4 bottom chambers (and the collector), the RED vs GREEN mass
// share at the final snapshot. `green_fraction` reads it off the tracer colour directly rather
// than tracking a separate scalar field -- the two tracer colours are (225,40,40) red and
// (40,200,90) green, which differ enough in the red channel alone (225 vs 40) to use as a linear
// interpolation parameter for however much a cell's colour has blended toward green. A chamber
// at 50/50 is fully mixed; 100/0 (or 0/100) is unmixed.
// -------------------------------------------------------------------------------------------

fn green_fraction(color: u32) -> f32 {
    let r = color_channel(color, 0) as f32;
    ((225.0 - r) / (225.0 - 40.0)).clamp(0.0, 1.0)
}

fn region_color_mass(mask: &[u8], heights: &[f32], colors: &[u32], w: usize, h: usize, region: impl Fn(f32, f32) -> bool) -> (f32, f32) {
    let center_x = (w - 1) as f32 / 2.0;
    let center_y = w as f32 / 2.0;
    let mut red = 0.0f32;
    let mut green = 0.0f32;
    for y in 0..h {
        let dy = y as f32 - center_y;
        for x in 0..w {
            let dx = x as f32 - center_x;
            let idx = y * w + x;
            if mask[idx] == MASK_OUTSIDE || !region(dx, dy) {
                continue;
            }
            let hgt = heights[idx];
            if hgt < 0.005 {
                continue;
            }
            let gf = green_fraction(colors[idx]);
            green += hgt * gf;
            red += hgt * (1.0 - gf);
        }
    }
    (red, green)
}

/// Prints the mixing table for one (variant, material) at its final snapshot: red/green mass and
/// green share for each of the 4 bottom chambers and the collector, plus the average absolute
/// deviation from 50/50 across the 4 bottom chambers (lower = more mixed). Returns that average.
fn mixing_table(label: &str, g: &Grid, mask: &[u8], snap: &Snap) -> f32 {
    println!("  -- mixing table [{label}] --");
    let mut abs_dev_sum = 0.0f32;
    for c in 0..G_COLS {
        let (cx, cy, hx, hy) = (g.col_c(c), g.row_c(2), g.chamber_hx, g.chamber_hy[2]);
        let (red, green) = region_color_mass(mask, &snap.heights, &snap.colors, GRID, GRID, |dx, dy| rounded_box_inside(dx, dy, cx, cy, hx, hy, G_CHAMBER_R));
        let total = red + green;
        // A chamber with ~no mass at the final snapshot counts as FULLY UNMIXED (deviation 0.5),
        // not excluded from the average -- excluding it would silently reward a routing that
        // simply never delivers anything there (or drains it before the snapshot) as if it were
        // perfectly mixed. "Received nothing" is not mixing.
        let share = if total > 1e-3 { green / total } else { 0.5 };
        let dev = if total > 1e-3 { (share - 0.5).abs() } else { 0.5 };
        abs_dev_sum += dev;
        println!("    bottom col {c}: red={red:7.1} green={green:7.1} green_share={share:.3} (mass={total:.1})");
    }
    let (red_c, green_c) = region_color_mass(mask, &snap.heights, &snap.colors, GRID, GRID, |_dx, dy| dy >= g.collector_boundary());
    let total_c = red_c + green_c;
    let share_c = if total_c > 1e-3 { green_c / total_c } else { 0.5 };
    println!("    collector : red={red_c:7.1} green={green_c:7.1} green_share={share_c:.3}");
    let avg_dev = abs_dev_sum / G_COLS as f32;
    println!("    avg |deviation from 0.5| across bottom chambers = {avg_dev:.3}");
    avg_dev
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
    let row_bounds = Some((g.mid_boundary(), g.collector_boundary()));

    struct Variant {
        name: &'static str,
        pipes: Vec<Shape>,
        water_ticks: [u32; 4],
        sand_ticks: [u32; 4],
    }

    let variants = vec![
        Variant { name: "R1_g4_baseline", pipes: build_route_pipes(&g, &R1_TABLE, &R1_TABLE, &[]), water_ticks: [0, 200, 700, 1600], sand_ticks: [0, 400, 1300, 3200] },
        Variant { name: "R2_neighbours", pipes: build_route_pipes(&g, &R2_TABLE, &R2_TABLE, &[]), water_ticks: [0, 200, 700, 1800], sand_ticks: [0, 400, 1300, 3500] },
        Variant { name: "R3_wide_spread", pipes: build_route_pipes(&g, &R3_TABLE, &R3_TABLE, &[]), water_ticks: [0, 200, 700, 2200], sand_ticks: [0, 400, 1300, 4500] },
        Variant { name: "R4_converging", pipes: build_route_pipes(&g, &R4_TABLE, &R4_TABLE, &[(1, 0, 1), (1, 2, 3), (2, 0, 1), (2, 2, 3)]), water_ticks: [0, 300, 1000, 2800], sand_ticks: [0, 500, 1800, 5500] },
        Variant { name: "R5_butterfly", pipes: build_route_pipes(&g, &R5_ROW0, &R5_ROW1, &[]), water_ticks: [0, 200, 700, 1800], sand_ticks: [0, 400, 1300, 3500] },
        Variant { name: "R6_neighbours_then_converge", pipes: build_route_pipes(&g, &R6_ROW0, &R6_ROW1, &[(2, 0, 1), (2, 2, 3)]), water_ticks: [0, 200, 700, 1900], sand_ticks: [0, 400, 1300, 4800] },
    ];

    let mut results = Vec::new();
    let mut all_masks = Vec::new();
    let mut ranking: Vec<(&'static str, f32, f32, f32, f32)> = Vec::new(); // (name, water_dev, sand_dev, sand_residual_frac, worst_ratio)
    for variant in variants {
        let mut shapes = g.all_chambers_flat();
        shapes.push(g.collector_shape());
        shapes.extend(variant.pipes.clone());
        let (worst_ratio, worst_drop, worst_run) = worst_pipe_ratio(&variant.pipes);
        println!(
            "\n{} geometry: worst pipe drop/run = {:.3}/{:.1} = {:.3} ({:.1} deg) vs repose floor 0.089 (~5.1 deg)",
            variant.name, worst_drop, worst_run, worst_ratio, worst_ratio.atan().to_degrees()
        );

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
            trace_mass(variant.name, &mut sim, MaterialMode::DrySand, 4000, 100, is_reservoir.clone(), is_collector.clone());
        }

        all_masks.push(mask_image(&mask, GRID, GRID));
        let (result, water_final, sand_final) = run_design(
            variant.name,
            out_dir,
            mask.clone(),
            fill,
            is_reservoir.clone(),
            is_collector.clone(),
            network_cap,
            &variant.water_ticks,
            &variant.sand_ticks,
            row_bounds,
        );

        let water_dev = mixing_table(&format!("{} water", variant.name), &g, &mask, &water_final);
        let sand_dev = mixing_table(&format!("{} dry sand", variant.name), &g, &mask, &sand_final);

        // Sand residual: fraction of total mass still in reservoir + row1 + row2 (i.e. NOT yet
        // in the collector) at the final sand snapshot -- the damming measure that killed G5.
        let center_x = (GRID - 1) as f32 / 2.0;
        let center_y = GRID as f32 / 2.0;
        let mut collector_mass = 0.0f32;
        let mut total_mass = 0.0f32;
        for y in 0..GRID {
            let dy = y as f32 - center_y;
            for x in 0..GRID {
                let dx = x as f32 - center_x;
                let hgt = sand_final.heights[y * GRID + x];
                total_mass += hgt;
                if is_collector(dx, dy) {
                    collector_mass += hgt;
                }
            }
        }
        let sand_residual_frac = if total_mass > 0.0 { 1.0 - collector_mass / total_mass } else { f32::NAN };

        ranking.push((variant.name, water_dev, sand_dev, sand_residual_frac, worst_ratio));
        results.push(result);
    }

    contact_sheet(&all_masks, 3)
        .save(out_dir.join("R_all_masks_comparison.png"))
        .expect("write comparison sheet");

    println!("\n=== summary ===");
    for r in &results {
        println!(
            "{}: mass={:.1} network_capacity={} fraction_of_network={:.3} water_err={:.6} sand_err={:.6} mirror_mismatches={}",
            r.name, r.initial_mass, r.network_capacity, r.fraction_of_network, r.water_mass_err, r.sand_mass_err, r.mirror_mismatches
        );
    }

    println!("\n=== ranking (lower mixing deviation = more mixed; lower sand residual = better drained) ===");
    let mut by_mix = ranking.clone();
    by_mix.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    println!("by water mixing deviation (best/most-mixed first):");
    for (name, wd, sd, res, ratio) in &by_mix {
        println!("  {name}: water_dev={wd:.3} sand_dev={sd:.3} sand_residual={:.1}% worst_pipe_ratio={ratio:.3}", res * 100.0);
    }
    let mut by_residual = ranking.clone();
    by_residual.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());
    println!("by dry-sand residual (best-drained first):");
    for (name, wd, sd, res, ratio) in &by_residual {
        println!("  {name}: sand_residual={:.1}% water_dev={wd:.3} sand_dev={sd:.3} worst_pipe_ratio={ratio:.3}", res * 100.0);
    }
}
