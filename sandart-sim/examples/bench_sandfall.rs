//! Sand-fall per-frame benchmark.
//!
//! `profile_sim.rs` measures the *Sandbox* configuration (Circle shape, marbles drawing a
//! Gosper curve, MoonDust, gravity zero). That is not where the app spends its time in
//! Sand-fall mode, and it is not what the flux solver work is trying to make faster.
//!
//! This example measures the thing that actually matters for Sand-fall: a 512x512 hourglass
//! draining under in-plane gravity, driven through the real `DrawingSimulation::update()`
//! entry point (so `sync_cell_colors_u8`, the quantile gate, and the budget bookkeeping are
//! all in the measurement, not just `settle_tick`).
//!
//! Run:
//!   cargo run --release --example bench_sandfall
//!   cargo run --release --example bench_sandfall -- --ticks 900 --materials water,drysand
//!
//! Flags:
//!   --ticks N          ticks per scenario (default 900)
//!   --warmup N         untimed ticks before the clock starts (default 20)
//!   --budgets a,b,c    budget_n values to sweep (default 1024,256,32).
//!                      1024 = full (32x32 blocks at block_size 16); 256 = the app's start
//!                      value; 32 = BUDGET_MIN, i.e. fully throttled.
//!   --materials a,b    water,drysand,calmwater,... (default water,drysand)
//!   --shape s          hourglass | multistage (default hourglass)
//!   --phases           also report a per-third breakdown (fill / stream / pool)
//!   --descent          report centre-of-mass descent (for the iterations experiment)
//!   --sync             report the isolated cost of the f32 -> u8 colour conversion

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, GRID_SIZE};
use std::time::Instant;

fn parse_material(s: &str) -> MaterialMode {
    match s.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
        "drysand" | "sand" => MaterialMode::DrySand,
        "coarsesand" => MaterialMode::CoarseSand,
        "kineticsand" => MaterialMode::KineticSand,
        "wetsand" => MaterialMode::WetSand,
        "finepowder" => MaterialMode::FinePowder,
        "snow" => MaterialMode::Snow,
        "moondust" => MaterialMode::MoonDust,
        "oobleck" => MaterialMode::Oobleck,
        "buttercream" => MaterialMode::ButterCream,
        "water" => MaterialMode::Water,
        "calmwater" => MaterialMode::CalmWater,
        "milk" => MaterialMode::Milk,
        "vegetableoil" | "oil" => MaterialMode::VegetableOil,
        "yogurt" => MaterialMode::Yogurt,
        other => panic!("unknown material: {other}"),
    }
}

fn make_sim(material: MaterialMode, shape: SandboxShape) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = shape;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(material);
    sim.initialize_hourglass();
    sim
}

fn mass(sim: &DrawingSimulation) -> f64 {
    sim.heightmap.data.iter().map(|&v| v as f64).sum()
}

/// Mass-weighted mean row index, normalised to 0.0 (top) .. 1.0 (bottom). The descent rate of
/// this number is "how fast the material is falling", independent of how much of it there is.
fn centre_of_mass_row(sim: &DrawingSimulation) -> f64 {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for y in 0..h {
        let row: f64 = sim.heightmap.data[y * w..(y + 1) * w]
            .iter()
            .map(|&v| v as f64)
            .sum();
        num += row * y as f64;
        den += row;
    }
    if den > 0.0 {
        num / den / (h - 1) as f64
    } else {
        f64::NAN
    }
}

struct Run {
    ms_per_tick: f64,
    thirds_ms: [f64; 3],
    mass_rel_err: f64,
    com_start: f64,
    com_end: f64,
    total_flow_proxy: f64,
    /// Mean per-tick count of blocks in each `BlockActivity` class. `Fast` = MUST-simulate
    /// (displacement over the active threshold, which bypasses `budget_n` entirely),
    /// `Medium` = admitted by the budget, `Slow` = forced in by staleness.
    avg_fast: f64,
    avg_medium: f64,
    avg_slow: f64,
    n_blocks: usize,
}

fn run_scenario(
    material: MaterialMode,
    shape: SandboxShape,
    budget: usize,
    ticks: usize,
    warmup: usize,
) -> Run {
    let mut sim = make_sim(material, shape);
    let targets = [None; 5];
    // last_frame_time_ms = 0.0 disables the adaptive budget block in `update()`, pinning
    // budget_n at the value under test for the whole run.
    let step = |sim: &mut DrawingSimulation| {
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, material, shape, 0.0, 16.0);
    };

    for _ in 0..warmup {
        step(&mut sim);
    }

    let mass0 = mass(&sim);
    let com_start = centre_of_mass_row(&sim);

    let third = ticks / 3;
    let mut thirds_ms = [0.0f64; 3];
    let (mut acc_fast, mut acc_medium, mut acc_slow) = (0u64, 0u64, 0u64);
    let start = Instant::now();
    let mut mark = start;
    for i in 0..ticks {
        step(&mut sim);
        // ~1024 enum compares against a ~10 ms tick: below the timer's noise floor, so this
        // stays inside the timed region rather than needing a second untimed pass.
        for a in &sim.active_blocks {
            match a {
                sandart_sim::BlockActivity::Fast => acc_fast += 1,
                sandart_sim::BlockActivity::Medium => acc_medium += 1,
                sandart_sim::BlockActivity::Slow => acc_slow += 1,
                sandart_sim::BlockActivity::Inactive => {}
            }
        }
        if third > 0 && (i + 1) % third == 0 {
            let seg = ((i + 1) / third).min(3) - 1;
            let now = Instant::now();
            thirds_ms[seg] = now.duration_since(mark).as_secs_f64() * 1000.0 / third as f64;
            mark = now;
        }
    }
    let elapsed = start.elapsed();

    let mass1 = mass(&sim);
    Run {
        ms_per_tick: elapsed.as_secs_f64() * 1000.0 / ticks as f64,
        thirds_ms,
        mass_rel_err: (mass1 - mass0).abs() / mass0.max(1e-12),
        com_start,
        com_end: centre_of_mass_row(&sim),
        total_flow_proxy: mass1,
        avg_fast: acc_fast as f64 / ticks as f64,
        avg_medium: acc_medium as f64 / ticks as f64,
        avg_slow: acc_slow as f64 / ticks as f64,
        n_blocks: sim.active_blocks.len(),
    }
}

fn measure_sync_cost(sim: &DrawingSimulation) -> f64 {
    // Standalone replica of `sync_cell_colors_u8` (private), to price the full-buffer
    // f32 -> u8 conversion that `update()` performs once per frame.
    let mut dst = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
    let reps = 200;
    let start = Instant::now();
    for _ in 0..reps {
        for (d, &s) in dst.iter_mut().zip(sim.cell_colors_f32.iter()) {
            *d = s.clamp(0.0, 255.0).round() as u8;
        }
        std::hint::black_box(&dst);
    }
    start.elapsed().as_secs_f64() * 1000.0 / reps as f64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let has = |name: &str| args.iter().any(|a| a == name);

    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(900);
    let warmup: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(20);
    let budgets: Vec<usize> = get("--budgets")
        .unwrap_or_else(|| "1024,256,32".to_string())
        .split(',')
        .map(|v| v.trim().parse().unwrap())
        .collect();
    let materials: Vec<MaterialMode> = get("--materials")
        .unwrap_or_else(|| "water,drysand".to_string())
        .split(',')
        .map(|v| parse_material(v.trim()))
        .collect();
    let shape = match get("--shape").unwrap_or_else(|| "hourglass".to_string()).as_str() {
        "multistage" | "multistagehourglass" => SandboxShape::MultiStageHourglass,
        _ => SandboxShape::Hourglass,
    };

    println!(
        "Sand-fall benchmark — {}x{} grid, shape {:?}, gravity (0.0, 0.04), {} ticks/scenario ({} warmup)",
        GRID_SIZE, GRID_SIZE, shape, ticks, warmup
    );
    println!(
        "budget_n: 1024 = full (32x32 blocks), 256 = app start value, 32 = BUDGET_MIN (throttled)\n"
    );

    println!(
        "{:<12} {:>7} {:>11} {:>11} {:>13} {:>8} {:>8} {:>8}",
        "material", "budget", "ms/tick", "ticks/s", "mass_rel_err", "must", "budgeted", "stale"
    );
    println!("{}", "-".repeat(84));

    // Steam Deck frequency/thermal scaling makes a single pass swing by tens of percent, so
    // every scenario is run `--reps` times (interleaved, outermost) and the *minimum* ms/tick
    // is reported: the fastest observed pass is the one least polluted by interference.
    let reps: usize = get("--reps").map(|v| v.parse().unwrap()).unwrap_or(3);
    let mut best: std::collections::HashMap<(usize, usize), f64> = std::collections::HashMap::new();
    for _ in 0..reps.saturating_sub(1) {
        for (mi, &m) in materials.iter().enumerate() {
            for (bi, &b) in budgets.iter().enumerate() {
                let r = run_scenario(m, shape, b, ticks, warmup);
                let e = best.entry((mi, bi)).or_insert(f64::MAX);
                *e = e.min(r.ms_per_tick);
            }
        }
    }

    let mut results = Vec::new();
    for (mi, &m) in materials.iter().enumerate() {
        for (bi, &b) in budgets.iter().enumerate() {
            let mut r = run_scenario(m, shape, b, ticks, warmup);
            let e = best.entry((mi, bi)).or_insert(f64::MAX);
            *e = e.min(r.ms_per_tick);
            r.ms_per_tick = *e;
            println!(
                "{:<12} {:>7} {:>11.4} {:>11.1} {:>13.2e} {:>8.1} {:>8.1} {:>8.1}",
                format!("{:?}", m),
                b,
                r.ms_per_tick,
                1000.0 / r.ms_per_tick,
                r.mass_rel_err,
                r.avg_fast,
                r.avg_medium,
                r.avg_slow
            );
            results.push((m, b, r));
        }
    }
    if let Some((_, _, r)) = results.first() {
        println!("(mean blocks simulated per tick, out of {} total)", r.n_blocks);
    }

    if has("--phases") {
        println!("\nPer-third breakdown (ms/tick): 1 = upper chamber full, 2 = streaming, 3 = pooling");
        println!("{:<12} {:>7} {:>10} {:>10} {:>10}", "material", "budget", "third 1", "third 2", "third 3");
        println!("{}", "-".repeat(54));
        for (m, b, r) in &results {
            println!(
                "{:<12} {:>7} {:>10.4} {:>10.4} {:>10.4}",
                format!("{:?}", m),
                b,
                r.thirds_ms[0],
                r.thirds_ms[1],
                r.thirds_ms[2]
            );
        }
    }

    if has("--descent") {
        println!("\nCentre-of-mass descent (0.0 = top row, 1.0 = bottom row)");
        println!(
            "{:<12} {:>7} {:>10} {:>10} {:>10} {:>12}",
            "material", "budget", "com_start", "com_end", "delta", "final_mass"
        );
        println!("{}", "-".repeat(66));
        for (m, b, r) in &results {
            println!(
                "{:<12} {:>7} {:>10.5} {:>10.5} {:>10.5} {:>12.2}",
                format!("{:?}", m),
                b,
                r.com_start,
                r.com_end,
                r.com_end - r.com_start,
                r.total_flow_proxy
            );
        }
    }

    if let Some(out) = get("--flamegraph") {
        let m = materials[0];
        let mut sim = make_sim(m, shape);
        let targets = [None; 5];
        for _ in 0..warmup {
            sim.budget_n = budgets[0];
            sim.update(0.016, &targets, 0.08, m, shape, 0.0, 16.0);
        }
        let guard = pprof::ProfilerGuard::new(997).unwrap();
        for _ in 0..ticks {
            sim.budget_n = budgets[0];
            sim.update(0.016, &targets, 0.08, m, shape, 0.0, 16.0);
        }
        if let Ok(report) = guard.report().build() {
            let file = std::fs::File::create(&out).unwrap();
            report.flamegraph(file).unwrap();
            println!("\nwrote {out}");
        }
    }

    if has("--sync") {
        let sim = make_sim(MaterialMode::DrySand, shape);
        println!(
            "\nsync_cell_colors_u8 equivalent (full {} element f32 -> u8): {:.4} ms/frame",
            GRID_SIZE * GRID_SIZE * 4,
            measure_sync_cost(&sim)
        );
    }
}
