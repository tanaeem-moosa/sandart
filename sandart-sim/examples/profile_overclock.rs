//! PERF-PROFILE.md: cost breakdown of an OVERCLOCKED tick at grid 512.
//!
//! Same scenario as `diag_blocks` (Hourglass, in-plane gravity, `overclocking_enabled = true`),
//! sampled with `pprof` (in-process, no `perf`/kernel dependency) at high frequency across many
//! ticks. Buckets every sample by scanning the WHOLE resolved stack (not just the leaf) for
//! known module/function markers, so cost that occurs inside an inlined call (LTO +
//! codegen-units=1 inlines aggressively) is still attributed to the right bucket rather than
//! collapsing into its caller.
//!
//! Run (inside distrobox, release build with debug info -- see Cargo.toml's local `debug = true`
//! note on `[profile.release]`):
//!   cargo run --release --example profile_overclock -- --material water --ticks 300
//!   cargo run --release --example profile_overclock -- --material drysand --ticks 300 --svg out.svg

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Bucket {
    Coarse,
    EquilSolver,
    Lut,
    Advection,
    Scaffolding,
    Other,
}

fn classify(symbols: &[String]) -> Bucket {
    // Priority order matters: a sample inside the coarse level's nested `settle_tick` call also
    // matches "equilibrium solver" / "advect_properties" / etc symbol names -- that cost belongs
    // to the coarse bucket, not double-counted into the fine-level buckets, so it is checked
    // first and short-circuits.
    let has = |needle: &str| symbols.iter().any(|s| s.contains(needle));
    if has("coarse::CoarseState") || has("sandart_sim::coarse::") {
        return Bucket::Coarse;
    }
    // Checked BEFORE the broader equilibrium-solver match: `cached_vertical_lut` is called FROM
    // inside `overfill_equilibrium_transfer` (its fast path for the common coarse_head==0 case),
    // so a LUT-path sample's stack also contains "overfill_equilibrium_transfer" higher up. The
    // task wants the LUT gather/build distinguished from the root-find arithmetic even though
    // both live inside the same wrapper, so LUT markers take priority and short-circuit here.
    if has("cached_vertical_lut") || has("lookup_equilibrium_lut") || has("build_vertical_equilibrium_lut")
        || has("LUT_CACHE")
    {
        return Bucket::Lut;
    }
    // `overfill_pressure_val`/`cell_potential` are the nonlinear pressure-excess term
    // `overfill_equilibrium_transfer`'s `phi`/`solve_forward`'s stress terms are built from; LTO
    // partially inlines the wrapper away at many (not all) call sites, leaving these as the
    // visible leaf while the wrapper's own frame is gone. Counted here, not scaffolding, since
    // this IS the solver's arithmetic regardless of which frame survived inlining.
    if has("overfill_equilibrium_transfer")
        || has("solve_forward")
        || has("overfill_pressure_val")
        || has("cell_potential")
        || has("coarse_delta_eta_budgeted")
        || has("relative_overfill")
    {
        return Bucket::EquilSolver;
    }
    if has("advect_properties") {
        return Bucket::Advection;
    }
    if has("settle_tick") {
        return Bucket::Scaffolding;
    }
    Bucket::Other
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(300);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let budget: usize = get("--budget").map(|v| v.parse().unwrap()).unwrap_or(256);
    let hz: i32 = get("--hz").map(|v| v.parse().unwrap()).unwrap_or(997);
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
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }

    let _ = sandart_sim::early_term_log_take(); // discard warmup's log
    eprintln!("warmed up; profiling {mat:?} for {ticks} ticks at {hz}Hz...");
    let guard = pprof::ProfilerGuard::new(hz).unwrap();
    let t0 = Instant::now();
    for _ in 0..ticks {
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let elapsed = t0.elapsed();
    let ms_per_frame = elapsed.as_secs_f64() * 1000.0 / ticks as f64;
    eprintln!("done: {ms_per_frame:.2} ms/frame over {ticks} ticks");

    let report = guard.report().build().expect("build profiler report");

    let mut totals: HashMap<Bucket, isize> = HashMap::new();
    let mut total: isize = 0;
    // Also keep the raw leaf-symbol counts inside each bucket, to name the actual hot function.
    let mut leaf_totals: HashMap<String, isize> = HashMap::new();

    for (frames, count) in report.data.iter() {
        // Flatten this stack's symbols (all native frames, all inlined symbols within each) into
        // a name list for `classify` to scan. `frames.frames` is ordered leaf-first per the
        // pprof `Report::flamegraph` implementation (which `.rev()`s it to get root-first for the
        // folded-stack format), so `frames.frames[0]` is the sample's actual PC.
        let mut names: Vec<String> = Vec::new();
        for frame in &frames.frames {
            for sym in frame {
                names.push(sym.name());
            }
        }
        let bucket = classify(&names);
        *totals.entry(bucket).or_insert(0) += count;
        total += count;
        if let Some(leaf) = names.first() {
            *leaf_totals.entry(leaf.clone()).or_insert(0) += count;
        }
    }

    println!("\n=== {mat:?} @ grid {grid}, overclock ON, {ticks} ticks, {ms_per_frame:.2} ms/frame, {hz}Hz sampling ===");
    println!("total samples: {total}");
    let mut rows: Vec<(Bucket, isize)> = totals.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    for (bucket, count) in &rows {
        println!("  {:<14} {:>7} samples  {:>6.2}%", format!("{:?}", bucket), count, 100.0 * *count as f64 / total.max(1) as f64);
    }

    println!("\n  top 20 leaf symbols (by resolved PC, across all buckets):");
    let mut leaves: Vec<(String, isize)> = leaf_totals.into_iter().collect();
    leaves.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, count) in leaves.iter().take(20) {
        println!("    {:>6.2}%  ({:>6} samples)  {}", 100.0 * *count as f64 / total.max(1) as f64, count, name);
    }

    // Job 2 candidate 5: how often does an overclocked block reach local equilibrium (real
    // last_displacements < MUST_SIMULATE_THRESHOLD) before its allotted sub-steps are used up?
    // `sandart_sim::early_term_log_take()` drains the TEMPORARY instrumentation added to
    // `update()`'s rep loop for this measurement (see lib.rs, not for landing).
    let log = sandart_sim::early_term_log_take();
    if !log.is_empty() {
        let mut by_target: HashMap<u32, (u64, u64, u64)> = HashMap::new(); // target -> (blocks, settled_early_count, sum_first_settled_rep_when_early)
        for (target, first) in &log {
            let e = by_target.entry(*target).or_insert((0, 0, 0));
            e.0 += 1;
            if *first >= 0 {
                e.1 += 1;
                e.2 += *first as u64;
            }
        }
        println!("\n  early-termination distribution (overclocked blocks only, {} block-frames total):", log.len());
        println!("    {:<8} {:>12} {:>16} {:>18}", "target n", "block-frames", "settled early %", "avg settle-rep");
        let mut targets: Vec<u32> = by_target.keys().cloned().collect();
        targets.sort();
        for t in targets {
            let (n, early, sum) = by_target[&t];
            let pct = 100.0 * early as f64 / n.max(1) as f64;
            let avg = if early > 0 { sum as f64 / early as f64 } else { f64::NAN };
            println!("    {:<8} {:>12} {:>15.1}% {:>18.2}", t, n, pct, avg);
        }
    }

    if let Some(out) = get("--svg") {
        if let Ok(report) = guard.report().build() {
            let file = std::fs::File::create(&out).unwrap();
            report.flamegraph(file).unwrap();
            eprintln!("wrote {out}");
        }
    }
}
