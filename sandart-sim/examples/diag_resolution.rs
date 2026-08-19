//! Is transport resolution-invariant? Same physical scenario at 128/256/512; every metric is
//! normalised to a FRACTION OF THE DOMAIN, so correct physics gives the same number per tick at
//! every resolution. A number that falls as 1/w means transport is limited in cells/tick.
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

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

/// Surface roughness of a settling pool: (max - min) column height over the columns that hold
/// material, normalised by the mean, so it is dimensionless and resolution-free.
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

fn make(grid: usize, shape: SandboxShape) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = shape;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim
}

/// Drop a narrow tall column of water into an empty square box and watch it level out.
fn make_pile(grid: usize) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    let (w, h) = (sim.heightmap.width, sim.heightmap.height);
    for i in sim.heightmap.data.iter_mut() { *i = 0.0; }
    // a column occupying the middle 1/8 of the width, bottom 3/4 of the height
    for y in (h / 4)..h {
        for x in (w * 7 / 16)..(w * 9 / 16) {
            if sim.shape_mask[y * w + x] != 0 { sim.heightmap.data[y * w + x] = 1.0; }
        }
    }
    sim
}

fn main() {
    let ticks: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(400);
    let targets = [None; 5];
    println!("Every metric is a FRACTION OF THE DOMAIN per tick: resolution-invariant physics gives");
    println!("the same value at every grid size. A value falling as 1/w means cells/tick limited.\n");

    println!("{:<8} {:>14} {:>16} {:>14}", "grid", "hourglass", "descent/tick", "vs 128");
    let mut base = 0.0;
    for (i, &g) in [128usize, 256, 512].iter().enumerate() {
        let mut sim = make(g, SandboxShape::Hourglass);
        for _ in 0..40 { sim.budget_n = 1024; sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6); }
        let c0 = com_row(&sim);
        for _ in 0..ticks { sim.budget_n = 1024; sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6); }
        let d = (com_row(&sim) - c0) / ticks as f64;
        if i == 0 { base = d; }
        println!("{:<8} {:>14} {:>16.3e} {:>13.2}x", g, "drain", d, d / base);
    }

    println!("\n{:<8} {:>10} {:>12} {:>12} {:>14} {:>12}", "grid", "pile s0", "after", "frac left", "ticks to 50%", "norm ticks");
    for &g in [128usize, 256, 512].iter() {
        let mut sim = make_pile(g);
        let s0 = surface_spread(&sim);
        let mut half = None;
        for t in 0..(ticks * 4) {
            sim.budget_n = 1024;
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
            if half.is_none() && surface_spread(&sim) < 0.5 * s0 { half = Some(t + 1); }
        }
        let s1 = surface_spread(&sim);
        println!("{:<8} {:>10.3} {:>12.3} {:>12.3} {:>14} {:>12.2}", g, s0, s1, s1 / s0.max(1e-12),
            half.map(|v| v.to_string()).unwrap_or_else(|| format!(">{}", ticks * 4)),
            half.map(|v| v as f64 / (g as f64 / 128.0)).unwrap_or(f64::NAN));
    }
}
