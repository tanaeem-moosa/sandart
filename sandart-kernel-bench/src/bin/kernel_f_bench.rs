//! Kernel F (hypothesis-1 hybrid, see `kernel_e2.rs`'s `stage5_props_e2_chunked`/
//! `stage5_colours_e2_chunked`): E2's stage1/2/3 unchanged, stage4 unchanged, stage5 (props +
//! colour mixing) restructured into 8-cell chunks with a cheap "any flow in this chunk?" test
//! before paying for the branch-free math -- an all-zero chunk gets a straight copy of the frozen
//! row instead. Checks equivalence to E2 at 1 and 200 passes (must be bit-identical -- the
//! no-flow branch is a proven identity, not an approximation), then times native.

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

fn check_equivalence(label: &str, bytes: &[u8], passes: u32) {
    let mut e2_state = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut e2_scratch = kernel_e2::Scratch::new(&e2_state);
    for _ in 0..passes {
        kernel_e2::run_pass(&mut e2_state, &mut e2_scratch);
    }

    let mut f_state = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut f_scratch = kernel_e2::Scratch::new(&f_state);
    for _ in 0..passes {
        kernel_e2::run_pass_f(&mut f_state, &mut f_scratch);
    }

    let n = e2_state.w * e2_state.h;
    let mut max_dh = 0.0f32;
    let mut max_dprop = 0.0f32;
    let mut max_dcolor = 0i32;
    for i in 0..n {
        max_dh = max_dh.max((e2_state.heights[i] - f_state.heights[i]).abs());
        for ch in 0..4 {
            max_dprop = max_dprop.max((e2_state.prop[ch][i] - f_state.prop[ch][i]).abs());
        }
        let a = e2_state.colors[i];
        let b = f_state.colors[i];
        for shift in [0, 8, 16, 24] {
            let da = ((a >> shift) & 0xFF) as i32 - ((b >> shift) & 0xFF) as i32;
            max_dcolor = max_dcolor.max(da.abs());
        }
    }
    println!("  [{label}] F vs E2 after {passes} pass(es): max|dh|={max_dh:e} max|dprop|={max_dprop:e} max|dcolor|={max_dcolor} (expect exactly 0)");
}

fn bench(label: &str, bytes: &[u8]) {
    println!("\n=== Kernel F -- {label} ===");
    check_equivalence(label, bytes, 1);
    check_equivalence(label, bytes, 200);

    for rep in 0..3 {
        let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratch = kernel_e2::Scratch::new(&state0);
        let cells = kernel_e2::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns_e2 = median_ns_per_call(|| kernel_e2::run_pass(&mut state, &mut scratch), 5, 51);

        let state0f = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratchf = kernel_e2::Scratch::new(&state0f);
        let mut statef = state0f;
        let ns_f = median_ns_per_call(|| kernel_e2::run_pass_f(&mut statef, &mut scratchf), 5, 51);

        println!(
            "  [rep {rep}] A=? E2={:.2} F={:.2} ns/cell/pass, F vs E2 delta {:.2}%",
            ns_e2 / cells as f64,
            ns_f / cells as f64,
            100.0 * (ns_f - ns_e2) / ns_e2
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
