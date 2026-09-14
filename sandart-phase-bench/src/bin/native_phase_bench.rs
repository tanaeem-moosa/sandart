//! Native half of Step 0 (SESSION-HANDOVER-2026-09-13.md §6). Drives the SAME `Bench` the wasm
//! cdylib exports use, so the native and wasm numbers come from one shared driver, not two
//! independently-written ones that could quietly diverge.
//!
//!   cd /home/deck/projects/sandart
//!   CARGO_BUILD_JOBS=2 cargo run -p sandart-phase-bench --release --bin native_phase_bench
//!
//! Env vars (all optional): WARMUP_TICKS (default 800), PHASE_TICKS (default 1000),
//! LATERAL_SUBSTEPS (default 2.5) -- WARMUP_TICKS exists so the measurement window can be moved
//! to wherever the block count is closer to a reference (e.g. the deployed page's), same as
//! `profile_sandfall_water.rs`'s identical knob.
use sandart_phase_bench::Bench;
use sandart_sim::phase_timing;

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}
fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let warmup = env_u32("WARMUP_TICKS", 800);
    let ticks = env_u32("PHASE_TICKS", 1000);
    let substeps = env_f32("LATERAL_SUBSTEPS", 2.5);

    let mut bench = Bench::new(substeps);
    bench.warmup(warmup);

    let t0 = std::time::Instant::now();
    bench.run_measured(ticks);
    let wall = t0.elapsed();

    let wall_ns = wall.as_nanos() as f64;
    let sec = phase_timing::snapshot();
    let cells_per_tick = bench.cells_per_tick();
    let blocks_per_tick = bench.blocks_per_tick();

    println!(
        "native_phase_bench: N={substeps} budget_n={} warmup={warmup} ticks={ticks} wall={:.4}ms/tick blocks/tick={:.1} cells/tick={:.0}",
        bench.sim.budget_n,
        wall_ns / 1e6 / ticks as f64,
        blocks_per_tick,
        cells_per_tick,
    );
    for i in 0..phase_timing::N_SECTIONS {
        println!(
            "  {:<20} {:>10.4} ms/tick  {:>6.2}%  {:>8.2} ns/cell",
            phase_timing::SECTION_NAMES[i],
            sec[i] as f64 / 1e6 / ticks as f64,
            100.0 * sec[i] as f64 / wall_ns,
            sec[i] as f64 / cells_per_tick / ticks as f64,
        );
    }
}
