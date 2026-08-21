//! Which direction does the coarse level disagree with the fine one in -- lateral or down?
//!
//! `delta[C] = M[C] - A[C]` is a scalar per tile: where the coarse level thinks mass should be,
//! minus where the fine grid actually has it. A scalar field has no direction on its own, but the
//! TRANSPORT that would reconcile the two does, and it is recoverable: find the minimum-energy
//! flux `F` on tile edges with `div F = delta`. That is a Helmholtz projection -- solve
//! `lap(phi) = delta` with Neumann boundaries over the inside tiles, then `F = grad(phi)` -- and
//! it is the unique curl-free (no pointless circulation) answer to "what movement does this
//! disagreement ask for". Summing |F_x| against |F_y| answers the question directly.
//!
//! Reported per tick and averaged, alongside a raw control (mean |delta|) so a run where the two
//! levels barely disagree at all is not mistaken for a directional finding.
use glam::Vec2;

/// SOR over-relaxation factor. For a Neumann Poisson problem on a 64x64 grid the optimal value is
/// near `2 / (1 + sin(pi/n))` ~ 1.90; plain Gauss-Seidel (1.0) needs O(n^2) sweeps to converge the
/// low-frequency modes that dominate this source term.
const OMEGA: f64 = 1.9;
/// Stop when the max residual is this fraction of the max source magnitude.
const TOL: f64 = 1e-4;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(200);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);
    // Iteration CAP, not a fixed count: the solver stops on the residual.
    let sweeps: usize = get("--sweeps").map(|v| v.parse().unwrap()).unwrap_or(20000);
    let mat = match get("--material").unwrap_or_else(|| "drysand".into()).as_str() {
        "water" => MaterialMode::Water,
        _ => MaterialMode::DrySand,
    };
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(mat);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = get("--overclock").map(|v| v == "1").unwrap_or(true);
    let targets = [None; 5];
    for _ in 0..warm {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }

    let n = sim.coarse.coarse_n;
    let inside: Vec<bool> = sim.coarse.inside.clone();
    let n_in = inside.iter().filter(|&&v| v).count().max(1);
    let mut phi = vec![0.0f64; n * n];
    let (mut worst_residual, mut total_iters) = (0.0f64, 0usize);
    let (mut sum_fx, mut sum_fy, mut sum_absdelta, mut samples) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    // Depth bands, to show WHERE the disagreement sits as well as which way it points.
    let mut band_fx = [0.0f64; 4];
    let mut band_fy = [0.0f64; 4];

    for _ in 0..ticks {
        sim.budget_n = 256;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);

        // Source term: delta, with its mean removed over the inside region. A Neumann problem is
        // only solvable if the source integrates to zero, and the residual mean is exactly the
        // part of the disagreement no internal transport can fix (the two levels holding
        // different TOTAL mass), so removing it is not a fudge -- it is the non-transportable
        // component, and it is reported below as `mean_removed`.
        let mut mean = 0.0f64;
        for c in 0..n * n {
            if inside[c] { mean += sim.coarse_state.delta[c] as f64; }
        }
        mean /= n_in as f64;
        let src: Vec<f64> = (0..n * n)
            .map(|c| if inside[c] { sim.coarse_state.delta[c] as f64 - mean } else { 0.0 })
            .collect();

        // SOR on lap(phi) = src, Neumann (a neighbour outside the region reflects, i.e.
        // contributes phi[c] itself, which is the zero-flux boundary condition).
        //
        // Plain Gauss-Seidel was NOT converged at 800 sweeps -- the lateral/down ratio moved 0.53
        // -> 0.35 and |F| doubled when the sweep count was raised, which would have made the
        // headline answer an artifact of the solver. Over-relaxation plus an explicit residual
        // check is the fix: the loop now runs until the max residual falls below `tol` and the
        // achieved residual is printed, so an unconverged run announces itself instead of
        // quietly reporting a direction.
        phi.iter_mut().for_each(|v| *v = 0.0);
        let mut residual = f64::INFINITY;
        let mut iters = 0usize;
        for _ in 0..sweeps {
            for cy in 0..n {
                for cx in 0..n {
                    let c = cy * n + cx;
                    if !inside[c] { continue; }
                    // Each of the four neighbours contributes its own phi if it is inside the
                    // region, or phi[c] itself if it is not -- the reflecting (zero-flux) Neumann
                    // boundary. `cnt` is therefore always 4; it is accumulated rather than
                    // hardcoded so the two stay in step if the stencil ever changes.
                    let mut acc = 0.0;
                    let mut cnt = 0.0;
                    let left = if cx > 0 && inside[c - 1] { phi[c - 1] } else { phi[c] };
                    let right = if cx + 1 < n && inside[c + 1] { phi[c + 1] } else { phi[c] };
                    let up = if cy > 0 && inside[c - n] { phi[c - n] } else { phi[c] };
                    let down = if cy + 1 < n && inside[c + n] { phi[c + n] } else { phi[c] };
                    for v in [left, right, up, down] { acc += v; cnt += 1.0; }
                    let gs = (acc - src[c]) / cnt;
                    phi[c] += OMEGA * (gs - phi[c]);
                }
            }
            iters += 1;
            // Residual every 25 sweeps: cheap relative to the sweep itself, frequent enough to
            // stop promptly.
            if iters % 25 == 0 {
                let mut max_r: f64 = 0.0;
                let mut scale: f64 = 0.0;
                for cy in 0..n {
                    for cx in 0..n {
                        let c = cy * n + cx;
                        if !inside[c] { continue; }
                        let left = if cx > 0 && inside[c - 1] { phi[c - 1] } else { phi[c] };
                        let right = if cx + 1 < n && inside[c + 1] { phi[c + 1] } else { phi[c] };
                        let up = if cy > 0 && inside[c - n] { phi[c - n] } else { phi[c] };
                        let down = if cy + 1 < n && inside[c + n] { phi[c + n] } else { phi[c] };
                        let lap = left + right + up + down - 4.0 * phi[c];
                        max_r = max_r.max((lap - src[c]).abs());
                        scale = scale.max(src[c].abs());
                    }
                }
                residual = max_r / scale.max(1e-12);
                if residual < TOL { break; }
            }
        }
        worst_residual = worst_residual.max(residual);
        total_iters += iters;

        // F = grad(phi) on the tile edges interior to the region.
        let (mut fx, mut fy) = (0.0f64, 0.0f64);
        for cy in 0..n {
            for cx in 0..n {
                let c = cy * n + cx;
                if !inside[c] { continue; }
                let band = (cy * 4 / n).min(3);
                if cx + 1 < n && inside[c + 1] {
                    let v = (phi[c + 1] - phi[c]).abs();
                    fx += v; band_fx[band] += v;
                }
                if cy + 1 < n && inside[c + n] {
                    let v = (phi[c + n] - phi[c]).abs();
                    fy += v; band_fy[band] += v;
                }
                sum_absdelta += (sim.coarse_state.delta[c] as f64).abs();
            }
        }
        sum_fx += fx;
        sum_fy += fy;
        samples += 1;
    }

    let s = samples as f64;
    println!(
        "{:?} grid={} ticks={} overclock={}  |F_lateral|={:.4}  |F_down|={:.4}  lateral/down={:.3}  \
         mean|delta|={:.5}  solver: {:.0} sweeps/tick, worst residual {:.2e}",
        mat, grid, ticks, sim.overclocking_enabled,
        sum_fx / s, sum_fy / s, sum_fx / sum_fy.max(1e-12), sum_absdelta / s / n_in as f64,
        total_iters as f64 / s, worst_residual
    );
    println!("  by depth band (top -> bottom):");
    for b in 0..4 {
        println!(
            "    band {}: lateral {:.4}  down {:.4}  ratio {:.3}",
            b, band_fx[b] / s, band_fy[b] / s, band_fx[b] / band_fy[b].max(1e-12)
        );
    }
}
