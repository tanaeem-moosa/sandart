//! Reproduce the reported failure of `coarse_delta_transport` (CREDIT-DEBT-TRANSPORT.md §2.3):
//! *"hourglass is not falling. It is attached to the right."*
//!
//! Prints, for the toggle off and on, the centre of mass and the vertical mass profile over time,
//! plus a coarse ASCII map. A rightward drift in `com_x` or a stalled `com_y` is the bug.

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

fn build(on: bool, rate: f32) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(256);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::DrySand);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    sim.coarse_delta_transport = on;
    sim.coarse_delta_transport_rate = rate;
    sim
}

fn com(sim: &DrawingSimulation) -> (f64, f64, f64) {
    let w = sim.heightmap.width;
    let (mut mx, mut my, mut m) = (0.0f64, 0.0f64, 0.0f64);
    for (i, &h) in sim.heightmap.data.iter().enumerate() {
        let h = h as f64;
        if h <= 0.0 {
            continue;
        }
        mx += h * (i % w) as f64;
        my += h * (i / w) as f64;
        m += h;
    }
    if m > 0.0 { (mx / m, my / m, m) } else { (0.0, 0.0, 0.0) }
}

/// 16x16 ASCII map of tile mass, so the shape is visible rather than inferred.
fn map(sim: &DrawingSimulation) -> String {
    let w = sim.heightmap.width;
    let n = 16;
    let t = w / n;
    let mut out = String::new();
    let mut cells = vec![0.0f64; n * n];
    for (i, &h) in sim.heightmap.data.iter().enumerate() {
        if h <= 0.0 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        cells[(y / t).min(n - 1) * n + (x / t).min(n - 1)] += h as f64;
    }
    let peak = cells.iter().cloned().fold(0.0f64, f64::max).max(1.0);
    for y in 0..n {
        for x in 0..n {
            let v = cells[y * n + x] / peak;
            out.push(match v {
                v if v <= 0.001 => '.',
                v if v < 0.15 => ':',
                v if v < 0.40 => '+',
                v if v < 0.70 => '*',
                _ => '#',
            });
        }
        out.push('\n');
    }
    out
}

/// Mean |discrete Laplacian| over interior cells with material. Maximal for a checkerboard, which
/// is the highest-frequency mode a 5-point stencil supports -- so this is the direct measure of the
/// artifact visible on screen.
fn checker(sim: &DrawingSimulation) -> f64 {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let (mut acc, mut n) = (0.0f64, 0usize);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if sim.heightmap.data[i] <= 0.01 {
                continue;
            }
            let c = sim.heightmap.data[i] as f64;
            let nb = (sim.heightmap.data[i - 1] as f64
                + sim.heightmap.data[i + 1] as f64
                + sim.heightmap.data[i - w] as f64
                + sim.heightmap.data[i + w] as f64)
                / 4.0;
            acc += (c - nb).abs();
            n += 1;
        }
    }
    if n > 0 { acc / n as f64 } else { 0.0 }
}

fn run(label: &str, on: bool, rate: f32, ticks: usize) {
    let mut sim = build(on, rate);
    let targets = [None; 5];
    println!("\n=== {label} ===");
    let (x0, y0, m0) = com(&sim);
    println!("t=  0  com=({x0:7.2},{y0:7.2})  mass={m0:12.1}");
    for tick in 1..=ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
        if tick % 40 == 0 || tick == 10 {
            let (x, y, m) = com(&sim);
            let st = &sim.last_frame_delta_transport;
            println!(
                "t={tick:3}  com=({x:7.2},{y:7.2})  mass={m:12.1}  \
                 dx={:+6.2} dy={:+6.2}  faces={}/{} applied={:.1}/{:.1} capped={} blocked={}",
                x - x0, y - y0,
                st.faces_moved, st.faces_considered, st.applied, st.requested,
                st.limited, st.blocked
            );
        }
    }
    println!("{}", map(&sim));
}

fn sweep(ticks: usize) {
    println!("\n=== RATE SWEEP: checkerboard amplitude (mean |laplacian|) ===");
    println!("PREDICTION: half-the-difference is the 1D-stable relaxation. In 2D a tile exchanges");
    println!("across 4 faces independently, so 1/2 per face is 2x the 2D stability limit of 1/4.");
    println!("If that is the mechanism, the artifact should switch on at rate ~0.5.\n");
    println!("{:>6}  {:>12}  {:>12}  {:>10}", "rate", "checker", "vs baseline", "descent");
    let base = {
        let mut sim = build(false, 0.0);
        let targets = [None; 5];
        let y0 = com(&sim).1;
        for _ in 0..ticks {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
        }
        (checker(&sim), com(&sim).1 - y0)
    };
    println!("{:>6}  {:>12.5}  {:>12}  {:>10.2}", "off", base.0, "1.00x", base.1);
    for &r in &[0.1f32, 0.25, 0.35, 0.45, 0.5, 0.55, 0.7, 1.0] {
        let mut sim = build(true, r);
        let targets = [None; 5];
        let y0 = com(&sim).1;
        for _ in 0..ticks {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
        }
        let c = checker(&sim);
        println!("{:>6.2}  {:>12.5}  {:>11.2}x  {:>10.2}", r, c, c / base.0.max(1e-9), com(&sim).1 - y0);
    }
}

fn main() {
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    if std::env::args().any(|a| a == "--sweep") {
        sweep(ticks);
        return;
    }
    run("OFF (baseline)", false, 0.0, ticks);
    run("ON rate=0.7", true, 0.7, ticks);
}
