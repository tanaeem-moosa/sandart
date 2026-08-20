//! SMOOTH-ETA.md, measurement 1: the user's reported defect ("I can see block boundaries" on the
//! pressure heat-map) and HIERARCHICAL-PRESSURE.md §0.2's predicted signature ("a sawtooth ...
//! repeating every 8 rows, locked to block boundaries"). Quantifies periodic structure in the
//! FINE fill field at `y % block_size == 0` vs interior rows, down a settled column -- before vs
//! after interpolating `eta` onto every fine cell instead of injecting it as a per-tile constant.
//!
//! Two independent metrics on the compression fraction `o = (h - cap) / cap` sampled down a
//! column (averaged over a small band of columns near the centre, to cut cell-level noise without
//! smoothing out the block-period signal itself -- the band is 5 columns wide, far below the
//! `block_size = 8` period being measured):
//!
//! 1. **Seam-vs-interior mean.** `mean(o at y % block_size == 0) - mean(o at y % block_size != 0)`
//!    over the sampled range. A pure block-boundary artifact shows up as a systematic offset here;
//!    a smooth field does not.
//! 2. **DFT magnitude at the block period.** `|sum_y o(y) * exp(-2pi*i*y/block_size)| / N`, the
//!    Fourier component exactly at the spatial frequency `1/block_size` -- the direct measure of
//!    "periodic structure at y % block_size == 0" the task asks for, robust to whether the offset
//!    in metric 1 happens to average out to near zero over the sampled range while still being
//!    large in magnitude every period.
//!
//! Run: `cargo run --release --example diag_smooth_eta_banding [grid] [ticks]`

use glam::Vec2;
use sandart_sim::physics::cell_capacity_for;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

const COARSE_GRID: usize = 64;

fn make_pool(grid: usize, coupling: bool) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.coarse_pressure_coupling = coupling;
    sim.generate_shape_mask();
    // Prefill most of the box, leaving headroom at the top for a free surface -- a deep column so
    // the interior rows genuinely carry compression for the sawtooth to ride on.
    let start_y = grid / 8;
    for y in start_y..grid {
        for x in 0..grid {
            let idx = y * grid + x;
            if sim.shape_mask[idx] != 0 {
                sim.heightmap.data[idx] = 1.0;
            }
        }
    }
    sim
}

/// `o(y)`, averaged over a `band`-wide strip of columns centred on the sandbox's own centre, for
/// `y` in `[lo, hi)`. Returns `(ys, o_values)`.
fn column_profile(sim: &DrawingSimulation, lo: usize, hi: usize, band: usize) -> (Vec<usize>, Vec<f64>) {
    let (w, _h) = (sim.heightmap.width, sim.heightmap.height);
    let cx0 = w / 2 - band / 2;
    let mut ys = Vec::new();
    let mut os = Vec::new();
    for y in lo..hi {
        let mut sum_o = 0.0f64;
        let mut n = 0u64;
        for x in cx0..(cx0 + band) {
            let i = y * w + x;
            if sim.shape_mask[i] == 0 {
                continue;
            }
            let cap = cell_capacity_for(sim.cell_props[i * 4]) as f64;
            let h = sim.heightmap.data[i] as f64;
            if h <= 1e-4 {
                continue;
            }
            sum_o += (h / cap - 1.0).max(0.0);
            n += 1;
        }
        if n > 0 {
            ys.push(y);
            os.push(sum_o / n as f64);
        }
    }
    (ys, os)
}

fn seam_vs_interior(ys: &[usize], os: &[f64], block_size: usize) -> (f64, f64, f64) {
    let (mut seam_sum, mut seam_n, mut int_sum, mut int_n) = (0.0f64, 0u64, 0.0f64, 0u64);
    for (&y, &o) in ys.iter().zip(os.iter()) {
        if y % block_size == 0 {
            seam_sum += o;
            seam_n += 1;
        } else {
            int_sum += o;
            int_n += 1;
        }
    }
    let seam_mean = if seam_n > 0 { seam_sum / seam_n as f64 } else { f64::NAN };
    let int_mean = if int_n > 0 { int_sum / int_n as f64 } else { f64::NAN };
    (seam_mean, int_mean, seam_mean - int_mean)
}

/// DFT magnitude of `os` at spatial frequency `1 / block_size` cycles per row, normalised by the
/// number of samples. `ys` need not be contiguous (gaps near the free surface are fine); the
/// transform uses each sample's real row index, not its position in the array, so the frequency
/// axis stays in physical units regardless of which rows got dropped.
fn dft_at_block_period(ys: &[usize], os: &[f64], block_size: usize) -> f64 {
    if ys.is_empty() {
        return f64::NAN;
    }
    let freq = 1.0 / block_size as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (&y, &o) in ys.iter().zip(os.iter()) {
        let theta = -2.0 * std::f64::consts::PI * freq * y as f64;
        re += o * theta.cos();
        im += o * theta.sin();
    }
    (re * re + im * im).sqrt() / ys.len() as f64
}

fn run(grid: usize, ticks: usize, coupling: bool, label: &str) {
    let mut sim = make_pool(grid, coupling);
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.budget_n = 1024;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 0.0, 16.6);
    }
    let block_size = grid / COARSE_GRID;
    let start_y = grid / 8;
    // Sample well inside the bulk: skip a margin below the free surface (transient, not yet
    // compressed) and above the floor (its own boundary condition), same reasoning
    // `diag_coarse_ab`'s `surface_spread` uses for excluding edge effects.
    let lo = start_y + 30;
    let hi = grid.saturating_sub(10);
    let (ys, os) = column_profile(&sim, lo, hi, 5);
    let (seam_mean, int_mean, offset) = seam_vs_interior(&ys, &os, block_size);
    let dft = dft_at_block_period(&ys, &os, block_size);
    let mean_o: f64 = if !os.is_empty() { os.iter().sum::<f64>() / os.len() as f64 } else { f64::NAN };
    println!(
        "{label:<28} block_size={block_size} rows_sampled={:<4} mean(o)={mean_o:.6}  seam_mean(o)={seam_mean:.6}  interior_mean(o)={int_mean:.6}  seam-interior={offset:.6}  dft@1/{block_size}={dft:.6}",
        ys.len()
    );
}

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(800);
    println!("=== SMOOTH-ETA.md measurement 1: block-boundary banding, grid {grid}, {ticks} ticks ===");
    run(grid, ticks, false, "coupling OFF");
    run(grid, ticks, true, "coupling ON");
}
