//! LATERAL-COARSE-CORRECTION.md §6: WHICH constraint actually bounds a lateral edge?
//!
//! The coarse conveyance boost raises `c_sq`. That can only change the outcome on an edge where
//! conveyance is the binding term, and the boost measured as no help at all on Water and as
//! saturating almost immediately on DrySand. The standing explanation is that for a body of water
//! a full sideways neighbour leaves no headroom inside the pile, while at the flank the driving
//! head saturates the +/-1 one-cell-per-tick clamp -- so `c_sq` is aimed at the one term that is
//! never the limit. This counts it instead of arguing it.
//!
//! Reads, per material:
//!   YIELD      below the angle-of-repose threshold -- nothing was going to move
//!   CLAMP      the +/-1.0 one-cell-per-tick limit
//!   ACCEPTOR   the receiving cell had no room
//!   DONOR      the donating cell did not hold enough
//!   CONVEYANCE `c_sq * driving` itself was smallest -- THE ONLY BIN A BOOST CAN MOVE
use glam::Vec2;
use sandart_sim::{physics, DrawingSimulation, MaterialMode, SandboxShape};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(200);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);

    println!(
        "grid={grid} ticks={ticks} warmup={warm}\n\
         Share of LATERAL edge evaluations by which term decided the flux.\n\
         Only CONVEYANCE-bound edges can be helped by raising c_sq.\n"
    );
    for mat in [MaterialMode::Water, MaterialMode::DrySand] {
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(mat);
        sim.initialize_hourglass();
        sim.overfill_pressure = true;
        let targets = [None; 5];
        for _ in 0..warm {
            sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        }
        physics::bind_census_enable(true);
        for _ in 0..ticks {
            sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        }
        let c = physics::bind_census_take();
        physics::bind_census_enable(false);
        let total = c[5].max(1.0);
        let names = ["YIELD", "CLAMP", "ACCEPTOR", "DONOR", "CONVEYANCE"];
        println!("=== {mat:?} === {:.0} lateral edge evaluations/tick", total / ticks as f64);
        for (i, n) in names.iter().enumerate() {
            println!("  {n:<11} {:>6.2}%  ({:>10.0} edges)", c[i] / total * 100.0, c[i]);
        }
        // By mass as well as by count: a bin can be rare and still carry most of the transport.
        let flux_share = if c[7] > 0.0 { c[6] / c[7] * 100.0 } else { 0.0 };
        println!(
            "  -> conveyance-bound edges carry {flux_share:.2}% of realised lateral flux\n"
        );
    }
}
