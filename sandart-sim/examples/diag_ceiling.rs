//! Does a settled pool at 512 actually pin its deep cells against the overfill ceiling?
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use sandart_sim::physics::{cell_capacity_for, overfill_ceiling_for};

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1500);
    let stiff: f32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(5.0);

    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    sim.overfill_stiffness = stiff;
    // ceiling as an INDEPENDENT dial, not tied to 1/stiffness
    let ceil_override: f32 = std::env::args().nth(4).and_then(|s| s.parse().ok())
        .unwrap_or_else(|| overfill_ceiling_for(stiff));
    sim.overfill_capacity = ceil_override;
    let (w, h) = (grid, grid);
    // fill the bottom 60% of the box with water
    for v in sim.heightmap.data.iter_mut() { *v = 0.0; }
    for y in (h * 2 / 5)..h {
        for x in 0..w {
            if sim.shape_mask[y * w + x] != 0 { sim.heightmap.data[y * w + x] = 1.0; }
        }
    }
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.budget_n = 1024;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
    }
    let cap = cell_capacity_for(sim.cell_props[(h / 2 * w + w / 2) * 4]);
    let o_max = (sim.overfill_capacity - 1.0).max(0.0);
    println!("grid {grid}, stiffness {stiff}, cap {cap:.2}, o_max {o_max:.3}, ceiling {:.3}, after {ticks} ticks\n",
             cap * (1.0 + o_max));
    println!("{:>8} {:>10} {:>10} {:>12}", "row", "mean fill", "overfill o", "o / o_max");
    let mut pinned = 0u64;
    let mut wet = 0u64;
    for y in 0..h {
        let mut s = 0.0f64;
        let mut n = 0u64;
        for x in 0..w {
            let i = y * w + x;
            if sim.shape_mask[i] != 0 && sim.heightmap.data[i] > 1e-4 {
                s += sim.heightmap.data[i] as f64;
                n += 1;
                wet += 1;
                if (sim.heightmap.data[i] / cap - 1.0) > 0.98 * o_max { pinned += 1; }
            }
        }
        if n > 0 && (y % (h / 16) == 0 || y == h - 1) {
            let mean = s / n as f64;
            let o = mean / cap as f64 - 1.0;
            println!("{:>8} {:>10.4} {:>10.4} {:>11.1}%", y, mean, o.max(0.0), 100.0 * o.max(0.0) / o_max as f64);
        }
    }
    println!("\ncells pinned at >98% of the ceiling: {pinned} of {wet} wet ({:.1}%)",
             100.0 * pinned as f64 / wet.max(1) as f64);
    // how flat is the free surface? (the thing that "takes forever to settle")
    let mut top = vec![0usize; w];
    for x in 0..w {
        for y in 0..h {
            if sim.shape_mask[y * w + x] != 0 && sim.heightmap.data[y * w + x] > 0.5 { top[x] = y; break; }
        }
    }
    let mx = *top.iter().max().unwrap();
    let mn = *top.iter().min().unwrap();
    println!("free-surface row: min {mn} max {mx} -> spread {} rows", mx - mn);
    let total: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    println!("total mass {total:.1}");
}
