//! Temporary probe for STEP3-ADAPTIVE-COARSE.md: `diag_support`'s free-falling-cells-carrying-
//! pressure percentage AND the §8 "no bang-bang transport" fire count, together, for both the
//! Hourglass and U-Tube flow-through scenarios, matching STEP3-FIXES.md's own methodology so the
//! numbers are directly comparable across sweep-count/restrict settings.
use glam::Vec2;
use sandart_sim::physics::{bang_bang_count, cell_capacity_for, reset_bang_bang_count};
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

fn run(shape: SandboxShape, label: &str, grid: usize, ticks: usize) {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = shape;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    let targets = [None; 5];
    reset_bang_bang_count();
    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, shape, 0.0, 16.6);
    }
    let bb = bang_bang_count();

    let (w, h) = (grid, grid);
    let transitively_supported = |i: usize| -> bool {
        let (x, mut y) = (i % w, i / w);
        loop {
            if y + 1 >= h {
                return true;
            }
            let b = (y + 1) * w + x;
            if sim.shape_mask[b] == 0 {
                return true;
            }
            let cap = cell_capacity_for(sim.cell_props[b * 4]);
            if sim.heightmap.data[b] / cap <= 0.98 {
                return false;
            }
            y += 1;
        }
    };
    let (mut nt0, mut falling_with_pressure) = (0u64, 0u64);
    for i in 0..w * h {
        if sim.shape_mask[i] == 0 || sim.heightmap.data[i] <= 1e-4 {
            continue;
        }
        let cap = cell_capacity_for(sim.cell_props[i * 4]);
        let o = (sim.heightmap.data[i] / cap - 1.0).max(0.0) as f64;
        if !transitively_supported(i) {
            nt0 += 1;
            if o > 1e-4 {
                falling_with_pressure += 1;
            }
        }
    }
    println!(
        "{label}: free-falling cells carrying nonzero pressure: {falling_with_pressure} of {nt0} ({:.1}%); bang-bang fires (coarse_head!=0): {bb} ({:.1}/tick)",
        100.0 * falling_with_pressure as f64 / nt0.max(1) as f64,
        bb as f64 / ticks as f64
    );
}

fn touched_fraction(grid: usize, ticks: usize) {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    let targets = [None; 5];
    for _ in 0..60 {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let mut sum_touched = 0u64;
    let mut sum_must = 0u64;
    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
        sum_touched += sim.blocks_touched.iter().filter(|&&v| v).count() as u64;
        sum_must += sim
            .active_blocks
            .iter()
            .filter(|a| !matches!(a, sandart_sim::BlockActivity::Inactive))
            .count() as u64;
    }
    let total = sim.blocks_touched.len() as f64;
    println!(
        "diag_blocks-matched Hourglass @ {grid}: avg touched/tick = {:.1} of {:.0} ({:.1}%), avg non-inactive (will_simulate-ish) = {:.1} ({:.1}%)",
        sum_touched as f64 / ticks as f64,
        total,
        100.0 * (sum_touched as f64 / ticks as f64) / total,
        sum_must as f64 / ticks as f64,
        100.0 * (sum_must as f64 / ticks as f64) / total,
    );
}

/// ms/tick and average touched-block fraction once a pool has actually settled -- the regime
/// incremental restriction is supposed to help most, unlike the continuously-draining Hourglass
/// `diag_blocks` benchmark.
fn settled_pool_cost(grid: usize) {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    for v in sim.heightmap.data.iter_mut() {
        *v = 0.0;
    }
    for y in (h / 2)..h {
        for x in 0..w {
            if sim.shape_mask[y * w + x] != 0 {
                sim.heightmap.data[y * w + x] = 1.0;
            }
        }
    }
    let targets = [None; 5];
    // Warm up well past settling.
    for _ in 0..3000 {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
    }
    let ticks = 200usize;
    let mut sum_touched = 0u64;
    let t0 = std::time::Instant::now();
    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
        sum_touched += sim.blocks_touched.iter().filter(|&&v| v).count() as u64;
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    let total = sim.blocks_touched.len() as f64;
    println!(
        "settled pool @ {grid}: {ms:.3} ms/tick, avg touched/tick = {:.1} of {:.0} ({:.1}%)",
        sum_touched as f64 / ticks as f64,
        total,
        100.0 * (sum_touched as f64 / ticks as f64) / total,
    );
}

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(400);
    if std::env::args().any(|a| a == "--settled") {
        settled_pool_cost(grid);
        return;
    }
    run(SandboxShape::Hourglass, "Hourglass", grid, ticks);
    run(SandboxShape::UTubeFlowThrough, "U-Tube", grid, ticks);
    touched_fraction(grid, 200);
}
