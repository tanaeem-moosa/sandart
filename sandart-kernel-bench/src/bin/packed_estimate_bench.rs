//! Task item 5: upper-bound estimate for a "packed" kernel (predicate pass over all cells, then
//! gather+E2-style branch-free math only on the edges/cells the predicate passes), from measured
//! component costs, compared against A_soa (the correct bar).
//!
//! Usage: `cargo run -p sandart-kernel-bench --release --bin packed_estimate_bench -- [snapshot_dir]`

use sandart_kernel_bench::consts::*;
use sandart_kernel_bench::kernel_e::StateE;
use sandart_kernel_bench::predicate::could_flow_bf;
use sandart_kernel_bench::row_span::{build_spans_raw, Span};
use sandart_kernel_bench::scalar_math::{cell_capacity_for, janssen_effective_depth, k_of_liquidity, liquidity};
use sandart_kernel_bench::snapshot;
use std::time::Instant;

fn read_file(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

/// Per-cell scalars the predicate needs, over EVERY cell in every span's data range (the
/// unavoidable "predicate pass over all cells" cost -- no `in_transit_at`, no hashes). Flat,
/// contiguous per span (like `kernel_e2`'s padded scratch), sized `n_data`.
struct CellFields {
    h: Vec<f32>,
    freecap: Vec<f32>,
    head_base: Vec<f32>,
    tau: Vec<f32>,
}

fn compute_cell_fields(state: &StateE, span: Span, w: usize) -> CellFields {
    let n_data = span.data_end() - span.x_start;
    let mut h = vec![0.0f32; n_data];
    let mut freecap = vec![0.0f32; n_data];
    let mut head_base = vec![0.0f32; n_data];
    let mut tau = vec![0.0f32; n_data];
    let row = span.y * w;
    for i in 0..n_data {
        let idx = row + span.x_start + i;
        let inside = state.shape_mask[idx] != MASK_OUTSIDE;
        let hh = state.heights[idx];
        let wetness = state.prop[PROP_WETNESS][idx];
        let threshold = state.prop[PROP_THRESHOLD][idx];
        let liq = liquidity(wetness);
        let cap = cell_capacity_for(wetness);
        let granular_share = 1.0 - liq;
        let k = k_of_liquidity(liq);
        let depth = janssen_effective_depth(state.column_depth[idx], liq);
        h[i] = hh * (inside as i32 as f32);
        freecap[i] = (cap - hh).max(0.0) * (inside as i32 as f32);
        head_base[i] = hh + k * LATERAL_PRESSURE_SCALE * depth;
        tau[i] = GRANULAR_TAU_SCALE * threshold * granular_share;
    }
    CellFields { h, freecap, head_base, tau }
}

/// The vectorisable predicate pass: for one span, compute `CellFields` (every cell) then evaluate
/// `could_flow_bf` for every owned edge. Returns the pass mask so the gather step can use it.
fn predicate_pass_span(state: &StateE, span: Span, w: usize) -> (CellFields, Vec<f32>) {
    let n_edges = span.x_owned_end - span.x_start;
    let fields = compute_cell_fields(state, span, w);
    let row = span.y * w;
    let mut mask = vec![0.0f32; n_edges];
    for e in 0..n_edges {
        let idx = row + span.x_start + e;
        mask[e] = could_flow_bf(
            fields.h[e], fields.h[e + 1],
            fields.freecap[e], fields.freecap[e + 1],
            fields.head_base[e], fields.head_base[e + 1],
            fields.tau[e],
            state.edge_vel_h[idx],
        );
    }
    (fields, mask)
}

/// The scalar-gather step: for each edge whose mask is nonzero, gather the handful of fields a
/// real E2-style stage2/3 candidate computation needs (head_a/head_b/tau/freecap x2/gjs x2/
/// edge_vel_prev -- 7 scattered reads) into flat packed buffers. Returns the packed buffers (so
/// the compiler can't dead-code-eliminate the gather) and the count gathered.
fn gather_pass(state: &StateE, span: Span, w: usize, fields: &CellFields, mask: &[f32]) -> (usize, f32) {
    let row = span.y * w;
    let mut acc = 0.0f32;
    let mut n = 0usize;
    for e in 0..mask.len() {
        if mask[e] == 0.0 {
            continue;
        }
        let idx = row + span.x_start + e;
        let head_a = fields.head_base[e];
        let head_b = fields.head_base[e + 1];
        let freecap_a = fields.freecap[e];
        let freecap_b = fields.freecap[e + 1];
        let tau = fields.tau[e];
        let v_prev = state.edge_vel_h[idx];
        let gjs_a = state.prop[PROP_GRAIN_SIZE][row + span.x_start + e];
        let gjs_b = state.prop[PROP_GRAIN_SIZE][row + span.x_start + e + 1];
        acc += head_a - head_b + freecap_a + freecap_b + tau + v_prev + gjs_a + gjs_b;
        n += 1;
    }
    (n, acc)
}

fn run_scene(label: &str, bytes: &[u8], a_soa_ns_per_cell: f64) {
    println!("\n=== Packed-kernel estimate -- {label} ===");
    let state = StateE::from_state(&snapshot::parse(bytes));
    let spans = build_spans_raw(state.cols, state.rows, state.block_size, state.w, state.h, &state.sim_blocks);
    let w = state.w;
    let total_cells: usize = spans.iter().map(|s| s.x_owned_end - s.x_start).sum();
    let total_edges: usize = spans.iter().map(|s| s.x_owned_end - s.x_start).sum();

    // Time the predicate pass (all spans, one full sweep) -- median over repeats.
    let warmup = 5;
    let iters = 31;
    let mut pred_samples = Vec::with_capacity(iters);
    let mut all_fields: Vec<CellFields> = Vec::new();
    let mut all_masks: Vec<Vec<f32>> = Vec::new();
    for i in 0..warmup + iters {
        let t0 = Instant::now();
        let mut fields_this_run = Vec::with_capacity(spans.len());
        let mut masks_this_run = Vec::with_capacity(spans.len());
        let mut acc = 0.0f32;
        for &span in &spans {
            let (fields, mask) = predicate_pass_span(&state, span, w);
            acc += mask.iter().sum::<f32>();
            fields_this_run.push(fields);
            masks_this_run.push(mask);
        }
        std::hint::black_box(acc);
        let elapsed = t0.elapsed().as_nanos() as f64;
        if i >= warmup {
            pred_samples.push(elapsed);
        }
        all_fields = fields_this_run;
        all_masks = masks_this_run;
    }
    let pred_ns = median(&mut pred_samples);
    let pred_ns_per_cell = pred_ns / total_cells as f64;

    // Count how many edges actually passed (real fraction, this scene/pass).
    let passed: usize = all_masks.iter().map(|m| m.iter().filter(|&&x| x != 0.0).count()).sum();
    let pass_fraction = passed as f64 / total_edges as f64;

    // Time the scalar gather over exactly the edges that passed.
    let mut gather_samples = Vec::with_capacity(iters);
    let mut total_gathered = 0usize;
    for i in 0..warmup + iters {
        let t0 = Instant::now();
        let mut acc = 0.0f32;
        let mut n = 0usize;
        for (si, &span) in spans.iter().enumerate() {
            let (gn, gacc) = gather_pass(&state, span, w, &all_fields[si], &all_masks[si]);
            n += gn;
            acc += gacc;
        }
        std::hint::black_box(acc);
        let elapsed = t0.elapsed().as_nanos() as f64;
        if i >= warmup {
            gather_samples.push(elapsed);
        }
        total_gathered = n;
    }
    let gather_ns = median(&mut gather_samples);
    let gather_ns_per_passed_edge = gather_ns / total_gathered.max(1) as f64;
    let gather_ns_per_cell_at_real_fraction = gather_ns / total_cells as f64;

    println!("  simulated cells/edges: {total_cells}");
    println!("  predicate pass: {pred_ns:.0} ns total, {pred_ns_per_cell:.3} ns/cell (all cells, unconditional)");
    println!("  real pass fraction (edges whose predicate passed): {:.2}% ({passed}/{total_edges})", 100.0 * pass_fraction);
    println!("  scalar gather over passed edges: {gather_ns:.0} ns total ({total_gathered} edges), {gather_ns_per_passed_edge:.2} ns/passed-edge, {gather_ns_per_cell_at_real_fraction:.3} ns/cell at the REAL pass fraction");

    // E2 native per-stage costs (ns/cell), from KERNEL-BENCH-2026-09-13.md's per-stage split:
    // stage1 26, stage2 18, stage3 16, stage4+5 52. The predicate pass stands in for (part of)
    // stage1's per-cell work, so the packed estimate's "pay only on pass fraction" portion is
    // stage2+3+4+5 = 86 ns/cell, applied ONLY to the pass fraction.
    const E2_STAGE1: f64 = 26.0;
    const E2_STAGE2: f64 = 18.0;
    const E2_STAGE3: f64 = 16.0;
    const E2_STAGE45: f64 = 52.0;
    let e2_variable = E2_STAGE2 + E2_STAGE3 + E2_STAGE45; // 86.0
    let _ = E2_STAGE1;

    let gather_ns_per_cell_full = gather_ns_per_passed_edge; // per edge that would pass, amortized by pass_fraction below

    let packed_cost = |f: f64| pred_ns_per_cell + f * gather_ns_per_cell_full + f * e2_variable;

    let real_estimate = packed_cost(pass_fraction);
    println!(
        "  packed estimate @ real pass fraction ({:.2}%): {:.3} ns/cell  (predicate {:.3} + gather {:.3} + E2-variable {:.3})",
        100.0 * pass_fraction, real_estimate, pred_ns_per_cell, pass_fraction * gather_ns_per_cell_full, pass_fraction * e2_variable
    );
    println!("  A_soa (measured, correct bar): {a_soa_ns_per_cell:.3} ns/cell");

    // Break-even pass fraction: packed_cost(f) == a_soa_ns_per_cell
    let denom = gather_ns_per_cell_full + e2_variable;
    let f_star = if denom > 0.0 { (a_soa_ns_per_cell - pred_ns_per_cell) / denom } else { f64::INFINITY };
    println!("  break-even pass fraction (packed == A_soa): {:.2}%", 100.0 * f_star);
    if f_star < 0.0 {
        println!("  => predicate-pass cost ALONE already exceeds A_soa; packed loses at any pass fraction.");
    } else if pass_fraction > f_star {
        println!("  => real pass fraction ({:.2}%) EXCEEDS break-even ({:.2}%): packed predicted to LOSE to A_soa on this scene.", 100.0 * pass_fraction, 100.0 * f_star);
    } else {
        println!("  => real pass fraction ({:.2}%) is BELOW break-even ({:.2}%): packed predicted to BEAT A_soa on this scene.", 100.0 * pass_fraction, 100.0 * f_star);
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    // A_soa ns/cell/pass, filled in from a_soa_bench's own run (see the report) -- kept as a
    // command-line-free constant pair here so this bin has no dependency ordering on that one;
    // update if a_soa_bench's numbers change.
    let a_soa_water = std::env::var("A_SOA_WATER_NS").ok().and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
    let a_soa_gradient = std::env::var("A_SOA_GRADIENT_NS").ok().and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);

    let water = read_file(&dir.join("water_snapshot.bin"));
    let gradient = read_file(&dir.join("gradient_snapshot.bin"));

    run_scene("water", &water, a_soa_water);
    run_scene("gradient", &gradient, a_soa_gradient);
}
