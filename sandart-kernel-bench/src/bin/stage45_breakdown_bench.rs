//! Hypothesis 3/4 diagnostic: splits E2's stage4+5 (52 ns/cell/pass headline) into its three
//! sub-phases -- stage4 (realize/gather/amounts), stage5 props (4-channel mix), stage5 colours
//! (unpack + mix + repack) -- to see where the ~17 ns/cell gap vs. D's stage4+5 (35 ns/cell,
//! which does NOT pay a separate colour unpack because D's copy-in already produced col_r/g/b)
//! actually is.

use sandart_kernel_bench::{kernel_e, kernel_e2, snapshot};
use std::time::Instant;

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn median(v: &mut Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn bench(label: &str, bytes: &[u8]) {
    println!("\n=== E2 stage4+5 breakdown -- {label} ===");
    let state0 = kernel_e::StateE::from_state(&snapshot::parse(bytes));
    let mut scratch = kernel_e2::Scratch::new(&state0);
    let mut state = state0;
    let cells = kernel_e2::simulated_cell_count(&scratch).max(1);

    let warmup = 5usize;
    let iters = 51usize;
    let mut t_pre = Vec::with_capacity(iters);
    let mut t_s2 = Vec::with_capacity(iters);
    let mut t_s3 = Vec::with_capacity(iters);
    let mut t_s4 = Vec::with_capacity(iters);
    let mut t_props = Vec::with_capacity(iters);
    let mut t_colours = Vec::with_capacity(iters);
    let mut t_swap = Vec::with_capacity(iters);

    for i in 0..warmup + iters {
        let t0 = Instant::now();
        kernel_e2::run_stage1(&state, &mut scratch);
        let t1 = Instant::now();
        kernel_e2::run_stage2(&state, &mut scratch);
        let t2 = Instant::now();
        std::hint::black_box(kernel_e2::run_stage3(&mut state, &mut scratch));
        let t3 = Instant::now();
        kernel_e2::run_stage4_only(&mut state, &mut scratch);
        let t4 = Instant::now();
        kernel_e2::run_stage5_props_only(&mut state, &mut scratch);
        let t5 = Instant::now();
        kernel_e2::run_stage5_colours_only(&mut state, &mut scratch);
        let t6 = Instant::now();
        sandart_kernel_bench::kernel_e::run_swap(&mut state);
        let t7 = Instant::now();
        if i >= warmup {
            t_pre.push((t1 - t0).as_nanos() as f64);
            t_s2.push((t2 - t1).as_nanos() as f64);
            t_s3.push((t3 - t2).as_nanos() as f64);
            t_s4.push((t4 - t3).as_nanos() as f64);
            t_props.push((t5 - t4).as_nanos() as f64);
            t_colours.push((t6 - t5).as_nanos() as f64);
            t_swap.push((t7 - t6).as_nanos() as f64);
        }
    }

    let (mp, m2, m3, m4, mprops, mcol, msw) = (
        median(&mut t_pre), median(&mut t_s2), median(&mut t_s3),
        median(&mut t_s4), median(&mut t_props), median(&mut t_colours), median(&mut t_swap),
    );
    let total = mp + m2 + m3 + m4 + mprops + mcol + msw;
    let stage45_total = m4 + mprops + mcol;
    println!("  (median ns/pass, share of total, ns/cell/pass)");
    for (name, ns) in [
        ("stage1", mp), ("stage2", m2), ("stage3", m3),
        ("stage4(realize/gather/amounts)", m4),
        ("stage5-props(4ch mix)", mprops),
        ("stage5-colours(unpack+mix+repack)", mcol),
        ("swap", msw),
    ] {
        println!("    {name:36}: {ns:8.1} ns  ({:5.1}%)  {:.3} ns/cell/pass", 100.0 * ns / total, ns / cells as f64);
    }
    println!("    -> stage4+5 combined: {:.1} ns ({:.3} ns/cell/pass)", stage45_total, stage45_total / cells as f64);
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
