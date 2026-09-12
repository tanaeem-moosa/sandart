//! Function-level profile of the Sand-fall water scene the LOD census measured
//! (artifacts/design/LOD-CENSUS-2026-09-12.md): grid 512, 3-neck hourglass, water, upper half
//! filled to 0.5, lateral_substeps 2.5, budget fixed at 128.
//!
//! Exists to explain a gap: a faithful standalone port of ONE lateral pass
//! (sandart-kernel-bench kernel R) costs ~105-115 ns/cell natively, while the census attributes
//! ~2.5-3x that per extra pass to the same work inside `settle_tick`.
//!
//!   cargo run -p sandart-sim --profile profiling --example profile_sandfall_water
//!
//! `--profile profiling` (LTO off, debug info) is required for stable symbol attribution; see
//! PERF-PROFILE.md.
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::collections::HashMap;

fn short(name: &str) -> String {
    // Strip the trailing `::h0123abcd` hash and generic noise so rows aggregate.
    let n = match name.rfind("::h") {
        Some(i) if name.len() - i == 19 => &name[..i],
        _ => name,
    };
    n.replace("sandart_sim::", "")
}

fn build(substeps: f32) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = SandboxShape::MultiNeckHourglass;
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = substeps;
    sim.budget_n = 128;
    sim.active_bounds.active = true;
    let w = sim.heightmap.width;
    for y in 0..w / 2 {
        for x in 0..w {
            let i = y * w + x;
            if sim.shape_mask[i] != 0 {
                sim.heightmap.data[i] = 0.5;
            }
        }
    }
    sim
}

/// `SUBSTEPS_SWEEP=1 cargo run -p sandart-sim --release --example profile_sandfall_water`
///
/// No profiler, shipped release profile. Times ticks 800..1800 at integer substep counts so the
/// marginal cost of ONE extra lateral pass per simulated cell can be compared directly with
/// sandart-kernel-bench's standalone kernels. Integer N only, so no stochastic roll.
fn sweep() {
    for &n in &[1.0f32, 2.0, 3.0] {
        let mut sim = build(n);
        let step = |sim: &mut DrawingSimulation| {
            let (r, m, s) = (sim.marble_radius, sim.material_mode, sim.sandbox_shape);
            sim.update(1.0 / 60.0, &[None; 5], r, m, s, 0.0, 0.0);
        };
        for _ in 0..800 {
            step(&mut sim);
        }
        let ticks = 1000;
        let mut block_ticks: u64 = 0;
        let t0 = std::time::Instant::now();
        for _ in 0..ticks {
            step(&mut sim);
            block_ticks += sim
                .active_blocks
                .iter()
                .filter(|b| **b != sandart_sim::BlockActivity::Inactive)
                .count() as u64;
        }
        let secs = t0.elapsed().as_secs_f64();
        let cells_per_tick = block_ticks as f64 * (sim.block_size * sim.block_size) as f64 / ticks as f64;
        println!(
            "N={n}: {:.3} ms/tick, {:.0} simulated cells/tick, {:.1} ns per simulated cell per tick",
            secs * 1000.0 / ticks as f64,
            cells_per_tick,
            secs * 1e9 / (cells_per_tick * ticks as f64)
        );
    }
}

fn main() {
    if std::env::var("SUBSTEPS_SWEEP").is_ok() {
        sweep();
        return;
    }
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = SandboxShape::MultiNeckHourglass;
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = 2.5;
    sim.budget_n = 128;
    sim.active_bounds.active = true;
    let w = sim.heightmap.width;
    for y in 0..w / 2 {
        for x in 0..w {
            let i = y * w + x;
            if sim.shape_mask[i] != 0 {
                sim.heightmap.data[i] = 0.5;
            }
        }
    }

    let step = |sim: &mut DrawingSimulation| {
        let (r, m, s) = (sim.marble_radius, sim.material_mode, sim.sandbox_shape);
        // 0.0 frame times keep the adaptive controller from touching budget_n.
        sim.update(1.0 / 60.0, &[None; 5], r, m, s, 0.0, 0.0);
    };

    for _ in 0..800 {
        step(&mut sim);
    }

    let ticks = 1000;
    let guard = pprof::ProfilerGuard::new(1000).unwrap();
    let t0 = std::time::Instant::now();
    for _ in 0..ticks {
        step(&mut sim);
    }
    let elapsed = t0.elapsed();
    let report = guard.report().build().unwrap();
    println!(
        "ticks 800..1800: {:.3} ms/tick (profiler running)",
        elapsed.as_secs_f64() * 1000.0 / ticks as f64
    );

    let mut self_counts: HashMap<String, isize> = HashMap::new();
    let mut incl_counts: HashMap<String, isize> = HashMap::new();
    let mut total: isize = 0;
    for (frames, count) in report.data.iter() {
        total += *count;
        // frames.frames[0] is the innermost frame; its first symbol the innermost inlined function.
        if let Some(sym) = frames.frames.first().and_then(|f| f.first()) {
            *self_counts.entry(short(&sym.name())).or_default() += *count;
        }
        let mut seen: Vec<String> = Vec::new();
        for f in &frames.frames {
            for sym in f {
                let n = short(&sym.name());
                if !seen.contains(&n) {
                    seen.push(n);
                }
            }
        }
        for n in seen {
            *incl_counts.entry(n).or_default() += *count;
        }
    }

    let print = |title: &str, m: &HashMap<String, isize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!("\n== {title} (of {total} samples)");
        for (name, c) in v.into_iter().take(30) {
            println!("{:6.2}%  {}", 100.0 * *c as f64 / total as f64, name);
        }
    };
    print("SELF (innermost function, inlining resolved)", &self_counts);
    print("INCLUSIVE (on stack)", &incl_counts);
}
