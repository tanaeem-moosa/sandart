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

/// Step 0 of "option 4" (SESSION-HANDOVER-2026-09-13.md §6): native per-section breakdown of one
/// tick, using `sandart_sim::phase_timing`. Requires `--features phase-timing` to be meaningful
/// (without it every section reports 0 -- the module is always present, but every timer is a
/// no-op; see that module's doc comment).
///
///   cargo run -p sandart-sim --release --features phase-timing --example profile_sandfall_water \
///     -- phase-breakdown
///
/// Same scene/warmup/window convention as `sweep()`: `build(2.5)`, 800 warmup ticks, then a
/// window of `ticks` measured ticks with `phase_timing` reset once before the window and read
/// once after, so section totals divide down to ms/tick directly.
fn phase_breakdown() {
    use sandart_sim::phase_timing;

    let mut sim = build(2.5);
    let step = |sim: &mut DrawingSimulation| {
        let (r, m, s) = (sim.marble_radius, sim.material_mode, sim.sandbox_shape);
        sim.update(1.0 / 60.0, &[None; 5], r, m, s, 0.0, 0.0);
    };
    // Coordinator note (mid-task): the deployed page's footer showed ~800 (937 at one moment)
    // simulated blocks/tick at this same scene/N, vs. this example's steady-state ~600 (see
    // below) -- WARMUP_TICKS lets Step-0's reporting move the measurement window to wherever the
    // block count is closer to the page's, without changing the scene itself.
    let warmup: u32 = std::env::var("WARMUP_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(800);
    for _ in 0..warmup {
        step(&mut sim);
    }

    let ticks: u32 = std::env::var("PHASE_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let mut block_ticks: u64 = 0;
    phase_timing::reset_tick();
    let t0 = std::time::Instant::now();
    for _ in 0..ticks {
        step(&mut sim);
        block_ticks += sim
            .active_blocks
            .iter()
            .filter(|b| **b != sandart_sim::BlockActivity::Inactive)
            .count() as u64;
    }
    let wall = t0.elapsed();
    let sec = phase_timing::snapshot();

    let wall_ns = wall.as_nanos() as f64;
    let cells_per_tick = block_ticks as f64 * (sim.block_size * sim.block_size) as f64 / ticks as f64;
    let blocks_per_tick = block_ticks as f64 / ticks as f64;

    let named_ns: f64 = phase_timing::SECTION_NAMES
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != phase_timing::SEC_SETTLE_TICK_TOTAL)
        .map(|(i, _)| sec[i] as f64)
        .sum();
    let settle_total_ns = sec[phase_timing::SEC_SETTLE_TICK_TOTAL] as f64;
    let settle_named_ns: f64 = named_ns - sec[phase_timing::SEC_FRESH_ACTIVE] as f64;
    let settle_residual_ns = (settle_total_ns - settle_named_ns).max(0.0);
    let update_remainder_ns =
        (wall_ns - sec[phase_timing::SEC_FRESH_ACTIVE] as f64 - settle_total_ns).max(0.0);

    println!(
        "phase_breakdown: N=2.5 budget_n={} ticks={ticks} wall={:.3}ms/tick blocks/tick={:.1} cells/tick={:.0}",
        sim.budget_n,
        wall_ns / 1e6 / ticks as f64,
        blocks_per_tick,
        cells_per_tick,
    );
    let row = |name: &str, ns: f64| {
        println!(
            "  {:<20} {:>10.4} ms/tick  {:>6.2}%  {:>8.2} ns/cell",
            name,
            ns / 1e6 / ticks as f64,
            100.0 * ns / wall_ns,
            ns / cells_per_tick / ticks as f64,
        );
    };
    row("fresh_active", sec[phase_timing::SEC_FRESH_ACTIVE] as f64);
    row("classification", sec[phase_timing::SEC_CLASSIFICATION] as f64);
    row("temp_heights_copy", sec[phase_timing::SEC_TEMP_HEIGHTS_COPY] as f64);
    row("phase0_collect", sec[phase_timing::SEC_PHASE0_COLLECT] as f64);
    row("phase0_apply", sec[phase_timing::SEC_PHASE0_APPLY] as f64);
    row("phase1_traversal", sec[phase_timing::SEC_PHASE1_TRAVERSAL] as f64);
    row("lateral_edge_pass", sec[phase_timing::SEC_LATERAL_EDGE_PASS] as f64);
    row("copy_back", sec[phase_timing::SEC_COPY_BACK] as f64);
    row("settle_tick_residual", settle_residual_ns);
    row("update_remainder", update_remainder_ns);
    println!(
        "  {:<20} {:>10.4} ms/tick  {:>6.2}%",
        "settle_tick_total", settle_total_ns / 1e6 / ticks as f64, 100.0 * settle_total_ns / wall_ns
    );
    let accounted = named_ns + settle_residual_ns + update_remainder_ns;
    println!(
        "  residual check: accounted={:.4}ms/tick wall={:.4}ms/tick diff={:.2}%",
        accounted / 1e6 / ticks as f64,
        wall_ns / 1e6 / ticks as f64,
        100.0 * (wall_ns - accounted) / wall_ns
    );
}

fn main() {
    if std::env::var("SUBSTEPS_SWEEP").is_ok() {
        sweep();
        return;
    }
    if std::env::args().any(|a| a == "phase-breakdown") {
        phase_breakdown();
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
