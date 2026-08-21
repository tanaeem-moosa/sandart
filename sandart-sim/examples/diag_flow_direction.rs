//! Which direction does material actually MOVE, fine versus coarse, and where do the two disagree?
//!
//! Counts realised mass transfer at the point it happens (`physics::flux_edge_apply`, past its
//! MIN_FLUX cutoff), split lateral/downward and by whether the edge crosses an LOD block boundary.
//! At the shipped geometry a block IS a coarse tile, so the fine level's cross-block flow and the
//! coarse level's every edge describe the same boundaries and are directly comparable.
//!
//! Units: the coarse level holds a tile's height as an AVERAGE, not a sum (see `NestedSim`), so
//! one unit of coarse flux corresponds to `t*t` units of fine mass. Both the raw and the scaled
//! coarse numbers are printed. The lateral/down RATIO within a level needs no scaling, and it is
//! the ratio the question is about.
use glam::Vec2;
use sandart_sim::{physics, DrawingSimulation, MaterialMode, SandboxShape};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(300);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);
    let mat = match get("--material").unwrap_or_else(|| "drysand".into()).as_str() {
        "water" => MaterialMode::Water,
        _ => MaterialMode::DrySand,
    };
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(mat);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = get("--overclock").map(|v| v == "1").unwrap_or(true);
    let targets = [None; 5];
    for _ in 0..warm {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }

    physics::flux_dir_enable(true);
    let mut acc = [0.0f64; 8];
    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        let v = physics::flux_dir_take();
        for i in 0..8 { acc[i] += v[i]; }
    }
    physics::flux_dir_enable(false);

    let n = ticks as f64;
    let t = (grid / sim.coarse.coarse_n).max(1) as f64;
    let scale = t * t;
    let (fh, fv, fhb, fvb) = (acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n);
    let (ch, cv) = (acc[4] / n, acc[5] / n);
    println!("{:?} grid={} ticks={} overclock={}", mat, grid, ticks, sim.overclocking_enabled);
    println!("  FINE, all edges          lateral {:>12.2}  down {:>12.2}  lateral/down {:.3}", fh, fv, fh / fv.max(1e-12));
    println!("  FINE, across block edges lateral {:>12.2}  down {:>12.2}  lateral/down {:.3}", fhb, fvb, fhb / fvb.max(1e-12));
    println!("  COARSE (raw)             lateral {:>12.2}  down {:>12.2}  lateral/down {:.3}", ch, cv, ch / cv.max(1e-12));
    println!("  COARSE (x{:.0}, fine mass units) lateral {:>12.2}  down {:>12.2}", scale, ch * scale, cv * scale);
    println!(
        "  DISAGREEMENT (coarse scaled - fine across blocks): lateral {:>+12.2}  down {:>+12.2}",
        ch * scale - fhb,
        cv * scale - fvb
    );
}
