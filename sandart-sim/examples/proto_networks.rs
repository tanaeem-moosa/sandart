//! PROTOTYPE ONLY. Renders pictures of candidate "network of chambers joined by channels"
//! vessels, replacing the old "Merging cascade" (`SandboxShape::MultiStageHourglass`). Does not
//! touch `sandart-sim/src`, the renderer, wasm, the UI, or `proto_cascades.rs` (another agent is
//! changing that file).
//!
//! Background: `artifacts/design/cascade-2026-09-17/README.md`. Round 2's A2 ("split and merge")
//! was a brick grid of enclosed chambers that the user liked structurally ("opens up many avenues
//! of complex network ... does not even have to be perpendicular ... can be at an angle") but it
//! did NOT mix: every stream fell straight down its own hole and red never touched green.
//!
//! Decisions already made by the user (do not re-litigate):
//!   - Mirror symmetry is NOT required for these networks.
//!   - Channels may run at any angle, not just vertical/horizontal.
//!   - Whether "mixing" should look like BLENDING or INTERLEAVING is undecided -- show both.
//!
//! Geometry is built as DATA for one small rasterizer (chambers = rounded boxes, channels =
//! capsules/line segments with a width), because that is how it would ship. See `Shape`,
//! `shape_inside`, `network_inside` below.
//!
//! Three designs:
//!   N1 -- crossing junctions: streams arrive at small shared junction chambers on angled
//!         channels that visually cross (X) before and after each chamber, so material from both
//!         approach directions is forced into the same small pool several times down the vessel.
//!   N2 -- split and recombine: a literal binary "swap the touching inner halves of each adjacent
//!         pair of bands" network, applied 3 times (2 -> 4 -> 8 -> 16 bands). This is the closest
//!         buildable 2D approximation of the classic split/recombine microfluidic mixer -- a real
//!         doubling-by-lamination mixer needs a third (out-of-plane) dimension to make one branch
//!         carry a full copy of the incoming pattern, which a flat height-field mask cannot do
//!         without the two branches physically merging. Report what the picture actually shows.
//!   N3 -- layering: a short direct path for the left (red) half and a long winding path for the
//!         right (green) half, both converging on ONE shared collector, so red should settle at
//!         the bottom before green arrives on top -- checked from the mass trace, not assumed.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && CARGO_BUILD_JOBS=2 cargo run -p sandart-sim --release --example proto_networks'
//!
//! Writes PNGs and a README to artifacts/design/network-2026-09-19/.

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

#[derive(Clone, Copy)]
enum Shape {
    /// Rounded box: centre (cx, cy), half-extents (hx, hy), corner radius r (r <= min(hx, hy)).
    Chamber { cx: f32, cy: f32, hx: f32, hy: f32, r: f32 },
    /// Capsule: line segment (x0,y0)-(x1,y1), half-width hw (rounded ends).
    Channel { x0: f32, y0: f32, x1: f32, y1: f32, hw: f32 },
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

fn shape_inside(s: &Shape, x: f32, y: f32) -> bool {
    match *s {
        Shape::Chamber { cx, cy, hx, hy, r } => rounded_box_inside(x, y, cx, cy, hx, hy, r),
        Shape::Channel { x0, y0, x1, y1, hw } => capsule_inside(x, y, x0, y0, x1, y1, hw),
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
// Shared vertical envelope for all three designs.
// -------------------------------------------------------------------------------------------

const HW_FRAC: f32 = 0.42; // * w_f, half-width of the network envelope
const TOTAL_HALF_FRAC: f32 = 0.46; // * h_f, half-height of the whole vessel

// dry-sand angle of repose in this sim is ~0.089 height-per-cell (per CLAUDE.md / physics.rs
// comments on the CA's repose threshold) -- effectively a very shallow ~5 degree critical slope.
// Every diagonal channel below is built with a drop-to-run ratio of at least 1.3 (>= 52 degrees
// from horizontal), an order of magnitude steeper than that floor, specifically so dry sand does
// not stall in a corridor. Checked against the measured dry-sand mass trace below, not asserted.
const MIN_DROP_RUN_RATIO: f32 = 1.3;

// -------------------------------------------------------------------------------------------
// N1 -- crossing junctions: reservoir -> N_STAGES small junction chambers, each pair of
// consecutive nodes (reservoir/chamber/collector) connected by an X of two crossing channels, so
// material from the "left" approach and the "right" approach are forced into the SAME chamber
// several times down the vessel.
// -------------------------------------------------------------------------------------------

const N1_STAGES: usize = 4;
const N1_LANE_FRAC: f32 = 0.20; // * w_f, x-offset of the crossing lanes either side of centre
const N1_CH_HW_FRAC: f32 = 0.11; // * w_f, chamber half-width
const N1_CHANNEL_HW: f32 = 6.0; // channel half-width in cells (width 12, well over the 2-cell min)
const N1_COLLECTOR_FRAC: f32 = 0.16; // * (net height), collector height

/// One X-crossing between a "from" node (at y=y_from) and a "to" node (at y=y_to): an incoming
/// point pair (from_l, from_r) at y_from and an outgoing point pair (to_l, to_r) at y_to, wired so
/// from_l connects to to_r and from_r connects to to_l (the crossing).
fn cross_pair(from_l: f32, from_r: f32, y_from: f32, to_l: f32, to_r: f32, y_to: f32, hw: f32) -> [Shape; 2] {
    [
        Shape::Channel { x0: from_l, y0: y_from, x1: to_r, y1: y_to, hw },
        Shape::Channel { x0: from_r, y0: y_from, x1: to_l, y1: y_to, hw },
    ]
}

fn build_n1(w_f: f32, h_f: f32) -> (Vec<Shape>, f32, f32, Vec<f32>) {
    let total_half = TOTAL_HALF_FRAC * h_f;
    let res_hw = HW_FRAC * w_f;
    let lane = N1_LANE_FRAC * w_f;
    let ch_hw = N1_CH_HW_FRAC * w_f;

    // Fixed-point sizing: reservoir height depends on network capacity, network capacity barely
    // depends on reservoir height (chambers are sized in absolute cells / fractions of stage
    // height, not of the leftover span), so this converges in a couple of iterations.
    let mut res_h = 20.0f32;
    let mut net_cap = 0usize;
    let mut chamber_cy: Vec<f32> = Vec::new();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut collector_y0 = 0.0f32;
    for _ in 0..5 {
        let res_y0 = -total_half;
        let res_y1 = res_y0 + res_h;
        let net_y0 = res_y1;
        let net_y1 = total_half;
        let net_h = net_y1 - net_y0;
        let collector_h = N1_COLLECTOR_FRAC * net_h;
        let stages_h = net_h - collector_h;
        let stage_h = stages_h / N1_STAGES as f32;

        chamber_cy.clear();
        for i in 0..N1_STAGES {
            let y0 = net_y0 + i as f32 * stage_h;
            chamber_cy.push(y0 + stage_h * 0.55);
        }
        collector_y0 = net_y0 + stages_h;

        shapes = Vec::new();
        shapes.push(Shape::Chamber { cx: 0.0, cy: (res_y0 + res_y1) / 2.0, hx: res_hw, hy: (res_y1 - res_y0) / 2.0, r: 6.0 });
        for &cy in &chamber_cy {
            shapes.push(Shape::Chamber { cx: 0.0, cy, hx: ch_hw, hy: stage_h * 0.30, r: 6.0 });
        }
        shapes.push(Shape::Chamber { cx: 0.0, cy: (collector_y0 + total_half) / 2.0, hx: res_hw, hy: (total_half - collector_y0) / 2.0, r: 6.0 });

        // reservoir -> chamber 0
        let inset = 4.0;
        shapes.extend(cross_pair(-lane, lane, res_y1 - inset, -ch_hw * 0.5, ch_hw * 0.5, chamber_cy[0] - stage_h * 0.30 + inset, N1_CHANNEL_HW));
        // chamber i -> chamber i+1
        for i in 0..N1_STAGES - 1 {
            let y_from = chamber_cy[i] + stage_h * 0.30 - inset;
            let y_to = chamber_cy[i + 1] - stage_h * 0.30 + inset;
            shapes.extend(cross_pair(-ch_hw * 0.5, ch_hw * 0.5, y_from, -ch_hw * 0.5, ch_hw * 0.5, y_to, N1_CHANNEL_HW));
        }
        // last chamber -> collector
        let last = N1_STAGES - 1;
        let y_from = chamber_cy[last] + stage_h * 0.30 - inset;
        shapes.extend(cross_pair(-ch_hw * 0.5, ch_hw * 0.5, y_from, -lane, lane, collector_y0 + inset, N1_CHANNEL_HW));

        let net_mask = rasterize(GRID, GRID, |dx, dy| dy >= net_y0 && network_inside(&shapes, dx, dy));
        net_cap = capacity(&net_mask);
        let target_area = (0.55 * net_cap as f32).ceil();
        res_h = (target_area / (2.0 * res_hw)).ceil().max(20.0);
    }

    (shapes, res_h, collector_y0, chamber_cy)
}

// -------------------------------------------------------------------------------------------
// N2 -- split and recombine: repeated "swap the touching inner halves of each adjacent pair of
// bands" doubling, applied 3 times (2 -> 4 -> 8 -> 16 bands). See the module doc comment for why
// this -- not a literal lamination duplicate -- is the buildable 2D version of the classic mixer.
// -------------------------------------------------------------------------------------------

const N2_STAGES: usize = 3; // 2 -> 4 -> 8 -> 16 bands
const N2_WALL_FRAC: f32 = 0.18; // fraction of a band's width kept as wall margin each side

/// One doubling stage: `n` input bands (equal-width slices of [-hw, hw]) at y=y0 become `2n`
/// output bands at y=y1. For each input pair (2k, 2k+1): input(2k)_left-half stays in place,
/// input(2k+1)_right-half stays in place, and the two TOUCHING inner halves --
/// input(2k)_right-half and input(2k+1)_left-half -- swap (a crossing X). This is the only
/// operation of the four that changes anything; the outer two are straight channels.
fn n2_doubling_stage(n: usize, hw: f32, y0: f32, y1: f32) -> Vec<Shape> {
    let w = 2.0 * hw;
    let n2 = 2 * n;
    let slot_out = w / n2 as f32;
    let margin = slot_out * N2_WALL_FRAC;
    let half_chan = slot_out / 2.0 - margin;
    let mut shapes = Vec::new();

    let out_slot_center = |j: usize| -hw + (j as f32 + 0.5) * slot_out;
    let in_slot_lo_hi = |k: usize| {
        let slot_in = w / n as f32;
        (-hw + k as f32 * slot_in, -hw + (k as f32 + 1.0) * slot_in)
    };

    for k in 0..n / 2 {
        // input pair (2k, 2k+1) -> output slots (4k, 4k+1, 4k+2, 4k+3)
        let (lo0, hi0) = in_slot_lo_hi(2 * k);
        let (lo1, hi1) = in_slot_lo_hi(2 * k + 1);
        let in0_l = (lo0 + hi0) / 2.0 - (hi0 - lo0) / 4.0;
        let in0_r = (lo0 + hi0) / 2.0 + (hi0 - lo0) / 4.0;
        let in1_l = (lo1 + hi1) / 2.0 - (hi1 - lo1) / 4.0;
        let in1_r = (lo1 + hi1) / 2.0 + (hi1 - lo1) / 4.0;

        let out4k = out_slot_center(4 * k);
        let out4k1 = out_slot_center(4 * k + 1);
        let out4k2 = out_slot_center(4 * k + 2);
        let out4k3 = out_slot_center(4 * k + 3);

        // straight (no crossing): in0_l -> out(4k), in1_r -> out(4k+3)
        shapes.push(Shape::Channel { x0: in0_l, y0, x1: out4k, y1, hw: half_chan });
        shapes.push(Shape::Channel { x0: in1_r, y0, x1: out4k3, y1, hw: half_chan });
        // crossing: in1_l -> out(4k+1), in0_r -> out(4k+2)
        shapes.push(Shape::Channel { x0: in1_l, y0, x1: out4k1, y1, hw: half_chan });
        shapes.push(Shape::Channel { x0: in0_r, y0, x1: out4k2, y1, hw: half_chan });
    }
    shapes
}

fn build_n2(w_f: f32, h_f: f32) -> (Vec<Shape>, f32, f32) {
    let total_half = TOTAL_HALF_FRAC * h_f;
    let hw = HW_FRAC * w_f;

    let mut res_h = 20.0f32;
    let mut net_cap = 0usize;
    let mut shapes: Vec<Shape> = Vec::new();
    let mut collector_y0 = 0.0f32;
    for _ in 0..5 {
        let res_y0 = -total_half;
        let res_y1 = res_y0 + res_h;
        let net_y0 = res_y1;
        let net_y1 = total_half;
        let net_h = net_y1 - net_y0;
        let collector_h = 0.20 * net_h;
        let stages_h = net_h - collector_h;
        let stage_h = stages_h / N2_STAGES as f32;
        // stage_h/slot_width for the final (finest) stage must satisfy MIN_DROP_RUN_RATIO -- the
        // finest stage moves material by half a slot width (see n2_doubling_stage), and the
        // narrowest slots are the LAST stage's outputs (16 bands).
        let finest_slot = (2.0 * hw) / 16.0;
        let min_stage_h = MIN_DROP_RUN_RATIO * finest_slot * 0.5;
        let stage_h = stage_h.max(min_stage_h);

        collector_y0 = net_y0 + stage_h * N2_STAGES as f32;

        shapes = Vec::new();
        shapes.push(Shape::Chamber { cx: 0.0, cy: (res_y0 + res_y1) / 2.0, hx: hw, hy: (res_y1 - res_y0) / 2.0, r: 6.0 });
        let mut n = 2usize;
        for s in 0..N2_STAGES {
            let y0 = net_y0 + s as f32 * stage_h;
            let y1 = y0 + stage_h;
            shapes.extend(n2_doubling_stage(n, hw, y0, y1));
            n *= 2;
        }
        shapes.push(Shape::Chamber { cx: 0.0, cy: (collector_y0 + total_half) / 2.0, hx: hw, hy: (total_half - collector_y0) / 2.0, r: 6.0 });

        let net_mask = rasterize(GRID, GRID, |dx, dy| dy >= net_y0 && network_inside(&shapes, dx, dy));
        net_cap = capacity(&net_mask);
        let target_area = (0.55 * net_cap as f32).ceil();
        res_h = (target_area / (2.0 * hw)).ceil().max(20.0);
    }

    (shapes, res_h, collector_y0)
}

// -------------------------------------------------------------------------------------------
// N3 -- layering: a short direct path (from the reservoir's LEFT / red half) and a long winding
// path (from the RIGHT / green half) both converge on one shared collector, so red should arrive
// and settle first, with green layering on top once it finally gets there.
// -------------------------------------------------------------------------------------------

const N3_FAST_HW: f32 = 14.0; // half-width of the fast (red) channel
const N3_SLOW_HW: f32 = 11.0; // half-width of the slow (green) channel -- narrower, so it also
                               // carries less flux per unit time, reinforcing the delay
const N3_SLOW_LEGS: usize = 10; // number of switchback traversals for the slow path -- confined
                                 // to the right half (see build_n3), so more legs is what buys
                                 // extra path length, not a shallower angle.

fn build_n3(w_f: f32, h_f: f32) -> (Vec<Shape>, f32, f32) {
    let total_half = TOTAL_HALF_FRAC * h_f;
    let hw = HW_FRAC * w_f;

    let mut res_h = 20.0f32;
    let mut net_cap = 0usize;
    let mut shapes: Vec<Shape> = Vec::new();
    let mut collector_y0 = 0.0f32;
    for _ in 0..5 {
        let res_y0 = -total_half;
        let res_y1 = res_y0 + res_h;
        let net_y0 = res_y1;
        let net_y1 = total_half;
        let net_h = net_y1 - net_y0;
        let collector_h = 0.22 * net_h;
        let paths_h = net_h - collector_h;
        collector_y0 = net_y0 + paths_h;

        shapes = Vec::new();
        shapes.push(Shape::Chamber { cx: 0.0, cy: (res_y0 + res_y1) / 2.0, hx: hw, hy: (res_y1 - res_y0) / 2.0, r: 6.0 });
        shapes.push(Shape::Chamber { cx: 0.0, cy: (collector_y0 + total_half) / 2.0, hx: hw, hy: (total_half - collector_y0) / 2.0, r: 6.0 });

        // FAST path: confined to the LEFT half of the envelope (under the reservoir's red half),
        // a short, nearly straight drop with one gentle bend for character. Kept entirely at
        // x < -8 so it never spatially overlaps the slow path below -- two channels that cross
        // in this flat 2D mask necessarily MERGE where they touch (there is no bridge/via), which
        // would blur exactly the "two distinguishable paths" this design depends on.
        let fast_lo = -hw + N3_FAST_HW + 2.0;
        let fast_x0 = fast_lo + (hw * 0.30);
        let fast_y0 = res_y1 - 4.0;
        let fast_x1 = fast_lo + (hw * 0.12);
        let fast_y1 = collector_y0 + 4.0;
        shapes.push(Shape::Channel { x0: fast_x0, y0: fast_y0, x1: fast_x1, y1: fast_y1, hw: N3_FAST_HW });

        // SLOW path: confined to the RIGHT half of the envelope (under the reservoir's green
        // half, x > 8), switchbacking within that half N3_SLOW_LEGS times before reaching the
        // collector -- several times the path LENGTH of the fast path for the same vertical
        // drop, hence much later arrival, WITHOUT ever crossing into the fast path's territory.
        // Confining the swing to one half (rather than the full envelope) means the ratio floor
        // is no longer the binding constraint on sweep -- it is the region width -- so more legs
        // (not a shallower angle) is what buys extra path length here; each leg's actual
        // drop/run ratio is checked afterwards and stays far above the repose floor.
        let slow_lo = 8.0;
        let slow_hi = hw - N3_SLOW_HW - 2.0;
        let amp_center = (slow_lo + slow_hi) / 2.0;
        let half_amp = (slow_hi - slow_lo) / 2.0;
        let leg_h = (paths_h - 8.0) / N3_SLOW_LEGS as f32;
        let mut x = amp_center + half_amp; // start at the right edge of the confined region
        let mut y = res_y1 - 4.0;
        let mut going_right = false;
        for _ in 0..N3_SLOW_LEGS {
            let nx = if going_right { amp_center + half_amp } else { amp_center - half_amp };
            let ny = y + leg_h;
            shapes.push(Shape::Channel { x0: x, y0: y, x1: nx, y1: ny, hw: N3_SLOW_HW });
            x = nx;
            y = ny;
            going_right = !going_right;
        }
        // final short drop into the collector, drifting back toward the right-half centre.
        shapes.push(Shape::Channel { x0: x, y0: y, x1: amp_center, y1: collector_y0 + 4.0, hw: N3_SLOW_HW });

        let net_mask = rasterize(GRID, GRID, |dx, dy| dy >= net_y0 && network_inside(&shapes, dx, dy));
        net_cap = capacity(&net_mask);
        let target_area = (0.55 * net_cap as f32).ceil();
        res_h = (target_area / (2.0 * hw)).ceil().max(20.0);
    }

    (shapes, res_h, collector_y0)
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

fn main() {
    let out_dir = Path::new("artifacts/design/network-2026-09-19");
    fs::create_dir_all(out_dir).expect("create output dir");

    let w_f = GRID as f32;
    let h_f = GRID as f32;
    let do_trace = std::env::var("TRACE_MASS").is_ok();

    // ===================================================================================
    // N1 -- crossing junctions
    // ===================================================================================
    let (n1_shapes, n1_res_h, n1_collector_y0, _n1_chamber_cy) = build_n1(w_f, h_f);
    let n1_shapes = std::rc::Rc::new(n1_shapes);
    let n1_total_half = TOTAL_HALF_FRAC * h_f;
    let n1_res_y0 = -n1_total_half;
    let n1_res_y1 = n1_res_y0 + n1_res_h;
    let n1_inside = {
        let shapes = n1_shapes.clone();
        move |dx: f32, dy: f32| network_inside(&shapes, dx, dy)
    };
    let n1_mask = rasterize(GRID, GRID, n1_inside.clone());
    let n1_is_reservoir = move |_dx: f32, dy: f32| dy < n1_res_y1;
    let n1_is_collector = move |_dx: f32, dy: f32| dy >= n1_collector_y0;
    let n1_net_cap = capacity(&n1_mask) - {
        let center_y = h_f / 2.0;
        n1_mask
            .iter()
            .enumerate()
            .filter(|&(i, &m)| m != MASK_OUTSIDE && (i / GRID) as f32 - center_y < n1_res_y1)
            .count()
    };
    let n1_fill = {
        let inside = n1_inside.clone();
        move |dx: f32, dy: f32| if n1_is_reservoir(dx, dy) && inside(dx, dy) { 1.0 } else { 0.0 }
    };

    if do_trace {
        let mut sim = build_sim(n1_mask.clone(), n1_fill.clone(), GRID);
        trace_mass("N1", &mut sim, MaterialMode::Water, 2500, 50, n1_is_reservoir, n1_is_collector);
        let mut sim = build_sim(n1_mask.clone(), n1_fill.clone(), GRID);
        trace_mass("N1", &mut sim, MaterialMode::DrySand, 3000, 100, n1_is_reservoir, n1_is_collector);
    }
    let water_ticks_n1 = [0u32, 300, 800, 2400];
    let sand_ticks_n1 = [0u32, 300, 900, 3000];
    let n1_result = run_design(
        "N1_crossing_junctions",
        out_dir,
        n1_mask,
        n1_fill,
        n1_is_reservoir,
        n1_is_collector,
        n1_net_cap,
        &water_ticks_n1,
        &sand_ticks_n1,
    );

    // ===================================================================================
    // N2 -- split and recombine
    // ===================================================================================
    let (n2_shapes, n2_res_h, n2_collector_y0) = build_n2(w_f, h_f);
    let n2_shapes = std::rc::Rc::new(n2_shapes);
    let n2_total_half = TOTAL_HALF_FRAC * h_f;
    let n2_res_y0 = -n2_total_half;
    let n2_res_y1 = n2_res_y0 + n2_res_h;
    let n2_inside = {
        let shapes = n2_shapes.clone();
        move |dx: f32, dy: f32| network_inside(&shapes, dx, dy)
    };
    let n2_mask = rasterize(GRID, GRID, n2_inside.clone());
    let n2_is_reservoir = move |_dx: f32, dy: f32| dy < n2_res_y1;
    let n2_is_collector = move |_dx: f32, dy: f32| dy >= n2_collector_y0;
    let n2_net_cap = capacity(&n2_mask) - {
        let center_y = h_f / 2.0;
        n2_mask
            .iter()
            .enumerate()
            .filter(|&(i, &m)| m != MASK_OUTSIDE && (i / GRID) as f32 - center_y < n2_res_y1)
            .count()
    };
    let n2_fill = {
        let inside = n2_inside.clone();
        move |dx: f32, dy: f32| if n2_is_reservoir(dx, dy) && inside(dx, dy) { 1.0 } else { 0.0 }
    };

    if do_trace {
        let mut sim = build_sim(n2_mask.clone(), n2_fill.clone(), GRID);
        trace_mass("N2", &mut sim, MaterialMode::Water, 2500, 50, n2_is_reservoir, n2_is_collector);
        let mut sim = build_sim(n2_mask.clone(), n2_fill.clone(), GRID);
        trace_mass("N2", &mut sim, MaterialMode::DrySand, 3000, 100, n2_is_reservoir, n2_is_collector);
    }
    let water_ticks_n2 = [0u32, 150, 350, 700];
    let sand_ticks_n2 = [0u32, 150, 400, 900];
    let n2_result = run_design(
        "N2_split_recombine",
        out_dir,
        n2_mask,
        n2_fill,
        n2_is_reservoir,
        n2_is_collector,
        n2_net_cap,
        &water_ticks_n2,
        &sand_ticks_n2,
    );

    // ===================================================================================
    // N3 -- layering
    // ===================================================================================
    let (n3_shapes, n3_res_h, n3_collector_y0) = build_n3(w_f, h_f);
    let n3_shapes = std::rc::Rc::new(n3_shapes);
    let n3_total_half = TOTAL_HALF_FRAC * h_f;
    let n3_res_y0 = -n3_total_half;
    let n3_res_y1 = n3_res_y0 + n3_res_h;
    let n3_inside = {
        let shapes = n3_shapes.clone();
        move |dx: f32, dy: f32| network_inside(&shapes, dx, dy)
    };
    let n3_mask = rasterize(GRID, GRID, n3_inside.clone());
    let n3_is_reservoir = move |_dx: f32, dy: f32| dy < n3_res_y1;
    let n3_is_collector = move |_dx: f32, dy: f32| dy >= n3_collector_y0;
    let n3_net_cap = capacity(&n3_mask) - {
        let center_y = h_f / 2.0;
        n3_mask
            .iter()
            .enumerate()
            .filter(|&(i, &m)| m != MASK_OUTSIDE && (i / GRID) as f32 - center_y < n3_res_y1)
            .count()
    };
    let n3_fill = {
        let inside = n3_inside.clone();
        move |dx: f32, dy: f32| if n3_is_reservoir(dx, dy) && inside(dx, dy) { 1.0 } else { 0.0 }
    };

    if do_trace {
        let mut sim = build_sim(n3_mask.clone(), n3_fill.clone(), GRID);
        trace_mass("N3", &mut sim, MaterialMode::Water, 6000, 200, n3_is_reservoir, n3_is_collector);
        let mut sim = build_sim(n3_mask.clone(), n3_fill.clone(), GRID);
        trace_mass("N3", &mut sim, MaterialMode::DrySand, 8000, 200, n3_is_reservoir, n3_is_collector);
    }
    let water_ticks_n3 = [0u32, 300, 800, 3800];
    let sand_ticks_n3 = [0u32, 600, 2000, 8000];
    let n3_result = run_design(
        "N3_layering",
        out_dir,
        n3_mask,
        n3_fill,
        n3_is_reservoir,
        n3_is_collector,
        n3_net_cap,
        &water_ticks_n3,
        &sand_ticks_n3,
    );

    println!("\n=== summary ===");
    for r in [&n1_result, &n2_result, &n3_result] {
        println!(
            "{}: mass={:.1} network_capacity={} fraction_of_network={:.3} water_err={:.6} sand_err={:.6} mirror_mismatches={}",
            r.name, r.initial_mass, r.network_capacity, r.fraction_of_network, r.water_mass_err, r.sand_mass_err, r.mirror_mismatches
        );
    }
}
