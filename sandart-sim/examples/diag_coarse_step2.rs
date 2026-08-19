//! Diagnostic instrument for Step 2 of `artifacts/design/HIERARCHICAL-PRESSURE.md`.
//! Measures:
//! 1. Restriction & coarse relaxation performance (ms per tick).
//! 2. Distribution of coarse-fine disagreement `|Delta| = |M - A|` across tiles in active flow vs resting equilibrium.
//! 3. Coarse hydraulic head `eta` profile down a column.

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(300);

    println!("=== Step 2 Diagnostic: Restriction, Coarse State, and |Delta| Distribution ===");
    println!("Grid: {grid}x{grid}, Ticks: {ticks}\n");

    // Scenario 1: Resting Pool
    println!("--- Scenario 1: Settled Pool (Square Sandbox) ---");
    let mut sim_pool = DrawingSimulation::new_with_size(grid);
    sim_pool.sandbox_shape = SandboxShape::Square;
    sim_pool.gravity_dir = Vec2::new(0.0, 0.04);
    sim_pool.apply_preset(MaterialMode::Water);
    sim_pool.overfill_pressure = true;
    sim_pool.generate_shape_mask();

    // Prefill bottom half
    let start_y = grid / 2;
    for y in start_y..grid {
        for x in 0..grid {
            let idx = y * grid + x;
            if sim_pool.shape_mask[idx] != 0 {
                sim_pool.heightmap.data[idx] = 1.0;
            }
        }
    }

    let targets = [None; 5];
    let t0 = Instant::now();
    for _ in 0..ticks {
        sim_pool.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
    }
    let elapsed_pool = t0.elapsed();
    println!("Simulated {ticks} ticks in {:.2}ms ({:.3}ms/tick)", elapsed_pool.as_secs_f64() * 1000.0, elapsed_pool.as_secs_f64() * 1000.0 / ticks as f64);
    report_delta_stats(&sim_pool, "Settled Pool");

    // Scenario 2: Active Draining Hourglass
    println!("\n--- Scenario 2: Active Draining Hourglass ---");
    let mut sim_hg = DrawingSimulation::new_with_size(grid);
    sim_hg.sandbox_shape = SandboxShape::Hourglass;
    sim_hg.gravity_dir = Vec2::new(0.0, 0.04);
    sim_hg.apply_preset(MaterialMode::Water);
    sim_hg.overfill_pressure = true;
    sim_hg.initialize_hourglass();

    let t0 = Instant::now();
    for _ in 0..ticks {
        sim_hg.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
    }
    let elapsed_hg = t0.elapsed();
    println!("Simulated {ticks} ticks in {:.2}ms ({:.3}ms/tick)", elapsed_hg.as_secs_f64() * 1000.0, elapsed_hg.as_secs_f64() * 1000.0 / ticks as f64);
    report_delta_stats(&sim_hg, "Active Hourglass");

    // Scenario 3: U-Tube Flow Through
    println!("\n--- Scenario 3: U-Tube Flow-Through ---");
    let mut sim_utube = DrawingSimulation::new_with_size(grid);
    sim_utube.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim_utube.gravity_dir = Vec2::new(0.0, 0.04);
    sim_utube.apply_preset(MaterialMode::Water);
    sim_utube.overfill_pressure = true;
    sim_utube.initialize_hourglass();

    let t0 = Instant::now();
    for _ in 0..ticks {
        sim_utube.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
    }
    let elapsed_utube = t0.elapsed();
    println!("Simulated {ticks} ticks in {:.2}ms ({:.3}ms/tick)", elapsed_utube.as_secs_f64() * 1000.0, elapsed_utube.as_secs_f64() * 1000.0 / ticks as f64);
    report_delta_stats(&sim_utube, "U-Tube Flow-Through");
}

fn report_delta_stats(sim: &DrawingSimulation, label: &str) {
    let geo = &sim.coarse;
    let state = &sim.coarse_state;
    if !geo.available {
        println!("[{label}] Coarse state unavailable at this grid size.");
        return;
    }

    let mut inside_tiles = 0usize;
    let mut wet_tiles = 0usize;
    let mut max_abs_delta = 0.0f32;
    let mut sum_abs_delta = 0.0f32;

    // Disagreement histogram buckets: [0..0.01), [0.01..0.1), [0.1..0.5), [0.5..2.0), [2.0..+)
    let mut b_tiny = 0usize;
    let mut b_small = 0usize;
    let mut b_med = 0usize;
    let mut b_large = 0usize;
    let mut b_huge = 0usize;

    for i in 0..state.delta.len() {
        if !geo.inside[i] {
            continue;
        }
        inside_tiles += 1;
        if state.a_mass[i] > 1e-4 || state.m_mass[i] > 1e-4 {
            wet_tiles += 1;
            let d = state.delta[i].abs();
            max_abs_delta = max_abs_delta.max(d);
            sum_abs_delta += d;

            if d < 0.01 {
                b_tiny += 1;
            } else if d < 0.1 {
                b_small += 1;
            } else if d < 0.5 {
                b_med += 1;
            } else if d < 2.0 {
                b_large += 1;
            } else {
                b_huge += 1;
            }
        }
    }

    let avg_delta = if wet_tiles > 0 { sum_abs_delta / wet_tiles as f32 } else { 0.0 };
    println!("\n[{label}] Disagreement Statistics across {inside_tiles} inside tiles ({wet_tiles} active/wet):");
    println!("  Max |Delta|: {max_abs_delta:.4}");
    println!("  Avg |Delta|: {avg_delta:.4}");
    println!("  Histogram of |Delta| over wet tiles:");
    println!("    [0.00..0.01) (Resting / In Sync):   {b_tiny:>4} ({:.1}%)", if wet_tiles > 0 { b_tiny as f32 * 100.0 / wet_tiles as f32 } else { 0.0 });
    println!("    [0.01..0.10) (Minor Adjustment):   {b_small:>4} ({:.1}%)", if wet_tiles > 0 { b_small as f32 * 100.0 / wet_tiles as f32 } else { 0.0 });
    println!("    [0.10..0.50) (Moderate Flow):      {b_med:>4} ({:.1}%)", if wet_tiles > 0 { b_med as f32 * 100.0 / wet_tiles as f32 } else { 0.0 });
    println!("    [0.50..2.00) (Active Front):       {b_large:>4} ({:.1}%)", if wet_tiles > 0 { b_large as f32 * 100.0 / wet_tiles as f32 } else { 0.0 });
    println!("    [2.00.. + )  (Surge / High Work):  {b_huge:>4} ({:.1}%)", if wet_tiles > 0 { b_huge as f32 * 100.0 / wet_tiles as f32 } else { 0.0 });

    // Print central column profile of M, A, Delta, and eta
    let n = geo.coarse_n;
    let cx = n / 2;
    println!("\n  Central Column (cx = {cx}) Profile:");
    println!("  {:>4} {:>10} {:>10} {:>10} {:>10} {:>10}", "cy", "A (Fine)", "M (Coarse)", "Delta", "P_coarse", "eta");
    for cy in 0..n {
        let idx = cy * n + cx;
        if !geo.inside[idx] {
            continue;
        }
        println!(
            "  {:>4} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>10.3}",
            cy, state.a_mass[idx], state.m_mass[idx], state.delta[idx], state.p_coarse[idx], state.eta[idx]
        );
    }
}
