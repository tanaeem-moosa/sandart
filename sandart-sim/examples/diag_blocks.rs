//! Cost/behaviour as a function of the MUST bar (patched per build) and budget.
use glam::Vec2;
use sandart_sim::{BlockActivity, DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

fn com(sim: &DrawingSimulation) -> f64 {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for y in 0..h {
        let r: f64 = sim.heightmap.data[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum();
        num += r * y as f64;
        den += r;
    }
    num / den / (h - 1) as f64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(300);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let budget: usize = get("--budget").map(|v| v.parse().unwrap()).unwrap_or(256);
    let sub: usize = get("--substeps").map(|v| v.parse().unwrap()).unwrap_or(1);
    let mat = match get("--material").unwrap_or_else(|| "water".into()).as_str() {
        "drysand" => MaterialMode::DrySand,
        _ => MaterialMode::Water,
    };
    let overfill = get("--overfill").map(|v| v == "1").unwrap_or(true);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);
    // OVERCLOCKING.md: `--overclock 1` turns on the multi-rate block scheduler.
    let overclock = get("--overclock").map(|v| v == "1").unwrap_or(false);
    // BLOCK-SIZE-SWEEP.md: `--blockdiv 64` (default) is the shipped geometry, block == coarse
    // tile. Smaller divisor, bigger blocks.
    let blockdiv: usize = get("--blockdiv").map(|v| v.parse().unwrap()).unwrap_or(64);
    let mut sim = DrawingSimulation::new_with_block_divisor(grid, blockdiv);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(mat);
    sim.initialize_hourglass();
    sim.overfill_pressure = overfill;
    sim.overclocking_enabled = overclock;
    // EARLY-STOP.md: the clock-rate range, the same runtime knob the UI's "Max clock rate" slider
    // drives. `--maxrate` bounds repetitions per frame; `--minrate 1` disables underclocking
    // without touching overclocking, which is the control run for "what is underclocking buying".
    if let Some(v) = get("--maxrate") { sim.max_clock_rate = v.parse().unwrap(); }
    if let Some(v) = get("--minrate") { sim.min_clock_rate = v.parse().unwrap(); }
    // `--rank 0` selects the absolute signal/CLOCK_DELTA_REF_FRAC rule instead of the default
    // rank-ladder allocation -- the A/B for whether allocating by position beats allocating by
    // threshold.
    if let Some(v) = get("--rank") { sim.rank_clock_rates = v == "1"; }
    if let Some(v) = get("--gate") { sim.rate_gated_reps = v == "1"; }
    if let Some(t) = get("--tension") { sim.underfill_tension = t.parse().unwrap(); }
    let targets = [None; 5];
    for _ in 0..warm {
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let m0: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let c0 = com(&sim);
    let (mut f, mut m, mut s) = (0u64, 0u64, 0u64);
    let t0 = Instant::now();
    for _ in 0..ticks {
        for _ in 0..sub {
            sim.budget_n = budget;
            sim.update(0.016 / sub as f32, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        }
        for a in &sim.active_blocks {
            match a {
                BlockActivity::Fast => f += 1,
                BlockActivity::Medium => m += 1,
                BlockActivity::Slow => s += 1,
                _ => {}
            }
        }
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    let m1: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let n = ticks as f64;
    println!(
        "grid={} block_size={} {:?} sub={} budget={:<4} overclock={} rank={} rate[{:.2},{:.2}] ms/frame {:>7.2}  must {:>6.1} budgeted {:>6.1} stale {:>5.1} run {:>6.1}  \
         com {:.5}->{:.5} (desc {:.5})  mass_err {:.2e}",
        grid, sim.block_size, mat, sub, budget, overclock, sim.rank_clock_rates,
        sim.min_clock_rate, sim.max_clock_rate, ms,
        f as f64 / n, m as f64 / n, s as f64 / n, (f + m + s) as f64 / n,
        c0, com(&sim), com(&sim) - c0, (m1 - m0).abs() / m0.max(1e-12)
    );
    if overclock {
        // OVERCLOCKING.md: the rate distribution -- how many blocks fall in each octave BUCKET
        // right now, and the mean -- is what says whether this is affordable. Rates themselves
        // are continuous (EARLY-STOP.md); the buckets are this readout's binning, not the
        // scheduler's, so a run whose rates are all powers of two and one whose rates are
        // arbitrary print the same shape here. `mean_rate` is over the raw values.
        let mut counts = [0u64; 7]; // idx 0 = 1/8 .. idx 6 = 8
        let mut sum_rate = 0.0f64;
        for &r in &sim.block_clock_rate {
            let idx = (r.max(1e-6).log2().round() as i32).clamp(-3, 3) + 3;
            counts[idx as usize] += 1;
            sum_rate += r as f64;
        }
        let total = sim.block_clock_rate.len().max(1) as f64;
        println!(
            "  rate distribution (blocks): 1/8={} 1/4={} 1/2={} 1x={} 2x={} 4x={} 8x={}  mean_rate={:.3}",
            counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6],
            sum_rate / total
        );
    }
}
