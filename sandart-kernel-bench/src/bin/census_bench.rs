//! Hypothesis 1 (sparsity) and hypothesis 2's subnormal-count half: runs `kernel_a::census_pass`
//! and `kernel_e2::census_pass` once each, on both snapshots, and prints the fractions the
//! WHY-IS-E2-SLOWER investigation asked for. Not a timing tool -- see `native_bench.rs` for that.
//!
//! Usage: `cargo run -p sandart-kernel-bench --release --bin census_bench -- [snapshot_dir]`

use sandart_kernel_bench::{kernel_a, kernel_e, kernel_e2, snapshot};

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn run_one(label: &str, bytes: &[u8]) {
    println!("\n=== Census -- {label}, pass 1 ===");

    let mut a_state = snapshot::parse(bytes);
    let mut a_scratch = kernel_a::Scratch::new(&a_state);
    let a_census = kernel_a::census_pass(&mut a_state, &mut a_scratch);
    a_census.report("A");

    let mut e2_state = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut e2_scratch = kernel_e2::Scratch::new(&e2_state);
    let e2_census = kernel_e2::census_pass(&mut e2_state, &mut e2_scratch);
    e2_census.report("E2");
}

/// Runs `n-1` ordinary passes (with the normal swap), then censuses pass `n` -- checks whether
/// subnormals or sparsity change once the scene has settled further toward equilibrium (tiny
/// residual fluxes are exactly the kind of value that can go subnormal).
fn run_after_n(label: &str, bytes: &[u8], n: u32) {
    println!("\n=== Census -- {label}, pass {n} ===");

    let mut a_state = snapshot::parse(bytes);
    let mut a_scratch = kernel_a::Scratch::new(&a_state);
    for _ in 0..n - 1 {
        kernel_a::run_pass(&mut a_state, &mut a_scratch);
    }
    let a_census = kernel_a::census_pass(&mut a_state, &mut a_scratch);
    a_census.report("A");

    let mut e2_state = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut e2_scratch = kernel_e2::Scratch::new(&e2_state);
    for _ in 0..n - 1 {
        kernel_e2::run_pass(&mut e2_state, &mut e2_scratch);
    }
    let e2_census = kernel_e2::census_pass(&mut e2_state, &mut e2_scratch);
    e2_census.report("E2");
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    let water = read_file(&dir.join("water_snapshot.bin"));
    let gradient = read_file(&dir.join("gradient_snapshot.bin"));

    run_one("water", &water);
    run_one("gradient", &gradient);
    run_after_n("water", &water, 200);
    run_after_n("gradient", &gradient, 200);
}
