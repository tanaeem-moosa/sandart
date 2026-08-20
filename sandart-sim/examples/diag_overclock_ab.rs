//! OVERCLOCKING.md: multi-rate block scheduler A/B at the production resolution (grid 512), ON
//! vs OFF via `DrawingSimulation::overclocking_enabled`. Structure and helpers borrowed almost
//! verbatim from `diag_coarse_ab.rs` (same shape, different toggle) so the two reports are
//! directly comparable. `coarse_pressure_coupling` (the driving-potential coupling) is left at
//! its SHIPPED DEFAULT (`false`) throughout -- the point of this instrument is what the
//! multi-rate scheduler alone buys, since it consumes `|Delta|` regardless of whether the
//! potential coupling is also on.
//!
//! 1. Pool levelling (ticks for a dropped column's surface spread to halve) + hourglass drain
//!    rate (descent/tick).
//! 2. Oscillation / settled churn + checkerboard parity (`vpar`) -- must not regress against the
//!    0.00002/cell/tick baseline (HANDOVER §9) with clocking off.
//! 3. Cost (ms/tick) for Water and DrySand, PLUS mass_err and the per-block clock-rate
//!    distribution -- what says whether this is affordable.
//! 4. Criterion 4 sanity check note: run `diag_support 512 400 hourglass overclock` separately
//!    for the free-falling-pressure count; not duplicated here.
//!
//! Run: `cargo run --release --example diag_overclock_ab`

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

const GRID: usize = 512;

// ---------------------------------------------------------------------------------------------
// Shared helpers (verbatim from diag_coarse_ab.rs)
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

fn make_pile(grid: usize, overclock: bool) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = overclock;
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    for i in sim.heightmap.data.iter_mut() { *i = 0.0; }
    for y in (h / 4)..h {
        for x in (w * 7 / 16)..(w * 9 / 16) {
            if sim.shape_mask[y * w + x] != 0 { sim.heightmap.data[y * w + x] = 1.0; }
        }
    }
    sim
}

fn make_hourglass(grid: usize, overclock: bool) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = overclock;
    sim
}

// ---------------------------------------------------------------------------------------------
// 1. Pool levelling + hourglass drain
// ---------------------------------------------------------------------------------------------

fn measure_pool_levelling() {
    println!("\n=== 1. POOL LEVELLING (grid {GRID}, dropped column, Square) ===");
    println!("{:<10} {:>10} {:>12} {:>12} {:>14}", "overclock", "s0", "s_final", "frac left", "ticks to 50%");
    // Reduced from diag_coarse_ab.rs's 6000 -- overclocking can be several-fold more expensive
    // per tick before the deferred performance stage lands (OVERCLOCKING.md), so this stays
    // tractable at the cost of possibly reporting ">MAX_TICKS" for the slower configuration,
    // which is itself an informative result.
    const MAX_TICKS: usize = 2000;
    let targets = [None; 5];
    for &overclock in &[true, false] {
        let mut sim = make_pile(GRID, overclock);
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
            overclock, s0, s1, s1 / s0.max(1e-12),
            half.map(|v| v.to_string()).unwrap_or_else(|| format!(">{MAX_TICKS}"))
        );
    }

    println!("\n=== 1b. HOURGLASS DRAIN RATE (grid {GRID}) ===");
    println!("{:<10} {:>16}", "overclock", "descent/tick");
    const WARM: usize = 40;
    const RUN: usize = 800;
    for &overclock in &[true, false] {
        let mut sim = make_hourglass(GRID, overclock);
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
        println!("{:<10} {:>16.4e}", overclock, d);
    }
}

// ---------------------------------------------------------------------------------------------
// 2. Oscillation / settled churn + checkerboard parity
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
    println!("\n=== 2. OSCILLATION / SETTLED CHURN (grid {GRID}, Square, settle then measure) ===");
    println!("baseline (task #70 fix, HANDOVER §9): 0.000025 per cell per tick, coupling off");
    println!("{:<10} {:<8} {:>14} {:>10}", "overclock", "material", "churn/cell/tick", "vpar");
    let targets = [None; 5];
    for &(name, mat) in &[("water", MaterialMode::Water), ("drysand", MaterialMode::DrySand)] {
        for &overclock in &[true, false] {
            let mut sim = DrawingSimulation::new_with_size(GRID);
            sim.sandbox_shape = SandboxShape::Square;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.apply_preset(mat);
            sim.overfill_pressure = true;
            sim.overclocking_enabled = overclock;
            sim.initialize_hourglass();
            // Reduced from diag_coarse_ab.rs's 4000/500 (overclocking can be several-fold more
            // expensive per tick before the deferred performance stage lands -- see
            // OVERCLOCKING.md -- so this instrument uses a shorter but still settle-then-measure
            // window to stay tractable) -- 1200 to settle, 300 to measure churn.
            for _ in 0..1200 {
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Square, 16.0, 16.0);
            }
            let occupied: Vec<bool> = sim.heightmap.data.iter().map(|&v| v > 0.5).collect();
            let n_occ = occupied.iter().filter(|&&b| b).count();
            paint_stripes(&mut sim, GRID);
            let mut prev_h = sim.heightmap.data.clone();
            let mut dh = 0.0f32;
            const STEP: usize = 300;
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
            println!("{:<10} {:<8} {:>14.6} {:>10.3}", overclock, name, churn, vp);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 3. Cost, mass error, and rate distribution
// ---------------------------------------------------------------------------------------------

fn measure_cost() {
    println!("\n=== 3. COST (ms/tick, grid {GRID}, Hourglass, shipped default budget=1024) ===");
    println!("{:<10} {:<8} {:>10} {:>12}", "overclock", "material", "ms/tick", "mass_err");
    const WARM: usize = 60;
    const TICKS: usize = 300;
    let targets = [None; 5];
    for &(name, mat) in &[("water", MaterialMode::Water), ("drysand", MaterialMode::DrySand)] {
        for &overclock in &[true, false] {
            let mut sim = DrawingSimulation::new_with_size(GRID);
            sim.sandbox_shape = SandboxShape::Hourglass;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.apply_preset(mat);
            sim.initialize_hourglass();
            sim.overfill_pressure = true;
            sim.overclocking_enabled = overclock;
            for _ in 0..WARM {
                sim.budget_n = 1024;
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
            }
            let m0: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
            let t0 = Instant::now();
            for _ in 0..TICKS {
                sim.budget_n = 1024;
                sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0 / TICKS as f64;
            let m1: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
            let mass_err = (m1 - m0).abs() / m0.max(1e-12);
            println!("{:<10} {:<8} {:>10.3} {:>12.2e}", overclock, name, ms, mass_err);

            if overclock {
                let mut counts = [0u64; 7]; // idx 0 = 1/8 .. idx 6 = 8
                let mut sum_rate = 0.0f64;
                for &r in &sim.block_clock_rate {
                    let idx = (r.max(1e-6).log2().round() as i32).clamp(-3, 3) + 3;
                    counts[idx as usize] += 1;
                    sum_rate += r as f64;
                }
                let total = sim.block_clock_rate.len().max(1) as f64;
                println!(
                    "  rate distribution (blocks, {name}): 1/8={} 1/4={} 1/2={} 1x={} 2x={} 4x={} 8x={}  mean_rate={:.3}",
                    counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6],
                    sum_rate / total
                );
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let only = args.first().cloned();
    match only.as_deref() {
        Some("pool") => measure_pool_levelling(),
        Some("oscillation") => measure_oscillation(),
        Some("cost") => measure_cost(),
        _ => {
            measure_cost();
            measure_oscillation();
            measure_pool_levelling();
        }
    }
}
