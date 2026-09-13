//! Hypothesis 3 experiment: does copying each prop channel's window into a small, reused
//! per-span buffer before mixing (D's own scheme) beat mixing directly off a window into the
//! real, whole-grid `state.prop[ch]` row (E2's current stage5_props_e2)? Checks equivalence
//! first (must be bit-identical -- pure data movement, no math change), then times both,
//! back-to-back in the same process, 3 repeats.

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

fn check_equivalence(label: &str, bytes: &[u8]) {
    let mut base = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut base_scratch = kernel_e2::Scratch::new(&base);
    kernel_e2::run_pass(&mut base, &mut base_scratch);

    let mut variant = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut variant_scratch = kernel_e2::Scratch::new(&variant);
    kernel_e2::run_pass_copy_variant(&mut variant, &mut variant_scratch);

    let n = base.w * base.h;
    let mut max_dh = 0.0f32;
    let mut max_dprop = 0.0f32;
    for i in 0..n {
        max_dh = max_dh.max((base.heights[i] - variant.heights[i]).abs());
        for ch in 0..4 {
            max_dprop = max_dprop.max((base.prop[ch][i] - variant.prop[ch][i]).abs());
        }
    }
    println!("  [{label}] copy-variant vs run_pass: max|dh|={max_dh:e} max|dprop|={max_dprop:e} (expect exactly 0)");
}

fn bench(label: &str, bytes: &[u8]) {
    println!("\n=== copy-variant (hypothesis 3) -- {label} ===");
    check_equivalence(label, bytes);

    for rep in 0..3 {
        let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratch = kernel_e2::Scratch::new(&state0);
        let cells = kernel_e2::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns_base = median_ns_per_call(|| kernel_e2::run_pass(&mut state, &mut scratch), 5, 51);

        let state0b = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratchb = kernel_e2::Scratch::new(&state0b);
        let mut stateb = state0b;
        let ns_copy = median_ns_per_call(|| kernel_e2::run_pass_copy_variant(&mut stateb, &mut scratchb), 5, 51);

        println!(
            "  [rep {rep}] E2 (window-read) {:.2} ns/cell/pass, E2 (copy-variant) {:.2} ns/cell/pass, delta {:.2}%",
            ns_base / cells as f64,
            ns_copy / cells as f64,
            100.0 * (ns_copy - ns_base) / ns_base
        );
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);
    let water = read_file(&dir.join("water_snapshot.bin"));
    let gradient = read_file(&dir.join("gradient_snapshot.bin"));

    bench("water", &water);
    bench("gradient", &gradient);
}
