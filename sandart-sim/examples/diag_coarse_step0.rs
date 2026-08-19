//! Build step 0 of `artifacts/design/HIERARCHICAL-PRESSURE.md`: falsify (or confirm) §0.1
//! before any coupling is written. No coupling is built here. This aggregates the existing
//! fine-level state into an offline 64x64 coarse grid and runs the *existing*
//! `overfill_equilibrium_transfer`/`cell_potential` over that coarse graph, exactly as §5 step 3
//! and §0.1 describe. Nothing here is read back into the fine sim.
//!
//! ROUND 2 of this instrument, after review found the first pass's headline reading was wrong in
//! three ways (see `artifacts/design/STEP0-MEASUREMENTS.md` for the full writeup):
//!
//! (A) The convergence metric was "largest unsatisfied stress on any edge", which is nonzero
//!     forever at any free surface (an empty cell above a filled one always has *some* stress it
//!     cannot act on, and that is what rest looks like, not a failure to converge). Replaced with
//!     the largest REALISED transfer magnitude in a sweep -- mass that has stopped moving.
//! (B) Q1 only tested one pool depth (~200 fine rows once the coarse relax finished compacting
//!     the original ~300-row fill). That depth turned out to be below the bounded law's own
//!     pinning threshold. Added a depth sweep to find where it actually pins.
//! (C) Added a DIRECT solve: for a resting connected coarse component, `eta` is a single scalar
//!     (§0.3), so equilibrium is one monotone root-find per component, not an N-sweep relaxation.
//!     Compared against the iterative relax on the same pool, plus a U-tube case.

use glam::Vec2;
use sandart_sim::physics::{
    cell_capacity_for, cell_potential, overfill_ceiling_for, overfill_equilibrium_transfer,
    GRAVITY_HEAD_SCALE, REFERENCE_GRID_HEIGHT,
};
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, PROP_WETNESS};
use std::time::Instant;

const COARSE: usize = 64;

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1500);
    let stiff: f32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let max_sweeps: usize = std::env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let tol_transfer: f64 = std::env::args().nth(5).and_then(|s| s.parse().ok()).unwrap_or(1e-5);
    let ticks_depth_sweep: usize =
        std::env::args().nth(6).and_then(|s| s.parse().ok()).unwrap_or(300);
    assert!(grid % COARSE == 0, "grid must be a multiple of {COARSE}");
    let t = grid / COARSE;

    let ceil = overfill_ceiling_for(stiff);
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / grid as f32;
    let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * stiff;
    let base_head = 0.04f32 * GRAVITY_HEAD_SCALE; // gravity_dir.y * GRAVITY_HEAD_SCALE
    let overfill_ratio = (ceil - 1.0).max(0.0); // o_max
    let underfill_tension = 1.0f32; // sim default
    let base_head_coarse = base_head * t as f32;
    let unit_coarse = overfill_head_unit as f64; // Q4/direct-solve stiffness, same constant as bounded law

    println!("=== setup ===");
    println!(
        "grid {grid}, coarse {COARSE}x{COARSE} (t={t}), stiffness {stiff}, overfill_capacity {ceil:.4}, o_max {overfill_ratio:.4}"
    );
    println!(
        "base_head {base_head:.4}, base_head_coarse {base_head_coarse:.4}, overfill_head_unit {overfill_head_unit:.2}, underfill_tension {underfill_tension:.3}"
    );
    println!(
        "convergence metric: largest realised mass transfer in one sweep < {tol_transfer} \
         (NOT unsatisfied stress -- see (A) in the file header)"
    );

    let bounded_transfer = |h_a: f32, cap_a: f32, h_b: f32, cap_b: f32, g: f32| -> f32 {
        let cap_a_eff = cap_a * (1.0 + overfill_ratio);
        let cap_b_eff = cap_b * (1.0 + overfill_ratio);
        overfill_equilibrium_transfer(
            h_a, cap_a, h_b, cap_b, cap_a_eff, cap_b_eff, g, 0.0, 0.0, 1.0, 1.0, overfill_ratio,
            overfill_head_unit, underfill_tension,
        )
    };
    let bounded_phi = |h: f32, cap: f32| -> f32 {
        cell_potential(h, cap, overfill_ratio, overfill_head_unit, underfill_tension, 1.0)
    };
    let phi_unbounded = |h: f64, cap: f64| -> f64 {
        if cap <= 0.0 {
            return 0.0;
        }
        let x = h / cap;
        if x <= 1.0 {
            x
        } else {
            x + unit_coarse * (x - 1.0)
        }
    };

    // ================= default pool: depth = 300 fine rows, per §0.1's own example =================
    let default_depth = 300usize;
    let sim = build_square_pool(grid, stiff, ceil, default_depth, ticks);
    let (capacity, a_mass) = restrict(&sim, grid, t);
    let total_fine: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let total_coarse: f64 = a_mass.iter().sum();
    println!(
        "\ndefault pool: depth {default_depth} fine rows, {ticks} ticks. total fine mass {total_fine:.1}, restricted {total_coarse:.1}"
    );

    // ================= Q1: bounded law on the default pool =================
    println!("\n=== Q1: bounded overfill law, default pool (transfer-magnitude convergence) ===");
    let mut m_bounded = a_mass.clone();
    let t0 = Instant::now();
    let (sweeps_q1, last_move_q1) =
        run_relax(&mut m_bounded, &capacity, COARSE, base_head_coarse, bounded_transfer, max_sweeps, tol_transfer, "Q1");
    let q1_time = t0.elapsed();
    println!(
        "stopped after {sweeps_q1} sweeps ({:.3}s), largest realised transfer in final sweep = {last_move_q1:.8} (tol {tol_transfer})",
        q1_time.as_secs_f64()
    );
    report_worst_edge(&m_bounded, &capacity, COARSE, base_head_coarse, bounded_phi, overfill_ratio, "Q1");
    let (max_o_q1, pinned_q1, wet_q1) = o_stats(&m_bounded, &capacity, overfill_ratio);
    let interior_q1 = interior_residual(&m_bounded, &capacity, COARSE, base_head_coarse, bounded_phi);
    println!(
        "max(o) = {max_o_q1:.4} (o_max {overfill_ratio:.4}); {pinned_q1} of {wet_q1} wet tiles >98% of o_max; \
         interior residual (both endpoints over capacity) = {interior_q1:.6}"
    );
    print_column(&m_bounded, &capacity, COARSE, base_head_coarse, bounded_phi, overfill_ratio, "Q1 (bounded, default pool)");

    // ================= Q2: elevation double-count, using the Q1-relaxed field =================
    println!("\n=== Q2: P[D]-P[C] between vertically adjacent coarse tiles vs t*base_head ===");
    let tx_c = COARSE / 2;
    println!("{:>4} {:>4} {:>12} {:>12} {:>10}", "C", "D", "P[D]-P[C]", "t*base_head", "ratio");
    let mut ratios = Vec::new();
    for ty in 0..COARSE - 1 {
        let c = ty * COARSE + tx_c;
        let d = (ty + 1) * COARSE + tx_c;
        if capacity[c] <= 0.0 || capacity[d] <= 0.0 {
            continue;
        }
        let p_c = (bounded_phi(m_bounded[c] as f32, capacity[c] as f32) as f64) - m_bounded[c] / capacity[c];
        let p_d = (bounded_phi(m_bounded[d] as f32, capacity[d] as f32) as f64) - m_bounded[d] / capacity[d];
        if p_c > 1e-6 || p_d > 1e-6 {
            let dp = p_d - p_c;
            let ratio = dp / base_head_coarse as f64;
            println!("{:>4} {:>4} {:>12.4} {:>12.4} {:>10.4}", ty, ty + 1, dp, base_head_coarse, ratio);
            ratios.push(ratio);
        }
    }
    if !ratios.is_empty() {
        let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
        println!("mean ratio over pressurised interior pairs: {mean:.4}");
    }

    // ================= (B) Q1 depth sweep: where does the bounded law pin? =================
    println!("\n=== Q1 depth sweep: where does the bounded law pin? (ticks={ticks_depth_sweep} per depth) ===");
    println!(
        "{:>6} {:>10} {:>8} {:>8} {:>16} {:>10}",
        "depth", "max(o)", "pinned", "wet", "interior_resid", "sweeps"
    );
    for &depth in &[150usize, 200, 250, 300, 400] {
        let sim_d = build_square_pool(grid, stiff, ceil, depth, ticks_depth_sweep);
        let (cap_d, a_d) = restrict(&sim_d, grid, t);
        let mut m = a_d.clone();
        let (sweeps, _last_move) =
            run_relax(&mut m, &cap_d, COARSE, base_head_coarse, bounded_transfer, max_sweeps, tol_transfer, "depth-sweep");
        let (max_o, pinned, wet) = o_stats(&m, &cap_d, overfill_ratio);
        let resid = interior_residual(&m, &cap_d, COARSE, base_head_coarse, bounded_phi);
        println!(
            "{:>6} {:>10.4} {:>8} {:>8} {:>16.6} {:>10}",
            depth, max_o, pinned, wet, resid, sweeps
        );
    }
    // Analytic cross-check, independent of the discrete solver: the linearised equilibrium
    // demand `unit*(o + o^2/o_max) = D*base_head` (§1), solved directly by bisection on `o`.
    println!("\nanalytic demanded o at depth D (D*base_head = unit*(o + o^2/o_max), independent check):");
    for &depth in &[150.0f64, 200.0, 250.0, 300.0, 400.0] {
        let target = depth * base_head as f64;
        let f = |o: f64| overfill_head_unit as f64 * (o + o * o / overfill_ratio as f64) - target;
        let mut lo = 0.0f64;
        let mut hi = overfill_ratio as f64 * 4.0;
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if f(mid) < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let o_demanded = 0.5 * (lo + hi);
        let pins = o_demanded > overfill_ratio as f64;
        println!(
            "  D={depth:>5.0}: demanded o = {o_demanded:.4} ({})",
            if pins { "EXCEEDS o_max -- would pin" } else { "within o_max" }
        );
    }

    // ================= Q3: 1-D chain, transfer-magnitude convergence =================
    println!("\n=== Q3: 1-D coarse chain, sweeps-to-settle vs chain length (transfer-magnitude convergence) ===");
    println!(
        "chain cells: capacity 1.0 each, no gravity head, tension 0. All mass starts in cell 0: \
         M[0] = 0.5*L (avg equilibrium fill 0.5, below capacity)."
    );
    let chain_transfer = |h_a: f32, cap_a: f32, h_b: f32, cap_b: f32, g: f32| -> f32 {
        let cap_a_eff = cap_a * (1.0 + overfill_ratio);
        let cap_b_eff = cap_b * (1.0 + overfill_ratio);
        overfill_equilibrium_transfer(
            h_a, cap_a, h_b, cap_b, cap_a_eff, cap_b_eff, g, 0.0, 0.0, 1.0, 1.0, overfill_ratio,
            overfill_head_unit, 0.0,
        )
    };
    println!("{:>6} {:>16} {:>16}", "L", "sweeps-to-settle", "final transfer");
    let mut q3_pts = Vec::new();
    for &length in &[8usize, 16, 32, 64] {
        let mut m = vec![0.0f64; length];
        let cap = vec![1.0f64; length];
        m[0] = 0.5 * length as f64;
        let (sweeps, last_move) =
            run_relax_1d(&mut m, &cap, length, chain_transfer, max_sweeps, tol_transfer);
        println!("{:>6} {:>16} {:>16.8}", length, sweeps, last_move);
        q3_pts.push((length, sweeps));
    }
    if q3_pts.len() >= 2 {
        let (l0, s0) = q3_pts[0];
        let (l1, s1) = *q3_pts.last().unwrap();
        let exponent = ((s1 as f64).ln() - (s0 as f64).ln()) / ((l1 as f64).ln() - (l0 as f64).ln());
        println!("estimated exponent L={l0}->L={l1}: sweeps ~ L^{exponent:.2}");
    }

    // ================= replaces (C): ticks-to-settle, N sweeps/tick, M persists across ticks =================
    // Not a single continuous relax-to-convergence: groups sweeps into "ticks" of N each, exactly
    // as the real per-tick coupling would (M carried forward, no reset, no anchor/lambda -- that
    // is a separate, later question). Answers "how many ticks does N sweeps/tick actually cost
    // to settle the worst-case (L=64) chain", which is what sizes N for the real build.
    println!("\n=== Ticks-to-settle: L=64 coarse chain, N sweeps/tick, M persists across ticks ===");
    println!(
        "{:>6} {:>16} {:>14} {:>26}",
        "N", "ticks-to-settle", "total sweeps", "per-tick cost / 1 fine sweep@512"
    );
    let length = 64usize;
    for &n_per_tick in &[8usize, 16, 32, 64, 128] {
        let mut m = vec![0.0f64; length];
        let cap = vec![1.0f64; length];
        m[0] = 0.5 * length as f64;
        let mut ticks = 0usize;
        let mut total_sweeps = 0usize;
        let max_ticks = max_sweeps / n_per_tick + 10;
        let mut last_move = f64::MAX;
        loop {
            for _ in 0..n_per_tick {
                last_move = relax_pass_1d(&mut m, &cap, length, chain_transfer);
                total_sweeps += 1;
            }
            ticks += 1;
            if last_move < tol_transfer || ticks >= max_ticks {
                break;
            }
        }
        // Cost model per §8: N sweeps over the FULL 64x64=4096-cell coarse grid (not the 64-cell
        // 1-D toy chain used to measure settling time), against one fine sweep over grid^2 cells.
        let coarse_cells = (COARSE * COARSE) as f64;
        let fine_cells = (grid * grid) as f64;
        let cost_fraction = n_per_tick as f64 * coarse_cells / fine_cells;
        let converged = last_move < tol_transfer;
        println!(
            "{:>6} {:>16} {:>14} {:>26.4}{}",
            n_per_tick, ticks, total_sweeps, cost_fraction,
            if converged { "" } else { "  (NOT settled at cap)" }
        );
    }

    // ================= Q4: unbounded law on the default pool =================
    println!("\n=== Q4: UNBOUNDED law, default pool (transfer-magnitude convergence) ===");
    let mut m_unbounded = a_mass.clone();
    let t0 = Instant::now();
    let (sweeps_q4, last_move_q4) = run_relax_f64(
        &mut m_unbounded, &capacity, COARSE, base_head_coarse as f64,
        |h_a, cap_a, h_b, cap_b, g| solve_unbounded_transfer(h_a, cap_a, h_b, cap_b, g, phi_unbounded),
        max_sweeps, tol_transfer, "Q4",
    );
    let q4_time = t0.elapsed();
    println!(
        "stopped after {sweeps_q4} sweeps ({:.3}s), largest realised transfer in final sweep = {last_move_q4:.8}",
        q4_time.as_secs_f64()
    );
    report_worst_edge_f64(&m_unbounded, &capacity, COARSE, base_head_coarse as f64, phi_unbounded, "Q4");
    let mut max_o_q4 = 0.0f64;
    for c in 0..capacity.len() {
        if capacity[c] > 0.0 {
            let o = (m_unbounded[c] / capacity[c] - 1.0).max(0.0);
            if o > max_o_q4 {
                max_o_q4 = o;
            }
        }
    }
    println!("max(o) = {max_o_q4:.4} (no ceiling to compare against)");
    print_column_f64(&m_unbounded, &capacity, COARSE, base_head_coarse as f64, phi_unbounded, "Q4 (unbounded, default pool)");
}

// ---------------- pool construction / restriction ----------------

fn build_square_pool(grid: usize, stiff: f32, ceil: f32, depth_rows: usize, ticks: usize) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.overfill_pressure = true;
    sim.overfill_stiffness = stiff;
    sim.overfill_capacity = ceil;
    let (w, h) = (grid, grid);
    for v in sim.heightmap.data.iter_mut() {
        *v = 0.0;
    }
    let start_row = h.saturating_sub(depth_rows);
    for y in start_row..h {
        for x in 0..w {
            if sim.shape_mask[y * w + x] != 0 {
                sim.heightmap.data[y * w + x] = 1.0;
            }
        }
    }
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.budget_n = 1024;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
    }
    sim
}

fn restrict(sim: &DrawingSimulation, grid: usize, t: usize) -> (Vec<f64>, Vec<f64>) {
    let w = grid;
    let n_coarse = COARSE * COARSE;
    let mut capacity = vec![0.0f64; n_coarse];
    let mut a_mass = vec![0.0f64; n_coarse];
    for ty in 0..COARSE {
        for tx in 0..COARSE {
            let c = ty * COARSE + tx;
            let mut cap_sum = 0.0f64;
            let mut mass_sum = 0.0f64;
            for dy in 0..t {
                for dx in 0..t {
                    let i = (ty * t + dy) * w + tx * t + dx;
                    if sim.shape_mask[i] == 0 {
                        continue;
                    }
                    cap_sum += cell_capacity_for(sim.cell_props[i * 4 + PROP_WETNESS]) as f64;
                    mass_sum += sim.heightmap.data[i] as f64;
                }
            }
            capacity[c] = cap_sum;
            a_mass[c] = mass_sum;
        }
    }
    (capacity, a_mass)
}

// ---------------- relaxation, returning the largest REALISED transfer (the (A) fix) ----------------

fn relax_pass<F: Fn(f32, f32, f32, f32, f32) -> f32>(
    m: &mut [f64],
    cap: &[f64],
    coarse: usize,
    base_head: f32,
    transfer: F,
) -> f64 {
    let mut max_move = 0.0f64;
    let mut delta = vec![0.0f64; m.len()];
    for ty in 0..coarse - 1 {
        for tx in 0..coarse {
            let a = ty * coarse + tx;
            let b = (ty + 1) * coarse + tx;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let d = transfer(m[a] as f32, cap[a] as f32, m[b] as f32, cap[b] as f32, base_head) as f64;
            max_move = max_move.max(d.abs());
            delta[a] -= d;
            delta[b] += d;
        }
    }
    for i in 0..m.len() {
        m[i] += delta[i];
    }
    let mut delta = vec![0.0f64; m.len()];
    for ty in 0..coarse {
        for tx in 0..coarse - 1 {
            let a = ty * coarse + tx;
            let b = ty * coarse + tx + 1;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let d = transfer(m[a] as f32, cap[a] as f32, m[b] as f32, cap[b] as f32, 0.0) as f64;
            max_move = max_move.max(d.abs());
            delta[a] -= d;
            delta[b] += d;
        }
    }
    for i in 0..m.len() {
        m[i] += delta[i];
    }
    max_move
}

fn run_relax<F: Fn(f32, f32, f32, f32, f32) -> f32>(
    m: &mut [f64],
    cap: &[f64],
    coarse: usize,
    base_head: f32,
    transfer: F,
    max_sweeps: usize,
    tol: f64,
    label: &str,
) -> (usize, f64) {
    let mut sweeps = 0usize;
    let mut last_move = f64::MAX;
    for s in 1..=max_sweeps {
        last_move = relax_pass(m, cap, coarse, base_head, &transfer);
        sweeps = s;
        if s % 1000 == 0 {
            eprintln!("[{label}] sweep {s}, largest transfer = {last_move:.8}");
        }
        if last_move < tol {
            break;
        }
    }
    (sweeps, last_move)
}

fn relax_pass_1d<F: Fn(f32, f32, f32, f32, f32) -> f32>(m: &mut [f64], cap: &[f64], len: usize, transfer: F) -> f64 {
    let mut max_move = 0.0f64;
    let mut delta = vec![0.0f64; len];
    for i in 0..len - 1 {
        let a = i;
        let b = i + 1;
        if cap[a] <= 0.0 || cap[b] <= 0.0 {
            continue;
        }
        let d = transfer(m[a] as f32, cap[a] as f32, m[b] as f32, cap[b] as f32, 0.0) as f64;
        max_move = max_move.max(d.abs());
        delta[a] -= d;
        delta[b] += d;
    }
    for i in 0..len {
        m[i] += delta[i];
    }
    max_move
}

fn run_relax_1d<F: Fn(f32, f32, f32, f32, f32) -> f32>(
    m: &mut [f64],
    cap: &[f64],
    len: usize,
    transfer: F,
    max_sweeps: usize,
    tol: f64,
) -> (usize, f64) {
    let mut sweeps = 0usize;
    let mut last_move = f64::MAX;
    for s in 1..=max_sweeps {
        last_move = relax_pass_1d(m, cap, len, &transfer);
        sweeps = s;
        if last_move < tol {
            break;
        }
    }
    (sweeps, last_move)
}

fn relax_pass_f64<F: Fn(f64, f64, f64, f64, f64) -> f64>(
    m: &mut [f64],
    cap: &[f64],
    coarse: usize,
    base_head: f64,
    transfer: F,
) -> f64 {
    let mut max_move = 0.0f64;
    let mut delta = vec![0.0f64; m.len()];
    for ty in 0..coarse - 1 {
        for tx in 0..coarse {
            let a = ty * coarse + tx;
            let b = (ty + 1) * coarse + tx;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let d = transfer(m[a], cap[a], m[b], cap[b], base_head);
            max_move = max_move.max(d.abs());
            delta[a] -= d;
            delta[b] += d;
        }
    }
    for i in 0..m.len() {
        m[i] += delta[i];
    }
    let mut delta = vec![0.0f64; m.len()];
    for ty in 0..coarse {
        for tx in 0..coarse - 1 {
            let a = ty * coarse + tx;
            let b = ty * coarse + tx + 1;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let d = transfer(m[a], cap[a], m[b], cap[b], 0.0);
            max_move = max_move.max(d.abs());
            delta[a] -= d;
            delta[b] += d;
        }
    }
    for i in 0..m.len() {
        m[i] += delta[i];
    }
    max_move
}

fn run_relax_f64<F: Fn(f64, f64, f64, f64, f64) -> f64>(
    m: &mut [f64],
    cap: &[f64],
    coarse: usize,
    base_head: f64,
    transfer: F,
    max_sweeps: usize,
    tol: f64,
    label: &str,
) -> (usize, f64) {
    let mut sweeps = 0usize;
    let mut last_move = f64::MAX;
    for s in 1..=max_sweeps {
        last_move = relax_pass_f64(m, cap, coarse, base_head, &transfer);
        sweeps = s;
        if s % 1000 == 0 {
            eprintln!("[{label}] sweep {s}, largest transfer = {last_move:.8}");
        }
        if last_move < tol {
            break;
        }
    }
    (sweeps, last_move)
}

/// Restricted to edges where BOTH endpoints are over their own nominal capacity -- the "wet
/// interior", away from any free surface. Answers (B)'s "does the interior satisfy
/// phi_below - phi_above = base_head?" directly.
fn interior_residual<P: Fn(f32, f32) -> f32>(m: &[f64], cap: &[f64], coarse: usize, base_head: f32, phi: P) -> f32 {
    let mut worst = 0.0f32;
    for ty in 0..coarse - 1 {
        for tx in 0..coarse {
            let a = ty * coarse + tx;
            let b = (ty + 1) * coarse + tx;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            if m[a] <= cap[a] || m[b] <= cap[b] {
                continue; // not "wet interior" -- one side is at or below its own nominal capacity
            }
            let s = (phi(m[a] as f32, cap[a] as f32) + base_head - phi(m[b] as f32, cap[b] as f32)).abs();
            worst = worst.max(s);
        }
    }
    worst
}

// ---------------- reporting helpers ----------------

fn report_worst_edge<P: Fn(f32, f32) -> f32>(
    m: &[f64],
    cap: &[f64],
    coarse: usize,
    base_head: f32,
    phi: P,
    o_max: f32,
    label: &str,
) {
    let mut worst = -1.0f32;
    let mut worst_desc = String::new();
    for ty in 0..coarse - 1 {
        for tx in 0..coarse {
            let a = ty * coarse + tx;
            let b = (ty + 1) * coarse + tx;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let s = (phi(m[a] as f32, cap[a] as f32) + base_head - phi(m[b] as f32, cap[b] as f32)).abs();
            if s > worst {
                worst = s;
                let o_a = (m[a] / cap[a] - 1.0).max(0.0);
                let o_b = (m[b] / cap[b] - 1.0).max(0.0);
                worst_desc = format!(
                    "VERTICAL (tx={tx},ty={ty})->({tx},{ty}+1): stress={s:.4} | a: h={:.4} cap={:.4} o={:.4} | b: h={:.4} cap={:.4} o={:.4}",
                    m[a], cap[a], o_a, m[b], cap[b], o_b
                );
            }
        }
    }
    println!(
        "[{label}] worst STRESS edge (expected nonzero at a free surface -- see file header (A)): {worst_desc}"
    );
    let _ = o_max;
}

fn report_worst_edge_f64<P: Fn(f64, f64) -> f64>(
    m: &[f64],
    cap: &[f64],
    coarse: usize,
    base_head: f64,
    phi: P,
    label: &str,
) {
    let mut worst = -1.0f64;
    let mut worst_desc = String::new();
    for ty in 0..coarse - 1 {
        for tx in 0..coarse {
            let a = ty * coarse + tx;
            let b = (ty + 1) * coarse + tx;
            if cap[a] <= 0.0 || cap[b] <= 0.0 {
                continue;
            }
            let s = (phi(m[a], cap[a]) + base_head - phi(m[b], cap[b])).abs();
            if s > worst {
                worst = s;
                worst_desc = format!(
                    "VERTICAL (tx={tx},ty={ty})->({tx},{ty}+1): stress={s:.4} | a: h={:.4} x={:.4} | b: h={:.4} x={:.4}",
                    m[a], m[a] / cap[a], m[b], m[b] / cap[b]
                );
            }
        }
    }
    println!(
        "[{label}] worst STRESS edge (expected nonzero at a free surface -- see file header (A)): {worst_desc}"
    );
}

fn o_stats(m: &[f64], cap: &[f64], o_max: f32) -> (f64, u64, u64) {
    let mut max_o = 0.0f64;
    let mut pinned = 0u64;
    let mut wet = 0u64;
    for i in 0..m.len() {
        if cap[i] <= 0.0 || m[i] <= 1e-6 {
            continue;
        }
        wet += 1;
        let o = (m[i] / cap[i] - 1.0).max(0.0);
        if o > max_o {
            max_o = o;
        }
        if o > 0.98 * o_max as f64 {
            pinned += 1;
        }
    }
    (max_o, pinned, wet)
}

fn print_column<P: Fn(f32, f32) -> f32>(
    m: &[f64],
    cap: &[f64],
    coarse: usize,
    base_head: f32,
    phi: P,
    o_max: f32,
    label: &str,
) {
    println!("\ncentre-column profile: {label}");
    println!("{:>4} {:>10} {:>12} {:>12}", "row", "o", "phi", "eta=phi-y*bhc");
    let tx_c = coarse / 2;
    for ty in 0..coarse {
        let c = ty * coarse + tx_c;
        if cap[c] <= 0.0 {
            println!("{:>4} {:>10} {:>12} {:>12}", ty, "-", "-", "wall");
            continue;
        }
        let o = (m[c] / cap[c] - 1.0).max(0.0).min((o_max as f64) * 10.0);
        let phi_v = phi(m[c] as f32, cap[c] as f32) as f64;
        let eta = phi_v - ty as f64 * base_head as f64;
        println!("{:>4} {:>10.4} {:>12.4} {:>12.4}", ty, o, phi_v, eta);
    }
}

fn print_column_f64<P: Fn(f64, f64) -> f64>(m: &[f64], cap: &[f64], coarse: usize, base_head: f64, phi: P, label: &str) {
    println!("\ncentre-column profile: {label}");
    println!("{:>4} {:>10} {:>12} {:>12}", "row", "o", "phi", "eta=phi-y*bhc");
    let tx_c = coarse / 2;
    for ty in 0..coarse {
        let c = ty * coarse + tx_c;
        if cap[c] <= 0.0 {
            println!("{:>4} {:>10} {:>12} {:>12}", ty, "-", "-", "wall");
            continue;
        }
        let o = (m[c] / cap[c] - 1.0).max(0.0);
        let phi_v = phi(m[c], cap[c]);
        let eta = phi_v - ty as f64 * base_head;
        println!("{:>4} {:>10.4} {:>12.4} {:>12.4}", ty, o, phi_v, eta);
    }
}

/// Solve `phi(h_a - d, cap_a) + gravity_head - phi(h_b + d, cap_b) = 0` for `d`, by bisection, for
/// the UNBOUNDED law. No acceptor ceiling: the only bound on `d` is that neither cell's mass may
/// go negative.
fn solve_unbounded_transfer<PHI: Fn(f64, f64) -> f64>(
    h_a: f64,
    cap_a: f64,
    h_b: f64,
    cap_b: f64,
    gravity_head: f64,
    phi: PHI,
) -> f64 {
    let lo = -h_b;
    let hi = h_a;
    if hi <= lo {
        return 0.0;
    }
    let stress = |d: f64| phi(h_a - d, cap_a) + gravity_head - phi(h_b + d, cap_b);
    if stress(hi) >= 0.0 {
        return hi;
    }
    if stress(lo) <= 0.0 {
        return lo;
    }
    let mut lo = lo;
    let mut hi = hi;
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if stress(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}
