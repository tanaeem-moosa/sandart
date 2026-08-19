//! STEP4-COARSE-IS-A-SIM.md's direct test: does a truly resting column produce `delta_eta = 0`
//! on its coarse vertical inter-tile seam edges? This is the acceptance test for "the fine edge
//! receives elevation exactly once" -- if elevation were still being stated twice (once by the
//! fine solver's own intrinsic `base_head`, once by a spurious coarse elevation term), a resting
//! column would show a large, systematic, nonzero `delta_eta` on every seam it has.
//!
//! Usage: `diag_coarse_resting_delta_eta [grid] [ticks]` (default 512, 6000).

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(6000);

    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.generate_shape_mask();

    // Prefill the whole interior with water so the column is deep and every coarse row down the
    // column has real material -- a deep resting column is the scenario the over-drive report
    // describes ("two similar-fill tiles").
    for idx in 0..grid * grid {
        if sim.shape_mask[idx] != 0 {
            sim.heightmap.data[idx] = 1.0;
        }
    }

    let targets = [None; 5];
    let report_every = (ticks / 8).max(1);
    for tick in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
        if (tick + 1) % report_every == 0 {
            let total_disp: f32 = sim.last_displacements.iter().sum();
            let max_disp = sim.last_displacements.iter().cloned().fold(0.0f32, f32::max);
            eprintln!("  tick {}: sum(last_displacements)={total_disp:.6} max={max_disp:.6}", tick + 1);
        }
    }

    // Confirm it is actually at rest before trusting the measurement.
    let total_disp: f32 = sim.last_displacements.iter().sum();
    let max_disp = sim.last_displacements.iter().cloned().fold(0.0f32, f32::max);
    println!(
        "after {ticks} ticks: sum(last_displacements)={total_disp:.6} max={max_disp:.6} \
         (near zero means settled)"
    );

    let n = sim.coarse.coarse_n;
    let eta = &sim.coarse_state.eta;
    let mut deltas: Vec<f32> = Vec::new();
    for cy in 0..n.saturating_sub(1) {
        for cx in 0..n {
            if !sim.coarse.k_y_at(cx, cy).is_finite() || sim.coarse.k_y_at(cx, cy) <= 0.0 {
                continue;
            }
            let a = cy * n + cx;
            let b = (cy + 1) * n + cx;
            if !sim.coarse.inside[a] || !sim.coarse.inside[b] {
                continue;
            }
            // Only seams where both tiles actually hold material -- an empty tile's eta is not a
            // meaningful "resting column" reading.
            if sim.coarse_state.a_mass[a] <= 1e-4 || sim.coarse_state.a_mass[b] <= 1e-4 {
                continue;
            }
            deltas.push(eta[a] - eta[b]);
        }
    }

    if deltas.is_empty() {
        println!("no wet vertical inter-tile seams found -- scenario did not settle as expected");
        return;
    }
    deltas.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let n_d = deltas.len();
    let mean: f64 = deltas.iter().map(|&v| v as f64).sum::<f64>() / n_d as f64;
    let pct = |p: f64| -> f32 {
        let idx = ((p * (n_d - 1) as f64).round() as usize).min(n_d - 1);
        deltas[idx]
    };
    println!("grid={grid} coarse_n={n} wet vertical inter-tile seam edges: {n_d}");
    println!(
        "delta_eta distribution: min={:.5} p10={:.5} p25={:.5} p50={:.5} p75={:.5} p90={:.5} max={:.5} mean={:.5}",
        deltas[0], pct(0.10), pct(0.25), pct(0.50), pct(0.75), pct(0.90), deltas[n_d - 1], mean
    );
    let abs_mean: f64 = deltas.iter().map(|&v| v.abs() as f64).sum::<f64>() / n_d as f64;
    println!("mean |delta_eta| = {abs_mean:.5}");
}
