//! Coarse pressure coupling A/B at the production resolution (grid 512), ON vs OFF via
//! `DrawingSimulation::coarse_pressure_coupling`. Written to answer task #70-followup's question
//! directly: is there anything VISIBLE to look at, in numbers a human can read off a screen, not
//! just a diff. Four measurements, each run twice (coupling true/false), same seed/scenario:
//!
//! 1. Pool levelling (ticks for a dropped column's surface spread to halve) -- `diag_resolution.rs`
//!    is the precedent this borrows `com_row`/`surface_spread`/`make_pile` from almost verbatim.
//! 2. U-tube riser rise (rows risen over a fixed tick budget) -- geometry borrowed from
//!    `tests/overfill_pressure_toggle.rs`'s `build_u_tube`/region consts, scaled from grid 128 to
//!    grid 512 (`U_TUBE_RECTS` is fractional, so ranges scale linearly with grid size).
//! 3. Oscillation / settled churn + checkerboard parity (`vpar`) -- borrowed from
//!    `overfill_pressure_toggle.rs`'s `diag_task70_rest_color_mixing_and_checkerboard` /
//!    `rest_metrics`.
//! 4. Cost (ms/tick) for Water and DrySand -- borrowed from `diag_blocks.rs`.
//!
//! Run: `cargo run --release --example diag_coarse_ab`

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

const GRID: usize = 512;

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

fn com_row(sim: &DrawingSimulation) -> f64 {
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for y in 0..h {
        let r: f64 = sim.heightmap.data[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum();
        num += r * y as f64;
        den += r;
    }
    if den > 0.0 { num / den / (h - 1) as f64 } else { f64::NAN }
}

fn surface_spread(sim: &DrawingSimulation) -> f64 {
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    let mut cols: Vec<f64> = Vec::new();
    for x in 0..w {
        let mut s = 0.0f64;
        for y in 0..h {
            if sim.shape_mask[y * w + x] != 0 { s += sim.heightmap.data[y * w + x] as f64; }
        }
        let inside = (0..h).filter(|&y| sim.shape_mask[y * w + x] != 0).count();
        if inside > 0 { cols.push(s / inside as f64); }
    }
    if cols.len() < 2 { return 0.0; }
    let mx = cols.iter().cloned().fold(f64::MIN, f64::max);
    let mn = cols.iter().cloned().fold(f64::MAX, f64::min);
    let mean: f64 = cols.iter().sum::<f64>() / cols.len() as f64;
    (mx - mn) / mean.max(1e-12)
}

/// Drop a narrow tall column of water into an empty square box and watch it level out.
/// Verbatim scaling of `diag_resolution.rs::make_pile`, parameterised on `coupling`.
fn make_pile(grid: usize, coupling: bool) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    sim.coarse_pressure_coupling = coupling;
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    for i in sim.heightmap.data.iter_mut() { *i = 0.0; }
    for y in (h / 4)..h {
        for x in (w * 7 / 16)..(w * 9 / 16) {
            if sim.shape_mask[y * w + x] != 0 { sim.heightmap.data[y * w + x] = 1.0; }
        }
    }
    sim
}

fn make_hourglass(grid: usize, coupling: bool) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.coarse_pressure_coupling = coupling;
    sim
}

// ---------------------------------------------------------------------------------------------
// 1. Pool levelling
// ---------------------------------------------------------------------------------------------

fn measure_pool_levelling() {
    println!("\n=== 1. POOL LEVELLING (grid {GRID}, dropped column, Square) ===");
    println!("{:<10} {:>10} {:>12} {:>12} {:>14}", "coupling", "s0", "s_final", "frac left", "ticks to 50%");
    const MAX_TICKS: usize = 6000;
    let targets = [None; 5];
    for &coupling in &[true, false] {
        let mut sim = make_pile(GRID, coupling);
        let s0 = surface_spread(&sim);
        let mut half = None;
        for t in 0..MAX_TICKS {
            sim.budget_n = 4096;
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
            if half.is_none() && surface_spread(&sim) < 0.5 * s0 { half = Some(t + 1); }
        }
        let s1 = surface_spread(&sim);
        println!(
            "{:<10} {:>10.3} {:>12.3} {:>12.3} {:>14}",
            coupling, s0, s1, s1 / s0.max(1e-12),
            half.map(|v| v.to_string()).unwrap_or_else(|| format!(">{MAX_TICKS}"))
        );
    }

    println!("\n=== 1b. HOURGLASS DRAIN RATE (grid {GRID}) ===");
    println!("{:<10} {:>16}", "coupling", "descent/tick");
    const WARM: usize = 40;
    const RUN: usize = 2000;
    for &coupling in &[true, false] {
        let mut sim = make_hourglass(GRID, coupling);
        for _ in 0..WARM {
            sim.budget_n = 4096;
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
        }
        let c0 = com_row(&sim);
        for _ in 0..RUN {
            sim.budget_n = 4096;
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
        }
        let d = (com_row(&sim) - c0) / RUN as f64;
        println!("{:<10} {:>16.4e}", coupling, d);
    }
}

// ---------------------------------------------------------------------------------------------
// 2. U-tube riser
// ---------------------------------------------------------------------------------------------

fn region_mass(sim: &DrawingSimulation, w: usize, xs: std::ops::Range<usize>, ys: std::ops::Range<usize>) -> f32 {
    ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
        .map(|(x, y)| sim.heightmap.data[y * w + x])
        .sum()
}

fn fill_height(sim: &DrawingSimulation, w: usize, xs: std::ops::Range<usize>, ys: std::ops::Range<usize>) -> usize {
    let floor = ys.end;
    ys.clone()
        .find(|&y| xs.clone().any(|x| sim.heightmap.data[y * w + x] > 0.1))
        .map(|top| floor - top)
        .unwrap_or(0)
}

/// `overfill_pressure_toggle.rs`'s U-tube region constants were derived at w=h=128, center=64.
/// `U_TUBE_RECTS` is fractional (x/y in [-1,1] about the grid centre), so every rect scales
/// linearly with grid size -- scale each 128-based bound by `grid/128`.
fn scale_range(r: std::ops::Range<usize>, grid: usize) -> std::ops::Range<usize> {
    let f = grid as f64 / 128.0;
    ((r.start as f64 * f).round() as usize)..((r.end as f64 * f).round() as usize)
}

fn measure_u_tube() {
    println!("\n=== 2. U-TUBE RISER (grid {GRID}) ===");
    const RESERVOIR_X: std::ops::Range<usize> = 11..34;
    const RESERVOIR_Y: std::ops::Range<usize> = 13..111;
    let _ = RESERVOIR_X;
    let _ = RESERVOIR_Y;
    const BASIN_X: std::ops::Range<usize> = 11..67;
    const BASIN_Y: std::ops::Range<usize> = 111..118;
    const RISER_X: std::ops::Range<usize> = 59..67;
    const RISER_Y: std::ops::Range<usize> = 77..111;

    let basin_x = scale_range(BASIN_X, GRID);
    let basin_y = scale_range(BASIN_Y, GRID);
    let riser_x = scale_range(RISER_X, GRID);
    let riser_y = scale_range(RISER_Y, GRID);
    let riser_rows = riser_y.end - riser_y.start;

    println!("riser probe: x {riser_x:?} y {riser_y:?} ({riser_rows} rows tall)");
    println!("{:<10} {:>8} {:>8} {:>8} {:>8} {:>10} {:>10}", "coupling", "t=1500", "t=3000", "t=4500", "t=6000", "basin_m", "riser_m");

    let targets = [None; 5];
    for &coupling in &[true, false] {
        let mut sim = DrawingSimulation::new_with_size(GRID);
        sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(MaterialMode::Water);
        sim.overfill_pressure = true;
        sim.overfill_capacity = 1.90;
        sim.coarse_pressure_coupling = coupling;
        sim.initialize_hourglass();

        let mut heights = Vec::new();
        for _ in 0..4 {
            for _ in 0..1500 {
                sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
            }
            heights.push(fill_height(&sim, GRID, riser_x.clone(), riser_y.clone()));
        }
        let basin_m = region_mass(&sim, GRID, basin_x.clone(), basin_y.clone());
        let riser_m = region_mass(&sim, GRID, riser_x.clone(), riser_y.clone());
        println!(
            "{:<10} {:>8} {:>8} {:>8} {:>8} {:>10.1} {:>10.1}",
            coupling, heights[0], heights[1], heights[2], heights[3], basin_m, riser_m
        );
    }
}

// ---------------------------------------------------------------------------------------------
// 3. Oscillation / settled churn + checkerboard parity
// ---------------------------------------------------------------------------------------------

fn paint_stripes(sim: &mut DrawingSimulation, w: usize) {
    let n = sim.heightmap.data.len();
    for i in 0..n {
        let x = i % w;
        let v: u8 = if (x / 4) % 2 == 0 { 240 } else { 16 };
        sim.cell_colors[i * 4] = v;
        sim.cell_colors[i * 4 + 1] = 128;
        sim.cell_colors[i * 4 + 2] = 255 - v;
        sim.cell_colors[i * 4 + 3] = 255;
    }
}

fn vpar(sim: &DrawingSimulation, w: usize, occupied: &[bool]) -> f32 {
    let h = sim.heightmap.data.len() / w;
    let (mut vs, mut vm) = (0.0f32, 0.0f32);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if !occupied[i] { continue; }
            let v = sim.edge_vel_v[i];
            let s = if (x + y) % 2 == 0 { 1.0 } else { -1.0 };
            vs += v * s;
            vm += v.abs();
        }
    }
    if vm > 0.0 { vs / vm } else { 0.0 }
}

fn measure_oscillation() {
    println!("\n=== 3. OSCILLATION / SETTLED CHURN (grid {GRID}, Square, settle then measure) ===");
    println!("baseline (task #70 fix, HANDOVER §9): 0.00002 per cell per tick, measured at a smaller grid");
    println!("{:<10} {:<8} {:>14} {:>10}", "coupling", "material", "churn/cell/tick", "vpar");
    let targets = [None; 5];
    for &(name, mat) in &[("water", MaterialMode::Water), ("drysand", MaterialMode::DrySand)] {
        for &coupling in &[true, false] {
            let mut sim = DrawingSimulation::new_with_size(GRID);
            sim.sandbox_shape = SandboxShape::Square;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.apply_preset(mat);
            sim.overfill_pressure = true;
            sim.coarse_pressure_coupling = coupling;
            sim.initialize_hourglass();
            for _ in 0..4000 {
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Square, 16.0, 16.0);
            }
            let occupied: Vec<bool> = sim.heightmap.data.iter().map(|&v| v > 0.5).collect();
            let n_occ = occupied.iter().filter(|&&b| b).count();
            paint_stripes(&mut sim, GRID);
            let mut prev_h = sim.heightmap.data.clone();
            let mut dh = 0.0f32;
            const STEP: usize = 500;
            for _ in 0..STEP {
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Square, 16.0, 16.0);
                for i in 0..prev_h.len() {
                    if occupied[i] {
                        dh += (sim.heightmap.data[i] - prev_h[i]).abs();
                    }
                }
                prev_h.copy_from_slice(&sim.heightmap.data);
            }
            let churn = dh / (STEP * n_occ.max(1)) as f32;
            let vp = vpar(&sim, GRID, &occupied);
            println!("{:<10} {:<8} {:>14.6} {:>10.3}", coupling, name, churn, vp);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 4. Cost
// ---------------------------------------------------------------------------------------------

fn measure_cost() {
    println!("\n=== 4. COST (ms/tick, grid {GRID}, Hourglass) ===");
    println!("{:<10} {:<8} {:>10}", "coupling", "material", "ms/tick");
    const WARM: usize = 60;
    const TICKS: usize = 300;
    let targets = [None; 5];
    for &(name, mat) in &[("water", MaterialMode::Water), ("drysand", MaterialMode::DrySand)] {
        for &coupling in &[true, false] {
            let mut sim = DrawingSimulation::new_with_size(GRID);
            sim.sandbox_shape = SandboxShape::Hourglass;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.apply_preset(mat);
            sim.initialize_hourglass();
            sim.overfill_pressure = true;
            sim.coarse_pressure_coupling = coupling;
            for _ in 0..WARM {
                sim.budget_n = 1024;
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
            }
            let t0 = Instant::now();
            for _ in 0..TICKS {
                sim.budget_n = 1024;
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0 / TICKS as f64;
            println!("{:<10} {:<8} {:>10.3}", coupling, name, ms);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let only = args.first().cloned();
    match only.as_deref() {
        Some("pool") => measure_pool_levelling(),
        Some("utube") => measure_u_tube(),
        Some("oscillation") => measure_oscillation(),
        Some("cost") => measure_cost(),
        _ => {
            measure_cost();
            measure_oscillation();
            measure_u_tube();
            measure_pool_levelling();
        }
    }
}
