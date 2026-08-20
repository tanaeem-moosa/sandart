//! EARLY-STOP.md: total ACTUAL per-block interior sweeps `update()` ran per frame, summed over a
//! run -- `DrawingSimulation::last_frame_block_steps`. Verifies the early-stop fix actually
//! reduces executed work (not just that the settling-time distribution looks the same), and is
//! the same quantity the web UI's "blk-steps" footer readout shows.
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(300);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);
    let mat = match get("--material").unwrap_or_else(|| "water".into()).as_str() {
        "drysand" => MaterialMode::DrySand,
        _ => MaterialMode::Water,
    };
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(mat);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = true;
    let targets = [None; 5];
    for _ in 0..warm {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let block_count = sim.active_blocks.len();
    let mut sum_steps: u64 = 0;
    let t0 = Instant::now();
    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        sum_steps += sim.last_frame_block_steps as u64;
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    println!(
        "{:?} grid={} ticks={} block_count={} avg_block_steps/frame={:.1} (x{:.3} of block_count) ms/frame={:.2}",
        mat, grid, ticks, block_count,
        sum_steps as f64 / ticks as f64,
        (sum_steps as f64 / ticks as f64) / block_count as f64,
        ms
    );
}
