//! Task items 1-3: chunk activity at chunk sizes 4/8/16/32 (three definitions), the cheap
//! "could-flow" predicate's accuracy, and the clustering (run-length/gap distribution) of active
//! edges within a row. All computed from `kernel_a::activity_pass`, on both snapshots, at pass 1
//! and after 200 passes (evolved with kernel A).
//!
//! Usage: `cargo run -p sandart-kernel-bench --release --bin sparsity_bench -- [snapshot_dir]`

use sandart_kernel_bench::kernel_a::{self, SpanActivity};
use sandart_kernel_bench::snapshot;

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Item 1: chunk `n` positions of a span into `[0,K)`, `[K,2K)`, ... (row-aligned WITHIN the
/// span, i.e. chunk 0 always starts at the span's own first owned position, per the task). Returns
/// `(total_chunks, inactive_chunks)` given a per-position `is_active` slice.
fn chunk_inactive_count(is_active: &[bool], k: usize) -> (u64, u64) {
    let n = is_active.len();
    if n == 0 {
        return (0, 0);
    }
    let mut total = 0u64;
    let mut inactive = 0u64;
    let mut i = 0;
    while i < n {
        let end = (i + k).min(n);
        total += 1;
        if is_active[i..end].iter().all(|&a| !a) {
            inactive += 1;
        }
        i = end;
    }
    (total, inactive)
}

struct ChunkStats {
    // [chunk_size_index] -> (total, inactive)
    def_a: [(u64, u64); 4], // no edge w/ nonzero candidate
    def_b: [(u64, u64); 4], // no cell w/ any flow
    def_c: [(u64, u64); 4], // fails predicate for every edge
}

const CHUNK_SIZES: [usize; 4] = [4, 8, 16, 32];

fn compute_chunk_stats(spans: &[SpanActivity]) -> ChunkStats {
    let mut def_a = [(0u64, 0u64); 4];
    let mut def_b = [(0u64, 0u64); 4];
    let mut def_c = [(0u64, 0u64); 4];
    for span in spans {
        for (ki, &k) in CHUNK_SIZES.iter().enumerate() {
            let (ta, ia) = chunk_inactive_count(&span.edge_nonzero_candidate, k);
            def_a[ki].0 += ta;
            def_a[ki].1 += ia;
            let (tb, ib) = chunk_inactive_count(&span.cell_has_flow, k);
            def_b[ki].0 += tb;
            def_b[ki].1 += ib;
            let (tc, ic) = chunk_inactive_count(&span.edge_predicate, k);
            def_c[ki].0 += tc;
            def_c[ki].1 += ic;
        }
    }
    ChunkStats { def_a, def_b, def_c }
}

fn report_chunk_stats(label: &str, stats: &ChunkStats) {
    println!("  [{label}] chunk activity (fraction of chunks fully inactive):");
    println!("    size  (a) no nonzero-candidate edge   (b) no cell w/ flow   (c) fails predicate every edge");
    for (ki, &k) in CHUNK_SIZES.iter().enumerate() {
        let pct = |t: u64, i: u64| if t == 0 { 0.0 } else { 100.0 * i as f64 / t as f64 };
        println!(
            "    {k:4}  {:6.2}% ({}/{})           {:6.2}% ({}/{})     {:6.2}% ({}/{})",
            pct(stats.def_a[ki].0, stats.def_a[ki].1), stats.def_a[ki].1, stats.def_a[ki].0,
            pct(stats.def_b[ki].0, stats.def_b[ki].1), stats.def_b[ki].1, stats.def_b[ki].0,
            pct(stats.def_c[ki].0, stats.def_c[ki].1), stats.def_c[ki].1, stats.def_c[ki].0,
        );
    }
}

struct PredicateStats {
    edges_total: u64,
    edges_passed: u64,
    edges_nonzero: u64,
    passed_and_nonzero: u64,
    false_negatives: u64,
}

fn compute_predicate_stats(spans: &[SpanActivity]) -> PredicateStats {
    let mut s = PredicateStats { edges_total: 0, edges_passed: 0, edges_nonzero: 0, passed_and_nonzero: 0, false_negatives: 0 };
    for span in spans {
        for e in 0..span.n_edges {
            s.edges_total += 1;
            let passed = span.edge_predicate[e];
            let nonzero = span.edge_nonzero_candidate[e];
            if passed {
                s.edges_passed += 1;
            }
            if nonzero {
                s.edges_nonzero += 1;
            }
            if passed && nonzero {
                s.passed_and_nonzero += 1;
            }
            if nonzero && !passed {
                s.false_negatives += 1;
            }
        }
    }
    s
}

fn report_predicate_stats(label: &str, s: &PredicateStats) {
    let pct = |n: u64, d: u64| if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 };
    let precision = if s.edges_passed == 0 { 0.0 } else { 100.0 * s.passed_and_nonzero as f64 / s.edges_passed as f64 };
    println!(
        "  [{label}] predicate: {}/{} edges pass ({:.2}%), {}/{} truly nonzero ({:.2}%), precision {:.2}%, FALSE NEGATIVES: {}",
        s.edges_passed, s.edges_total, pct(s.edges_passed, s.edges_total),
        s.edges_nonzero, s.edges_total, pct(s.edges_nonzero, s.edges_total),
        precision, s.false_negatives,
    );
}

/// Item 3: run lengths of consecutive `true` (active) and consecutive `false` (gap) within each
/// span's `edge_nonzero_final` -- "carries realised flux", the same bar the report's sparsity
/// table uses for "edges w/ final flux > MIN_FLUX".
fn run_lengths(spans: &[SpanActivity]) -> (Vec<usize>, Vec<usize>) {
    let mut active_runs = Vec::new();
    let mut gap_runs = Vec::new();
    for span in spans {
        let a = &span.edge_nonzero_final;
        let mut i = 0;
        while i < a.len() {
            let v = a[i];
            let start = i;
            while i < a.len() && a[i] == v {
                i += 1;
            }
            let len = i - start;
            if v {
                active_runs.push(len);
            } else {
                gap_runs.push(len);
            }
        }
    }
    (active_runs, gap_runs)
}

fn summarize_runs(label: &str, runs: &[usize]) {
    if runs.is_empty() {
        println!("    {label}: (none)");
        return;
    }
    let mut sorted = runs.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let sum: usize = sorted.iter().sum();
    let mean = sum as f64 / n as f64;
    let pct = |p: f64| sorted[((n as f64 - 1.0) * p).round() as usize];
    println!(
        "    {label}: n={n} mean={mean:.2} p50={} p75={} p90={} p99={} max={}",
        pct(0.50), pct(0.75), pct(0.90), pct(0.99), sorted[n - 1]
    );
    // Compact histogram of run length 1, 2, 3, 4, 5-8, 9-16, 17-32, 33+
    let buckets = [(1, 1), (2, 2), (3, 3), (4, 4), (5, 8), (9, 16), (17, 32), (33, usize::MAX)];
    print!("      histogram: ");
    for (lo, hi) in buckets {
        let c = sorted.iter().filter(|&&v| v >= lo && v <= hi).count();
        let label = if hi == usize::MAX { format!("{lo}+") } else if lo == hi { format!("{lo}") } else { format!("{lo}-{hi}") };
        print!("[{label}]={c} ({:.1}%) ", 100.0 * c as f64 / n as f64);
    }
    println!();
}

fn run_scene(label: &str, bytes: &[u8], pass_n: u32) {
    println!("\n=== {label}, pass {pass_n} ===");
    let mut state = snapshot::parse(bytes);
    let mut scratch = kernel_a::Scratch::new(&state);
    for _ in 0..pass_n - 1 {
        kernel_a::run_pass(&mut state, &mut scratch);
    }
    let spans = kernel_a::activity_pass(&mut state, &mut scratch);

    let chunk_stats = compute_chunk_stats(&spans);
    report_chunk_stats(label, &chunk_stats);

    let pred_stats = compute_predicate_stats(&spans);
    report_predicate_stats(label, &pred_stats);

    let (active_runs, gap_runs) = run_lengths(&spans);
    println!("  [{label}] clustering of edges w/ realised final flux (run lengths per row):");
    summarize_runs("active runs", &active_runs);
    summarize_runs("gap runs", &gap_runs);
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    let water = read_file(&dir.join("water_snapshot.bin"));
    let gradient = read_file(&dir.join("gradient_snapshot.bin"));

    run_scene("water", &water, 1);
    run_scene("water", &water, 200);
    run_scene("gradient", &gradient, 1);
    run_scene("gradient", &gradient, 200);
}
