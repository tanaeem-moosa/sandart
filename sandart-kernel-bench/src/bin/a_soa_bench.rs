//! Task item 4: A vs A_soa equivalence (must match A bit-identically, or to float noise forced by
//! the colour pack/unpack), plus native timing for A, A_soa and E2 side by side.
//!
//! Usage: `cargo run -p sandart-kernel-bench --release --bin a_soa_bench -- [snapshot_dir]`

use sandart_kernel_bench::{kernel_a, kernel_a_soa, kernel_e, kernel_e2, snapshot};
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

fn run_equivalence(label: &str, bytes: &[u8]) {
    println!("\n=== A vs A_soa equivalence -- {label} ===");
    for &passes in &[1u32, 200u32] {
        let mut a_state = snapshot::parse(bytes);
        let mut a_scratch = kernel_a::Scratch::new(&a_state);
        for _ in 0..passes {
            kernel_a::run_pass(&mut a_state, &mut a_scratch);
        }

        let mut asoa_state = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut asoa_scratch = kernel_a_soa::Scratch::new(&asoa_state);
        for _ in 0..passes {
            kernel_a_soa::run_pass(&mut asoa_state, &mut asoa_scratch);
        }
        let asoa_as_state = asoa_state.to_state();

        let n = a_state.w * a_state.h;
        let mut max_dh = 0.0f32;
        let mut max_dprop = 0.0f32;
        let mut max_dcolor = 0i32;
        let mut n_dh = 0usize;
        let mut n_dprop = 0usize;
        let mut n_dcolor = 0usize;
        for i in 0..n {
            let dh = (a_state.heights[i] - asoa_as_state.heights[i]).abs();
            if dh != 0.0 {
                n_dh += 1;
            }
            max_dh = max_dh.max(dh);
            for ch in 0..4 {
                let dp = (a_state.cell_props[i * 4 + ch] - asoa_as_state.cell_props[i * 4 + ch]).abs();
                if dp != 0.0 {
                    n_dprop += 1;
                }
                max_dprop = max_dprop.max(dp);
                // Alpha channel (ch 3) is intentionally not colour-mixed by A_soa (see its module
                // doc comment) -- both A and A_soa force it to 255, so it's always equal; skip it
                // from the max/count so a genuine RGB mismatch isn't diluted.
                if ch < 3 {
                    let dc = (a_state.cell_colors[i * 4 + ch] as i32 - asoa_as_state.cell_colors[i * 4 + ch] as i32).abs();
                    if dc != 0 {
                        n_dcolor += 1;
                    }
                    max_dcolor = max_dcolor.max(dc);
                }
            }
        }
        println!(
            "  after {passes} pass(es): max|dh|={max_dh:e} ({n_dh} cells differ) max|dprop|={max_dprop:e} ({n_dprop} differ) max|dcolor(rgb)|={max_dcolor} ({n_dcolor} differ)"
        );
    }
}

fn run_timing(label: &str, bytes: &[u8]) {
    println!("\n=== Native timing -- {label} ===");
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_a::Scratch::new(&state0);
        let cells = kernel_a::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_a::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  A:      {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratch = kernel_a_soa::Scratch::new(&state0);
        let cells = kernel_a_soa::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_a_soa::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  A_soa:  {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
        let mut scratch = kernel_e2::Scratch::new(&state0);
        let cells = kernel_e2::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_e2::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  E2:     {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    let snaps = [
        ("water", read_file(&dir.join("water_snapshot.bin"))),
        ("gradient", read_file(&dir.join("gradient_snapshot.bin"))),
    ];

    for (name, bytes) in &snaps {
        run_equivalence(name, bytes);
    }
    for (name, bytes) in &snaps {
        run_timing(name, bytes);
    }
}
