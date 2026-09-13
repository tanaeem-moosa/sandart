//! Native driver: Step 1's validation, Step 3's equivalence tables, and Step 4's native timing.
//! Wasm timing and SIMD instruction counts live in `bench_wasm.mjs` (see that file and the
//! report for why equivalence itself is only ever checked natively).
//!
//! Usage: `cargo run -p sandart-kernel-bench --release --bin native_bench -- [snapshot_dir]`
//! (defaults to this session's scratchpad, matching `dump_kernel_bench_snapshots`'s own default).

use sandart_kernel_bench::{kernel_a, kernel_b, kernel_c, kernel_c8, kernel_d, metrics, kernel_r, snapshot};
use std::time::Instant;

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn validate_r(dir: &std::path::Path) {
    println!("\n=== Step 1: validating kernel R against a real lateral pass ===");
    let bytes = read_file(&dir.join("validation_snapshot.bin"));
    let (mut state, consumed) = snapshot::parse_at(&bytes);
    let trailer = &bytes[consumed..];
    let n = state.w * state.h;
    let mut off = 0usize;
    let f32_slice = |b: &[u8], off: &mut usize, n: usize| -> Vec<f32> {
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let o = *off + i * 4;
            v.push(f32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
        }
        *off += n * 4;
        v
    };
    let post_heights = f32_slice(trailer, &mut off, n);
    let post_props = f32_slice(trailer, &mut off, n * 4);
    let post_colors = trailer[off..off + n * 4].to_vec();

    let mut scratch = kernel_r::Scratch::new(state.w, state.h);
    kernel_r::run_pass(&mut state, &mut scratch);

    let mut max_dh = 0.0f32;
    let mut max_dprop = 0.0f32;
    let mut max_dcolor = 0i32;
    for i in 0..n {
        max_dh = max_dh.max((state.heights[i] - post_heights[i]).abs());
        for ch in 0..4 {
            max_dprop = max_dprop.max((state.cell_props[i * 4 + ch] - post_props[i * 4 + ch]).abs());
            max_dcolor = max_dcolor.max((state.cell_colors[i * 4 + ch] as i32 - post_colors[i * 4 + ch] as i32).abs());
        }
    }
    println!(
        "kernel R vs. one real TestSim::tick (64x3, middle row only, water/dry-sand gradient):\n\
         max |dh| = {max_dh:e}, max |dprop| = {max_dprop:e}, max |dcolor| = {max_dcolor}"
    );
    let ok = max_dh < 1e-5 && max_dprop < 1e-5 && max_dcolor <= 1;
    println!("=> {}", if ok { "MATCH within float tolerance" } else { "MISMATCH -- see report" });
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

struct Snap {
    name: &'static str,
    bytes: Vec<u8>,
}

fn run_equivalence(label: &str, bytes: &[u8]) {
    println!("\n=== Step 3: equivalence -- {label} ===");
    let base = snapshot::parse(bytes);

    for &passes in &[1u32, 200u32] {
        let mut r_state = snapshot::parse(bytes);
        let mut r_scratch = kernel_r::Scratch::new(r_state.w, r_state.h);
        for _ in 0..passes {
            kernel_r::run_pass(&mut r_state, &mut r_scratch);
        }

        let mut a_state = snapshot::parse(bytes);
        let mut a_scratch = kernel_a::Scratch::new(&a_state);
        for _ in 0..passes {
            kernel_a::run_pass(&mut a_state, &mut a_scratch);
        }

        let mut b_state = snapshot::parse(bytes);
        let mut b_scratch = kernel_b::Scratch::new(&b_state);
        for _ in 0..passes {
            kernel_b::run_pass(&mut b_state, &mut b_scratch);
        }

        let mut c_state = snapshot::parse(bytes);
        let mut c_scratch = kernel_c::Scratch::new(&c_state);
        for _ in 0..passes {
            kernel_c::run_pass(&mut c_state, &mut c_scratch);
        }

        let mut c8_state = snapshot::parse(bytes);
        let mut c8_scratch = kernel_c::Scratch::new(&c8_state);
        for _ in 0..passes {
            kernel_c8::run_pass(&mut c8_state, &mut c8_scratch);
        }

        let mut d_state = snapshot::parse(bytes);
        let mut d_scratch = kernel_d::Scratch::new(&d_state);
        for _ in 0..passes {
            kernel_d::run_pass(&mut d_state, &mut d_scratch);
        }

        let rep_a = metrics::compare(&r_state, &a_state);
        let rep_b = metrics::compare(&r_state, &b_state);
        let rep_c = metrics::compare(&r_state, &c_state);
        let rep_c8 = metrics::compare(&r_state, &c8_state);
        let rep_d = metrics::compare(&r_state, &d_state);
        let rep_c_vs_a = metrics::compare(&a_state, &c_state);
        let rep_d_vs_c = metrics::compare(&c_state, &d_state);

        println!("-- after {passes} pass(es) --");
        for (name, rep) in [
            ("A vs R", &rep_a),
            ("B vs R", &rep_b),
            ("C vs R", &rep_c),
            ("C8 vs R", &rep_c8),
            ("D vs R", &rep_d),
            ("C vs A", &rep_c_vs_a),
            ("D vs C", &rep_d_vs_c),
        ] {
            println!(
                "  {name}: mass R={:.6} test={:.6} delta={:.3e} | max|dh|={:.3e} mean|dh|={:.3e} | \
                 max(h-cap) R={:.3e} test={:.3e} | max|dwet|={:.3e} mean|dwet|={:.3e} | mean|dcolor|={:.3e} | \
                 mirror R={:.4} test={:.4}",
                rep.mass_a, rep.mass_b, rep.mass_delta,
                rep.max_abs_dh, rep.mean_abs_dh,
                rep.max_over_capacity_ref, rep.max_over_capacity_test,
                rep.max_abs_dwetness, rep.mean_abs_dwetness,
                rep.mean_abs_dcolor,
                rep.mirror_asymmetry_a, rep.mirror_asymmetry_b,
            );
        }
    }
    let _ = base;
}

fn run_timing(label: &str, bytes: &[u8]) {
    println!("\n=== Step 4: native timing -- {label} ===");

    {
        let state0 = snapshot::parse(bytes);
        let cells = kernel_r::simulated_cell_count(&state0);
        let mut state = state0;
        let mut scratch = kernel_r::Scratch::new(state.w, state.h);
        let ns = median_ns_per_call(|| kernel_r::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  R: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_a::Scratch::new(&state0);
        let cells = kernel_a::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_a::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  A: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_b::Scratch::new(&state0);
        let cells = kernel_b::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_b::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  B: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        // Precompute cost, isolated: `kernel_c::precompute_head_static` alone, called on a fresh
        // parse each time (not fed back into a Scratch) -- the one-time, per-snapshot-load cost
        // the report keeps separate from ns/cell/pass. See kernel_c.rs's module doc comment.
        let state0 = snapshot::parse(bytes);
        let n = state0.w * state0.h;
        let ns = median_ns_per_call(|| kernel_c::precompute_head_static(&state0).len() as f64, 5, 51);
        println!("  C precompute: {:.2} ns/call, {:.3} ns/cell ({n} cells)", ns, ns / n as f64);
    }
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_c::Scratch::new(&state0);
        let cells = kernel_c::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_c::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  C: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_c::Scratch::new(&state0);
        let cells = kernel_c8::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_c8::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  C8: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        // D's precompute cost, isolated exactly like C's: fresh parse each call, not fed into a
        // Scratch, denominator is the cells the precompute ACTUALLY visits (span cells), not w*h.
        let state0 = snapshot::parse(bytes);
        let spans = sandart_kernel_bench::row_span::build_spans(&state0);
        let n = kernel_d::precompute_cell_count(&spans);
        let ns = median_ns_per_call(|| kernel_d::precompute_head_static_d(&state0, &spans).len() as f64, 5, 51);
        println!("  D precompute: {:.2} ns/call, {:.3} ns/cell ({n} simulated cells)", ns, ns / n.max(1) as f64);
    }
    {
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_d::Scratch::new(&state0);
        let cells = kernel_d::simulated_cell_count(&scratch);
        let mut state = state0;
        let ns = median_ns_per_call(|| kernel_d::run_pass(&mut state, &mut scratch), 5, 51);
        println!("  D: {:.2} ns/pass, {:.2} ns/cell/pass ({cells} cells)", ns, ns / cells as f64);
    }
    {
        // D's per-stage split, native `Instant` around each stage's own whole-pass runner (task:
        // "time inside Rust via a cheap counter... Native `Instant` is fine"). Each sample re-runs
        // ALL FIVE stages in sequence (so the state stays valid pass-over-pass, matching `run_d`'s
        // own evolution) but only the named stage's own call is timed.
        let state0 = snapshot::parse(bytes);
        let mut scratch = kernel_d::Scratch::new(&state0);
        let mut state = state0;
        let warmup = 5usize;
        let iters = 51usize;
        let mut t_copy_in = Vec::with_capacity(iters);
        let mut t_stage2 = Vec::with_capacity(iters);
        let mut t_stage3 = Vec::with_capacity(iters);
        let mut t_stage45 = Vec::with_capacity(iters);
        let mut t_copy_out = Vec::with_capacity(iters);
        for i in 0..warmup + iters {
            let t0 = Instant::now();
            kernel_d::run_copy_in(&state, &mut scratch);
            let t1 = Instant::now();
            kernel_d::run_stage2(&state, &mut scratch);
            let t2 = Instant::now();
            std::hint::black_box(kernel_d::run_stage3(&mut scratch));
            let t3 = Instant::now();
            kernel_d::run_stage45(&mut scratch);
            let t4 = Instant::now();
            kernel_d::run_copy_out(&mut state, &scratch);
            let t5 = Instant::now();
            if i >= warmup {
                t_copy_in.push((t1 - t0).as_nanos() as f64);
                t_stage2.push((t2 - t1).as_nanos() as f64);
                t_stage3.push((t3 - t2).as_nanos() as f64);
                t_stage45.push((t4 - t3).as_nanos() as f64);
                t_copy_out.push((t5 - t4).as_nanos() as f64);
            }
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        let cells = kernel_d::simulated_cell_count(&scratch).max(1);
        let (mi, m2, m3, m45, mo) = (median(&mut t_copy_in), median(&mut t_stage2), median(&mut t_stage3), median(&mut t_stage45), median(&mut t_copy_out));
        let total = mi + m2 + m3 + m45 + mo;
        println!("  D per-stage (median ns/pass, share of D's own stage total, ns/cell/pass):");
        for (name, ns) in [("copy-in", mi), ("stage2", m2), ("stage3", m3), ("stage4+5", m45), ("copy-out", mo)] {
            println!("    {name:10}: {ns:8.1} ns  ({:5.1}%)  {:.3} ns/cell/pass", 100.0 * ns / total, ns / cells as f64);
        }
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    validate_r(dir);

    let snaps = [
        Snap { name: "water", bytes: read_file(&dir.join("water_snapshot.bin")) },
        Snap { name: "gradient", bytes: read_file(&dir.join("gradient_snapshot.bin")) },
    ];

    for s in &snaps {
        run_equivalence(s.name, &s.bytes);
    }
    for s in &snaps {
        run_timing(s.name, &s.bytes);
    }
}
