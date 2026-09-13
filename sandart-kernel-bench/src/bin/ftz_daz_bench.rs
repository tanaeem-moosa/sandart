//! Hypothesis 2's timing half: does setting MXCSR FTZ+DAZ change E2's native ns/cell/pass? The
//! census (`census_bench`) already found ZERO subnormal values anywhere in E2's intermediate
//! arrays on both snapshots at pass 1 and pass 200, which predicts this should be a no-op -- this
//! bin checks that prediction empirically rather than skipping the timing on the census alone.
//!
//! x86_64-only (uses `_mm_getcsr`/`_mm_setcsr`). Native only -- run under a quiet machine
//! (check `uptime` first; only trust this if the 1-min load average was < 1.5 at run time).

use sandart_kernel_bench::{kernel_e, kernel_e2, snapshot};
use std::time::Instant;

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn median_ns_per_call<F: FnMut() -> f64>(mut f: F, warmup: usize, iters: usize) -> f64 {
    for _ in 0..warmup {
        std::hint::black_box(f());
    }
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        std::hint::black_box(f());
        samples.push(t0.elapsed().as_nanos() as f64);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

#[cfg(target_arch = "x86_64")]
fn with_ftz_daz<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
    unsafe {
        let orig = _mm_getcsr();
        let mxcsr = if enabled {
            orig | (1 << 15) | (1 << 6) // FTZ (bit 15), DAZ (bit 6)
        } else {
            orig & !(1 << 15) & !(1 << 6)
        };
        _mm_setcsr(mxcsr);
        let r = f();
        _mm_setcsr(orig);
        r
    }
}

fn bench_e2(label: &str, bytes: &[u8], ftz_daz: bool) -> f64 {
    let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut scratch = kernel_e2::Scratch::new(&state0);
    let cells = kernel_e2::simulated_cell_count(&scratch);
    let mut state = state0;
    let ns = with_ftz_daz(ftz_daz, || median_ns_per_call(|| kernel_e2::run_pass(&mut state, &mut scratch), 5, 51));
    let per_cell = ns / cells as f64;
    println!("  {label} ftz_daz={ftz_daz}: {ns:.2} ns/pass, {per_cell:.3} ns/cell/pass");
    per_cell
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);
    let water = read_file(&dir.join("water_snapshot.bin"));
    let gradient = read_file(&dir.join("gradient_snapshot.bin"));

    for (label, bytes) in [("water", &water), ("gradient", &gradient)] {
        println!("\n=== E2 FTZ/DAZ -- {label} (3 repeats each, back-to-back) ===");
        for rep in 0..3 {
            let off = bench_e2(&format!("[rep {rep}] off"), bytes, false);
            let on = bench_e2(&format!("[rep {rep}] on "), bytes, true);
            println!("    delta: {:.2}%", 100.0 * (on - off) / off);
        }
    }
}
