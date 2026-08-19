//! Does falling material carry pressure today? That is what breaks the hourglass whenever
//! pressure-driven flow is strengthened to make the U-tube work.
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use sandart_sim::physics::cell_capacity_for;

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(400);
    let shape = match std::env::args().nth(3).as_deref() {
        Some("utube") => SandboxShape::UTubeFlowThrough,
        _ => SandboxShape::Hourglass,
    };
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = shape;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.budget_n = 1024;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, shape, 0.0, 16.6);
    }
    let (w, h) = (grid, grid);
    // one-cell support test, the same shape `support_fraction` uses
    let supported = |i: usize| -> bool {
        let y = i / w;
        if y + 1 >= h { return true; }
        let b = i + w;
        if sim.shape_mask[b] == 0 { return true; }
        let cap = cell_capacity_for(sim.cell_props[b * 4]);
        sim.heightmap.data[b] / cap > 0.98
    };
    // transitive: walk down the column until we hit floor/casing or an unsupported gap
    let transitively_supported = |i: usize| -> bool {
        let (x, mut y) = (i % w, i / w);
        loop {
            if y + 1 >= h { return true; }
            let b = (y + 1) * w + x;
            if sim.shape_mask[b] == 0 { return true; }
            let cap = cell_capacity_for(sim.cell_props[b * 4]);
            if sim.heightmap.data[b] / cap <= 0.98 { return false; }
            y += 1;
        }
    };
    let (mut n1, mut o1, mut n0, mut o0) = (0u64, 0.0f64, 0u64, 0.0f64);
    let (mut nt1, mut ot1, mut nt0, mut ot0) = (0u64, 0.0f64, 0u64, 0.0f64);
    let mut falling_with_pressure = 0u64;
    for i in 0..w * h {
        if sim.shape_mask[i] == 0 || sim.heightmap.data[i] <= 1e-4 { continue; }
        let cap = cell_capacity_for(sim.cell_props[i * 4]);
        let o = (sim.heightmap.data[i] / cap - 1.0).max(0.0) as f64;
        if supported(i) { n1 += 1; o1 += o; } else { n0 += 1; o0 += o; }
        if transitively_supported(i) { nt1 += 1; ot1 += o; } else {
            nt0 += 1; ot0 += o;
            if o > 1e-4 { falling_with_pressure += 1; }
        }
    }
    println!("{shape:?} {grid}x{grid}, {ticks} ticks\n");
    println!("{:<34} {:>10} {:>14}", "classification", "cells", "mean overfill o");
    println!("{:<34} {:>10} {:>14.5}", "supported (one cell below)", n1, o1 / n1.max(1) as f64);
    println!("{:<34} {:>10} {:>14.5}", "unsupported (one cell below)", n0, o0 / n0.max(1) as f64);
    println!("{:<34} {:>10} {:>14.5}", "TRANSITIVELY supported", nt1, ot1 / nt1.max(1) as f64);
    println!("{:<34} {:>10} {:>14.5}", "TRANSITIVELY unsupported", nt0, ot0 / nt0.max(1) as f64);
    println!("\nfree-falling cells carrying nonzero pressure: {falling_with_pressure} of {nt0} ({:.1}%)",
             100.0 * falling_with_pressure as f64 / nt0.max(1) as f64);
}
